use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::counter_type::CounterType;
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};

const MIND_SWAPPER: &str = "Name:Mind Swapper\nManaCost:2 U B\nTypes:Legendary Creature Spider Human Hero\nPT:4/4\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | NewName$ Mind Swapper | AddTypes$ Spider & Human & Hero | SetPower$ 4 | SetToughness$ 4 | SpellDescription$ You may have CARDNAME enter as a copy of any creature card in a graveyard.\nOracle:";
const SAGA_BEAST: &str = "Name:Saga Beast\nManaCost:9\nTypes:Enchantment Creature Saga Dragon\nPT:9/9\nK:Chapter:2:DBDraw,DBDraw\nSVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn a_creature_entering_as_a_copy_of_a_saga_enters_with_a_lore_counter() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, SAGA_BEAST, p0, ZoneType::Graveyard);
    let swapper = put(&mut game, MIND_SWAPPER, p0, ZoneType::Hand);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    game.move_card_with_agents(swapper, ZoneType::Battlefield, p0, &mut agents);
    let card = game.card(swapper);
    assert_eq!(card.zone, ZoneType::Battlefield);
    assert!(card.type_line.has_subtype("Saga"));
    assert_eq!(card.counter_count(&CounterType::Lore), 1);
}

const WALKER_BEAST: &str = "Name:Walker Beast\nManaCost:2 U B\nTypes:Legendary Creature Planeswalker Kaito\nPT:3/4\nLoyalty:4\nA:AB$ GainLife | Cost$ AddCounter<1/LOYALTY> | Planeswalker$ True | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";

#[test]
fn a_creature_entering_as_a_copy_of_a_planeswalker_enters_with_its_loyalty() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, WALKER_BEAST, p0, ZoneType::Graveyard);
    let swapper = put(&mut game, MIND_SWAPPER, p0, ZoneType::Hand);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    game.move_card_with_agents(swapper, ZoneType::Battlefield, p0, &mut agents);
    let card = game.card(swapper);
    assert_eq!(card.zone, ZoneType::Battlefield);
    assert!(card.type_line.is_planeswalker());
    assert_eq!(card.counter_count(&CounterType::Loyalty), 4);
}

const MOON_PUP: &str = "Name:Moon Pup\nManaCost:1 G\nTypes:Creature Wolf\nPT:2/2\nT:Mode$ Transformed | ValidCard$ Card.Self | Execute$ TrigGain | TriggerDescription$ Whenever this creature transforms into CARDNAME, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Moon Hound\nManaCost:no cost\nTypes:Creature Wolf\nPT:4/4\nOracle:";

fn transform_self(game: &mut GameState, card: CardId, agents: &mut [Box<dyn PlayerAgent>]) {
    let sa = manabrew_engine::spellability::SpellAbility::new_simple(
        Some(card),
        game.card(card).controller,
        "DB$ SetState | Defined$ Self | Mode$ Transform",
    );
    game.stack.push(manabrew_engine::spellability::StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    manabrew_engine::game_loop::GameLoop::new(2).step_with_priority(game, agents, false);
}

const MOON_HERO: &str = "Name:Moon Hero\nManaCost:1 G\nTypes:Creature Human Hero\nPT:2/2\nAlternateMode:Modal\nOracle:\n\nALTERNATE\n\nName:Moon Champion\nManaCost:no cost\nTypes:Creature Human Hero\nPT:4/4\nOracle:";

#[test]
fn a_modal_double_faced_creature_transforms() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let hero = put(&mut game, MOON_HERO, p0, ZoneType::Battlefield);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    transform_self(&mut game, hero, &mut agents);
    assert_eq!(game.card(hero).card_name, "Moon Champion");
}

#[test]
fn a_creature_entering_as_a_copy_of_a_transforming_card_does_not_transform() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    put(&mut game, MOON_PUP, p0, ZoneType::Graveyard);
    let pup = put(&mut game, MOON_PUP, p0, ZoneType::Battlefield);
    let swapper = put(&mut game, MIND_SWAPPER, p0, ZoneType::Hand);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    game.move_card_with_agents(swapper, ZoneType::Battlefield, p0, &mut agents);
    assert!(game.card(swapper).type_line.has_subtype("Wolf"));
    transform_self(&mut game, pup, &mut agents);
    transform_self(&mut game, swapper, &mut agents);
    manabrew_engine::game_loop::GameLoop::new(2).resolve_stack(&mut game, &mut agents);
    assert_eq!(game.card(pup).card_name, "Moon Hound");
    assert_eq!(game.player(p0).life, 20);
    assert_eq!(game.card(swapper).card_name, "Mind Swapper");
    assert!(!game.card(swapper).is_transformed);
}
