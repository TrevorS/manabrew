use super::*;

use forge_foundation::{ManaCost, ZoneType};

use crate::ability::activated::ActivatedAbility;

#[derive(Clone, Copy)]
pub(crate) struct ManaPaymentSession<'a> {
    pub player: PlayerId,
    pub card_id: CardId,
    pub card_name: &'a str,
    pub mana_cost: &'a ManaCost,
    pub cost_str: &'a str,
    pub cost_display_str: &'a str,
    pub cost_checkpoint_str: &'a str,
    pub is_activated_ability: bool,
    pub reserved_sacrifices: &'a [CardId],
    pub current_spell: Option<CardId>,
    pub allow_reserved_source_reuse: bool,
    pub payment_ctx: Option<&'a mana::ManaPaymentContext>,
}

#[derive(Clone, Copy)]
pub(crate) struct ManaPaymentResult {
    pub paid: bool,
    pub preserve_taps_on_failure: bool,
}

impl ManaPaymentResult {
    fn paid() -> Self {
        Self {
            paid: true,
            preserve_taps_on_failure: false,
        }
    }

    fn failed() -> Self {
        Self {
            paid: false,
            preserve_taps_on_failure: false,
        }
    }

    fn failed_preserving_taps() -> Self {
        Self {
            paid: false,
            preserve_taps_on_failure: true,
        }
    }
}

fn notify_mana_payment_resolved(
    agents: &mut [Box<dyn PlayerAgent>],
    player: PlayerId,
    actions: &[ManaCostAction],
) {
    let notification = crate::agent::notification::GameNotification::ManaPaymentResolved {
        player,
        actions: actions.to_vec(),
    };
    for agent in agents.iter_mut() {
        agent.notify(notification.clone());
    }
}

pub(crate) trait ManaPaymentHost {
    type UndoRecord;

