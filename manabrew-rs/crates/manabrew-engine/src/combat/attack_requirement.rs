use forge_foundation::{CoreType, ZoneType};

use crate::HashMap;

use super::DefenderId;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::player::player_predicates::is_opponent_of;
use crate::staticability::static_ability_must_attack;

/// Represents a requirement for a creature to attack.
/// Mirrors Java's `AttackRequirement.java`.
#[derive(Debug, Clone)]
pub struct AttackRequirement {
    /// The creature that must attack.
    pub attacker: CardId,
    /// True if the creature must attack any legal defender.
    pub must_attack_any: bool,
    /// The player that goaded this creature (it can't attack that player).
    pub goaded_by: Option<PlayerId>,
    /// Per-defender requirement counts: defender → number of reasons to attack it.
    /// Mirrors Java's `defenderSpecific` map.
    pub defender_specific: HashMap<DefenderId, i32>,
}

impl AttackRequirement {
    /// Mirrors Java's `hasRequirement()`.
    /// Returns true if this creature has any reason it must attack.
    pub fn has_requirement(&self) -> bool {
        self.defender_specific.values().any(|&v| v > 0) || self.must_attack_any
    }

    /// Mirrors Java's `countViolations()`.
    /// Count how many attack requirements are violated if this creature is
    /// attacking `defender` (or `None` if not attacking at all).
    pub fn count_violations(&self, defender: Option<DefenderId>) -> i32 {
        if !self.has_requirement() {
            return 0;
        }

        let total: i32 = self.defender_specific.values().sum();
        let is_attacking = defender.is_some();

        let credit = if is_attacking {
            defender
                .and_then(|d| self.defender_specific.get(&d).copied())
                .unwrap_or(0)
        } else {
            0
        };

        total - credit
    }

    /// Get sorted requirements: (defender, count) pairs sorted ascending.
    /// Mirrors Java's `getSortedRequirements()`.
    pub fn get_sorted_requirements(&self) -> Vec<(DefenderId, i32)> {
        let mut entries: Vec<(DefenderId, i32)> = self
            .defender_specific
            .iter()
            .map(|(&d, &c)| (d, c))
            .collect();
        entries.sort_by_key(|&(_, c)| c);
        entries
    }
}

/// Compute attack requirements for all available creatures.
/// Returns a list of requirements — creatures that must attack if able.
///
/// Sources of must-attack:
/// 1. Static abilities with `MustAttack` mode (existing `must_attack()` check)
/// 2. Goad: creature is goaded and must attack a player other than the goader
pub fn compute_attack_requirements(
    game: &GameState,
    available: &[CardId],
    defending: PlayerId,
) -> Vec<AttackRequirement> {
    compute_attack_requirements_with_defenders(game, available, &[DefenderId::Player(defending)])
}

/// Compute attack requirements with a full list of possible defenders.
/// Mirrors the Java `AttackRequirement` constructor.
pub fn compute_attack_requirements_with_defenders(
    game: &GameState,
    available: &[CardId],
    possible_defenders: &[DefenderId],
) -> Vec<AttackRequirement> {
    let mut requirements = Vec::new();

    for &attacker_id in available {
        let card = game.card(attacker_id);
        let goaded = card.goaded_by;

        let mut n_attack_anything: i32 = 0;
        if goaded.is_some() {
            n_attack_anything += 1;
        }

        let mut defender_specific: HashMap<DefenderId, i32> = HashMap::default();
        for entity in static_ability_must_attack::entities_must_attack(game, card) {
            let defender = match entity {
                static_ability_must_attack::MustAttackEntity::Any => {
                    n_attack_anything += 1;
                    continue;
                }
                static_ability_must_attack::MustAttackEntity::Player(pid) => {
                    DefenderId::Player(pid)
                }
                static_ability_must_attack::MustAttackEntity::Card(cid) => {
                    DefenderId::Permanent(cid)
                }
            };
            *defender_specific.entry(defender).or_insert(0) += 1;
        }

        for &defender in possible_defenders {
            *defender_specific.entry(defender).or_insert(0) += n_attack_anything;
        }

        defender_specific.retain(|defender, _| match *defender {
            DefenderId::Player(pid) => game.player(pid).is_alive(),
            DefenderId::Permanent(cid) => {
                let defender_card = game.card(cid);
                defender_card.zone == ZoneType::Battlefield
                    && game.player(defender_card.controller).is_alive()
                    && (defender_card
                        .type_line
                        .core_types
                        .contains(&CoreType::Battle)
                        || is_opponent_of(game, defender_card.controller, card.controller))
            }
        });

        requirements.push(AttackRequirement {
            attacker: attacker_id,
            must_attack_any: n_attack_anything > 0,
            goaded_by: goaded,
            defender_specific,
        });
    }

    requirements
}

/// Get all creature IDs that must attack (from requirements).
pub fn must_attack_ids(requirements: &[AttackRequirement]) -> Vec<CardId> {
    requirements
        .iter()
        .filter(|r| r.has_requirement())
        .map(|r| r.attacker)
        .collect()
}
