use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::parsing::CompiledSelector;
use crate::staticability::StaticMode;

/// What a `Mode$ MustAttack` static asks of one creature: any defender, or a named player or card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MustAttackEntity {
    Any,
    Player(PlayerId),
    Card(CardId),
}

/// Mirrors Java `StaticAbilityMustAttack.entitiesMustAttack`: the entities the attacker must
/// attack, with the active player and the active player's cards dropped (CR 506.2).
pub fn entities_must_attack(game: &GameState, attacker: &Card) -> Vec<MustAttackEntity> {
    let mut entities = Vec::new();
    let active_player = game.active_player();
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_conditions_full(&StaticMode::MustAttack, source, game))
        {
            if !matches_valid_creature(st_ab.ir.valid_creature.as_ref(), attacker, source, game) {
                continue;
            }
            match st_ab.ir.must_attack.as_deref() {
                None => entities.push(MustAttackEntity::Any),
                Some(defined) => {
                    let host = Some(source.id);
                    let activator = Some(source.controller);
                    for player in crate::ability::ability_utils::get_defined_players(
                        game, host, defined, activator,
                    ) {
                        if player != active_player {
                            entities.push(MustAttackEntity::Player(player));
                        }
                    }
                    for card in crate::ability::ability_utils::get_defined_cards(
                        game, host, defined, activator,
                    ) {
                        if game.card(card).controller != active_player {
                            entities.push(MustAttackEntity::Card(card));
                        }
                    }
                }
            }
        }
    }
    entities
}

fn matches_valid_creature(
    valid: Option<&CompiledSelector>,
    card: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    match valid {
        None => card.is_creature(),
        Some(_) => valid_filter::matches_valid_card_selector_opt_in_game(valid, card, source, game),
    }
}
