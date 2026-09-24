use super::mana_payment::ManaPaymentSession;
use super::*;

#[derive(Clone, Copy)]
pub(crate) enum CostPaymentContext {
    TriggerResolve,
    ActivatedAbility,
    ManaAbility,
}

impl GameLoop {
    fn current_reserved_sacrifices(&self) -> &[CardId] {
        self.reserved_sacrifice_stack
            .last()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn current_allow_reserved_source_reuse(&self) -> bool {
        self.reserved_source_reuse_stack
            .last()
            .copied()
            .unwrap_or(false)
    }

    fn emit_untap_all_cost_trigger(&mut self, player: PlayerId, card_id: CardId) {
        self.trigger_handler.run_trigger(
            TriggerType::UntapAll,
            RunParams {
                map: Some(vec![(player, vec![card_id])]),
                ..Default::default()
            },
            false,
        );
    }

    fn record_paid_cost_exile(&mut self, game: &mut GameState, source: CardId, exiled: CardId) {
        let host = game.card_mut(source);
        if !host.paid_cost_exiled_cards.contains(&exiled) {
            host.paid_cost_exiled_cards.push(exiled);
        }
        if game.card(exiled).is_token {
            return;
        }
        if matches!(
            game.card(source).zone,
            ZoneType::Battlefield | ZoneType::Stack | ZoneType::Command
        ) {
            game.card_mut(source).add_exiled_card(exiled);
        }
        game.card_mut(exiled).exiled_with = Some(source);
    }

    fn choose_cost_card_from_zone(
        &mut self,
        _game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        valid: &[CardId],
        zone: ZoneType,
        source: CardId,
    ) -> Option<CardId> {
        match zone {
            ZoneType::Battlefield => {
                agents[player.index()].choose_sacrifice(player, valid, Some(source))
            }
            _ => agents[player.index()]
                .choose_cards_for_effect(player, valid, 1, 1)
                .into_iter()
                .next(),
        }
    }

    fn choose_cost_card_mixed(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        valid: &[CardId],
        source: CardId,
    ) -> Option<CardId> {
        if let Some(first_zone) = valid.first().map(|&cid| game.card(cid).zone) {
            if valid.iter().all(|&cid| game.card(cid).zone == first_zone) {
                return self
                    .choose_cost_card_from_zone(game, agents, player, valid, first_zone, source);
            }
        }
        agents[player.index()]
            .choose_cards_for_effect(player, valid, 1, 1)
            .into_iter()
            .next()
    }
    fn pay_roll_dice_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        card_id: CardId,
        amount: i32,
        sides: i32,
        result_svar: &str,
        sa: Option<&SpellAbility>,
    ) -> bool {
        let Some(sa) = sa else {
            return false;
        };
        let result = {
            let mut ctx = crate::ability::effects::EffectContext {
                game,
                combat: Some(&mut self.combat),
                agents,
                trigger_handler: &mut self.trigger_handler,
                token_templates: &self.token_templates,
                token_art_variants: &self.token_art_variants,
                token_fallback: &self.token_fallback,
                edition_dates: &self.edition_dates,
                mana_pools: &mut self.mana_pools,
                parent_target_card: None,
                rng: &mut *self.game_rng,
            };
            crate::ability::effects::roll_dice_effect::roll_for_player(
                &mut ctx, sa, card_id, player, sides, amount, false,
            )
        };
        game.card_mut(card_id)
            .svars
            .insert(result_svar.to_string(), result.to_string());
        true
    }
    fn pay_flip_coin_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        amount: i32,
        sa: Option<&SpellAbility>,
    ) -> bool {
        let Some(sa) = sa else {
            return false;
        };
        let mut ctx = crate::ability::effects::EffectContext {
            game,
            combat: Some(&mut self.combat),
            agents,
            trigger_handler: &mut self.trigger_handler,
            token_templates: &self.token_templates,
            token_art_variants: &self.token_art_variants,
            token_fallback: &self.token_fallback,
            edition_dates: &self.edition_dates,
            mana_pools: &mut self.mana_pools,
            parent_target_card: None,
            rng: &mut *self.game_rng,
        };
        crate::ability::effects::flip_coin_effect::flip_coins(&mut ctx, player, sa, amount);
        true
    }

    /// Pay life and fire the LifeLost trigger.
    pub(crate) fn pay_life_cost(
        &mut self,
        game: &mut GameState,
        player: PlayerId,
        source: CardId,
        amount: i32,
    ) {
        let amount = crate::cost::resolve_dynamic_amount(game, source, player, amount);
        if crate::staticability::static_ability_cant_gain_lose_pay_life::cant_pay_life(
            game, player, true, None,
        ) {
            return;
        }
        // Run PayLife replacement effects before paying life.
        {
            use crate::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
            use crate::replacement::ReplacementResult;
            let mut event = ReplacementEvent::PayLife { player, amount };
            let result = apply_replacements(game, &mut event);
            if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
                return;
            }
        }
        game.player_lose_life(player, amount);
        self.trigger_handler.run_trigger(
            TriggerType::LifeLost,
            RunParams {
                player: Some(player),
                life_amount: Some(amount),
                ..Default::default()
            },
            false,
        );
    }

    /// Discard N cards from hand via agent choice and fire Discarded triggers.
    /// Mirrors Java's `CostDiscard.doListPayment()`.
    fn pay_discard_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) -> Vec<CardId> {
        let eligible: Vec<CardId> = if type_filter == "Hand" {
            game.cards_in_zone(ZoneType::Hand, player).to_vec()
        } else if type_filter == "Card" || type_filter.is_empty() {
            game.cards_in_zone(ZoneType::Hand, player).to_vec()
        } else {
            game.cards_in_zone(ZoneType::Hand, player)
                .iter()
                .copied()
                .filter(|&cid| {
                    crate::ability::effects::matches_change_type(game.card(cid), type_filter, &[])
                })
                .collect()
        };
        let eligible: Vec<CardId> = eligible
            .into_iter()
            .filter(|&cid| cid != source || game.card(source).owner != player)
            .collect();
        let chosen = if type_filter == "Hand" {
            eligible
        } else {
            agents[player.index()].choose_discard(player, &eligible, amount as usize)
        };
        for &cid in &chosen {
            game.discard_card(
                cid,
                player,
                None,
                Some(agents),
                &mut self.replacement_runtime(),
            );
        }
        chosen
    }

    fn cost_part_kind(part: &CostPart) -> &'static str {
        match part {
            CostPart::Tap => "Tap",
            CostPart::Untap => "Untap",
            CostPart::Mana { .. } => "Mana",
            CostPart::PayLife(_) => "PayLife",
            CostPart::Sacrifice { .. } => "Sacrifice",
            CostPart::Discard { .. } => "Discard",
            CostPart::ExileFromAnyGrave { .. } => "ExileFromAnyGrave",
            CostPart::ExileFromSameGrave { .. } => "ExileFromSameGrave",
            CostPart::SubCounter { .. } => "SubCounter",
            CostPart::AddCounter { .. } => "AddCounter",
            CostPart::PutCounterYou { .. } => "PutCounterYou",
            CostPart::Exile { .. } => "Exile",
            CostPart::Return { .. } => "Return",
            CostPart::TapType { .. } => "TapType",
            CostPart::UntapType { .. } => "UntapType",
            CostPart::PayEnergy(_) => "PayEnergy",
            CostPart::PayShards(_) => "PayShards",
            CostPart::DamageYou(_) => "DamageYou",
            CostPart::Draw { .. } => "Draw",
            CostPart::Mill(_) => "Mill",
            CostPart::Reveal { .. } => "Reveal",
            CostPart::Exert { .. } => "Exert",
            CostPart::GainLife(_) => "GainLife",
            CostPart::GainControl { .. } => "GainControl",
            CostPart::RemoveAnyCounter { .. } => "RemoveAnyCounter",
            CostPart::Unattach { .. } => "Unattach",
            CostPart::ExiledMoveToGrave { .. } => "ExiledMoveToGrave",
            CostPart::AddMana { .. } => "AddMana",
            CostPart::Waterbend { .. } => "Waterbend",
            CostPart::ChooseColor(_) => "ChooseColor",
            CostPart::ChooseCreatureType(_) => "ChooseCreatureType",
            CostPart::FlipCoin(_) => "FlipCoin",
            CostPart::RollDice { .. } => "RollDice",
            CostPart::ExileFromStack { .. } => "ExileFromStack",
            CostPart::CollectEvidence(_) => "CollectEvidence",
            CostPart::Forage => "Forage",
            CostPart::PutCardToLib { .. } => "PutCardToLib",
            CostPart::Enlist { .. } => "Enlist",
            CostPart::PromiseGift => "PromiseGift",
            CostPart::RevealChosen { .. } => "RevealChosen",
            CostPart::Behold { .. } => "Behold",
            CostPart::Blight(_) => "Blight",
            CostPart::ExileCtrlOrGrave { .. } => "ExileCtrlOrGrave",
        }
    }

    fn should_confirm_payment(
        part: &CostPart,
        source_is_planeswalker: bool,
        mandatory: bool,
    ) -> bool {
        match part {
            // HumanCostDecision.confirmAction(...) branches
            CostPart::AddMana { .. } => true,
            CostPart::DamageYou(_) => true,
            CostPart::Draw { .. } => true,
            CostPart::Exile {
                type_filter, from, ..
            } => {
                type_filter == "All"
                    || type_filter == "CARDNAME"
                    || type_filter == "NICKNAME"
                    || type_filter == "OriginalHost"
                    || *from == ZoneType::Library
            }
            CostPart::Exert { type_filter, .. } => {
                type_filter == "CARDNAME" || type_filter == "OriginalHost"
            }
            CostPart::FlipCoin(_) => true,
            CostPart::Forage => true,
            CostPart::Mill(_) => true,
            CostPart::PayLife(_) => !mandatory,
            CostPart::PayEnergy(_) => true,
            CostPart::PayShards(_) => true,
            CostPart::PutCardToLib { type_filter, .. } => {
                type_filter == "CARDNAME" || type_filter == "OriginalHost"
            }
            CostPart::Return { type_filter, .. } => {
                type_filter == "CARDNAME" || type_filter == "OriginalHost"
            }
            CostPart::RollDice { .. } => true,
            CostPart::Sacrifice { type_filter, .. } => {
                ((type_filter == "CARDNAME" || type_filter == "NICKNAME") && !mandatory)
                    || type_filter == "OriginalHost"
            }
            CostPart::SubCounter { .. } => !source_is_planeswalker,
            CostPart::Unattach { .. } => true,

            // HumanPlay.payCostDuringAbilityResolve(...) explicit branches
            CostPart::Discard { type_filter, .. } => {
                type_filter == "Hand" || type_filter == "Random"
            }
            CostPart::Mana {
                cost: mana_cost, ..
            } => mana_cost.is_zero(),
            _ => false,
        }
    }

    fn choose_unattach_target(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        sa: Option<&SpellAbility>,
    ) -> CardId {
        if type_filter == "CARDNAME" || type_filter == "NICKNAME" || type_filter == "OriginalHost" {
            let candidates =
                crate::cost::cost_unattach::find_card_to_unattach(game, source, type_filter, sa);
            return candidates.into_iter().next().unwrap_or(source);
        }

        let candidates =
            crate::cost::cost_unattach::find_card_to_unattach(game, source, type_filter, sa);
        if candidates.is_empty() {
            return source;
        }

        agents[player.index()]
            .choose_cards_for_effect(player, &candidates, 1, 1)
            .into_iter()
            .next()
            .unwrap_or(candidates[0])
    }

    fn confirm_cost_part_payment(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        part: &CostPart,
        api: Option<crate::ability::api_type::ApiType>,
        mandatory: bool,
        _context: &CostPaymentContext,
        sa: Option<&SpellAbility>,
    ) -> bool {
        let source_is_planeswalker = game.card(source).type_line.is_planeswalker();
        if !Self::should_confirm_payment(part, source_is_planeswalker, mandatory) {
            return true;
        }
        // Java's harness short-circuits confirm for `CostPartMana` (always
        // returns true without consuming RNG or emitting a callback). Mirror
        // that by returning true directly instead of asking the agent.
        if matches!(part, CostPart::Mana { .. }) {
            return true;
        }
        if sa.is_some_and(|sa| {
            crate::ability::effects::cost_payment::is_spell_payment_context(sa, game)
        }) {
            return true;
        }
        let card_name = game.card(source).card_name.clone();
        let kind = Self::cost_part_kind(part);
        let message = format!(
            "Pay {} for {}?",
            crate::ability::effects::cost_payment::effect_cost_part_display(part),
            card_name
        );
        agents[player.index()].confirm_payment(player, kind, &message, Some(source), api)
    }

    /// Pay the cost parts of an activated ability (tap, mana, life, sacrifice, etc.).
    /// Mirrors Java's `CostPayment.payCost()` iterating over `CostPart`s.
    pub(crate) fn pay_ability_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        card_id: CardId,
        cost: &crate::cost::Cost,
        api: Option<crate::ability::api_type::ApiType>,
        mandatory: bool,
        context: CostPaymentContext,
        mut sa: Option<&mut SpellAbility>,
    ) -> bool {
        game.card_mut(card_id).paid_cost_exiled_cards.clear();
        // Java CostPayment is transactional: if any later cost part fails,
        // previously applied parts are undone. Mirror that via full snapshot.
        let payment_snapshot = self.make_snapshot(game, true);
        let mut payment_ok = true;

        // Java's CostPayment.payComputerCosts uses a two-phase approach:
        // Phase 1 (accept/visit): iterate all cost parts, gather decisions
        //   - CostDiscard.visit() → pick cards to discard (consumes RNG)
        //   - CostSacrifice(CARDNAME).visit() → confirmPayment (consumes RNG)
        //   - Other parts → no-op
        // Phase 2 (payAsDecided): iterate all cost parts, execute payments
        //   - CostPartMana → auto-tap (may trigger mana abilities, consuming RNG)
        //   - CostDiscard → discard the pre-picked cards
        //   - CostSacrifice → sacrifice
        //
        // We match this by pre-picking discards and typed sacrifices before
        // the main payment loop.

        // Phase 1: visit/decide (matching Java's accept loop order).
        let mut pre_picked_discards: Vec<CardId> = Vec::new();
        let mut pre_picked_sacrifices: Vec<CardId> = Vec::new();
        let mut decided_cards: Vec<Option<Vec<CardId>>> = vec![None; cost.parts.len()];
        let mut reserved_sacrifices: Vec<CardId> = self.current_reserved_sacrifices().to_vec();
        let allow_reserved_source_reuse = self.current_allow_reserved_source_reuse();
        for (idx, part) in cost.parts.clone().into_iter().enumerate() {
            match &part {
                CostPart::Discard {
                    type_filter,
                    amount,
                } => {
                    if type_filter != "CARDNAME" && type_filter != "NICKNAME" {
                        if !self.confirm_cost_part_payment(
                            game,
                            agents,
                            player,
                            card_id,
                            &part,
                            api,
                            mandatory,
                            &context,
                            sa.as_deref(),
                        ) {
                            payment_ok = false;
                            break;
                        }
                        let eligible: Vec<CardId> = if type_filter == "Hand" {
                            game.cards_in_zone(ZoneType::Hand, player).to_vec()
                        } else if type_filter == "Card" || type_filter.is_empty() {
                            game.cards_in_zone(ZoneType::Hand, player).to_vec()
                        } else {
                            game.cards_in_zone(ZoneType::Hand, player)
                                .iter()
                                .copied()
                                .filter(|&cid| {
                                    crate::ability::effects::matches_change_type(
                                        game.card(cid),
                                        type_filter,
                                        &[],
                                    )
                                })
                                .collect()
                        };
                        let eligible: Vec<CardId> = eligible
                            .into_iter()
                            .filter(|&cid| cid != card_id || game.card(card_id).owner != player)
                            .collect();
                        let chosen = if type_filter == "Hand" && eligible.len() > 1 {
                            agents[player.index()].order_move_to_zone_list(
                                game,
                                player,
                                &eligible,
                                ZoneType::Graveyard,
                            )
                        } else if type_filter == "Hand" {
                            eligible
                        } else {
                            agents[player.index()].choose_discard(
                                player,
                                &eligible,
                                amount.resolve(game, card_id, player) as usize,
                            )
                        };
                        pre_picked_discards.extend(chosen);
                    }
                }
                CostPart::Sacrifice {
                    type_filter,
                    amount,
                } if (type_filter == "CARDNAME" || type_filter == "NICKNAME")
                    && amount.resolve(game, card_id, player) > 0 =>
                {
                    if !reserved_sacrifices.contains(&card_id) {
                        reserved_sacrifices.push(card_id);
                    }
                    if !self.confirm_cost_part_payment(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        api,
                        mandatory,
                        &context,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::Sacrifice { type_filter, .. } if type_filter == "OriginalHost" => {
                    if !self.confirm_cost_part_payment(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        api,
                        mandatory,
                        &context,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                    let Some(host) = sa
                        .as_deref()
                        .and_then(|s| s.original_host)
                        .filter(|host| !reserved_sacrifices.contains(host))
                    else {
                        payment_ok = false;
                        break;
                    };
                    pre_picked_sacrifices.push(host);
                    reserved_sacrifices.push(host);
                }
                CostPart::Sacrifice {
                    type_filter,
                    amount,
                } if type_filter != "CARDNAME" && type_filter != "NICKNAME" => {
                    let mut valid = cost::get_sacrifice_targets_for_cost(
                        game,
                        player,
                        type_filter,
                        sa.as_deref(),
                    );
                    if !allow_reserved_source_reuse {
                        valid.retain(|cid| !reserved_sacrifices.contains(cid));
                    }
                    let required = (amount.resolve(game, card_id, player)).max(0) as usize;
                    if valid.len() < required {
                        payment_ok = false;
                        break;
                    }
                    if required > 0 {
                        let chosen = agents[player.index()].choose_permanents_to_sacrifice(
                            player,
                            required,
                            required,
                            &valid,
                            sa.as_deref().and_then(|s| s.source),
                        );
                        if chosen.len() < required {
                            payment_ok = false;
                            break;
                        }
                        reserved_sacrifices.extend(chosen.iter().copied());
                        pre_picked_sacrifices.extend(chosen);
                    }
                }
                CostPart::SubCounter {
                    amount,
                    counter_type,
                    type_filter,
                } => {
                    if !self.confirm_cost_part_payment(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        api,
                        mandatory,
                        &context,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                    let all_or_any = matches!(
                        amount,
                        crate::cost::AmountSpec::Svar(s) if s == "All" || s == "Any"
                    );
                    if all_or_any {
                        continue;
                    }
                    let amount_n = amount.resolve(game, card_id, player);
                    let candidates = if type_filter.eq_ignore_ascii_case("CARDNAME")
                        || type_filter.eq_ignore_ascii_case("NICKNAME")
                    {
                        vec![card_id]
                    } else {
                        crate::cost::get_sub_counter_targets(game, player, card_id, type_filter)
                    };
                    if amount_n <= 0
                        || !candidates
                            .iter()
                            .any(|&cid| game.card(cid).counter_count(counter_type) >= amount_n)
                    {
                        payment_ok = false;
                        break;
                    }
                }
                _ if Self::decides_cost_part_cards(&part) => {
                    if !self.confirm_cost_part_payment(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        api,
                        mandatory,
                        &context,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                    match self.decide_cost_part_cards(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        sa.as_deref(),
                    ) {
                        Some(cards) => decided_cards[idx] = Some(cards),
                        None => {
                            payment_ok = false;
                            break;
                        }
                    }
                }
                _ => {
                    // Confirm decisions for parts that need them
                    if !self.confirm_cost_part_payment(
                        game,
                        agents,
                        player,
                        card_id,
                        &part,
                        api,
                        mandatory,
                        &context,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
            }
        }
        if !payment_ok {
            self.restore_snapshot(game, &payment_snapshot);
            return false;
        }

        self.reserved_sacrifice_stack
            .push(reserved_sacrifices.clone());
        self.reserved_source_reuse_stack
            .push(allow_reserved_source_reuse);

        // Phase 2: execute payments.
        let mut pre_sac_idx = 0usize;
        let mut failed_auto_pay_taps: Vec<CardId> = Vec::new();
        let mut failed_auto_pay_pool: Option<crate::mana::ManaPool> = None;
        let mut failed_non_undoable_choices: Vec<(CardId, usize, u16)> = Vec::new();
        let outer_change_zone_table = game.pending_change_zone_table.take();
        for (idx, part) in cost.parts.clone().into_iter().enumerate() {
            self.handle_change_zone_trigger(game, sa.as_deref());
            if !matches!(part, CostPart::Mana { .. }) {
                game.ensure_pending_change_zone_table();
            }
            let decided = decided_cards[idx].as_deref();
            match &part {
                CostPart::Tap => {
                    game.tap(card_id);
                    self.trigger_handler.run_trigger(
                        TriggerType::Taps,
                        RunParams {
                            card: Some(card_id),
                            player: Some(player),
                            ..Default::default()
                        },
                        false,
                    );
                }
                CostPart::Untap => {
                    let was_tapped = game.card(card_id).tapped;
                    game.untap(card_id);
                    if was_tapped {
                        self.emit_untap_all_cost_trigger(player, card_id);
                    }
                }
                CostPart::Mana { .. } => {
                    let saved_matrix = crate::cost::cost_part_mana::save_matrix_before_payment(
                        &self.mana_pools[player.index()],
                    );
                    let mut mana_cost = crate::cost::cost_part_mana::get_mana_cost_for(
                        game,
                        card_id,
                        sa.as_deref(),
                        &part,
                    );
                    let x_count = mana_cost.count_x();
                    if x_count > 0 {
                        let x_paid = sa.as_deref().map_or(0, |sa| sa.x_mana_cost_paid);
                        mana_cost =
                            mana_cost
                                .without_x()
                                .add(&forge_foundation::ManaCost::generic(
                                    (x_paid as i32).saturating_mul(x_count as i32),
                                ));
                    }
                    let card_name = game.card(card_id).card_name.clone();
                    let cost_str = mana_cost.to_string();
                    let payable_mana_cost =
                        crate::mana::apply_player_life_payment_keywords(game, player, &mana_cost);
                    if payable_mana_cost.is_zero() {
                        continue;
                    }
                    let session = ManaPaymentSession {
                        player,
                        card_id,
                        card_name: &card_name,
                        mana_cost: &payable_mana_cost,
                        cost_str: &cost_str,
                        cost_display_str: &cost_str,
                        cost_checkpoint_str: &cost_str,
                        is_activated_ability: matches!(
                            context,
                            CostPaymentContext::ActivatedAbility
                        ),
                        reserved_sacrifices: &reserved_sacrifices,
                    };
                    let payment_ctx = sa
                        .as_deref()
                        .map(|sa| crate::mana::payment_context_for_sa(game, sa));
                    let pay_from_pool = |pool: &mut crate::mana::ManaPool,
                                         cost: &forge_foundation::ManaCost,
                                         life| {
                        match payment_ctx.as_ref() {
                            Some(ctx) => pool.try_pay_for_spell_converted_with_phyrexian_life(
                                cost, ctx, false, life,
                            ),
                            None => pool.try_pay_with_phyrexian_life_unrestricted(cost, life),
                        }
                    };
                    let mana_payment = self.pay_mana_cost_session(
                        game,
                        agents,
                        session,
                        |game, player, land_id, ab, reserved_sacrifices| {
                            Self::mana_ability_available_for_payment_with_reserved_and_reuse(
                                game,
                                player,
                                land_id,
                                ab,
                                reserved_sacrifices,
                                allow_reserved_source_reuse,
                            )
                        },
                        |slf, game, agents, session| {
                            let (trace, incremental_payment) = {
                                let game_ptr: *mut GameState = game;
                                let mut replacement_pools =
                                    (0..game.players.len()).map(|_| ManaPool::new()).collect();
                                let (pool, mut runtime) = slf
                                    .mana_payment_runtime(session.player, &mut replacement_pools);
                                let mut callback = Self::make_mana_payment_callback(
                                    &mut runtime,
                                    game_ptr,
                                    agents,
                                    session.player,
                                    session.card_id,
                                );
                                let exclude_source = if matches!(
                                    context,
                                    CostPaymentContext::ManaAbility
                                ) {
                                    Some(session.card_id)
                                } else {
                                    None
                                };
                                match payment_ctx.as_ref() {
                                    Some(ctx)
                                        if matches!(
                                            context,
                                            CostPaymentContext::ActivatedAbility
                                        ) =>
                                    {
                                        let paid = mana::auto_tap_lands_allow_reserved_source_reuse_pay_incremental_with_callbacks_reserved_and_ctx(
                                            game,
                                            pool,
                                            session.player,
                                            session.mana_cost,
                                            exclude_source,
                                            session.reserved_sacrifices,
                                            &mut callback,
                                            ctx,
                                        );
                                        (
                                            paid.choices,
                                            Some(paid.paid.then_some(paid.payment.life_paid)),
                                        )
                                    }
                                    Some(ctx) => (
                                        mana::auto_tap_lands_allow_reserved_source_reuse_trace_with_callbacks_reserved_and_ctx(
                                            game,
                                            pool,
                                            session.player,
                                            session.mana_cost,
                                            exclude_source,
                                            session.reserved_sacrifices,
                                            &mut callback,
                                            ctx,
                                        ),
                                        None,
                                    ),
                                    None => (
                                        mana::auto_tap_lands_allow_reserved_source_reuse_trace_with_callbacks_and_reserved_sacrifices(
                                            game,
                                            pool,
                                            session.player,
                                            session.mana_cost,
                                            exclude_source,
                                            session.reserved_sacrifices,
                                            &mut callback,
                                        ),
                                        None,
                                    ),
                                }
                            };
                            slf.resolve_auto_tapped_mana_sub_abilities(
                                game,
                                agents,
                                session.player,
                                &trace,
                            );
                            let mut actions: Vec<ManaCostAction> = trace
                                .iter()
                                .map(|choice| ManaCostAction::TapForMana {
                                    card_id: choice.card_id,
                                    mana_ability_index: Some(
                                        choice.mana_ability_index.unwrap_or(0),
                                    ),
                                    express_choice: if choice.needs_express_choice {
                                        Some(choice.chosen_atom)
                                    } else {
                                        None
                                    },
                                })
                                .collect();
                            let life_paid = incremental_payment.unwrap_or_else(|| {
                                pay_from_pool(
                                    &mut slf.mana_pools[session.player.index()],
                                    session.mana_cost,
                                    game.player(session.player).life,
                                )
                            });
                            if let Some(life_to_pay) = life_paid {
                                if life_to_pay > 0 {
                                    slf.pay_life_cost(
                                        game,
                                        session.player,
                                        session.card_id,
                                        life_to_pay,
                                    );
                                }
                                Some(actions)
                            } else {
                                failed_auto_pay_taps
                                    .extend(trace.iter().map(|choice| choice.card_id));
                                failed_auto_pay_pool =
                                    Some(slf.mana_pools[session.player.index()].clone());
                                failed_non_undoable_choices.extend(trace.iter().filter_map(
                                    |choice| {
                                        let idx = choice.mana_ability_index?;
                                        game.card(choice.card_id)
                                            .activated_abilities
                                            .get(idx)
                                            .is_some_and(|ab| !ab.is_undoable())
                                            .then_some((choice.card_id, idx, choice.chosen_atom))
                                    },
                                ));
                                actions.push(ManaCostAction::AttemptedAndFailed);
                                Some(actions)
                            }
                        },
                        |slf, game, player| {
                            let mut test_pool = slf.mana_pools[player.index()].clone();
                            if let Some(test_life_to_pay) = pay_from_pool(
                                &mut test_pool,
                                &payable_mana_cost,
                                game.player(player).life,
                            ) {
                                let life_to_pay = pay_from_pool(
                                    &mut slf.mana_pools[player.index()],
                                    &payable_mana_cost,
                                    game.player(player).life,
                                )
                                .expect("tested phyrexian payment should still be legal");
                                if life_to_pay != test_life_to_pay {
                                    return false;
                                }
                                if life_to_pay > 0 {
                                    slf.pay_life_cost(game, player, card_id, life_to_pay);
                                }
                                true
                            } else {
                                false
                            }
                        },
                    );
                    crate::cost::cost_part_mana::restore_matrix_after_payment(
                        &mut self.mana_pools[player.index()],
                        &saved_matrix,
                    );
                    if !mana_payment.paid {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::PayLife(amount) => {
                    let amount = amount.resolve_for_sa(game, card_id, player, sa.as_deref());
                    self.pay_life_cost(game, player, card_id, amount);
                }
                CostPart::Sacrifice {
                    type_filter,
                    amount,
                } => {
                    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
                        super::perform_sacrifice(
                            game,
                            &mut self.replacement_runtime(),
                            agents,
                            &[card_id],
                        );
                        Self::record_sacrificed_cost_cards(sa.as_deref_mut(), &[card_id]);
                    } else if !self.pay_sacrifice_cost_internal(
                        game,
                        agents,
                        player,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        sa.as_deref_mut(),
                        Some(&pre_picked_sacrifices),
                        &mut pre_sac_idx,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::Discard {
                    type_filter,
                    amount,
                } => {
                    game.begin_discard_batch();
                    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
                        game.discard_card(
                            card_id,
                            player,
                            None,
                            Some(agents),
                            &mut self.replacement_runtime(),
                        );
                    } else if !pre_picked_discards.is_empty() {
                        // Use pre-picked cards from visit phase
                        let discard_count = if type_filter == "Hand" {
                            pre_picked_discards.len()
                        } else {
                            (amount.resolve(game, card_id, player) as usize)
                                .min(pre_picked_discards.len())
                        };
                        let to_discard: Vec<CardId> =
                            pre_picked_discards.drain(..discard_count).collect();
                        for &cid in &to_discard {
                            game.discard_card(
                                cid,
                                player,
                                None,
                                Some(agents),
                                &mut self.replacement_runtime(),
                            );
                        }
                        if let Some(sa) = sa.as_deref_mut() {
                            sa.discarded_cost_cards.extend(to_discard);
                        }
                    } else {
                        let discarded = self.pay_discard_cost(
                            game,
                            agents,
                            player,
                            card_id,
                            type_filter,
                            amount.resolve(game, card_id, player),
                        );
                        if let Some(sa) = sa.as_deref_mut() {
                            sa.discarded_cost_cards.extend(discarded);
                        }
                    }
                    game.end_discard_batch(&mut self.trigger_handler);
                }
                CostPart::ExileFromAnyGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exile_from_any_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::ExileFromSameGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exile_from_same_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::SubCounter {
                    amount,
                    counter_type,
                    type_filter,
                } => {
                    let amount_n = amount.resolve(game, card_id, player);
                    let target = if type_filter.eq_ignore_ascii_case("CARDNAME")
                        || type_filter.eq_ignore_ascii_case("NICKNAME")
                    {
                        Some(card_id)
                    } else {
                        crate::cost::get_sub_counter_targets(game, player, card_id, type_filter)
                            .into_iter()
                            .find(|cid| game.card(*cid).counter_count(counter_type) >= amount_n)
                    };
                    if let Some(target) = target {
                        game.card_mut(target).remove_counter(counter_type, amount_n);
                    }
                }
                CostPart::AddCounter {
                    amount,
                    counter_type,
                    type_filter,
                } => {
                    let amount_n = amount.resolve(game, card_id, player);
                    let Some(counter_target) = self.choose_put_counter_target(
                        game,
                        agents,
                        player,
                        card_id,
                        sa.as_deref(),
                        type_filter,
                    ) else {
                        continue;
                    };
                    crate::ability::effects::effect_context::add_counter_with_context(
                        game,
                        Some(&mut self.trigger_handler),
                        Some(agents),
                        counter_target,
                        counter_type,
                        amount_n,
                        RunParams {
                            source_player: Some(player),
                            cause: sa.as_deref().cloned(),
                            ..Default::default()
                        },
                        false,
                    );
                }
                CostPart::Exile {
                    amount,
                    type_filter,
                    from,
                } => {
                    if type_filter == "CARDNAME"
                        || type_filter == "NICKNAME"
                        || type_filter == "OriginalHost"
                    {
                        self.move_card_with_runtime(
                            game,
                            card_id,
                            ZoneType::Exile,
                            game.card(card_id).owner,
                            agents,
                        );
                        self.record_paid_cost_exile(game, card_id, card_id);
                    } else {
                        self.pay_exile_cost(
                            game,
                            agents,
                            player,
                            card_id,
                            type_filter,
                            amount.resolve(game, card_id, player),
                            *from,
                            decided,
                        );
                    }
                }
                CostPart::Return {
                    amount,
                    type_filter,
                } => {
                    if type_filter == "CARDNAME" {
                        let owner = game.card(card_id).owner;
                        self.move_card_with_runtime(game, card_id, ZoneType::Hand, owner, agents);
                    } else {
                        let Some(returned) = self.pay_return_cost(
                            game,
                            agents,
                            player,
                            card_id,
                            type_filter,
                            amount.resolve(game, card_id, player),
                            sa.as_deref(),
                            decided,
                        ) else {
                            return false;
                        };
                        if let Some(sa) = sa.as_deref_mut() {
                            for card in &returned {
                                let id = card.0.to_string();
                                sa.add_cost_to_hash_list(crate::cost::cost_return::HASH_CARDS, &id);
                                sa.add_cost_to_hash_list(crate::cost::cost_return::HASH_LKI, &id);
                            }
                        }
                    }
                }
                CostPart::TapType {
                    amount,
                    type_filter,
                    min_total_power,
                    can_tap_source,
                } => {
                    if !self.pay_tap_type_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        *min_total_power,
                        *can_tap_source,
                        sa.as_deref_mut(),
                        decided,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::UntapType {
                    amount,
                    type_filter,
                    can_untap_source,
                } => {
                    self.pay_untap_type_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        *can_untap_source,
                    );
                }
                CostPart::PayEnergy(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    game.player_add_energy(player, -resolved_amount);
                }
                CostPart::PayShards(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    game.player_add_shards(player, -resolved_amount);
                }
                CostPart::DamageYou(amount) => {
                    // Java CostDamage calls game.getAction().dealDamage() — use the
                    // same path so damage prevention, replacement effects, and
                    // DamageDone triggers all fire correctly.
                    let damage = amount.resolve(game, card_id, player);
                    let mut runtime = self.replacement_runtime();
                    game.deal_damage(
                        card_id,
                        crate::card::card_damage_map::DamageTarget::Player(player),
                        damage,
                        agents,
                        &mut runtime,
                    );
                    self.trigger_handler.run_trigger(
                        TriggerType::DamageDone,
                        RunParams {
                            damage_target_player: Some(player),
                            damage_amount: Some(amount.resolve(game, card_id, player)),
                            is_combat_damage: Some(false),
                            ..Default::default()
                        },
                        false,
                    );
                    game.lose_life_simultaneously(&mut self.trigger_handler, Some(agents));
                }
                CostPart::Draw { .. } => {
                    crate::cost::cost_draw::pay_as_decided(
                        game,
                        Some(&mut self.trigger_handler),
                        Some(agents),
                        player,
                        card_id,
                        sa.as_deref(),
                        &part,
                    );
                }
                CostPart::Mill(amount) => {
                    for _ in 0..amount.resolve(game, card_id, player) {
                        if let Some(top) = game.take_top_card_from_zone(ZoneType::Library, player) {
                            self.move_card_with_runtime(
                                game,
                                top,
                                ZoneType::Graveyard,
                                player,
                                agents,
                            );
                            self.trigger_handler.run_trigger(
                                TriggerType::Milled,
                                RunParams {
                                    card: Some(top),
                                    player: Some(player),
                                    ..Default::default()
                                },
                                false,
                            );
                            crate::ability::effects::emit_zone_trigger(
                                &mut self.trigger_handler,
                                top,
                                ZoneType::Library,
                                ZoneType::Graveyard,
                            );
                        }
                    }
                }
                CostPart::Reveal {
                    amount,
                    type_filter,
                    from,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_reveal_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        from,
                        sa.as_deref_mut(),
                        decided,
                    );
                }
                CostPart::Exert {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_exert_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::GainLife(amount) => {
                    // Opponent gains life
                    let opponent = game.opponent_of(player);
                    game.player_gain_life(opponent, amount.resolve(game, card_id, player));
                }
                CostPart::GainControl {
                    amount,
                    type_filter,
                } => {
                    self.pay_gain_control_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::RemoveAnyCounter {
                    amount,
                    type_filter,
                    counter_type,
                } => {
                    self.pay_remove_any_counter_cost(
                        game,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        counter_type.as_ref(),
                    );
                }
                CostPart::Unattach { type_filter, .. } => {
                    let target = self.choose_unattach_target(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        sa.as_deref(),
                    );
                    game.detach(target);
                    self.trigger_handler.run_trigger(
                        TriggerType::Unattached,
                        RunParams {
                            card: Some(target),
                            player: Some(player),
                            ..Default::default()
                        },
                        false,
                    );
                }
                CostPart::ExiledMoveToGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exiled_move_to_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::AddMana { amount, mana_type } => {
                    use forge_foundation::mana::ManaAtom;
                    let atom = match mana_type.to_uppercase().as_str() {
                        "W" | "WHITE" => ManaAtom::WHITE,
                        "U" | "BLUE" => ManaAtom::BLUE,
                        "B" | "BLACK" => ManaAtom::BLACK,
                        "R" | "RED" => ManaAtom::RED,
                        "G" | "GREEN" => ManaAtom::GREEN,
                        "C" | "COLORLESS" => ManaAtom::COLORLESS,
                        _ => ManaAtom::COLORLESS,
                    };
                    for _ in 0..amount.resolve(game, card_id, player) {
                        let mut m = crate::mana::Mana::simple(atom);
                        m.source_card = Some(card_id);
                        self.mana_pools[player.index()].add_mana(m);
                    }
                }
                CostPart::Waterbend { amount } => {
                    if !self.pay_waterbend_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        amount.resolve(game, card_id, player),
                        mandatory,
                        context,
                        sa.as_deref_mut(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::ChooseColor(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    let valid_colors = vec![
                        "White".to_string(),
                        "Blue".to_string(),
                        "Black".to_string(),
                        "Red".to_string(),
                        "Green".to_string(),
                    ];
                    game.card_mut(card_id).clear_chosen_colors();
                    for _ in 0..resolved_amount {
                        if let Some(color) =
                            agents[player.index()].choose_color(player, &valid_colors)
                        {
                            game.card_mut(card_id).add_chosen_color(color);
                        }
                    }
                }
                CostPart::ChooseCreatureType(_) => {
                    let valid_types = crate::game::TypeRegistry::creature_types().to_vec();
                    if let Some(chosen) =
                        agents[player.index()].choose_type(player, "Creature", &valid_types)
                    {
                        game.card_mut(card_id)
                            .set_chosen_type(Some(chosen), None, true);
                    }
                }
                CostPart::FlipCoin(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_flip_coin_cost(
                        game,
                        agents,
                        player,
                        resolved_amount,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::RollDice {
                    amount,
                    sides,
                    result_svar,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_roll_dice_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        resolved_amount,
                        *sides,
                        result_svar,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::ExileFromStack {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_exile_from_stack_cost(
                        game,
                        agents,
                        card_id,
                        player,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::CollectEvidence(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_collect_evidence_cost(
                        game,
                        agents,
                        player,
                        resolved_amount,
                        decided,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::PutCounterYou { .. } => {
                    crate::cost::cost_put_counter_you::pay_as_decided(
                        game,
                        Some(&mut self.trigger_handler),
                        Some(agents),
                        player,
                        card_id,
                        sa.as_deref(),
                        &part,
                    );
                }
                CostPart::Forage => {
                    if !self.pay_forage_cost(game, agents, player, card_id, sa.as_deref(), decided)
                    {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::PutCardToLib {
                    amount,
                    lib_pos,
                    type_filter,
                    from,
                    same_zone,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_put_card_to_lib_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        *lib_pos,
                        *from,
                        *same_zone,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::Enlist {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_enlist_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::PromiseGift => {
                    let opps: Vec<_> = game
                        .alive_players()
                        .into_iter()
                        .filter(|&pid| pid != player)
                        .collect();
                    let chosen = agents[player.index()].choose_target_player(player, &opps, None);
                    game.card_mut(card_id).set_promised_gift(chosen);
                }
                CostPart::RevealChosen { reveal_type } => {
                    let source = game.card_mut(card_id);
                    if reveal_type.eq_ignore_ascii_case("Player") {
                        source.chosen_player_revealed = true;
                    } else if reveal_type.eq_ignore_ascii_case("Type") {
                        source.chosen_type_revealed = true;
                    }
                }
                CostPart::Behold {
                    amount,
                    type_filter,
                    exile,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_behold_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        *exile,
                        None,
                    );
                }
                CostPart::Blight(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_blight_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        resolved_amount,
                        sa.as_deref_mut(),
                        decided.and_then(|cards| cards.first().copied()),
                    );
                }
                CostPart::ExileCtrlOrGrave {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_exile_ctrl_or_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        decided,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
            }
        }
        self.handle_change_zone_trigger(game, sa.as_deref());
        game.pending_change_zone_table = outer_change_zone_table;
        self.reserved_sacrifice_stack.pop();
        self.reserved_source_reuse_stack.pop();
        if !payment_ok {
            self.restore_snapshot(game, &payment_snapshot);
            if !matches!(context, CostPaymentContext::ManaAbility)
                && cost.parts.iter().any(|part| matches!(part, CostPart::Tap))
                && game.card_is_in_zone(card_id, ZoneType::Battlefield)
            {
                game.tap(card_id);
            }
            if !matches!(context, CostPaymentContext::ActivatedAbility) {
                for tapped_id in failed_auto_pay_taps {
                    if game.card_is_in_zone(tapped_id, ZoneType::Battlefield) {
                        game.card_mut(tapped_id).set_tapped(true);
                    }
                }
                if let Some(pool) = failed_auto_pay_pool {
                    self.mana_pools[player.index()] = pool;
                }
            } else {
                for (source_id, ability_index, chosen_atom) in failed_non_undoable_choices {
                    let mut replacement_pools =
                        (0..game.players.len()).map(|_| ManaPool::new()).collect();
                    let (pool, mut runtime) =
                        self.mana_payment_runtime(player, &mut replacement_pools);
                    crate::mana::computer_util_mana::reapply_non_undoable_payment_ability(
                        game,
                        pool,
                        &mut runtime,
                        agents,
                        player,
                        source_id,
                        ability_index,
                        chosen_atom,
                    );
                }
            }
            return false;
        }
        true
    }

    /// Pay additional costs from an SP$ ability line (non-mana cost parts only).
    /// Used during spell casting for costs like `Sac<1/Creature>`.
    /// Mirrors Java's `CostPayment.payCost()` for spell abilities.
    pub(crate) fn pay_additional_costs(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        card_id: CardId,
        spell_cost: &crate::cost::Cost,
        _api: Option<&str>,
        _mandatory: bool,
        mut sa: Option<&mut SpellAbility>,
        prechosen_sacrifices: Option<&[CardId]>,
        prechosen_discards: Option<&[CardId]>,
        prechosen_tap_type: Option<&[CardId]>,
        prechosen_beholds: Option<&[CardId]>,
        prechosen_evidence: Option<&[CardId]>,
        decided_cards: Option<&[Option<Vec<CardId>>]>,
    ) -> bool {
        let payment_snapshot = self.make_snapshot(game, true);
        game.card_mut(card_id).paid_cost_exiled_cards.clear();
        let mut payment_ok = true;
        let mut pre_sac_idx = 0usize;
        let mut pre_discard_idx = 0usize;
        let mut pre_tap_idx = 0usize;
        let mut pre_behold_idx = 0usize;
        let outer_change_zone_table = game.pending_change_zone_table.take();
        for (idx, part) in spell_cost.parts.clone().into_iter().enumerate() {
            self.handle_change_zone_trigger(game, sa.as_deref());
            if !matches!(part, CostPart::Mana { .. }) {
                game.ensure_pending_change_zone_table();
            }
            let decided = decided_cards
                .and_then(|cards| cards.get(idx))
                .and_then(|cards| cards.as_deref());
            // Java deterministic parity does not route confirm-payment prompts
            // through RNG while paying spell costs; confirmPayment() returns true
            // for spell payment context. Keep Rust aligned to avoid decision-RNG
            // drift when spell-cost prompt timing differs between engines.
            match &part {
                // Mana is already paid by play_card's main mana payment flow.
                // Tap/Untap are not applicable to spell additional costs.
                CostPart::Mana { .. } | CostPart::Tap | CostPart::Untap => {}
                CostPart::PayLife(amount) => {
                    self.pay_life_cost(
                        game,
                        player,
                        card_id,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::Sacrifice {
                    type_filter,
                    amount,
                } => {
                    if type_filter != "CARDNAME"
                        && !self.pay_sacrifice_cost_internal(
                            game,
                            agents,
                            player,
                            type_filter,
                            amount.resolve(game, card_id, player),
                            sa.as_deref_mut(),
                            prechosen_sacrifices,
                            &mut pre_sac_idx,
                        )
                    {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::Discard {
                    type_filter,
                    amount,
                } => {
                    if type_filter != "CARDNAME" {
                        let discarded = if let Some(prechosen) = prechosen_discards {
                            let mut eligible: Vec<CardId> = if type_filter == "Hand" {
                                game.cards_in_zone(ZoneType::Hand, player).to_vec()
                            } else if type_filter == "Card" || type_filter.is_empty() {
                                game.cards_in_zone(ZoneType::Hand, player).to_vec()
                            } else {
                                game.cards_in_zone(ZoneType::Hand, player)
                                    .iter()
                                    .copied()
                                    .filter(|&cid| {
                                        crate::ability::effects::matches_change_type(
                                            game.card(cid),
                                            type_filter,
                                            &[],
                                        )
                                    })
                                    .collect()
                            };
                            eligible.retain(|&cid| {
                                cid != card_id || game.card(card_id).owner != player
                            });
                            let needed = if type_filter == "Hand" {
                                eligible.len()
                            } else {
                                (amount.resolve(game, card_id, player)).max(0) as usize
                            };
                            if pre_discard_idx + needed > prechosen.len() {
                                payment_ok = false;
                                break;
                            }
                            let chosen =
                                prechosen[pre_discard_idx..pre_discard_idx + needed].to_vec();
                            pre_discard_idx += needed;
                            if !chosen.iter().all(|cid| eligible.contains(cid)) {
                                payment_ok = false;
                                break;
                            }
                            game.begin_discard_batch();
                            for &cid in &chosen {
                                game.discard_card(
                                    cid,
                                    player,
                                    None,
                                    Some(agents),
                                    &mut self.replacement_runtime(),
                                );
                            }
                            game.end_discard_batch(&mut self.trigger_handler);
                            chosen
                        } else {
                            game.begin_discard_batch();
                            let discarded = self.pay_discard_cost(
                                game,
                                agents,
                                player,
                                card_id,
                                type_filter,
                                amount.resolve(game, card_id, player),
                            );
                            game.end_discard_batch(&mut self.trigger_handler);
                            discarded
                        };
                        if type_filter != "Hand"
                            && discarded.len()
                                < (amount.resolve(game, card_id, player)).max(0) as usize
                        {
                            payment_ok = false;
                            break;
                        }
                        if let Some(sa) = sa.as_deref_mut() {
                            sa.discarded_cost_cards.extend(discarded);
                        }
                    }
                }
                CostPart::ExileFromAnyGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exile_from_any_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::ExileFromSameGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exile_from_same_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::SubCounter {
                    amount,
                    counter_type,
                    type_filter,
                } => {
                    let amount_n = amount.resolve(game, card_id, player);
                    let target = if type_filter.eq_ignore_ascii_case("CARDNAME")
                        || type_filter.eq_ignore_ascii_case("NICKNAME")
                    {
                        Some(card_id)
                    } else {
                        crate::cost::get_sub_counter_targets(game, player, card_id, type_filter)
                            .into_iter()
                            .find(|cid| game.card(*cid).counter_count(counter_type) >= amount_n)
                    };
                    if let Some(target) = target {
                        if game.card(target).zone == ZoneType::Battlefield {
                            game.card_mut(target).remove_counter(counter_type, amount_n);
                        }
                    }
                }
                CostPart::AddCounter {
                    amount,
                    counter_type,
                    type_filter,
                } => {
                    let amount_n = amount.resolve(game, card_id, player);
                    let counter_target = self
                        .choose_put_counter_target(
                            game,
                            agents,
                            player,
                            card_id,
                            sa.as_deref(),
                            type_filter,
                        )
                        .unwrap_or(card_id);
                    if game.card(counter_target).zone == ZoneType::Battlefield {
                        crate::ability::effects::effect_context::add_counter_with_context(
                            game,
                            Some(&mut self.trigger_handler),
                            Some(agents),
                            counter_target,
                            counter_type,
                            amount_n,
                            RunParams {
                                source_player: Some(player),
                                cause: sa.as_deref().cloned(),
                                ..Default::default()
                            },
                            false,
                        );
                    }
                }
                CostPart::Exile {
                    amount,
                    type_filter,
                    from,
                } => {
                    if type_filter == "CARDNAME"
                        || type_filter == "NICKNAME"
                        || type_filter == "OriginalHost"
                    {
                        if game.card(card_id).zone == *from {
                            let owner = game.card(card_id).owner;
                            self.move_card_with_runtime(
                                game,
                                card_id,
                                ZoneType::Exile,
                                owner,
                                agents,
                            );
                            self.record_paid_cost_exile(game, card_id, card_id);
                        }
                    } else {
                        self.pay_exile_cost(
                            game,
                            agents,
                            player,
                            card_id,
                            type_filter,
                            amount.resolve(game, card_id, player),
                            *from,
                            decided,
                        );
                    }
                }
                CostPart::Return {
                    amount,
                    type_filter,
                } => {
                    let mut returned = Vec::new();
                    if type_filter != "CARDNAME"
                        && !self.pay_return_cost_internal(
                            game,
                            agents,
                            player,
                            card_id,
                            type_filter,
                            amount.resolve(game, card_id, player),
                            sa.as_deref(),
                            prechosen_sacrifices,
                            &mut pre_sac_idx,
                            &mut returned,
                        )
                    {
                        payment_ok = false;
                        break;
                    }
                    if let Some(sa) = sa.as_deref_mut() {
                        for card in &returned {
                            let id = card.0.to_string();
                            sa.add_cost_to_hash_list(crate::cost::cost_return::HASH_CARDS, &id);
                            sa.add_cost_to_hash_list(crate::cost::cost_return::HASH_LKI, &id);
                        }
                    }
                }
                CostPart::TapType {
                    amount,
                    type_filter,
                    min_total_power,
                    can_tap_source,
                } => {
                    if let Some(prechosen) = prechosen_tap_type {
                        let needed = (amount.resolve(game, card_id, player)).max(0) as usize;
                        if pre_tap_idx + needed > prechosen.len() {
                            payment_ok = false;
                            break;
                        }
                        let chosen = prechosen[pre_tap_idx..pre_tap_idx + needed].to_vec();
                        pre_tap_idx += needed;
                        for cid in chosen {
                            if game.card(cid).zone == ZoneType::Battlefield
                                && !game.card(cid).tapped
                            {
                                game.tap(cid);
                                self.trigger_handler.run_trigger(
                                    TriggerType::Taps,
                                    RunParams {
                                        card: Some(cid),
                                        player: Some(player),
                                        ..Default::default()
                                    },
                                    false,
                                );
                            }
                            if let Some(sa) = sa.as_deref_mut() {
                                let value = cid.to_string();
                                sa.add_cost_to_hash_list(
                                    crate::cost::cost_tap_type::HASH_LKI,
                                    &value,
                                );
                                sa.add_cost_to_hash_list(
                                    crate::cost::cost_tap_type::HASH_CARDS,
                                    &value,
                                );
                            }
                        }
                    } else if !self.pay_tap_type_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        *min_total_power,
                        *can_tap_source,
                        sa.as_deref_mut(),
                        None,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::UntapType {
                    amount,
                    type_filter,
                    can_untap_source,
                } => {
                    self.pay_untap_type_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        *can_untap_source,
                    );
                }
                CostPart::PayEnergy(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    game.player_add_energy(player, -resolved_amount);
                }
                CostPart::PayShards(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    game.player_add_shards(player, -resolved_amount);
                }
                CostPart::DamageYou(amount) => {
                    // Java CostDamage calls game.getAction().dealDamage() — use the
                    // same path so damage prevention, replacement effects, and
                    // DamageDone triggers all fire correctly.
                    let damage = amount.resolve(game, card_id, player);
                    let mut runtime = self.replacement_runtime();
                    game.deal_damage(
                        card_id,
                        crate::card::card_damage_map::DamageTarget::Player(player),
                        damage,
                        agents,
                        &mut runtime,
                    );
                    self.trigger_handler.run_trigger(
                        TriggerType::DamageDone,
                        RunParams {
                            damage_target_player: Some(player),
                            damage_amount: Some(amount.resolve(game, card_id, player)),
                            is_combat_damage: Some(false),
                            ..Default::default()
                        },
                        false,
                    );
                    game.lose_life_simultaneously(&mut self.trigger_handler, Some(agents));
                }
                CostPart::Draw { .. } => {
                    crate::cost::cost_draw::pay_as_decided(
                        game,
                        Some(&mut self.trigger_handler),
                        Some(agents),
                        player,
                        card_id,
                        sa.as_deref(),
                        &part,
                    );
                }
                CostPart::Mill(amount) => {
                    for _ in 0..amount.resolve(game, card_id, player) {
                        if let Some(top) = game.take_top_card_from_zone(ZoneType::Library, player) {
                            self.move_card_with_runtime(
                                game,
                                top,
                                ZoneType::Graveyard,
                                player,
                                agents,
                            );
                            self.trigger_handler.run_trigger(
                                TriggerType::Milled,
                                RunParams {
                                    card: Some(top),
                                    player: Some(player),
                                    ..Default::default()
                                },
                                false,
                            );
                            crate::ability::effects::emit_zone_trigger(
                                &mut self.trigger_handler,
                                top,
                                ZoneType::Library,
                                ZoneType::Graveyard,
                            );
                        }
                    }
                }
                CostPart::Reveal {
                    amount,
                    type_filter,
                    from,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_reveal_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        from,
                        sa.as_deref_mut(),
                        decided,
                    );
                }
                CostPart::Exert {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_exert_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::GainLife(amount) => {
                    let opponent = game.opponent_of(player);
                    game.player_gain_life(opponent, amount.resolve(game, card_id, player));
                }
                CostPart::GainControl {
                    amount,
                    type_filter,
                } => {
                    self.pay_gain_control_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::RemoveAnyCounter {
                    amount,
                    type_filter,
                    counter_type,
                } => {
                    self.pay_remove_any_counter_cost(
                        game,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                        counter_type.as_ref(),
                    );
                }
                CostPart::Unattach { type_filter, .. } => {
                    let target = self.choose_unattach_target(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        sa.as_deref(),
                    );
                    game.detach(target);
                    self.trigger_handler.run_trigger(
                        TriggerType::Unattached,
                        RunParams {
                            card: Some(target),
                            player: Some(player),
                            ..Default::default()
                        },
                        false,
                    );
                }
                CostPart::ExiledMoveToGrave {
                    amount,
                    type_filter,
                } => {
                    self.pay_exiled_move_to_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        amount.resolve(game, card_id, player),
                    );
                }
                CostPart::AddMana { amount, mana_type } => {
                    use forge_foundation::mana::ManaAtom;
                    let atom = match mana_type.to_uppercase().as_str() {
                        "W" | "WHITE" => ManaAtom::WHITE,
                        "U" | "BLUE" => ManaAtom::BLUE,
                        "B" | "BLACK" => ManaAtom::BLACK,
                        "R" | "RED" => ManaAtom::RED,
                        "G" | "GREEN" => ManaAtom::GREEN,
                        "C" | "COLORLESS" => ManaAtom::COLORLESS,
                        _ => ManaAtom::COLORLESS,
                    };
                    for _ in 0..amount.resolve(game, card_id, player) {
                        let mut m = crate::mana::Mana::simple(atom);
                        m.source_card = Some(card_id);
                        self.mana_pools[player.index()].add_mana(m);
                    }
                }
                CostPart::Waterbend { amount } => {
                    if !self.pay_waterbend_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        amount.resolve(game, card_id, player),
                        _mandatory,
                        CostPaymentContext::TriggerResolve,
                        sa.as_deref_mut(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::ChooseColor(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    let valid_colors = vec![
                        "White".to_string(),
                        "Blue".to_string(),
                        "Black".to_string(),
                        "Red".to_string(),
                        "Green".to_string(),
                    ];
                    game.card_mut(card_id).clear_chosen_colors();
                    for _ in 0..resolved_amount {
                        if let Some(color) =
                            agents[player.index()].choose_color(player, &valid_colors)
                        {
                            game.card_mut(card_id).add_chosen_color(color);
                        }
                    }
                }
                CostPart::ChooseCreatureType(_) => {
                    let valid_types = crate::game::TypeRegistry::creature_types().to_vec();
                    if let Some(chosen) =
                        agents[player.index()].choose_type(player, "Creature", &valid_types)
                    {
                        game.card_mut(card_id)
                            .set_chosen_type(Some(chosen), None, true);
                    }
                }
                CostPart::FlipCoin(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_flip_coin_cost(
                        game,
                        agents,
                        player,
                        resolved_amount,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::RollDice {
                    amount,
                    sides,
                    result_svar,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_roll_dice_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        resolved_amount,
                        *sides,
                        result_svar,
                        sa.as_deref(),
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::ExileFromStack {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_exile_from_stack_cost(
                        game,
                        agents,
                        card_id,
                        player,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::CollectEvidence(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_collect_evidence_cost(
                        game,
                        agents,
                        player,
                        resolved_amount,
                        prechosen_evidence,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::PutCounterYou { .. } => {
                    crate::cost::cost_put_counter_you::pay_as_decided(
                        game,
                        Some(&mut self.trigger_handler),
                        Some(agents),
                        player,
                        card_id,
                        sa.as_deref(),
                        &part,
                    );
                }
                CostPart::Forage => {
                    if !self.pay_forage_cost(game, agents, player, card_id, sa.as_deref(), decided)
                    {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::PutCardToLib {
                    amount,
                    lib_pos,
                    type_filter,
                    from,
                    same_zone,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_put_card_to_lib_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        *lib_pos,
                        *from,
                        *same_zone,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
                CostPart::Enlist {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_enlist_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                    );
                }
                CostPart::PromiseGift => {
                    let opps: Vec<_> = game
                        .alive_players()
                        .into_iter()
                        .filter(|&pid| pid != player)
                        .collect();
                    let chosen = agents[player.index()].choose_target_player(player, &opps, None);
                    game.card_mut(card_id).set_promised_gift(chosen);
                }
                CostPart::RevealChosen { reveal_type } => {
                    let source = game.card_mut(card_id);
                    if reveal_type.eq_ignore_ascii_case("Player") {
                        source.chosen_player_revealed = true;
                    } else if reveal_type.eq_ignore_ascii_case("Type") {
                        source.chosen_type_revealed = true;
                    }
                }
                CostPart::Behold {
                    amount,
                    type_filter,
                    exile,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    let prechosen = prechosen_beholds.map(|picks| {
                        let take =
                            (resolved_amount.max(0) as usize).min(picks.len() - pre_behold_idx);
                        let slice = &picks[pre_behold_idx..pre_behold_idx + take];
                        pre_behold_idx += take;
                        slice.to_vec()
                    });
                    self.pay_behold_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        *exile,
                        prechosen.as_deref(),
                    );
                }
                CostPart::Blight(amount) => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    self.pay_blight_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        resolved_amount,
                        sa.as_deref_mut(),
                        decided.and_then(|cards| cards.first().copied()),
                    );
                }
                CostPart::ExileCtrlOrGrave {
                    amount,
                    type_filter,
                } => {
                    let resolved_amount = amount.resolve(game, card_id, player);
                    if !self.pay_exile_ctrl_or_grave_cost(
                        game,
                        agents,
                        player,
                        card_id,
                        type_filter,
                        resolved_amount,
                        decided,
                    ) {
                        payment_ok = false;
                        break;
                    }
                }
            }
        }
        self.handle_change_zone_trigger(game, sa.as_deref());
        game.pending_change_zone_table = outer_change_zone_table;
        if !payment_ok {
            self.restore_snapshot(game, &payment_snapshot);
            return false;
        }
        true
    }

    fn choose_put_counter_target(
        &self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        sa: Option<&SpellAbility>,
        type_filter: &str,
    ) -> Option<CardId> {
        if crate::cost::cost_put_counter::pays_from_source(type_filter) {
            return Some(source);
        }
        let choices: Vec<crate::agent::GameEntity> =
            crate::cost::cost_put_counter::candidates(game, source, sa, type_filter)
                .into_iter()
                .map(crate::agent::GameEntity::Card)
                .collect();
        if choices.is_empty() {
            return None;
        }
        agents[player.index()].snapshot_state(game, &self.mana_pools);
        match agents[player.index()].choose_single_entity_for_effect(player, &choices, false) {
            Some(crate::agent::GameEntity::Card(chosen)) => Some(chosen),
            _ => None,
        }
    }

    pub(crate) fn prechoose_additional_cost_sacrifices(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        spell_cost: &crate::cost::Cost,
        sa: Option<&SpellAbility>,
    ) -> Option<Vec<CardId>> {
        let mut picked: Vec<CardId> = Vec::new();
        for part in spell_cost.parts.clone() {
            match part {
                CostPart::Sacrifice {
                    type_filter,
                    amount,
                } => {
                    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
                        continue;
                    }
                    let amount_n = sa
                        .and_then(|s| s.source)
                        .map(|src| amount.resolve(game, src, player))
                        .unwrap_or_else(|| amount.as_literal().unwrap_or(0))
                        .max(0);
                    let valid =
                        cost::get_sacrifice_targets_for_cost(game, player, &type_filter, sa);
                    if valid.len() < amount_n as usize {
                        return None;
                    }
                    if type_filter == "OriginalHost" {
                        let confirmed = agents[player.index()].confirm_payment(
                            player,
                            "Sacrifice",
                            "Confirm sacrifice cost",
                            sa.and_then(|s| s.source),
                            sa.and_then(|s| s.api),
                        );
                        if !confirmed {
                            return None;
                        }
                    }
                    if amount_n > 0 {
                        let chosen = agents[player.index()].choose_permanents_to_sacrifice(
                            player,
                            amount_n as usize,
                            amount_n as usize,
                            &valid,
                            sa.and_then(|s| s.source),
                        );
                        if chosen.len() < amount_n as usize {
                            return None;
                        }
                        picked.extend(chosen);
                    }
                }
                CostPart::Return {
                    type_filter,
                    amount,
                } => {
                    if type_filter == "CARDNAME" {
                        continue;
                    }
                    let amount_n = sa
                        .and_then(|s| s.source)
                        .map(|src| amount.resolve(game, src, player))
                        .unwrap_or_else(|| amount.as_literal().unwrap_or(0))
                        .max(0);
                    let mut valid =
                        cost::get_sacrifice_targets_for_cost(game, player, &type_filter, sa);
                    if valid.len() < amount_n as usize {
                        return None;
                    }
                    for _ in 0..amount_n {
                        let chosen = agents[player.index()]
                            .choose_cards_for_effect(player, &valid, 1, 1)
                            .into_iter()
                            .next()?;
                        picked.push(chosen);
                        valid.retain(|&cid| cid != chosen);
                    }
                }
                _ => {}
            }
        }
        Some(picked)
    }

    pub(crate) fn prechoose_additional_cost_taps(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        spell_cost: &crate::cost::Cost,
    ) -> Option<Vec<CardId>> {
        let mut picked: Vec<CardId> = Vec::new();
        for part in &spell_cost.parts {
            if let CostPart::TapType {
                amount,
                type_filter,
                min_total_power: None,
                can_tap_source,
            } = part
            {
                let valid =
                    cost::get_tap_type_targets(game, player, type_filter, source, *can_tap_source);
                let needed = amount.resolve(game, source, player).max(0) as usize;
                if valid.len() < needed {
                    return None;
                }
                let chosen =
                    agents[player.index()].choose_cards_for_effect(player, &valid, needed, needed);
                if chosen.len() < needed
                    || !chosen.iter().take(needed).all(|cid| valid.contains(cid))
                {
                    return None;
                }
                picked.extend(chosen.into_iter().take(needed));
            }
        }
        Some(picked)
    }

    pub(crate) fn prechoose_additional_cost_beholds(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        spell_cost: &crate::cost::Cost,
    ) -> Option<Vec<CardId>> {
        let mut picked: Vec<CardId> = Vec::new();
        for part in spell_cost.parts.clone() {
            if let CostPart::Behold {
                type_filter,
                amount,
                ..
            } = part
            {
                let resolved = amount.resolve(game, source, player);
                picked.extend(Self::choose_behold_cards(
                    game,
                    agents,
                    player,
                    source,
                    &type_filter,
                    resolved,
                ));
            }
        }
        Some(picked)
    }

    pub(crate) fn prechoose_additional_cost_evidence(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        spell_cost: &crate::cost::Cost,
    ) -> Option<Vec<CardId>> {
        let mut picked: Vec<CardId> = Vec::new();
        for part in &spell_cost.parts {
            if let CostPart::CollectEvidence(amount) = part {
                picked.extend(Self::choose_evidence_cost_cards(
                    game,
                    agents,
                    player,
                    amount.resolve(game, source, player),
                )?);
            }
        }
        Some(picked)
    }

    fn choose_evidence_cost_cards(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        amount: i32,
    ) -> Option<Vec<CardId>> {
        let valid: Vec<CardId> = game
            .cards_in_zone(ZoneType::Graveyard, player)
            .iter()
            .copied()
            .filter(|&cid| can_exile_for_cost(game, cid))
            .collect();
        if valid.is_empty() {
            return None;
        }
        let chosen: Vec<CardId> = agents[player.index()]
            .choose_cards_for_effect(player, &valid, 0, valid.len())
            .into_iter()
            .filter(|cid| valid.contains(cid))
            .collect();
        let total_mv: i32 = chosen
            .iter()
            .map(|&cid| game.card(cid).mana_cost.cmc())
            .sum();
        if total_mv < amount {
            return None;
        }
        Some(chosen)
    }

    fn choose_cost_cards_exactly(
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        valid: &[CardId],
        amount: i32,
    ) -> Option<Vec<CardId>> {
        let amount = amount.max(0) as usize;
        if valid.len() < amount {
            return None;
        }
        let chosen = agents[player.index()].choose_cards_for_effect(player, valid, amount, amount);
        (chosen.len() >= amount).then_some(chosen)
    }

    fn decides_cost_part_cards(part: &CostPart) -> bool {
        match part {
            CostPart::Exile {
                type_filter, from, ..
            } => {
                !matches!(
                    type_filter.as_str(),
                    "CARDNAME" | "NICKNAME" | "OriginalHost" | "All"
                ) && *from != ZoneType::Library
            }
            CostPart::Return { type_filter, .. } => type_filter != "CARDNAME",
            CostPart::TapType {
                type_filter,
                min_total_power,
                ..
            } => min_total_power.is_none() && type_filter != "OriginalHost",
            CostPart::ExileCtrlOrGrave { .. }
            | CostPart::CollectEvidence(_)
            | CostPart::Reveal { .. }
            | CostPart::Blight(_)
            | CostPart::Forage => true,
            _ => false,
        }
    }

    fn decide_cost_part_cards(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        part: &CostPart,
        sa: Option<&SpellAbility>,
    ) -> Option<Vec<CardId>> {
        match part {
            CostPart::Exile {
                amount,
                type_filter,
                from,
            } => {
                let amount = amount.resolve(game, source, player);
                if amount <= 0 {
                    return Some(Vec::new());
                }
                Self::choose_cost_cards_exactly(
                    agents,
                    player,
                    &Self::exile_cost_candidates(game, player, source, type_filter, *from),
                    amount,
                )
            }
            CostPart::ExileCtrlOrGrave {
                amount,
                type_filter,
            } => {
                let amount = amount.resolve(game, source, player);
                if amount <= 0 {
                    return Some(Vec::new());
                }
                Self::choose_cost_cards_exactly(
                    agents,
                    player,
                    &Self::exile_ctrl_or_grave_candidates(game, player, source, type_filter),
                    amount,
                )
            }
            CostPart::Return {
                amount,
                type_filter,
            } => Self::choose_cost_cards_exactly(
                agents,
                player,
                &cost::get_sacrifice_targets_for_cost(game, player, type_filter, sa),
                amount.resolve(game, source, player),
            ),
            CostPart::TapType {
                amount,
                type_filter,
                can_tap_source,
                ..
            } => {
                let amount = amount.resolve(game, source, player);
                if amount <= 0 {
                    return Some(Vec::new());
                }
                Self::choose_cost_cards_exactly(
                    agents,
                    player,
                    &cost::get_tap_type_targets_for_cost(
                        game,
                        player,
                        type_filter,
                        source,
                        *can_tap_source,
                        sa,
                    ),
                    amount,
                )
            }
            CostPart::CollectEvidence(amount) => Self::choose_evidence_cost_cards(
                game,
                agents,
                player,
                amount.resolve(game, source, player),
            ),
            CostPart::Reveal {
                amount,
                type_filter,
                from,
            } => Some(self.choose_reveal_cost_cards(
                game,
                agents,
                player,
                source,
                type_filter,
                amount.resolve(game, source, player),
                from,
            )),
            CostPart::Blight(_) => {
                let choices: Vec<crate::agent::GameEntity> = Self::blight_targets(game, player)
                    .into_iter()
                    .map(crate::agent::GameEntity::Card)
                    .collect();
                if choices.is_empty() {
                    return None;
                }
                match agents[player.index()]
                    .choose_single_entity_for_effect(player, &choices, false)
                {
                    Some(crate::agent::GameEntity::Card(chosen)) => Some(vec![chosen]),
                    _ => None,
                }
            }
            CostPart::Forage => Self::choose_forage_cost_cards(game, agents, player, source, sa),
            _ => Some(Vec::new()),
        }
    }

    pub(crate) fn prechoose_additional_cost_cards(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        spell_cost: &crate::cost::Cost,
        sa: Option<&SpellAbility>,
    ) -> Option<Vec<Option<Vec<CardId>>>> {
        let mut decided: Vec<Option<Vec<CardId>>> = vec![None; spell_cost.parts.len()];
        for (idx, part) in spell_cost.parts.iter().enumerate() {
            if matches!(
                part,
                CostPart::Exile { .. }
                    | CostPart::ExileCtrlOrGrave { .. }
                    | CostPart::Reveal { .. }
                    | CostPart::Blight(_)
                    | CostPart::Forage
            ) && Self::decides_cost_part_cards(part)
            {
                decided[idx] =
                    Some(self.decide_cost_part_cards(game, agents, player, source, part, sa)?);
            }
        }
        Some(decided)
    }

    pub(crate) fn prechoose_additional_cost_discards(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        spell_cost: &crate::cost::Cost,
    ) -> Option<Vec<CardId>> {
        let mut picked: Vec<CardId> = Vec::new();
        let mut available_hand: Vec<CardId> = game.cards_in_zone(ZoneType::Hand, player).to_vec();
        for part in spell_cost.parts.clone() {
            if let CostPart::Discard {
                type_filter,
                amount,
            } = part
            {
                if type_filter == "CARDNAME" {
                    continue;
                }
                if type_filter == "Hand" {
                    let hand: Vec<CardId> = available_hand
                        .iter()
                        .copied()
                        .filter(|&cid| cid != source || game.card(source).owner != player)
                        .collect();
                    if hand.len() > 1 {
                        picked.extend(agents[player.index()].order_move_to_zone_list(
                            game,
                            player,
                            &hand,
                            ZoneType::Graveyard,
                        ));
                    } else {
                        picked.extend(hand);
                    }
                    available_hand.clear();
                    continue;
                }
                let mut eligible: Vec<CardId> = if type_filter == "Card" || type_filter.is_empty() {
                    available_hand.clone()
                } else {
                    available_hand
                        .iter()
                        .copied()
                        .filter(|&cid| {
                            crate::ability::effects::matches_change_type(
                                game.card(cid),
                                &type_filter,
                                &[],
                            )
                        })
                        .collect()
                };
                eligible.retain(|&cid| cid != source || game.card(source).owner != player);
                let amount_n = amount.resolve(game, source, player).max(0);
                if eligible.len() < amount_n as usize {
                    return None;
                }
                let chosen =
                    agents[player.index()].choose_discard(player, &eligible, amount_n as usize);
                if chosen.len() < amount_n as usize {
                    return None;
                }
                for cid in chosen {
                    if !eligible.contains(&cid) {
                        return None;
                    }
                    picked.push(cid);
                    available_hand.retain(|&hand_cid| hand_cid != cid);
                }
            }
        }
        Some(picked)
    }

    /// Pay a Waterbend cost: pay `amount` generic mana, but the player can tap
    /// artifacts and/or creatures to help (each tapped pays {1}).
    /// Mirrors Java's CostWaterbend + adjustCostByWaterbend.
    pub(crate) fn pay_waterbend_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        card_id: CardId,
        amount: i32,
        mandatory: bool,
        context: CostPaymentContext,
        sa: Option<&mut SpellAbility>,
    ) -> bool {
        if amount <= 0 {
            return true;
        }
        // Gather tappable artifacts/creatures (excluding the source card)
        let untapped: Vec<CardId> = game
            .cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .filter(|&&cid| {
                let c = game.card(cid);
                !c.tapped && cid != card_id && (c.is_creature() || c.type_line.is_artifact())
            })
            .copied()
            .collect();

        let mut remaining = amount;

        if !untapped.is_empty() && remaining > 0 {
            // Reuse the convoke agent method — waterbend is convoke+improvise combined
            let _card_name = game.card(card_id).card_name.clone();
            let generic_cost = forge_foundation::ManaCost::generic(remaining);
            agents[player.index()].snapshot_state(game, &self.mana_pools);
            let to_tap = agents[player.index()].choose_convoke(
                player,
                &untapped,
                &generic_cost,
                Some(card_id),
            );
            let max_tap = remaining as usize;
            let mut count = 0usize;
            for &cid in &to_tap {
                if count >= max_tap {
                    break;
                }
                if !untapped.contains(&cid) {
                    continue;
                }
                game.tap(cid);
                remaining -= 1;
                count += 1;
            }
        }

        if remaining <= 0 {
            return true;
        }
        let remaining_cost = crate::cost::Cost {
            parts: vec![CostPart::Mana {
                cost: forge_foundation::ManaCost::generic(remaining),
                x_min: 0,
                is_exiled_creature_cost: false,
                is_enchanted_creature_cost: false,
                is_cost_pay_any_number_of_times: false,
                max_waterbend: None,
            }],
            has_tap: false,
            mandatory,
        };
        self.pay_ability_cost(
            game,
            agents,
            player,
            card_id,
            &remaining_cost,
            None,
            mandatory,
            context,
            sa,
        )
    }

    /// Exile `amount` cards from ANY player's graveyard matching `type_filter`.
    /// Mirrors Java's CostExile with zoneMode=-1 (ExileAnyGrave).
    pub(crate) fn pay_exile_from_any_grave_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        let base_filter = crate::cost::normalize_exile_base_filter(type_filter);
        // TriggeredNewCard → the source card (it just entered the new zone,
        // e.g. Greenwarden's graveyard instance for its death trigger).
        if base_filter.contains("TriggeredNewCard") {
            let src = game.card(source);
            if src.zone != ZoneType::Graveyard || !can_exile_for_cost(game, source) {
                return;
            }
            // Java uses chooseCardsForEffect here (pick_count+pick_index+
            // pick_many_unique) — match the RNG pattern even with 1 option.
            let valid = vec![source];
            let chosen = agents[player.index()]
                .choose_cards_for_effect(player, &valid, 1, 1)
                .into_iter()
                .next();
            if let Some(chosen) = chosen {
                let owner = game.card(chosen).owner;
                self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
                self.record_paid_cost_exile(game, source, chosen);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    chosen,
                    ZoneType::Graveyard,
                    ZoneType::Exile,
                );
            }
            return;
        }
        for _ in 0..amount {
            let valid: Vec<CardId> = game
                .players
                .iter()
                .map(|p| p.id)
                .flat_map(|pid| game.cards_in_zone(ZoneType::Graveyard, pid).to_vec())
                .filter(|&cid| {
                    (base_filter == "Card"
                        || base_filter.is_empty()
                        || crate::ability::effects::matches_change_type(
                            game.card(cid),
                            &base_filter,
                            &[],
                        ))
                        && can_exile_for_cost(game, cid)
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            if let Some(chosen) = self.choose_cost_card_from_zone(
                game,
                agents,
                player,
                &valid,
                ZoneType::Graveyard,
                source,
            ) {
                let owner = game.card(chosen).owner;
                self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
                self.record_paid_cost_exile(game, source, chosen);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    chosen,
                    ZoneType::Graveyard,
                    ZoneType::Exile,
                );
            }
        }
    }

    /// Exile `amount` cards from the same graveyard matching `type_filter`.
    /// Mirrors Java's CostExile zoneMode=0 (ExileSameGrave).
    pub(crate) fn pay_exile_from_same_grave_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        let base_filter = crate::cost::normalize_exile_base_filter(type_filter);
        let mut chosen_owner: Option<PlayerId> = None;
        for _ in 0..amount {
            let valid: Vec<CardId> = game
                .players
                .iter()
                .map(|p| p.id)
                .flat_map(|pid| game.cards_in_zone(ZoneType::Graveyard, pid).to_vec())
                .filter(|&cid| {
                    if let Some(owner) = chosen_owner {
                        if game.card(cid).owner != owner {
                            return false;
                        }
                    }
                    (base_filter == "Card"
                        || base_filter.is_empty()
                        || crate::ability::effects::matches_change_type(
                            game.card(cid),
                            &base_filter,
                            &[],
                        ))
                        && can_exile_for_cost(game, cid)
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            if let Some(chosen) = self.choose_cost_card_from_zone(
                game,
                agents,
                player,
                &valid,
                ZoneType::Graveyard,
                source,
            ) {
                chosen_owner = Some(game.card(chosen).owner);
                let owner = game.card(chosen).owner;
                self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
                self.record_paid_cost_exile(game, source, chosen);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    chosen,
                    ZoneType::Graveyard,
                    ZoneType::Exile,
                );
            }
        }
    }

    fn exile_cost_candidates(
        game: &GameState,
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        from: ZoneType,
    ) -> Vec<CardId> {
        let base_filter = crate::cost::normalize_exile_base_filter(type_filter);
        let mut valid = cost::get_zone_targets(game, player, from, &base_filter, source);
        valid.retain(|&cid| can_exile_for_cost(game, cid));
        if from == ZoneType::Hand
            && game.card(source).zone == ZoneType::Hand
            && game.card(source).owner == player
        {
            valid.retain(|&cid| cid != source);
        }
        valid
    }

    /// Exile `amount` cards from `zone` matching `type_filter` for `player`.
    /// Mirrors Java's `CostExile.doListPayment()`.
    pub(crate) fn pay_exile_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        from: ZoneType,
        prechosen: Option<&[CardId]>,
    ) {
        if amount <= 0 {
            return;
        }
        let amount = amount as usize;
        let chosen = match prechosen {
            Some(picks) => picks.to_vec(),
            None => {
                let valid = Self::exile_cost_candidates(game, player, source, type_filter, from);
                if valid.len() < amount {
                    return;
                }
                agents[player.index()].choose_cards_for_effect(player, &valid, amount, amount)
            }
        };
        for chosen in chosen {
            let owner = game.card(chosen).owner;
            self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
            self.record_paid_cost_exile(game, source, chosen);
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                chosen,
                from,
                ZoneType::Exile,
            );
        }
    }

    /// Exile spell(s) from stack as a cost.
    /// Mirrors Java CostExileFromStack.
    pub(crate) fn pay_exile_from_stack_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        source: CardId,
        player: PlayerId,
        type_filter: &str,
        amount: i32,
    ) {
        for _ in 0..amount {
            let valid_entries: Vec<u32> = game
                .stack
                .iter()
                .filter(|e| e.spell_ability.is_spell)
                .filter(|e| {
                    e.spell_ability.source.is_some_and(|cid| {
                        crate::cost::matches_exile_from_stack_filter(
                            game,
                            cid,
                            source,
                            player,
                            type_filter,
                        )
                    })
                })
                .map(|e| e.id)
                .collect();
            if valid_entries.is_empty() {
                break;
            }
            let Some(chosen_entry) =
                agents[player.index()].choose_target_spell(player, &valid_entries, Some(source))
            else {
                break;
            };
            if let Some(entry) = game.stack.remove_by_id(chosen_entry) {
                if let Some(chosen_card) = entry.spell_ability.source {
                    let owner = game.card(chosen_card).owner;
                    self.move_card_with_runtime(game, chosen_card, ZoneType::Exile, owner, agents);
                    crate::ability::effects::emit_zone_trigger(
                        &mut self.trigger_handler,
                        chosen_card,
                        ZoneType::Stack,
                        ZoneType::Exile,
                    );
                }
            }
        }
    }

    /// Collect evidence N as a cost: exile cards from your graveyard with total MV >= N.
    pub(crate) fn pay_collect_evidence_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        amount: i32,
        prechosen: Option<&[CardId]>,
    ) -> bool {
        let valid: Vec<CardId> = game
            .cards_in_zone(ZoneType::Graveyard, player)
            .iter()
            .copied()
            .filter(|&cid| can_exile_for_cost(game, cid))
            .collect();
        if valid.is_empty() {
            return false;
        }

        let selected = match prechosen {
            Some(picks) => picks.to_vec(),
            None => agents[player.index()].choose_cards_for_effect(player, &valid, 0, valid.len()),
        };
        let chosen: Vec<CardId> = selected
            .into_iter()
            .filter(|cid| valid.contains(cid))
            .collect();

        let total_mv: i32 = chosen
            .iter()
            .map(|&cid| game.card(cid).mana_cost.cmc())
            .sum();
        if total_mv < amount {
            return false;
        }

        for cid in chosen {
            let owner = game.card(cid).owner;
            self.move_card_with_runtime(game, cid, ZoneType::Exile, owner, agents);
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                cid,
                ZoneType::Graveyard,
                ZoneType::Exile,
            );
        }
        self.trigger_handler.run_trigger(
            TriggerType::CollectEvidence,
            RunParams {
                player: Some(player),
                ..Default::default()
            },
            false,
        );
        true
    }

    fn choose_forage_cost_cards(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        sa: Option<&SpellAbility>,
    ) -> Option<Vec<CardId>> {
        let battlefield_cards: Vec<_> = game
            .players
            .iter()
            .flat_map(|p| game.cards_in_zone(ZoneType::Battlefield, p.id))
            .map(|&cid| game.cards[cid.index()].clone())
            .collect();
        let foods: Vec<CardId> = game
            .cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .copied()
            .filter(|&cid| game.card(cid).type_line.has_subtype("Food"))
            .filter(|&cid| {
                !crate::staticability::static_ability_cant_sacrifice::cant_sacrifice(
                    &battlefield_cards,
                    game.card(cid),
                    None,
                    true,
                )
            })
            .collect();
        let gy: Vec<CardId> = game
            .cards_in_zone(ZoneType::Graveyard, player)
            .iter()
            .copied()
            .filter(|&cid| can_exile_for_cost(game, cid))
            .collect();
        let can_exile = gy.len() >= 3;
        if foods.is_empty() && !can_exile {
            return None;
        }
        let choose_food = !foods.is_empty()
            && (!can_exile
                || agents[player.index()].choose_binary(
                    player,
                    "Forage: sacrifice Food instead of exiling three cards?",
                    crate::agent::BinaryChoiceKind::AddOrRemove,
                    None,
                    Some(source),
                    sa.and_then(|s| s.api),
                ));
        if choose_food {
            return agents[player.index()]
                .choose_sacrifice(player, &foods, Some(source))
                .map(|food| vec![food]);
        }
        let chosen = agents[player.index()].choose_cards_for_effect(player, &gy, 3, 3);
        (chosen.len() == 3).then_some(chosen)
    }

    /// Forage as a cost: exile 3 from graveyard or sacrifice a Food.
    pub(crate) fn pay_forage_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        sa: Option<&SpellAbility>,
        prechosen: Option<&[CardId]>,
    ) -> bool {
        let chosen = match prechosen {
            Some(picks) => picks.to_vec(),
            None => match Self::choose_forage_cost_cards(game, agents, player, source, sa) {
                Some(picks) => picks,
                None => return false,
            },
        };
        if chosen.len() == 1 {
            super::perform_sacrifice(game, &mut self.replacement_runtime(), agents, &chosen);
        } else {
            for cid in chosen {
                let owner = game.card(cid).owner;
                self.move_card_with_runtime(game, cid, ZoneType::Exile, owner, agents);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    cid,
                    ZoneType::Graveyard,
                    ZoneType::Exile,
                );
            }
        }

        self.trigger_handler.run_trigger(
            TriggerType::Forage,
            RunParams {
                player: Some(player),
                ..Default::default()
            },
            false,
        );
        true
    }

    fn choose_reveal_cost_cards(
        &mut self,
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        from: &crate::cost::RevealFrom,
    ) -> Vec<CardId> {
        if amount <= 0 {
            return Vec::new();
        }

        let mut candidates: Vec<CardId> = match from {
            crate::cost::RevealFrom::Hand => game.cards_in_zone(ZoneType::Hand, player).to_vec(),
            crate::cost::RevealFrom::Exile => game.cards_in_zone(ZoneType::Exile, player).to_vec(),
            crate::cost::RevealFrom::HandOrBattlefield => {
                let mut v = game.cards_in_zone(ZoneType::Hand, player).to_vec();
                v.extend(
                    game.cards_in_zone(ZoneType::Battlefield, player)
                        .iter()
                        .copied(),
                );
                v
            }
            crate::cost::RevealFrom::All => {
                let mut v = game.cards_in_zone(ZoneType::Hand, player).to_vec();
                v.extend(
                    game.cards_in_zone(ZoneType::Battlefield, player)
                        .iter()
                        .copied(),
                );
                v.extend(
                    game.cards_in_zone(ZoneType::Graveyard, player)
                        .iter()
                        .copied(),
                );
                v.extend(
                    game.cards_in_zone(ZoneType::Library, player)
                        .iter()
                        .copied(),
                );
                v.extend(game.cards_in_zone(ZoneType::Exile, player).iter().copied());
                v
            }
        };

        if matches!(
            from,
            crate::cost::RevealFrom::Hand
                | crate::cost::RevealFrom::HandOrBattlefield
                | crate::cost::RevealFrom::All
        ) && game.card(source).zone == ZoneType::Hand
        {
            candidates.retain(|&cid| cid != source);
        }

        let mut revealed: Vec<CardId> = Vec::new();
        if type_filter == "Hand" {
            revealed = candidates;
        } else if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
            revealed.push(source);
        } else if type_filter == "SameColor" {
            if let Some(first) =
                self.choose_cost_card_mixed(game, agents, player, &candidates, source)
            {
                let color = game.card(first).color;
                revealed.push(first);
                while (revealed.len() as i32) < amount {
                    let valid: Vec<CardId> = candidates
                        .iter()
                        .copied()
                        .filter(|cid| !revealed.contains(cid))
                        .filter(|&cid| game.card(cid).color.shares_color_with(color))
                        .collect();
                    if valid.is_empty() {
                        break;
                    }
                    let next = self
                        .choose_cost_card_mixed(game, agents, player, &valid, source)
                        .unwrap_or(valid[0]);
                    revealed.push(next);
                }
            }
        } else {
            let candidates =
                crate::cost::reveal_candidates(game, player, source, type_filter, from);
            revealed = agents[player.index()].choose_cards_for_effect(
                player,
                &candidates,
                amount as usize,
                amount as usize,
            );
        }
        revealed
    }

    pub(crate) fn pay_reveal_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        from: &crate::cost::RevealFrom,
        sa: Option<&mut SpellAbility>,
        prechosen: Option<&[CardId]>,
    ) {
        if amount <= 0 {
            return;
        }
        let revealed = match prechosen {
            Some(picks) => picks.to_vec(),
            None => self.choose_reveal_cost_cards(
                game,
                agents,
                player,
                source,
                type_filter,
                amount,
                from,
            ),
        };
        if revealed.is_empty() {
            return;
        }

        // Java `CostReveal.getHashForLKIList` is "Revealed", which is what an SVar
        // like `Revealed$CardPower` reads back after the cost is paid.
        if let Some(sa) = sa {
            for cid in &revealed {
                let value = cid.to_string();
                sa.add_cost_to_hash_list(crate::cost::cost_reveal::HASH_LKI, &value);
                sa.add_cost_to_hash_list(crate::cost::cost_reveal::HASH_CARDS, &value);
            }
        }

        let names = revealed
            .iter()
            .map(|&cid| game.card(cid).card_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        crate::agent::notify_all_agents(
            agents,
            crate::agent::GameLogEvent::action(format!(
                "{} reveals {}",
                game.player(player).name,
                names
            ))
            .with_player(player),
        );
    }

    /// Put selected cards to top/bottom of library as a cost.
    pub(crate) fn pay_put_card_to_lib_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        lib_pos: i32,
        from: ZoneType,
        same_zone: bool,
    ) -> bool {
        if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
            // Self payment path: move the source card itself if it's in the expected zone.
            if game.card(source).zone == from {
                let owner = game.card(source).owner;
                if lib_pos == 0 {
                    self.move_card_with_runtime(game, source, ZoneType::Library, owner, agents);
                } else {
                    game.put_on_bottom_of_library(source, owner);
                }
                return true;
            }
            return false;
        }
        let mut chosen_controller: Option<PlayerId> = None;
        let mut paid = 0i32;
        for _ in 0..amount {
            let valid: Vec<CardId> = if same_zone {
                game.players
                    .iter()
                    .flat_map(|p| game.cards_in_zone(from, p.id).to_vec())
                    .filter(|&cid| {
                        if let Some(ctrl) = chosen_controller {
                            if game.card(cid).controller != ctrl {
                                return false;
                            }
                        }
                        type_filter == "Card"
                            || type_filter.is_empty()
                            || crate::ability::effects::matches_change_type(
                                game.card(cid),
                                type_filter,
                                &[],
                            )
                    })
                    .collect()
            } else {
                crate::cost::get_zone_targets(game, player, from, type_filter, source)
            };
            if valid.is_empty() {
                return false;
            }
            let Some(chosen) =
                self.choose_cost_card_from_zone(game, agents, player, &valid, from, source)
            else {
                return false;
            };
            if same_zone {
                chosen_controller = Some(game.card(chosen).controller);
            }
            let origin = game.card(chosen).zone;
            let owner = game.card(chosen).owner;
            if lib_pos == 0 {
                self.move_card_with_runtime(game, chosen, ZoneType::Library, owner, agents);
            } else {
                game.put_on_bottom_of_library(chosen, owner);
            }
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                chosen,
                origin,
                ZoneType::Library,
            );
            paid += 1;
        }
        paid == amount
    }

    pub(crate) fn pay_exert_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        if amount <= 0 {
            return;
        }
        if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
            if game.card(source).zone == ZoneType::Battlefield {
                game.card_mut(source).exert();
                self.trigger_handler.run_trigger(
                    TriggerType::Exerted,
                    RunParams {
                        card: Some(source),
                        player: Some(player),
                        ..Default::default()
                    },
                    false,
                );
            }
            return;
        }
        for _ in 0..amount {
            let valid: Vec<CardId> = game
                .cards_in_zone(ZoneType::Battlefield, player)
                .iter()
                .copied()
                .filter(|&cid| {
                    crate::ability::effects::matches_change_type(game.card(cid), type_filter, &[])
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            let Some(chosen) =
                agents[player.index()].choose_sacrifice(player, &valid, Some(source))
            else {
                break;
            };
            game.card_mut(chosen).exert();
            self.trigger_handler.run_trigger(
                TriggerType::Exerted,
                RunParams {
                    card: Some(chosen),
                    player: Some(player),
                    ..Default::default()
                },
                false,
            );
        }
    }

    /// Enlist as a cost.
    pub(crate) fn pay_enlist_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        for _ in 0..amount {
            let valid: Vec<CardId> = crate::cost::get_enlist_targets(game, player)
                .into_iter()
                .filter(|&cid| {
                    type_filter.eq_ignore_ascii_case("Creature")
                        || type_filter.is_empty()
                        || crate::ability::effects::matches_change_type(
                            game.card(cid),
                            type_filter,
                            &[],
                        )
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            let Some(chosen) =
                agents[player.index()].choose_sacrifice(player, &valid, Some(source))
            else {
                break;
            };
            let enlisted_power = game.card(chosen).power();
            game.tap(chosen);
            game.card_mut(source).mark_enlisted_this_combat();
            self.trigger_handler.run_trigger(
                TriggerType::TapAll,
                RunParams {
                    card: Some(chosen),
                    player: Some(player),
                    ..Default::default()
                },
                false,
            );
            self.trigger_handler.register_delayed_trigger(
                crate::trigger::handler::DelayedTrigger {
                    mode: TriggerType::Immediate,
                    trigger_mode: Box::new(crate::trigger::trigger_immediate::TriggerImmediate),
                    params: crate::parsing::Params::default(),
                    execute_svar: "TrigEnlist".to_string(),
                    controller: player,
                    source_card: source,
                    created_turn: game.turn.turn_number,
                    created_phase: game.turn.phase,
                    target_card: None,
                    remembered_amount: enlisted_power,
                    remembered_cards: vec![chosen],
                    remembered_players: Vec::new(),
                    remembered_lki_cards: vec![chosen],
                    target_card_zone_timestamp: None,
                    sort_after_active: true,
                    trigger_order: None,
                    source_timestamp: None,
                    spawning_ability: None,
                },
            );
            self.trigger_handler.run_trigger(
                TriggerType::Enlisted,
                RunParams {
                    card: Some(source),
                    enlisted: Some(chosen),
                    player: Some(player),
                    ..Default::default()
                },
                false,
            );
        }
    }

    /// Behold as a cost (optionally exile revealed cards).
    ///
    /// Mirrors Java's `DeterministicCostPlumbing.visit(CostBehold)` which uses
    /// `chooseCardsForEffect` (not `choose_sacrifice`). For ChosenType filters,
    /// Java does two separate calls: pick 1 first, filter by shared creature
    /// type, then pick `amount` from the filtered set.
    pub(crate) fn pay_behold_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        exile: bool,
        prechosen: Option<&[CardId]>,
    ) {
        let chosen_cards = match prechosen {
            Some(picks) => picks.to_vec(),
            None => Self::choose_behold_cards(game, agents, player, source, type_filter, amount),
        };
        self.apply_behold_cost(game, agents, source, exile, chosen_cards);
    }

    pub(crate) fn choose_behold_cards(
        game: &GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) -> Vec<CardId> {
        let build_pool = |game: &GameState| -> Vec<CardId> {
            let mut valid: Vec<CardId> = game
                .cards_in_zone(ZoneType::Hand, player)
                .iter()
                .chain(game.cards_in_zone(ZoneType::Battlefield, player).iter())
                .copied()
                .collect();
            valid.retain(|&cid| {
                if cid == source {
                    return false;
                }
                type_filter == "Card"
                    || type_filter.is_empty()
                    || crate::ability::effects::matches_change_type(
                        game.card(cid),
                        type_filter,
                        &[],
                    )
            });
            valid
        };

        if type_filter.ends_with("ChosenType") {
            // Java two-phase approach: pick 1 first, then pick `amount` from
            // cards sharing a creature type with the first pick.
            let pool = build_pool(game);
            if pool.is_empty() {
                return Vec::new();
            }
            let first_pick = agents[player.index()].choose_cards_for_effect(player, &pool, 1, 1);
            if first_pick.is_empty() {
                return Vec::new();
            }
            let first = first_pick[0];
            let same_type: Vec<CardId> = pool
                .into_iter()
                .filter(|&cid| shares_creature_type(game, first, cid))
                .collect();
            if (same_type.len() as i32) < amount {
                return Vec::new();
            }
            agents[player.index()].choose_cards_for_effect(
                player,
                &same_type,
                amount as usize,
                amount as usize,
            )
        } else {
            // Non-ChosenType: pick `amount` cards at once.
            let pool = build_pool(game);
            if pool.is_empty() {
                return Vec::new();
            }
            agents[player.index()].choose_cards_for_effect(
                player,
                &pool,
                amount as usize,
                amount as usize,
            )
        }
    }

    fn apply_behold_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        source: CardId,
        exile: bool,
        chosen_cards: Vec<CardId>,
    ) {
        for chosen in chosen_cards {
            if exile {
                let origin = game.card(chosen).zone;
                let owner = game.card(chosen).owner;
                self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
                // Track the exile-with relationship so "Defined$ ExiledWith"
                // can find this card later (e.g. Champions of the Shoal's
                // leave-battlefield trigger returns the exiled card to hand).
                // We do NOT use `exiled_by` here because that field is reserved
                // for ChangeZoneAll Duration$ UntilHostLeavesPlay effects which
                // auto-return to battlefield in move_card.  BeholdExile cards
                // have their own dedicated leave-trigger to handle the return.
                game.card_mut(source).add_remembered_card(chosen);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    chosen,
                    origin,
                    ZoneType::Exile,
                );
            }
        }
    }

    /// Blight as a cost: put -1/-1 counters on creatures you control.
    pub(crate) fn blight_targets(game: &GameState, player: PlayerId) -> Vec<CardId> {
        game.cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .copied()
            .filter(|&card| {
                game.card(card).is_creature()
                    && !game.card(card).phased_out
                    && !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                        &game.cards,
                        game.card(card),
                        &crate::card::CounterType::M1M1,
                    )
            })
            .collect()
    }

    pub(crate) fn pay_blight_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        _source: CardId,
        amount: i32,
        cause: Option<&mut SpellAbility>,
        prechosen: Option<CardId>,
    ) {
        let chosen = match prechosen {
            Some(chosen) => Some(chosen),
            None => {
                let choices: Vec<crate::agent::GameEntity> = Self::blight_targets(game, player)
                    .into_iter()
                    .map(crate::agent::GameEntity::Card)
                    .collect();
                if choices.is_empty() {
                    return;
                }
                match agents[player.index()]
                    .choose_single_entity_for_effect(player, &choices, false)
                {
                    Some(crate::agent::GameEntity::Card(chosen)) => Some(chosen),
                    _ => None,
                }
            }
        };
        if let Some(chosen) = chosen {
            let mut table = crate::game_entity_counter_table::GameEntityCounterTable::default();
            table.put(
                Some(player),
                crate::agent::GameEntity::Card(chosen),
                crate::card::CounterType::M1M1,
                amount,
            );
            table.replace_counter_effect(
                game,
                Some(&mut self.trigger_handler),
                Some(agents),
                cause.as_deref(),
                false,
                Default::default(),
            );
            if let Some(sa) = cause {
                let id = chosen.0.to_string();
                sa.add_cost_to_hash_list(crate::cost::cost_put_counter::HASH_LKI, &id);
                sa.add_cost_to_hash_list(crate::cost::cost_put_counter::HASH_CARDS, &id);
            }
        }
    }

    fn exile_ctrl_or_grave_candidates(
        game: &GameState,
        player: PlayerId,
        source: CardId,
        type_filter: &str,
    ) -> Vec<CardId> {
        let base_filter = crate::cost::normalize_exile_base_filter(type_filter);
        let mut valid: Vec<CardId> = crate::cost::get_zone_targets(
            game,
            player,
            ZoneType::Battlefield,
            &base_filter,
            source,
        );
        valid.extend(crate::cost::get_zone_targets(
            game,
            player,
            ZoneType::Graveyard,
            &base_filter,
            source,
        ));
        valid.retain(|&cid| can_exile_for_cost(game, cid));
        valid
    }

    /// Exile cards from controller battlefield or graveyard (craft helper).
    pub(crate) fn pay_exile_ctrl_or_grave_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        prechosen: Option<&[CardId]>,
    ) -> bool {
        let chosen = match prechosen {
            Some(picks) => picks.to_vec(),
            None => {
                let valid = Self::exile_ctrl_or_grave_candidates(game, player, source, type_filter);
                match Self::choose_cost_cards_exactly(agents, player, &valid, amount) {
                    Some(picks) => picks,
                    None => return false,
                }
            }
        };
        for chosen in chosen {
            let origin = game.card(chosen).zone;
            let owner = game.card(chosen).owner;
            self.move_card_with_runtime(game, chosen, ZoneType::Exile, owner, agents);
            self.record_paid_cost_exile(game, source, chosen);
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                chosen,
                origin,
                ZoneType::Exile,
            );
        }
        true
    }

    /// Return `amount` permanents matching `type_filter` for `player` to hand.
    /// Mirrors Java's `CostReturn.doListPayment()`.
    pub(crate) fn pay_return_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        _source: CardId,
        type_filter: &str,
        amount: i32,
        sa: Option<&SpellAbility>,
        prechosen: Option<&[CardId]>,
    ) -> Option<Vec<CardId>> {
        let chosen = match prechosen {
            Some(picks) => picks.to_vec(),
            None => {
                let valid = cost::get_sacrifice_targets_for_cost(game, player, type_filter, sa);
                Self::choose_cost_cards_exactly(agents, player, &valid, amount)?
            }
        };
        for &chosen in &chosen {
            let owner = game.card(chosen).owner;
            let from_zone = game.card(chosen).zone;
            self.move_card_with_runtime(game, chosen, ZoneType::Hand, owner, agents);
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                chosen,
                from_zone,
                ZoneType::Hand,
            );
        }
        Some(chosen)
    }

    fn pay_return_cost_internal(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        _source: CardId,
        type_filter: &str,
        amount: i32,
        sa: Option<&SpellAbility>,
        prechosen_returns: Option<&[CardId]>,
        pre_return_idx: &mut usize,
        returned: &mut Vec<CardId>,
    ) -> bool {
        for _ in 0..amount {
            let valid = cost::get_sacrifice_targets_for_cost(game, player, type_filter, sa);
            if valid.is_empty() {
                return false;
            }
            let chosen = if let Some(prechosen) = prechosen_returns {
                if *pre_return_idx >= prechosen.len() {
                    return false;
                }
                let cid = prechosen[*pre_return_idx];
                *pre_return_idx += 1;
                if valid.contains(&cid) {
                    Some(cid)
                } else {
                    return false;
                }
            } else {
                agents[player.index()]
                    .choose_cards_for_effect(player, &valid, 1, 1)
                    .into_iter()
                    .next()
            };
            let Some(chosen) = chosen else {
                return false;
            };
            let owner = game.card(chosen).owner;
            let from_zone = game.card(chosen).zone;
            self.move_card_with_runtime(game, chosen, ZoneType::Hand, owner, agents);
            crate::ability::effects::emit_zone_trigger(
                &mut self.trigger_handler,
                chosen,
                from_zone,
                ZoneType::Hand,
            );
            returned.push(chosen);
        }
        true
    }

    /// Tap `amount` other permanents matching `type_filter` as cost.
    /// When `min_total_power` is Some(N), ask for any number of cards whose
    /// total power is at least N (Crew).
    /// Otherwise tap exactly `amount` matching permanents.
    /// Mirrors Java's `CostTapType.doListPayment()`.
    pub(crate) fn pay_tap_type_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        min_total_power: Option<i32>,
        can_tap_source: bool,
        sa: Option<&mut SpellAbility>,
        prechosen: Option<&[CardId]>,
    ) -> bool {
        let mut tapped_cards = Vec::new();
        if let Some(power_threshold) = min_total_power {
            let valid = cost::get_tap_type_targets_for_cost(
                game,
                player,
                type_filter,
                source,
                can_tap_source,
                sa.as_deref(),
            );
            if !valid.is_empty() {
                let card_powers: Vec<(CardId, i32)> = valid
                    .iter()
                    .map(|&cid| {
                        (
                            cid,
                            crate::cost::cost_tap_type::tap_power_value(game, cid, sa.as_deref()),
                        )
                    })
                    .collect();
                let card_sort_powers: Vec<(CardId, i32)> = valid
                    .iter()
                    .map(|&cid| (cid, game.card(cid).power()))
                    .collect();
                let mut chosen = agents[player.index()].choose_tap_type_for_cost(
                    player,
                    &valid,
                    power_threshold,
                    &card_powers,
                    &card_sort_powers,
                    sa.as_deref(),
                );
                chosen.retain(|cid| valid.contains(cid));
                chosen.dedup();

                let chosen_power: i32 = chosen
                    .iter()
                    .filter_map(|cid| {
                        card_powers
                            .iter()
                            .find(|(card_id, _)| card_id == cid)
                            .map(|(_, power)| *power)
                    })
                    .sum();
                if chosen_power < power_threshold {
                    return false;
                }

                let mut accum = 0;
                for cid in chosen {
                    if !valid.contains(&cid) || tapped_cards.contains(&cid) {
                        continue;
                    }
                    game.tap(cid);
                    tapped_cards.push(cid);
                    accum += crate::cost::cost_tap_type::tap_power_value(game, cid, sa.as_deref());
                    self.trigger_handler.run_trigger(
                        TriggerType::Taps,
                        RunParams {
                            card: Some(cid),
                            player: Some(player),
                            ..Default::default()
                        },
                        false,
                    );
                    if accum >= power_threshold {
                        break;
                    }
                }
            }
        } else {
            let valid = cost::get_tap_type_targets_for_cost(
                game,
                player,
                type_filter,
                source,
                can_tap_source,
                sa.as_deref(),
            );
            if valid.len() < amount.max(0) as usize {
                return false;
            }
            let chosen_cards = if let Some(picks) = prechosen {
                picks.to_vec()
            } else if type_filter == "OriginalHost" {
                valid.clone()
            } else if amount <= 0 {
                Vec::new()
            } else {
                agents[player.index()].choose_cards_for_effect(
                    player,
                    &valid,
                    amount.max(0) as usize,
                    amount.max(0) as usize,
                )
            };
            if chosen_cards.len() < amount.max(0) as usize {
                return false;
            }
            for chosen in chosen_cards.into_iter().take(amount.max(0) as usize) {
                if !valid.contains(&chosen) || tapped_cards.contains(&chosen) {
                    continue;
                }
                game.tap(chosen);
                tapped_cards.push(chosen);
                self.trigger_handler.run_trigger(
                    TriggerType::Taps,
                    RunParams {
                        card: Some(chosen),
                        player: Some(player),
                        ..Default::default()
                    },
                    false,
                );
            }
        }
        if let Some(sa) = sa {
            for cid in tapped_cards {
                let value = cid.to_string();
                sa.add_cost_to_hash_list(crate::cost::cost_tap_type::HASH_LKI, &value);
                sa.add_cost_to_hash_list(crate::cost::cost_tap_type::HASH_CARDS, &value);
            }
        }
        true
    }

    /// Untap `amount` tapped permanents matching `type_filter` as cost.
    /// Mirrors Java's `CostUntapType.doListPayment()`.
    pub(crate) fn pay_untap_type_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        can_untap_source: bool,
    ) {
        let mut untapped_ids: Vec<CardId> = Vec::new();
        for _ in 0..amount {
            let valid: Vec<CardId> = game
                .players
                .iter()
                .flat_map(|p| game.cards_in_zone(ZoneType::Battlefield, p.id).to_vec())
                .filter(|&cid| {
                    if !can_untap_source && cid == source {
                        return false;
                    }
                    let c = game.card(cid);
                    let stun = crate::card::CounterType::Named("STUN".to_string());
                    (type_filter == "Card"
                        || type_filter.is_empty()
                        || crate::ability::effects::matches_change_type(c, type_filter, &[]))
                        && c.can_untap()
                        && (c.counter_count(&stun) == 0 || c.can_remove_counters(&stun))
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            if let Some(chosen) =
                agents[player.index()].choose_sacrifice(player, &valid, Some(source))
            {
                let was_tapped = game.card(chosen).tapped;
                game.untap(chosen);
                if was_tapped {
                    untapped_ids.push(chosen);
                }
            }
        }
        for card_id in untapped_ids {
            self.emit_untap_all_cost_trigger(player, card_id);
        }
    }

    /// Gain control of `amount` permanents matching `type_filter` as cost.
    /// Mirrors Java's `CostGainControl.doPayment()` which calls `addTempController`.
    pub(crate) fn pay_gain_control_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        for _ in 0..amount {
            // Rebuild valid list each iteration so already-controlled permanents are excluded.
            let valid: Vec<CardId> = game
                .players
                .iter()
                .map(|p| p.id)
                .flat_map(|pid| game.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
                .filter(|&cid| {
                    game.card(cid).controller != player
                        && crate::ability::effects::matches_change_type(
                            game.card(cid),
                            type_filter,
                            &[],
                        )
                })
                .collect();
            if valid.is_empty() {
                break;
            }
            if let Some(chosen) =
                agents[player.index()].choose_sacrifice(player, &valid, Some(source))
            {
                game.change_controller(chosen, player);
            }
        }
    }

    /// Remove `amount` counters (of `counter_type` or any type if None) from permanents
    /// matching `type_filter` as cost. Mirrors Java's `CostRemoveAnyCounter.payAsDecided()`.
    pub(crate) fn pay_remove_any_counter_cost(
        &mut self,
        game: &mut GameState,
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
        counter_type: Option<&crate::card::CounterType>,
    ) {
        let mut remaining = amount;
        for card_id in
            crate::cost::cost_remove_any_counter::valid_cards(game, player, source, type_filter)
        {
            let types: Vec<crate::card::CounterType> = match counter_type {
                Some(ct) => vec![ct.clone()],
                None => game.card(card_id).counters.keys().cloned().collect(),
            };
            for ct in types {
                let remove = remaining.min(game.card(card_id).counter_count(&ct));
                for _ in 0..remove {
                    game.card_mut(card_id).remove_counter(&ct, 1);
                    let new_counter_amount = game.card(card_id).counter_count(&ct);
                    self.trigger_handler.run_trigger(
                        TriggerType::CounterRemoved,
                        RunParams {
                            card: Some(card_id),
                            player: Some(player),
                            counter_type: Some(ct.to_string()),
                            counter_amount: Some(1),
                            new_counter_amount: Some(new_counter_amount),
                            ..Default::default()
                        },
                        false,
                    );
                }
                remaining -= remove.max(0);
                if remaining <= 0 {
                    return;
                }
            }
        }
    }

    /// Move `amount` cards from exile to graveyard as cost.
    /// Mirrors Java's `CostExiledMoveToGrave.doPayment()`.
    pub(crate) fn pay_exiled_move_to_grave_cost(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        source: CardId,
        type_filter: &str,
        amount: i32,
    ) {
        for _ in 0..amount {
            let valid = cost::get_exiled_targets(game, type_filter);
            if valid.is_empty() {
                break;
            }
            if let Some(chosen) =
                agents[player.index()].choose_sacrifice(player, &valid, Some(source))
            {
                let owner = game.card(chosen).owner;
                self.move_card_with_runtime(game, chosen, ZoneType::Graveyard, owner, agents);
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    chosen,
                    ZoneType::Exile,
                    ZoneType::Graveyard,
                );
            }
        }
    }

    fn pay_sacrifice_cost_internal(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        type_filter: &str,
        amount: i32,
        sa: Option<&mut SpellAbility>,
        prechosen_sacrifices: Option<&[CardId]>,
        pre_sac_idx: &mut usize,
    ) -> bool {
        // Collect chosen cards across all iterations, then sacrifice as one batch.
        // Batching matters for `SacrificedOnce` (Tasteful Offering / Camellia) which
        // Java fires once per controller per call, not once per card.
        let mut to_sacrifice: Vec<CardId> = Vec::with_capacity(amount.max(0) as usize);
        for _ in 0..amount {
            let valid =
                cost::get_sacrifice_targets_for_cost(game, player, type_filter, sa.as_deref());
            if valid.is_empty() {
                return false;
            }
            let chosen = if let Some(prechosen) = prechosen_sacrifices {
                if *pre_sac_idx >= prechosen.len() {
                    return false;
                }
                let cid = prechosen[*pre_sac_idx];
                *pre_sac_idx += 1;
                if valid.contains(&cid) {
                    Some(cid)
                } else if game.card(cid).zone != ZoneType::Battlefield {
                    // Java's activated-cost autopay can reserve a permanent for a
                    // sacrifice cost, then use that same permanent's CARDNAME
                    // mana ability to pay the mana part. At payAsDecided time the
                    // reserved permanent is already gone; treat that prechosen
                    // sacrifice as consumed instead of failing or choosing another.
                    continue;
                } else {
                    return false;
                }
            } else {
                agents[player.index()].choose_sacrifice(
                    player,
                    &valid,
                    sa.as_deref().and_then(|s| s.source),
                )
            };
            if let Some(chosen) = chosen {
                to_sacrifice.push(chosen);
            } else {
                return false;
            }
        }
        if !to_sacrifice.is_empty() {
            super::perform_sacrifice(game, &mut self.replacement_runtime(), agents, &to_sacrifice);
            Self::record_sacrificed_cost_cards(sa, &to_sacrifice);
        }
        true
    }

    fn handle_change_zone_trigger(&mut self, game: &mut GameState, sa: Option<&SpellAbility>) {
        if let Some(table) = game.pending_change_zone_table.take() {
            table.trigger_changes_zone_all(&mut self.trigger_handler, game, sa);
        }
    }

    fn record_sacrificed_cost_cards(sa: Option<&mut SpellAbility>, cards: &[CardId]) {
        let Some(sa) = sa else {
            return;
        };
        for card in cards {
            let id = card.0.to_string();
            sa.add_cost_to_hash_list(crate::cost::cost_sacrifice::HASH_CARDS, &id);
            sa.add_cost_to_hash_list(crate::cost::cost_sacrifice::HASH_LKI, &id);
        }
    }

    pub(crate) fn resolve_auto_tapped_mana_sub_abilities(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        trace: &[crate::mana::AutoTapChoice],
    ) {
        for choice in trace {
            let crate::mana::AutoTapChoice {
                card_id,
                mana_ability_index: Some(ability_index),
                ..
            } = *choice
            else {
                continue;
            };
            self.resolve_mana_sub_ability(game, agents, player, card_id, ability_index);
        }
    }

    pub(crate) fn resolve_mana_sub_ability(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        player: PlayerId,
        card_id: CardId,
        ability_index: usize,
    ) {
        let Some(sub_svar_name) = game
            .card(card_id)
            .activated_abilities
            .get(ability_index)
            .and_then(|ab| ab.sub_ability.as_deref())
            .map(str::to_string)
        else {
            return;
        };
        let Some(sub_text) = game
            .card(card_id)
            .get_s_var(&sub_svar_name)
            .map(str::to_string)
        else {
            return;
        };
        let sub_sa = crate::spellability::build_spell_ability(game, card_id, &sub_text, player);
        self.resolve_single_effect(game, agents, &sub_sa, None);
        self.mark_mana_undo_disqualified();
    }
}

fn shares_creature_type(game: &GameState, a: CardId, b: CardId) -> bool {
    let ca = game.card(a);
    let cb = game.card(b);
    ca.shares_creature_type_with(cb)
}

fn can_exile_for_cost(game: &GameState, card_id: CardId) -> bool {
    !crate::staticability::static_ability_cant_exile::cant_exile(
        &game.cards,
        game.card(card_id),
        None,
        true,
    )
}
