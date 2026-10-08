use super::{GameLoop, PlayEffectCast};
use crate::ability::effects::play_effect;
use crate::agent::{DecisionContext, PlayerAgent};
use crate::game::GameState;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

impl GameLoop {
    pub(crate) fn resolve_play_effect(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        sa: &SpellAbility,
        parent_target_card: Option<CardId>,
    ) {
        let mut candidates =
            play_effect::get_tgt_cards(&self.effect_context(game, agents, parent_target_card), sa);
        if candidates.is_empty() {
            return;
        }

        let controller = sa.activating_player;
        let valid_sa = crate::parsing::raw_get(&sa.ability_text, crate::parsing::keys::VALID_SA)
            .map(|filter| (filter, sa));
        let without_mana_cost = sa.ir.without_mana_cost;
        let play_cost = sa.ir.play_cost_text.as_deref().map(crate::cost::parse_cost);
        let mana_conversion =
            crate::parsing::raw_get(&sa.ability_text, "ManaConversion").map(str::to_string);
        let (is_madness, mut amount) = {
            let ctx = self.effect_context(game, agents, parent_target_card);
            (
                play_effect::is_madness(&ctx, sa),
                play_effect::play_amount(&ctx, sa, candidates.len()),
            )
        };
        let single_option = candidates.len() == 1 && amount == 1 && sa.ir.optional;
        let mut turned_face_up = None;
        while !candidates.is_empty() && amount > 0 {
            play_effect::turn_unplayed_card_face_down(game, turned_face_up.take());
            let Some(card_id) = play_effect::choose_card_to_play(
                &mut self.effect_context(game, agents, parent_target_card),
                sa,
                &candidates,
                single_option,
            ) else {
                break;
            };
            turned_face_up = play_effect::turn_chosen_card_face_up(game, card_id);
            candidates.retain(|&cid| cid != card_id);
            let card_id = play_effect::copy_card_to_play(
                &mut self.effect_context(game, agents, parent_target_card),
                sa,
                card_id,
            );

            let abilities = crate::ability::ability_utils::get_spells_from_play_effect(
                game,
                card_id,
                controller,
                !without_mana_cost && play_cost.is_none(),
                valid_sa,
            );
            if abilities.is_empty() {
                continue;
            }
            let Some(mut tgt_sa) = agents[controller.index()]
                .get_ability_to_play(
                    DecisionContext::new(game, &self.mana_pools),
                    controller,
                    &abilities,
                )
                .and_then(|idx| abilities.into_iter().nth(idx))
            else {
                continue;
            };

            if tgt_sa.is_land_ability {
                play_effect::play_land(
                    &mut self.effect_context(game, agents, parent_target_card),
                    controller,
                    card_id,
                    &tgt_sa,
                );
                amount -= 1;
                play_effect::remember_played(game, sa, card_id);
                continue;
            }

            if !without_mana_cost
                && play_cost.is_none()
                && game.card(card_id).mana_cost.is_no_cost()
            {
                continue;
            }
            if is_madness {
                tgt_sa.alt_cost = Some(crate::spellability::AlternativeCost::Madness);
            }
            let rollback_effect = sa.ir.replace_graveyard.clone().map(|zone| {
                let host_card = sa.source.unwrap_or(card_id);
                play_effect::add_replace_graveyard_effect(
                    &mut self.effect_context(game, agents, parent_target_card),
                    card_id,
                    host_card,
                    sa,
                    &zone,
                )
            });

            let play = PlayEffectCast {
                without_mana_cost,
                play_cost: play_cost.clone(),
                mana_conversion: mana_conversion.clone(),
                zone: game.card(card_id).zone,
            };
            if self.play_sa_from_play_effect(game, agents, controller, tgt_sa, &play) {
                play_effect::remember_played(game, sa, card_id);
            } else {
                if let Some(effect) = rollback_effect {
                    crate::phase::PhaseCommand::ExileEffect { effect }
                        .run(game, &mut *self.game_rng);
                }
            }
            amount -= 1;
        }
        play_effect::turn_unplayed_card_face_down(game, turned_face_up);
    }
}
