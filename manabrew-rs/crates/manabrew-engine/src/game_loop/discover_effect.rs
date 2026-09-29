use super::{GameLoop, PlayEffectCast};
use crate::ability::effects::discover_effect;
use crate::agent::{DecisionContext, PlayerAgent};
use crate::game::GameState;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

impl GameLoop {
    pub(crate) fn resolve_discover_effect(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        sa: &SpellAbility,
        parent_target_card: Option<CardId>,
    ) {
        let num = discover_effect::discover_num(
            &self.effect_context(game, agents, parent_target_card),
            sa,
        );
        for player in
            crate::ability::spell_ability_effect::get_defined_players_or_targeted(game, sa)
        {
            let (found, rest) = discover_effect::exile_until_found(
                &mut self.effect_context(game, agents, parent_target_card),
                sa,
                player,
                num,
            );
            if let Some(card_id) = found {
                let play = discover_effect::confirm_cast(
                    &mut self.effect_context(game, agents, parent_target_card),
                    card_id,
                    player,
                );
                let mut cancel = false;
                if play {
                    let sas: Vec<SpellAbility> =
                        crate::ability::ability_utils::get_spells_from_play_effect(
                            game, card_id, player, false, None,
                        )
                        .into_iter()
                        .filter(|sp| !sp.is_land_ability)
                        .filter(|sp| {
                            sp.pay_costs
                                .as_ref()
                                .map_or(game.card(card_id).mana_cost.cmc(), |cost| {
                                    Self::mana_from_cost(cost).cmc()
                                })
                                <= num
                        })
                        .collect();
                    if !sas.is_empty() {
                        match agents[player.index()]
                            .get_ability_to_play(
                                DecisionContext::new(game, &self.mana_pools),
                                player,
                                &sas,
                            )
                            .and_then(|idx| sas.into_iter().nth(idx))
                        {
                            Some(tgt_sa) => {
                                let cast = PlayEffectCast {
                                    without_mana_cost: true,
                                    play_cost: None,
                                    mana_conversion: None,
                                    zone: game.card(card_id).zone,
                                };
                                self.play_sa_from_play_effect(game, agents, player, tgt_sa, &cast);
                            }
                            None => cancel = true,
                        }
                    }
                }
                if !play || cancel {
                    discover_effect::put_into_hand(
                        &mut self.effect_context(game, agents, parent_target_card),
                        card_id,
                        player,
                    );
                }
            }
            let mut ctx = self.effect_context(game, agents, parent_target_card);
            discover_effect::put_rest_on_bottom(&mut ctx, rest, player);
            discover_effect::run_discover_trigger(&mut ctx, player, num);
        }
    }
}