    fn parts(&mut self) -> (&mut GameState, &mut [Box<dyn PlayerAgent>], &mut [ManaPool]);
    fn auto_pay(&mut self, session: ManaPaymentSession<'_>) -> Option<Vec<ManaCostAction>>;
    fn try_pay_from_pool(&mut self, player: PlayerId) -> bool;
    fn resolve_mana_ability(
        &mut self,
        player: PlayerId,
        card_id: CardId,
        ab: &ActivatedAbility,
        express_choice: Option<u16>,
    ) -> bool;
    fn on_basic_land_tap(&mut self, player: PlayerId, land_id: CardId);
    fn undoable_mana_sources(&self, player: PlayerId) -> Vec<CardId>;
    fn begin_mana_undo(&mut self, player: PlayerId, source: CardId) -> Self::UndoRecord;
    fn finish_mana_undo(&mut self, record: Self::UndoRecord, produced_count: usize);
    fn undo_mana_action(&mut self, player: PlayerId, source: CardId) -> bool;
}

pub(crate) fn pay_mana_cost_session_generic<H, FAvail>(
    host: &mut H,
    session: ManaPaymentSession<'_>,
    mana_ability_available: FAvail,
) -> ManaPaymentResult
where
    H: ManaPaymentHost,
    FAvail: Fn(&GameState, PlayerId, CardId, &ActivatedAbility, &[CardId]) -> bool,
{
    let saved_pool = host.parts().2[session.player.index()].clone();
    let mut mana_loop_invalid_count = 0u32;
    let mut executed_actions: Vec<ManaCostAction> = Vec::new();

    loop {
        let untappable_lands = host.undoable_mana_sources(session.player);
        let (game, agents, mana_pools) = host.parts();
        let mana_sources =
            mana::collect_mana_payment_sources(game, session.player, session.reserved_sacrifices);
        let lands = mana_sources.source_cards.clone();
        let convoke_sources = convoke_payment_sources(game, session.player, session.card_id);
        let mut tappable_lands = lands;
        tappable_lands.extend(convoke_sources.iter().copied());
        let mana_ability_options = mana_sources.mana_ability_options;
        let pool_ref = mana_pools[session.player.index()].clone();
        let can_confirm_from_pool = {
            let mut confirm_pool = pool_ref.clone();
            confirm_pool.source_colors = None;
            confirm_pool.total_sources = None;
            confirm_pool.try_pay(session.mana_cost)
        };

        agents[session.player.index()].snapshot_state(game, mana_pools);
        agents[session.player.index()].observe_mana_payment(
            game,
            &pool_ref,
            session.player,
            session.card_id,
            session.mana_cost,
            session.current_spell,
            session.allow_reserved_source_reuse,
            session.reserved_sacrifices,
            session.payment_ctx,
        );
        let action = agents[session.player.index()].pay_mana_cost(
            session.player,
            session.card_id,
            session.card_name,
            session.cost_str,
            session.cost_display_str,
            session.cost_checkpoint_str,
            can_confirm_from_pool,
            session.is_activated_ability,
            session.reserved_sacrifices,
            &mana_ability_options,
            &tappable_lands,
            &untappable_lands,
            &pool_ref,
        );

        match action {
            ManaCostAction::TapForMana {
                card_id: land_id,
                mana_ability_index,
                express_choice,
            } => {
                if convoke_sources.contains(&land_id) {
                    mana_loop_invalid_count = 0;
                    let player_idx = session.player.index();
                    let atom = express_choice
                        .filter(|&a| a != 0)
                        .unwrap_or_else(|| convoke_atom(game, land_id, session.mana_cost));
                    game.tap(land_id);
                    mana_pools[player_idx].add(atom, 1);
                    game.card_mut(land_id).last_mana_produced = Some(vec![atom]);
                    executed_actions.push(ManaCostAction::TapForMana {
                        card_id: land_id,
                        mana_ability_index: None,
                        express_choice: Some(atom),
                    });
                    continue;
                }
                if !tappable_lands.contains(&land_id) {
                    mana_loop_invalid_count += 1;
                    if mana_loop_invalid_count > 3 {
                        mana_pools[session.player.index()] = saved_pool.clone();
                        return ManaPaymentResult::failed();
                    }
                    continue;
                }
                mana_loop_invalid_count = 0;
                float_mana_from_source(
                    host,
                    session,
                    land_id,
                    mana_ability_index,
                    express_choice,
                    &mana_ability_available,
                    &mut executed_actions,
                    false,
                );
            }
            ManaCostAction::Untap(land_id) => {
                if !untappable_lands.contains(&land_id) {
                    continue;
                }
                if host.undo_mana_action(session.player, land_id) {
                    executed_actions.push(ManaCostAction::Untap(land_id));
                }
            }
            ManaCostAction::Pay { auto } => {
                let floats_mana = auto && agents[session.player.index()].auto_pay_floats_mana();
                if auto && !floats_mana {
                    let auto_trace = host.auto_pay(session);
                    let (_, agents, mana_pools) = host.parts();
                    if let Some(mut auto_trace) = auto_trace {
                        let attempted_and_failed =
                            matches!(auto_trace.last(), Some(ManaCostAction::AttemptedAndFailed));
                        executed_actions.append(&mut auto_trace);
                        if attempted_and_failed {
                            notify_mana_payment_resolved(agents, session.player, &executed_actions);
                            return ManaPaymentResult::failed();
                        }
                        executed_actions.push(ManaCostAction::Pay { auto: false });
                        notify_mana_payment_resolved(agents, session.player, &executed_actions);
                        return ManaPaymentResult::paid();
                    }
                    executed_actions.push(ManaCostAction::AttemptedAndFailed);
                    notify_mana_payment_resolved(agents, session.player, &executed_actions);
                    mana_pools[session.player.index()] = saved_pool.clone();
                    return ManaPaymentResult::failed();
                }

                let floated = floats_mana
                    && float_mana_for_cost(
                        host,
                        session,
                        &mana_ability_available,
                        &mut executed_actions,
                    );
                let paid_from_pool = host.try_pay_from_pool(session.player);
                let (_, agents, mana_pools) = host.parts();
                if paid_from_pool {
                    executed_actions.push(ManaCostAction::Pay { auto: false });
                    notify_mana_payment_resolved(agents, session.player, &executed_actions);
                    return ManaPaymentResult::paid();
                }
                if floated {
                    mana_loop_invalid_count = 0;
                    continue;
                }

                mana_loop_invalid_count += 1;
                if mana_loop_invalid_count > 3 {
                    executed_actions.push(ManaCostAction::AttemptedAndFailed);
                    notify_mana_payment_resolved(agents, session.player, &executed_actions);
                    mana_pools[session.player.index()] = saved_pool.clone();
                    return ManaPaymentResult::failed();
                }
            }
            ManaCostAction::AttemptedAndFailed => {
                executed_actions.push(ManaCostAction::AttemptedAndFailed);
                notify_mana_payment_resolved(agents, session.player, &executed_actions);
                mana_pools[session.player.index()] = saved_pool.clone();
                if agents[session.player.index()].auto_pay_floats_mana() {
                    return ManaPaymentResult::failed();
                }
                return ManaPaymentResult::failed_preserving_taps();
            }
        }
    }
}

// Keep in sync with the host's `AutoPay.floatManaForCost` (forge-harness common/AutoPay.java).
fn float_mana_for_cost<H, FAvail>(
    host: &mut H,
    session: ManaPaymentSession<'_>,
    mana_ability_available: &FAvail,
    executed_actions: &mut Vec<ManaCostAction>,
) -> bool
where
    H: ManaPaymentHost,
    FAvail: Fn(&GameState, PlayerId, CardId, &ActivatedAbility, &[CardId]) -> bool,
{
    let mut floated = false;
    for _ in 0..128 {
        let (game, _, mana_pools) = host.parts();
        let Some(choice) = mana::next_auto_float_choice(
            game,
            &mana_pools[session.player.index()],
            session.player,
            session.mana_cost,
            session.current_spell,
            session.allow_reserved_source_reuse,
            session.reserved_sacrifices,
            session.payment_ctx,
        ) else {
            break;
        };
        if !float_mana_from_source(
            host,
            session,
            choice.card_id,
            choice.mana_ability_index,
            choice.needs_express_choice.then_some(choice.chosen_atom),
            mana_ability_available,
            executed_actions,
            true,
        ) {
            break;
        }
        floated = true;
    }
    floated
}

#[allow(clippy::too_many_arguments)]
fn float_mana_from_source<H, FAvail>(
    host: &mut H,
    session: ManaPaymentSession<'_>,
    land_id: CardId,
    mana_ability_index: Option<usize>,
    express_choice: Option<u16>,
    mana_ability_available: &FAvail,
    executed_actions: &mut Vec<ManaCostAction>,
    keep_undo_on_decline: bool,
) -> bool
where
    H: ManaPaymentHost,
    FAvail: Fn(&GameState, PlayerId, CardId, &ActivatedAbility, &[CardId]) -> bool,
{
    let (game, _, _) = host.parts();
    let mana_ab = {
        let c = game.card(land_id);
        mana_ability_index
            .and_then(|idx| c.activated_abilities.get(idx))
            .filter(|ab| {
                ab.is_mana_ability
                    && mana_ability_available(
                        game,
                        session.player,
                        land_id,
                        ab,
                        session.reserved_sacrifices,
                    )
            })
            .cloned()
            .or_else(|| {
                c.activated_abilities
                    .iter()
                    .find(|ab| {
                        ab.is_mana_ability
                            && mana_ability_available(
                                game,
                                session.player,
                                land_id,
                                ab,
                                session.reserved_sacrifices,
                            )
                    })
                    .cloned()
            })
    };
    if let Some(ab) = mana_ab {
        // Snapshot BEFORE the ability produces mana so we can
        // capture everything this tap adds to the pool —
        // base production, aura-granted mana, doublers, and
        // TapsForMana trigger payloads — in a single diff.
        // Without this, the untap path below only knows about
        // the land's native atoms and leaves the aura-added
        // mana orphaned in the pool.
        let player_idx = session.player.index();
        let undo_record = host.begin_mana_undo(session.player, land_id);
        let pool_snapshot = host.parts().2[player_idx].begin_tap_tracking();
        let resolved = host.resolve_mana_ability(session.player, land_id, &ab, express_choice);
        let (game, _, mana_pools) = host.parts();
        let produced = mana_pools[player_idx].end_tap_tracking(&pool_snapshot);
        let produced_count = produced.len();
        if resolved {
            executed_actions.push(ManaCostAction::TapForMana {
                card_id: land_id,
                mana_ability_index: Some(mana_ability_index.unwrap_or(0)),
                express_choice,
            });
        }
        if resolved && !produced.is_empty() {
            game.card_mut(land_id).last_mana_produced = Some(produced);
        }
        if resolved || !keep_undo_on_decline {
            host.finish_mana_undo(undo_record, produced_count);
        }
        resolved && produced_count > 0
    } else if let Some(atom) = basic_land_mana_atom(game.card(land_id)) {
        executed_actions.push(ManaCostAction::TapForMana {
            card_id: land_id,
            mana_ability_index: Some(0),
            express_choice: None,
        });
        let player_idx = session.player.index();
        let undo_record = host.begin_mana_undo(session.player, land_id);
        let (game, agents, mana_pools) = host.parts();
        let pool_snapshot = mana_pools[player_idx].begin_tap_tracking();
        game.tap(land_id);
        // Fire ProduceMana replacement (e.g. Nyxbloom Ancient triples mana)
        // before adding to pool. Mirrors Java AbilityManaPart.produceMana
        // which always invokes ReplacementHandler.run(ProduceMana, ...)
        // even for the implicit basic-land tap (every land has an
        // intrinsic AbilityManaPart in Forge's CardFactoryUtil).
        let mana_letter = crate::mana::ManaPool::atom_to_letter(atom).to_string();
        let mut event = crate::replacement::replacement_handler::ReplacementEvent::ProduceMana {
            source: land_id,
            activator: session.player,
            mana: mana_letter.clone(),
        };
        let result = crate::replacement::replacement_handler::apply_replacements_with_agents(
            game, agents, &mut event,
        );
        let final_mana = if result == crate::replacement::ReplacementResult::Updated {
            if let crate::replacement::replacement_handler::ReplacementEvent::ProduceMana {
                mana,
                ..
            } = event
            {
                mana
            } else {
                mana_letter
            }
        } else {
            mana_letter
        };
        for token in final_mana.split_whitespace() {
            if let Some(produced_atom) = crate::mana::mana_atom_from_produced(token) {
                mana_pools[player_idx].add(produced_atom, 1);
            }
        }
        host.on_basic_land_tap(session.player, land_id);
        let (game, _, mana_pools) = host.parts();
        let produced = mana_pools[player_idx].end_tap_tracking(&pool_snapshot);
        let produced_count = produced.len();
        if !produced.is_empty() {
            game.card_mut(land_id).last_mana_produced = Some(produced);
        }
        host.finish_mana_undo(undo_record, produced_count);
        produced_count > 0
    } else {
        false
    }
}

impl GameLoop {
    pub(crate) fn make_mana_payment_callback<'a, 'r: 'a>(
        runtime: &'a mut crate::replacement::replacement_handler::ReplacementRuntime<'r>,
        agents: &'a mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
    ) -> impl FnMut(mana::ManaPayCallback<'_>) -> Option<CardId> + use<'a, 'r> {
        move |kind: mana::ManaPayCallback<'_>| -> Option<CardId> {
            match kind {
                mana::ManaPayCallback::ChooseSacrifice(valid) => {
                    agents[player.index()].choose_sacrifice(player, valid, Some(source))
                }
                mana::ManaPayCallback::ChooseColor(valid_colors) => {
                    // Always invoke the agent — humans go through their
                    // interactive `ChooseColor` modal, AI returns a
                    // default. The engine never branches on agent kind.
                    let _ = agents[player.index()].choose_color(player, valid_colors);
                    None
                }
                mana::ManaPayCallback::ChooseManaColor { options, chosen } => {
                    *chosen = agents[player.index()].choose_color(player, options);
                    None
                }
                mana::ManaPayCallback::ChooseManaFromPool {
                    mana_choices,
                    chosen,
                } => {
                    *chosen = agents[player.index()].choose_mana_from_pool(player, mana_choices);
                    None
                }
                mana::ManaPayCallback::ChooseCards {
                    valid,
                    min,
                    max,
                    chosen,
                } => {
                    chosen.extend(
                        agents[player.index()].choose_cards_for_effect(player, valid, min, max),
                    );
                    chosen.first().copied()
                }
                mana::ManaPayCallback::ConfirmSelfSacrifice(source_id) => {
                    if agents[player.index()].confirm_payment(
                        player,
                        "Sacrifice",
                        "Sacrifice for mana",
                        Some(source_id),
                        Some(crate::ability::api_type::ApiType::Mana),
                    ) {
                        Some(source_id)
                    } else {
                        None
                    }
                }
                mana::ManaPayCallback::ConfirmSubCounter(source_id) => {
                    if agents[player.index()].confirm_payment(
                        player,
                        "SubCounter",
                        "Remove counter for mana",
                        Some(source_id),
                        Some(crate::ability::api_type::ApiType::Mana),
                    ) {
                        Some(source_id)
                    } else {
                        None
                    }
                }
                mana::ManaPayCallback::ConfirmSourceExile(source_id) => {
                    if agents[player.index()].confirm_payment(
                        player,
                        "Exile",
                        "Exile for mana",
                        Some(source_id),
                        Some(crate::ability::api_type::ApiType::Mana),
                    ) {
                        Some(source_id)
                    } else {
                        None
                    }
                }
                mana::ManaPayCallback::ConfirmPayLife(source_id) => {
                    if agents[player.index()].confirm_payment(
                        player,
                        "PayLife",
                        "Pay life for mana",
                        Some(source_id),
                        Some(crate::ability::api_type::ApiType::Mana),
                    ) {
                        Some(source_id)
                    } else {
                        None
                    }
                }
                mana::ManaPayCallback::NotifySacrificeForMana(game, sacrificed_id) => {
                    perform_sacrifice(game, runtime, agents, &[sacrificed_id]);
                    Some(sacrificed_id)
                }
                mana::ManaPayCallback::ExileCostCardsForMana {
                    game,
                    player,
                    cards,
                    collect_evidence,
                } => {
                    exile_cost_cards(game, runtime, agents, player, cards, collect_evidence);
                    cards.first().copied()
                }
                mana::ManaPayCallback::ApplyProduceManaReplacement {
                    game,
                    activator,
                    source_card,
                    mana,
                } => {
                    let mut event =
                        crate::replacement::replacement_handler::ReplacementEvent::ProduceMana {
                            source: source_card,
                            activator,
                            mana: mana.clone(),
                        };
                    let result =
                        crate::replacement::replacement_handler::apply_replacements_with_agents(
                            game, agents, &mut event,
                        );
                    if result == crate::replacement::ReplacementResult::Updated {
                        if let crate::replacement::replacement_handler::ReplacementEvent::ProduceMana {
                            mana: new_mana,
                            ..
                        } = event
                        {
                            *mana = new_mana;
                        }
                    }
                    None
                }
            }
        }
    }

    pub(crate) fn pay_mana_cost_session<FAvail, FAuto, FTryPay>(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        session: ManaPaymentSession<'_>,
        mana_ability_available: FAvail,
        auto_pay: FAuto,
        try_pay_from_pool: FTryPay,
    ) -> ManaPaymentResult
    where
        FAvail: Fn(&GameState, PlayerId, CardId, &ActivatedAbility, &[CardId]) -> bool,
        FAuto: FnMut(
            &mut GameLoop,
            &mut GameState,
            &mut [Box<dyn PlayerAgent>],
            ManaPaymentSession<'_>,
        ) -> Option<Vec<ManaCostAction>>,
        FTryPay: FnMut(&mut GameLoop, &mut GameState, PlayerId) -> bool,
    {
        let mut host = GameLoopManaPayment {
            game_loop: self,
            game,
            agents,
            auto_pay,
            try_pay_from_pool,
        };
        let paid = pay_mana_cost_session_generic(&mut host, session, mana_ability_available);
        self.invalidate_mana_undo_for_player(session.player);
        paid
    }
}

