//! Exert permanents as a cost. Mirrors Java's `CostExert`.

// NOTE: pay_as_decided is handled by GameLoop::pay_exert_cost() in game_action.rs
// because it requires agent interaction and trigger firing (Exerted).

pub const HASH_LKI: &str = "Exerted";
pub const HASH_CARDS: &str = "ExertedCards";

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Exert {
        amount,
        type_filter,
    } = part
    else {
        return false;
    };
    let resolved_amount = amount.resolve(game, source, player);
    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
        return resolved_amount <= 1;
    }
    let count = game
        .cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
        .iter()
        .filter(|&&cid| {
            crate::cost::is_valid_cost_card(
                game,
                game.card(cid),
                type_filter,
                game.card(source),
                player,
                ability,
            )
        })
        .count() as i32;
    count >= resolved_amount
}
