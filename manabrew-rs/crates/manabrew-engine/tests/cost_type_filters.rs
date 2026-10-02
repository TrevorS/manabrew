use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::card::CardInstance;
use manabrew_engine::cost::{cost_exile, parse_cost, CostPart};
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::spellability::SpellAbility;

const FORCE: &str = "Name:Force of Will\nManaCost:3 U U\nTypes:Instant\nOracle:";
const BLUE: &str = "Name:Opt\nManaCost:U\nTypes:Instant\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn exile_part() -> CostPart {
    parse_cost("ExileFromHand<1/Card.Blue+Other>")
        .parts
        .into_iter()
        .find(|part| matches!(part, CostPart::Exile { .. }))
        .expect("exile part")
}

#[test]
fn force_of_will_pitches_the_one_other_blue_card_in_hand() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let force = put(&mut game, FORCE, p0, ZoneType::Hand);
    let sa = SpellAbility::new_empty(Some(force), p0);
    let part = exile_part();

    assert!(!cost_exile::can_pay(
        &game,
        &ManaPool::default(),
        force,
        p0,
        Some(&sa),
        &part
    ));
    put(&mut game, BLUE, p0, ZoneType::Hand);
    assert!(cost_exile::can_pay(
        &game,
        &ManaPool::default(),
        force,
        p0,
        Some(&sa),
        &part
    ));
}
