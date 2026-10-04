use forge_carddb::parse_card_script;
use forge_foundation::{ManaCost, ZoneType};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::{
    can_pay_spell_mana_cost_for_action_space, ActionSpaceManaProbe, ManaPaymentContext, ManaPool,
};

const WIND_TEMPLE: &str = "Name:Wind Temple\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ W | SpellDescription$ Add {W}.\nA:AB$ PutCounterAll | Cost$ 3 W T | ValidCards$ Creature.YouCtrl | CounterType$ P1P1 | CounterNum$ 1 | SpellDescription$ Put a +1/+1 counter on each creature you control.\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const TREASURE: &str = "Name:Treasure\nManaCost:no cost\nTypes:Artifact Treasure\nA:AB$ Mana | Cost$ T Sac<1/CARDNAME/this artifact> | Produced$ Any | SpellDescription$ Add one mana of any color.\nOracle:";
const GLASS_HULK: &str =
    "Name:Glass Hulk\nManaCost:G G W W\nTypes:Artifact Creature Construct\nPT:4/4\nOracle:";

fn put(game: &mut GameState, script: &str, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, PlayerId(0)));
    game.move_card(card, zone, PlayerId(0));
    card
}

#[test]
fn the_probe_spends_a_treasure_after_a_land_for_the_same_colour() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    put(&mut game, WIND_TEMPLE, ZoneType::Battlefield);
    put(&mut game, FOREST, ZoneType::Battlefield);
    put(&mut game, TREASURE, ZoneType::Battlefield);
    put(&mut game, TREASURE, ZoneType::Battlefield);
    let hulk = put(&mut game, GLASS_HULK, ZoneType::Hand);
    let payable = can_pay_spell_mana_cost_for_action_space(
        &game,
        &ManaPool::new(),
        PlayerId(0),
        hulk,
        &ManaCost::parse("G G W W"),
        &ManaPaymentContext {
            is_spell: true,
            mana_value: Some(4),
            ..Default::default()
        },
        None,
    );
    assert!(payable);
}