struct GameLoopManaPayment<'a, FAuto, FTryPay> {
    game_loop: &'a mut GameLoop,
    game: &'a mut GameState,
    agents: &'a mut [Box<dyn PlayerAgent>],
    auto_pay: FAuto,
    try_pay_from_pool: FTryPay,
}

impl<FAuto, FTryPay> ManaPaymentHost for GameLoopManaPayment<'_, FAuto, FTryPay>
where
    FAuto: FnMut(
        &mut GameLoop,
        &mut GameState,
        &mut [Box<dyn PlayerAgent>],
        ManaPaymentSession<'_>,
    ) -> Option<Vec<ManaCostAction>>,
    FTryPay: FnMut(&mut GameLoop, &mut GameState, PlayerId) -> bool,
{
    type UndoRecord = super::mana_action_undo::ManaUndoRecord;

    fn parts(&mut self) -> (&mut GameState, &mut [Box<dyn PlayerAgent>], &mut [ManaPool]) {
        (self.game, self.agents, &mut self.game_loop.mana_pools)
    }

    fn auto_pay(&mut self, session: ManaPaymentSession<'_>) -> Option<Vec<ManaCostAction>> {
        (self.auto_pay)(self.game_loop, self.game, self.agents, session)
    }

    fn try_pay_from_pool(&mut self, player: PlayerId) -> bool {
        (self.try_pay_from_pool)(self.game_loop, self.game, player)
    }

    fn resolve_mana_ability(
        &mut self,
        player: PlayerId,
        card_id: CardId,
        ab: &ActivatedAbility,
        express_choice: Option<u16>,
    ) -> bool {
        self.game_loop.resolve_mana_ability(
            self.game,
            self.agents,
            player,
            card_id,
            ab,
            express_choice,
        )
    }

    fn on_basic_land_tap(&mut self, player: PlayerId, land_id: CardId) {
        let this = &mut *self.game_loop;
        this.trigger_handler.run_trigger(
            TriggerType::TapsForMana,
            RunParams {
                card: Some(land_id),
                player: Some(player),
                ..Default::default()
            },
            false,
        );
        this.trigger_handler.run_trigger(
            TriggerType::ManaAdded,
            RunParams {
                card: Some(land_id),
                player: Some(player),
                activator: Some(player),
                ..Default::default()
            },
            false,
        );
        let pending = this.trigger_handler.run_waiting_triggers(self.game);
        if !pending.is_empty() {
            this.mark_mana_undo_disqualified();
        }
        for pt in pending {
            this.resolve_single_effect(self.game, self.agents, &pt.entry.spell_ability, None);
        }
    }

    fn undoable_mana_sources(&self, player: PlayerId) -> Vec<CardId> {
        self.game_loop.undoable_mana_sources(player)
    }

    fn begin_mana_undo(&mut self, player: PlayerId, source: CardId) -> Self::UndoRecord {
        self.game_loop
            .begin_mana_undo_action(self.game, player, source)
    }

    fn finish_mana_undo(&mut self, record: Self::UndoRecord, produced_count: usize) {
        self.game_loop
            .finish_mana_undo_action(record, produced_count);
    }

    fn undo_mana_action(&mut self, player: PlayerId, source: CardId) -> bool {
        self.game_loop.undo_mana_action(self.game, player, source)
    }
}

fn convoke_payment_sources(game: &GameState, player: PlayerId, spell: CardId) -> Vec<CardId> {
    let card = game.card(spell);
    let has_convoke = card.has_keyword("Convoke");
    let has_improvise = card.has_keyword("Improvise");
    if !has_convoke && !has_improvise {
        return Vec::new();
    }
    game.cards_in_zone(ZoneType::Battlefield, player)
        .iter()
        .filter(|&&cid| {
            if cid == spell {
                return false;
            }
            let c = game.card(cid);
            !c.tapped
                && ((has_convoke && c.is_creature())
                    || (has_improvise && c.type_line.is_artifact()))
        })
        .copied()
        .collect()
}

fn convoke_atom(game: &GameState, source: CardId, cost: &ManaCost) -> u16 {
    let card = game.card(source);
    if !card.is_creature() {
        return forge_foundation::ManaAtom::COLORLESS;
    }
    let colors = card.color.mask() as u16 & forge_foundation::ManaAtom::ALL_MANA_COLORS;
    if colors == 0 {
        return forge_foundation::ManaAtom::COLORLESS;
    }
    let needed = colors & (cost.color_profile() as u16);
    let pick = if needed != 0 { needed } else { colors };
    1u16 << pick.trailing_zeros()
}
