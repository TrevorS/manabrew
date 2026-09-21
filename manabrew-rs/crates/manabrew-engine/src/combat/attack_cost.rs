//! Attack cost computation (Propaganda, Ghostly Prison, etc.).
//!
//! Mirrors Java Forge's `CombatUtil.getAttackCost()` — scans battlefield for
//! `CantAttackUnless` static abilities and accumulates the mana cost an
//! attacker must pay to attack a given defender.

use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::combat::DefenderId;
use crate::staticability::StaticMode;

/// Compute the total generic mana cost required for `attacker` to attack `defender`.
///
/// Returns the accumulated cost as a generic mana amount, or 0 if no cost.
/// Scans all battlefield permanents for `Mode$ CantAttackUnless` statics.
///
/// Card script example (Propaganda):
/// ```text
/// S:Mode$ CantAttackUnless | ValidCard$ Creature | Target$ You | Cost$ 2
/// ```
pub fn get_attack_cost(
    game: &crate::game::GameState,
    attacker: &Card,
    defender: DefenderId,
) -> i32 {
    let cards = &game.cards;
    let mut total_cost = 0;

    for source in cards.iter().filter(|c| c.zone == ZoneType::Battlefield) {
        for sa in &source.static_abilities {
            if !sa.check_mode(&StaticMode::CantAttackUnless) {
                continue;
            }

            // Java `StaticAbility.getAttackCost:310` bails on checkConditions, which is
            // what makes Archangel of Tithes stop taxing once it is tapped (IsPresent$).
            if !sa.check_conditions(source, game) {
                continue;
            }

            // Check ValidCard$ matches the attacker
            if !valid_filter::matches_valid_card_selector_opt(
                sa.ir.valid_card.as_ref(),
                attacker,
                source,
            ) {
                continue;
            }

            // Check Target$ — who is being defended
            if let Some(target_param) = sa.ir.target_text.as_deref() {
                match target_param {
                    "You" => {
                        // Only applies when attacking the source's controller
                        let defends_controller = match defender {
                            DefenderId::Player(pid) => pid == source.controller,
                            DefenderId::Permanent(cid) => {
                                cards[cid.index()].controller == source.controller
                            }
                        };
                        if !defends_controller {
                            continue;
                        }
                    }
                    // Java `matchesValidParam("Target", target)` is a valid string, not a
                    // fixed word, so `You,Planeswalker.YouCtrl` only covers those defenders.
                    other => {
                        let matched = match defender {
                            DefenderId::Player(pid) => valid_filter::matches_valid(
                                other,
                                None,
                                Some(pid),
                                source,
                                source.controller,
                            ),
                            DefenderId::Permanent(cid) => valid_filter::matches_valid(
                                other,
                                Some(&cards[cid.index()]),
                                None,
                                source,
                                source.controller,
                            ),
                        };
                        if !matched {
                            continue;
                        }
                    }
                }
            }

            // Parse Cost$ parameter as generic mana amount
            if let Some(cost_str) = sa.ir.cost.as_deref() {
                if let Ok(cost) = cost_str.trim().parse::<i32>() {
                    total_cost += cost;
                }
            }
        }
    }

    total_cost
}
