use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::ids::CardId;
use crate::staticability::StaticMode;

/// Check whether damage from `source_id` cannot be prevented.
/// Mirrors Java's StaticAbilityCantPreventDamage.cantPreventDamage().
pub fn cant_prevent_damage(game: &GameState, source_id: CardId, is_combat: bool) -> bool {
    let source_card = game.card(source_id);
    for host in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source() || c.id == source_id)
    {
        for st_ab in host
            .static_abilities
            .iter()
            .filter(|sa| sa.check_conditions_full(&StaticMode::CantPreventDamage, host, game))
        {
            if applies(st_ab, source_card, host, is_combat) {
                return true;
            }
        }
    }
    false
}

/// Mirrors Java's `StaticAbilityCantPreventDamage.applyCantPreventDamage()`.
pub fn apply_cant_prevent_damage(
    st_ab: &crate::staticability::static_ability::StaticAbility,
    damage_source: &Card,
    host: &Card,
    is_combat: bool,
) -> bool {
    applies(st_ab, damage_source, host, is_combat)
}

fn applies(
    st_ab: &crate::staticability::static_ability::StaticAbility,
    damage_source: &Card,
    host: &Card,
    is_combat: bool,
) -> bool {
    if let Some(required) = st_ab.ir.is_combat {
        if required != is_combat {
            return false;
        }
    }

    valid_filter::matches_valid_card_selector_opt(
        st_ab.ir.valid_source.as_ref(),
        damage_source,
        host,
    )
}
