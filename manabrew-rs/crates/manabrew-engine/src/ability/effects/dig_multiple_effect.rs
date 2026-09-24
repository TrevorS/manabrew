use forge_foundation::ZoneType;

use super::{emit_zone_trigger, resolve_numeric_svar, EffectContext};
use crate::ids::CardId;

/// Mirrors Java's `DigEffect.java`.
///
/// `SP$ Dig | DigNum$ N | ChangeNum$ K | DestinationZone$ Hand | DestinationZone2$ Library`
///
/// Looks at the top N cards of the target player's library.
/// The activating player chooses up to K of them and moves them to DestinationZone (default Hand).
/// The rest go to DestinationZone2 (default Library bottom).
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DigMultipleEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DigMultipleEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let dig_num = resolve_numeric_svar(ctx.game, sa, "DigNum", 1).max(0) as usize;
    let optional = sa.ir.optional;

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

    // Determine the player whose library we dig through.
    let Some(dig_player) =
        crate::ability::spell_ability_effect::get_defined_players_or_targeted(ctx.game, sa)
            .into_iter()
            .next()
    else {
        return;
    };

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

    let pools: Vec<Vec<CardId>> = java_hash_map_order(change_valid.split(','))
        .into_iter()
        .filter_map(|valid| {
            let list: Vec<CardId> = top_n
                .iter()
                .copied()
                .filter(|&id| {
                    super::helpers::matches_valid_cards_for_sa(
                        ctx.game,
                        sa,
                        ctx.game.card(id),
                        None,
                        valid,
                    )
                })
                .collect();
            (!list.is_empty()).then_some(list)
        })
        .collect();
    let valid: Vec<CardId> = pools.iter().flatten().copied().collect();

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

    let chosen: Vec<CardId> = if pools.is_empty() {
        Vec::new()
    } else {
        ctx.agents[dig_player.index()].snapshot_state(ctx.game, ctx.mana_pools);
        ctx.agents[dig_player.index()]
            .choose_cards_for_effect_multiple(dig_player, &pools, optional)
            .into_iter()
            .filter(|id| valid.contains(id))
            .collect()
    };

    let mut rest: Vec<_> = top_n
        .iter()
        .copied()
        .filter(|id| !chosen.contains(id))
        .collect();

    // Move chosen cards to dest_zone1.
    for &id in &chosen {
        let owner = ctx.game.card(id).owner;
        let dest_owner = if dest_zone1 == ZoneType::Battlefield {
            sa.activating_player
        } else {
            owner
        };
        ctx.move_card(id, dest_zone1, dest_owner);
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
        emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, dest_zone1);
    }

    if dest_zone2 == ZoneType::Library || dest_zone2 == ZoneType::Graveyard {
        if sa.ir.rest_random_order {
            for i in (1..rest.len()).rev() {
                let j = ctx.rng.next_int((i + 1) as i32) as usize;
                rest.swap(i, j);
            }
        }
        if lib_position2 != -1 {
            rest.reverse();
        }
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
        } else {
            let dest_owner = if dest_zone2 == ZoneType::Battlefield {
                sa.activating_player
            } else {
                owner
            };
            ctx.move_card(id, dest_zone2, dest_owner);
            emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, dest_zone2);
        }
    }
}

/// Keep in sync with Java's `HashMap<String, _>` iteration: 16 buckets by the spread
/// `String.hashCode`, insertion order within a bucket.
fn java_hash_map_order<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut keyed: Vec<(usize, usize, &str)> = Vec::new();
    for key in keys {
        if keyed.iter().any(|(_, _, k)| *k == key) {
            continue;
        }
        let h = key
            .encode_utf16()
            .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
        let spread = (h ^ ((h as u32) >> 16) as i32) as u32;
        keyed.push(((spread & 15) as usize, keyed.len(), key));
    }
    keyed.sort_by_key(|(bucket, order, _)| (*bucket, *order));
    keyed.into_iter().map(|(_, _, key)| key).collect()
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
            _player: PlayerId,
            _action_space: Option<&crate::agent::PriorityActionSpace>,
            _request_action_space: &mut dyn FnMut() -> crate::agent::PriorityActionSpace,
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

        super::DigMultipleEffect::resolve(&mut ctx, &sa);

        // 1 card goes to hand, 2 go to graveyard.
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Hand, p0).len(), 1);
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Graveyard, p0).len(), 2);
        assert_eq!(ctx.game.cards_in_zone(ZoneType::Library, p0).len(), 0);
    }
}
