use forge_foundation::ZoneType;

use super::{emit_zone_trigger, resolve_numeric_svar, EffectContext};
use crate::agent::{notify_all_agents, GameLogEvent};
use crate::card::card_zone_table::CardZoneTable;
use crate::parsing::keys;

/// Mirrors Java's `DigEffect.java`.
///
/// `SP$ Dig | DigNum$ N | ChangeNum$ K | DestinationZone$ Hand | DestinationZone2$ Library`
///
/// Looks at the top N cards of the target player's library.
/// The activating player chooses up to K of them and moves them to DestinationZone (default Hand).
/// The rest go to DestinationZone2 (default Library bottom).
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DigEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DigEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let mut chooser = sa.activating_player;
    for dig_player in
        crate::ability::spell_ability_effect::get_defined_players_or_targeted(ctx.game, sa)
    {
        resolve_for_player(ctx, sa, dig_player, &mut chooser);
    }
}

fn resolve_for_player(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    dig_player: crate::ids::PlayerId,
    chooser: &mut crate::ids::PlayerId,
) {
    let dig_num = resolve_numeric_svar(ctx.game, sa, "DigNum", 1).max(0) as usize;
    let optional = sa.ir.optional;
    let skip_reorder = sa.ir.skip_reorder;
    let rest_random_order = sa.ir.rest_random_order;
    let change_all = sa
        .ir
        .change_num_text
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("All"))
        .unwrap_or(false);
    let any_number = sa
        .ir
        .change_num_text
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("Any"))
        .unwrap_or(false);
    let change_num = if change_all || any_number {
        dig_num
    } else {
        resolve_numeric_svar(ctx.game, sa, keys::CHANGE_NUM, 1).max(0) as usize
    };

    let dest_zone1 = sa.destination_zone().unwrap_or(ZoneType::Hand);
    let lib_position1: i32 = sa
        .library_position()
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);
    let dest_zone2 = sa.ir.destination_zone_2.unwrap_or(ZoneType::Library);

    // Library position for zone2 placement: -1 = bottom, 0 = top
    let lib_position2: i32 = sa
        .library_position_2()
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);

    let change_valid = sa
        .ir
        .change_valid
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let lib_len = ctx.game.cards_in_zone(ZoneType::Library, dig_player).len();
    if lib_len == 0 {
        return;
    }

    let count = dig_num.min(lib_len);

    // Take top N cards off the library.
    let mut top_n = ctx
        .game
        .take_top_cards_from_zone(ZoneType::Library, dig_player, count);
    // Java DigEffect iterates top cards in top-first order.
    // Our library uses index 0 = bottom, so split_off returns deepest->top.
    // Reverse to expose the same chooser order Java uses.
    top_n.reverse();

    // Filter valid choices by ChangeValid$ (e.g. "Creature").
    let valid: Vec<_> = if change_valid.is_empty() {
        top_n.clone()
    } else {
        top_n
            .iter()
            .copied()
            .filter(|&id| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(id),
                    None,
                    &change_valid,
                )
            })
            .collect()
    };

    if let Some(choser) = crate::parsing::raw_get(&sa.ability_text, "Choser") {
        let activator = sa.activating_player;
        let choosers = crate::ability::ability_utils::resolve_defined_players_with_sa(
            choser, sa, activator, ctx.game,
        );
        if !choosers.is_empty() {
            let entities: Vec<crate::agent::GameEntity> = choosers
                .into_iter()
                .map(crate::agent::GameEntity::Player)
                .collect();
            ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
            if let Some(crate::agent::GameEntity::Player(pid)) = ctx.agents[activator.index()]
                .choose_single_entity_for_effect(activator, &entities, false)
            {
                *chooser = pid;
            }
        }
        if crate::parsing::raw_has_key(&sa.ability_text, "SetChosenPlayer") {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).set_chosen_player(
                    Some(*chooser),
                    Some(activator),
                    true,
                );
            }
        }
    }
    let chooser = *chooser;

    // Java DigEffect only prompts for optional skip when PromptToSkipOptionalAbility is set.
    // Otherwise Optional$ True is modeled by allowing 0 selected cards in choose_dig.
    let may_be_skipped = sa.ir.prompt_to_skip_optional_ability;
    if optional && may_be_skipped && !valid.is_empty() {
        let _source_name = sa.source.map(|cid| ctx.game.card(cid).card_name.clone());
        let prompt = sa
            .ir
            .optional_ability_prompt
            .as_deref()
            .unwrap_or("Would you like to proceed with this optional ability?");
        let accepted = ctx.agents[dig_player.index()].confirm_action(
            dig_player,
            None,
            prompt,
            &[],
            sa.source,
            Some(crate::ability::api_type::ApiType::Dig),
        );
        if !accepted {
            // Put cards back into library — reverse to restore original deepest→top order.
            top_n.reverse();
            for card_id in top_n {
                ctx.game
                    .add_card_to_zone(ZoneType::Library, dig_player, card_id);
            }
            return;
        }
    }

    if sa.ir.reveal_true {
        for &card_id in &top_n {
            notify_all_agents(
                ctx.agents,
                GameLogEvent::rule("Reveal Library cards")
                    .with_player(dig_player)
                    .with_card(card_id),
            );
        }
    }

    // Ask the chooser (activating player) which cards to take.
    // Java DigEffect skips the prompt entirely when no valid cards exist,
    // so we must also skip to avoid consuming extra RNG.
    let max_take = change_num.min(valid.len());
    let chosen = if change_all {
        valid.clone()
    } else if valid.is_empty() {
        Vec::new()
    } else if let Some(mut totcmc) = sa.ir.with_total_cmc {
        let mut valid_cmc: Vec<_> = valid
            .iter()
            .copied()
            .filter(|&id| ctx.game.card(id).mana_value() <= totcmc)
            .collect();
        let mut moved = Vec::new();
        while !valid_cmc.is_empty() && (any_number || moved.len() < change_num) {
            let options: Vec<crate::agent::GameEntity> = valid_cmc
                .iter()
                .map(|&id| crate::agent::GameEntity::Card(id))
                .collect();
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let Some(crate::agent::GameEntity::Card(chosen)) = ctx.agents[chooser.index()]
                .choose_single_entity_for_effect(chooser, &options, any_number || optional)
            else {
                break;
            };
            moved.push(chosen);
            totcmc -= ctx.game.card(chosen).mana_value();
            valid_cmc.retain(|&id| id != chosen && ctx.game.card(id).mana_value() <= totcmc);
        }
        moved
    } else {
        ctx.agents[chooser.index()].choose_dig(
            ctx.game,
            chooser,
            &valid,
            max_take,
            optional || any_number,
        )
    };

    let mut chosen: Vec<_> = chosen
        .into_iter()
        .filter(|id| valid.contains(id))
        .take(max_take)
        .collect();
    // Java reverses moved cards before moving them so the final destination
    // order matches the chooser's intended top-first order.
    chosen.reverse();
    if dest_zone1 == ZoneType::Battlefield || dest_zone1 == ZoneType::Library {
        chosen =
            ctx.game
                .order_cards_by_their_owners(chosen, dest_zone1, &mut Some(&mut *ctx.agents));
    }

    let mut rest: Vec<_> = top_n
        .iter()
        .copied()
        .filter(|id| !chosen.contains(id))
        .collect();

    // `DestZone2Optional$ True` — the player says whether the rest go to the second zone
    // (Java `DigEffect.java` line 436).
    if !rest.is_empty() && crate::parsing::raw_has_key(&sa.ability_text, "DestZone2Optional") {
        let decider = sa.activating_player;
        ctx.agents[decider.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[decider.index()].confirm_action(
            decider,
            None,
            &format!("Do you want to put that card into {dest_zone2:?}?"),
            &[],
            sa.source,
            sa.api,
        ) {
            // The dig already took these off the library, so declining puts them back on top
            // in their original order; Java never moved them.
            let mut back = std::mem::take(&mut rest);
            back.reverse();
            for card_id in back {
                ctx.game
                    .add_card_to_zone(ZoneType::Library, dig_player, card_id);
            }
        }
    }

    // `RestRandomOrder$ True` — Java Forge (`DigEffect.java` line 437) calls
    // `Collections.shuffle(afterOrder, MyRandom.getRandom())` on the leftover
    // list before moving each card to `dest_zone2`. We must consume the same
    // `nextInt(N-1), nextInt(N-2), ..., nextInt(1)` sequence on the game RNG
    // so subsequent shuffles (e.g. a following Farseek) stay in sync with
    // Java, and produce the same permutation of the rest cards.
    //
    // NOTE: `GameRng::shuffle_cards` does a reverse/shuffle/reverse for
    // library orientation, which is *not* what `Collections.shuffle` does on
    // a generic list. We apply Java's Fisher-Yates directly via `next_int`.
    if rest_random_order && rest.len() > 1 {
        for i in (1..rest.len()).rev() {
            let j = ctx.rng.next_int((i + 1) as i32) as usize;
            rest.swap(i, j);
        }
    } else if !skip_reorder
        && rest.len() > 1
        && (dest_zone2 == ZoneType::Library || dest_zone2 == ZoneType::Graveyard)
    {
        ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let reordered =
            ctx.agents[chooser.index()].choose_reorder_library(ctx.game, chooser, &rest);
        if reordered.len() == rest.len() && rest.iter().all(|id| reordered.contains(id)) {
            rest = reordered;
        }
    }

    let mut zone_movements = CardZoneTable::default();

    // Move chosen cards to dest_zone1.
    for &id in &chosen {
        let owner = ctx.game.card(id).owner;
        let dest_owner = if dest_zone1 == ZoneType::Battlefield {
            sa.activating_player
        } else {
            owner
        };
        ctx.move_card(id, dest_zone1, dest_owner);
        zone_movements.put(Some(ZoneType::Library), Some(dest_zone1), id);
        if dest_zone1 == ZoneType::Library {
            match lib_position1 {
                pos if pos < 0 => {
                    ctx.game
                        .reorder_card_in_zone(ZoneType::Library, dest_owner, id, 0)
                }
                0 => {}
                pos => {
                    let len = ctx.game.cards_in_zone(ZoneType::Library, dest_owner).len();
                    let from_top = pos as usize;
                    let index = len.saturating_sub(from_top + 1);
                    ctx.game
                        .reorder_card_in_zone(ZoneType::Library, dest_owner, id, index);
                }
            }
        }
        if dest_zone1 == ZoneType::Exile && !ctx.game.card(id).is_token {
            if let Some(source_id) = sa.source {
                if matches!(
                    ctx.game.card(source_id).zone,
                    ZoneType::Battlefield | ZoneType::Stack | ZoneType::Command
                ) {
                    ctx.game.card_mut(source_id).add_exiled_card(id);
                } else if sa
                    .trigger_source_zone_timestamp
                    .or(sa.source_zone_timestamp)
                    .is_some_and(|timestamp| timestamp != ctx.game.card(source_id).zone_timestamp)
                {
                    ctx.game.add_lki_exiled_card(source_id, id);
                }
            }
        }
        if sa.is_exile_face_down() {
            ctx.game.card_mut(id).set_face_down(true);
        }
        if sa.param_is_true(keys::IMPRINT) {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).add_imprinted_card(id);
            }
        }
        if sa.is_remember_changed() {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).add_remembered_card(id);
            }
        }
        if dest_zone1 == ZoneType::Battlefield {
            if sa.ir.tapped {
                ctx.game.tap(id);
            }
            ctx.trigger_handler.register_active_trigger(ctx.game, id);
            let _ = super::add_to_combat(ctx, sa, id, keys::ATTACKING);
        }
        emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, dest_zone1);
    }

    // Move rest to dest_zone2.
    for &id in &rest {
        let owner = ctx.game.card(id).owner;
        if dest_zone2 == ZoneType::Library {
            // Put back into the library at the specified position.
            // lib_position2 == -1 means bottom (index 0), 0 means top.
            if lib_position2 == 0 {
                // top of library
                ctx.game.add_card_to_zone(ZoneType::Library, owner, id);
                ctx.game.card_mut(id).set_zone(ZoneType::Library);
            } else {
                // bottom of library
                ctx.game
                    .add_card_to_zone_bottom(ZoneType::Library, owner, id);
                ctx.game.card_mut(id).set_zone(ZoneType::Library);
            }
            zone_movements.put(Some(ZoneType::Library), Some(ZoneType::Library), id);
        } else {
            let dest_owner = if dest_zone2 == ZoneType::Battlefield {
                sa.activating_player
            } else {
                owner
            };
            ctx.move_card(id, dest_zone2, dest_owner);
            zone_movements.put(Some(ZoneType::Library), Some(dest_zone2), id);
            if dest_zone2 == ZoneType::Battlefield {
                ctx.trigger_handler.register_active_trigger(ctx.game, id);
            }
            emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, dest_zone2);
        }
    }

    if !zone_movements.all_cards().is_empty() {
        zone_movements.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
    }
}

