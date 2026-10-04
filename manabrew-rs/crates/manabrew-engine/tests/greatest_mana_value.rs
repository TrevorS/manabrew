use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::svar::resolve_count_svar;

const ASHLING: &str = "Name:Day Ashling\nManaCost:1 R\nTypes:Legendary Creature Elemental Sorcerer\nPT:1/3\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Ashling\nManaCost:no cost\nColors:blue\nTypes:Legendary Creature Elemental Wizard\nPT:1/3\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn a_transformed_back_face_counts_its_front_face_mana_value_as_the_greatest() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let ashling = put(&mut game, ASHLING, p0, ZoneType::Battlefield);
    let source = put(&mut game, BEARS, p0, ZoneType::Hand);
    game.card_mut(ashling).transform();
    assert_eq!(game.card(ashling).card_name, "Night Ashling");
    assert_eq!(
        resolve_count_svar(
            "Count$Valid Elemental.YouCtrl$GreatestCardManaCost",
            &game,
            source,
            p0
        ),
        2
    );
}
