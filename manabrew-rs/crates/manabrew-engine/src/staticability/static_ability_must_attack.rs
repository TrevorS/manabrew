use std::sync::Arc;

use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::ids::PlayerId;
use crate::parsing::CompiledSelector;
use crate::staticability::StaticMode;

/// What a `Mode$ MustAttack` static asks of one creature: any defender, or a named player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MustAttackEntity {
    Any,
    Player(PlayerId),
}

/// Mirrors Java `StaticAbilityMustAttack.entitiesMustAttack`: the entities the attacker must
/// attack, with the active player dropped (CR 506.2, a player cannot attack themselves).
pub fn entities_must_attack(
    cards: &[Arc<Card>],
    attacker: &Card,
    active_player: PlayerId,
) -> Vec<MustAttackEntity> {
    let mut entities = Vec::new();
    for source in cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield || c.zone == ZoneType::Command)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&StaticMode::MustAttack))
        {
            if !matches_valid_creature(st_ab.ir.valid_creature.as_ref(), attacker, source) {
                continue;
            }
            match st_ab.ir.must_attack.as_deref() {
                None => entities.push(MustAttackEntity::Any),
                Some(defined) => {
                    for player in must_attack_players(defined, source) {
                        if player != active_player {
                            entities.push(MustAttackEntity::Player(player));
                        }
                    }
                }
            }
        }
    }
    entities
}

fn must_attack_players(defined: &str, source: &Card) -> Vec<PlayerId> {
    match defined {
        "You" => vec![source.controller],
        "CardOwner" => vec![source.owner],
        "Remembered" | "RememberedPlayer" | "Player.IsRemembered" => {
            source.remembered_players.clone()
        }
        "ChosenPlayer" => source.chosen_player.into_iter().collect(),
        _ => {
            crate::census::unhandled("must-attack-defined", defined);
            Vec::new()
        }
    }
}

fn matches_valid_creature(valid: Option<&CompiledSelector>, card: &Card, source: &Card) -> bool {
    match valid {
        None => card.is_creature(),
        Some(selector) => valid_filter::matches_valid_card_selector(selector, card, source),
    }
}