#[cfg(test)]
mod tests {
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    use crate::ability::effects::EffectContext;
    use crate::agent::{PassAgent, PlayerAgent};
    use crate::card::Card;
    use crate::combat::DefenderId;
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::handler::TriggerHandler;
    use crate::HashMap;

    fn make_land(game: &mut GameState, owner: PlayerId) -> CardId {
        let c = Card::new(
            CardId(0),
            "Island".into(),
            owner,
            CardTypeLine::parse("Basic Land Island"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            None,
            None,
            vec![],
            vec![],
        );
        game.create_card(c)
    }

    /// Agent that always picks the first card offered during dig.
    struct TakeFirstAgent;
    impl PlayerAgent for TakeFirstAgent {
        fn mulligan_decision(&mut self, _: PlayerId, _: &[CardId], _: u32) -> bool {
            true
        }
        fn choose_action(
            &mut self,
            player: PlayerId,
            action_space: Option<&crate::agent::PriorityActionSpace>,
            request_action_space: &mut dyn FnMut() -> crate::agent::PriorityActionSpace,
        ) -> crate::player::actions::PlayerAction {
            crate::player::actions::PlayerAction::PassPriority
        }
        fn choose_attackers(
            &mut self,
            _: PlayerId,
            _: &[CardId],
            _: &[DefenderId],
        ) -> Vec<(CardId, DefenderId)> {
            vec![]
        }
        fn choose_blockers(
            &mut self,
            _: PlayerId,
            _: &[CardId],
            _: &[CardId],
            _: Option<usize>,
        ) -> Vec<(CardId, CardId)> {
            vec![]
        }
        fn choose_target_player(
            &mut self,
            _: PlayerId,
            v: &[PlayerId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> Option<PlayerId> {
            v.first().copied()
        }
        fn choose_target_card(
            &mut self,
            _: PlayerId,
            v: &[CardId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> Option<CardId> {
            v.first().copied()
        }
        fn choose_target_any(
            &mut self,
            _: PlayerId,
            vp: &[PlayerId],
            vc: &[CardId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> crate::agent::TargetChoice {
            vp.first()
                .copied()
                .map(crate::agent::TargetChoice::Player)
                .or_else(|| vc.first().copied().map(crate::agent::TargetChoice::Card))
                .unwrap_or(crate::agent::TargetChoice::None)
        }
        fn choose_land_or_spell(&mut self, _: PlayerId) -> Option<bool> {
            None
        }
        fn choose_dig(
            &mut self,
            _game: &GameState,
            _player: PlayerId,
            cards: &[CardId],
            max: usize,
            _optional: bool,
        ) -> Vec<CardId> {
            cards.iter().copied().take(max).collect()
        }
        fn choose_targets_for(
            &mut self,
            _sa: &mut SpellAbility,
            _game: &GameState,
            _mana_pools: &[ManaPool],
        ) -> bool {
            false
        }
    }

    #[test]
    fn dig_moves_chosen_to_hand() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);

        let a = make_land(&mut game, p0);
        let b = make_land(&mut game, p0);
        let c = make_land(&mut game, p0);
        // Library (bottom→top): a, b, c  → c is on top
        game.replace_zone_cards(ZoneType::Library, p0, vec![a, b, c]);

        // Dig 3, take 1 to hand, rest go to graveyard.
        let sa = SpellAbility::new_simple(
            None,
            p0,
            "SP$ Dig | DigNum$ 3 | ChangeNum$ 1 | DestinationZone2$ Graveyard | NoReveal$ True",
        );
        let mut trigger_handler = TriggerHandler::new();
        let mut agents: Vec<Box<dyn PlayerAgent>> =
            vec![Box::new(TakeFirstAgent), Box::new(PassAgent)];
        let mut mana_pools = vec![ManaPool::default(), ManaPool::default()];
        let token_templates = HashMap::default();
        let templates_variants: HashMap<(String, String), usize> = HashMap::default();
        let token_fallback: HashMap<String, String> = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter::default();
        let mut ctx = EffectContext {
            game: &mut game,
            combat: None,
            agents: &mut agents,
            trigger_handler: &mut trigger_handler,
            token_templates: &token_templates,
            token_art_variants: &templates_variants,
            token_fallback: &token_fallback,
            edition_dates: &edition_dates,
            mana_pools: &mut mana_pools,
            parent_target_card: None,
            rng: &mut rng_adapter,
        };

        super::DigEffect::resolve(&mut ctx, &sa);

        // 1 card goes to hand, 2 go to graveyard.
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Hand, p0).len(), 1);
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Graveyard, p0).len(), 2);
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Library, p0).len(), 0);
    }
}
