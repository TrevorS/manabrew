use forge_carddb::parse_card_script;
use forge_foundation::{ColorSet, PhaseType, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::{GameState, TypeRegistry};
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::{SpellAbility, StackEntry};
use manabrew_engine::staticability::layer::apply_continuous_effects;

const SANCTUARY: &str = "Name:Test Sanctuary\nManaCost:no cost\nTypes:Land\nOracle:";
const BONE_CAPTAIN: &str = "Name:Bone Captain\nManaCost:1 B\nTypes:Creature Human\nPT:1/1\nS:Mode$ Continuous | Affected$ Creature.Skeleton+YouCtrl | AddPower$ 1 | Description$ Skeletons you control get +1/+0.\nOracle:";
const SWAMP_MAKER: &str = "Name:Swamp Maker\nManaCost:2 B\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Land | AddType$ Swamp | Description$ Each land is a Swamp in addition to its other land types.\nOracle:";
const MOUNTAIN_MOON: &str = "Name:Mountain Moon\nManaCost:2 R\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Land.nonBasic | AddType$ Mountain | RemoveLandTypes$ True | Description$ Nonbasic lands are Mountains.\nOracle:";
const GATE: &str = "Name:Test Gate\nManaCost:no cost\nTypes:Land Gate\nOracle:";

fn load_types() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn main_phase(game: &mut GameState, player: PlayerId) {
    game.turn.active_player = player;
    game.new_turn_for_player(player);
    game.turn.phase = PhaseType::Main1;
}

fn pass_agents() -> Vec<Box<dyn PlayerAgent>> {
    vec![Box::new(PassAgent), Box::new(PassAgent)]
}

fn resolve(
    game: &mut GameState,
    game_loop: &mut GameLoop,
    source: CardId,
    text: &str,
    target: CardId,
) {
    let controller = game.card(source).controller;
    let mut sa = SpellAbility::new_simple(Some(source), controller, text);
    sa.target_chosen.target_card = Some(target);
    game.stack.push(StackEntry {
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
    let mut agents = pass_agents();
    game_loop.step_with_priority(game, &mut agents, false);
}

#[test]
fn an_end_of_turn_animate_ending_keeps_a_permanent_animates_creature_types() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let sanctuary = put(&mut game, SANCTUARY, p0, ZoneType::Battlefield);
    let captain = put(&mut game, BONE_CAPTAIN, p0, ZoneType::Battlefield);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Animate | Defined$ Targeted | Types$ Creature | AddAllCreatureTypes$ True | Power$ 3 | Toughness$ 3 | Duration$ Permanent",
        sanctuary,
    );
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Animate | Defined$ Targeted | Keywords$ Trample",
        sanctuary,
    );
    apply_continuous_effects(&mut game);
    assert_eq!(game.card(sanctuary).power(), 4);
    let mut agents = pass_agents();
    game_loop.step_cleanup(&mut game, &mut agents);
    apply_continuous_effects(&mut game);
    let card = game.card(sanctuary);
    assert!(card.is_creature());
    assert!(card.type_line.all_creature_types);
    assert_eq!(card.power(), 4);
}

#[test]
fn a_permanent_animate_outlives_an_older_end_of_turn_animate() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let sanctuary = put(&mut game, SANCTUARY, p0, ZoneType::Battlefield);
    let captain = put(&mut game, BONE_CAPTAIN, p0, ZoneType::Battlefield);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Animate | Defined$ Targeted | Types$ Creature,Elemental | Power$ 2 | Toughness$ 2",
        sanctuary,
    );
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Animate | Defined$ Targeted | Types$ Artifact | Duration$ Permanent",
        sanctuary,
    );
    let mut agents = pass_agents();
    game_loop.step_cleanup(&mut game, &mut agents);
    apply_continuous_effects(&mut game);
    let card = game.card(sanctuary);
    assert!(!card.is_creature());
    assert!(card.type_line.is_artifact());
    assert!(card.type_line.is_land());
    assert!(!card.type_line.has_subtype("Elemental"));
}

#[test]
fn a_token_copy_of_an_animated_land_copies_only_its_own_types() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let sanctuary = put(&mut game, SANCTUARY, p0, ZoneType::Battlefield);
    let captain = put(&mut game, BONE_CAPTAIN, p0, ZoneType::Battlefield);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Animate | Defined$ Targeted | Types$ Creature,Elemental | Power$ 2 | Toughness$ 2 | Colors$ Red",
        sanctuary,
    );
    assert_eq!(game.card(sanctuary).color, ColorSet::RED);
    let before = game.cards.len();
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ CopyPermanent | Defined$ Targeted",
        sanctuary,
    );
    let token = CardId(before as u32);
    let copy = game.card(token);
    assert!(copy.is_token);
    assert!(copy.type_line.is_land());
    assert!(!copy.is_creature());
    assert!(!copy.type_line.has_subtype("Elemental"));
    assert!(copy.color.is_colorless());
}

fn land_types_under_both_statics(swamp_maker_first: bool) -> (bool, bool) {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let gate = put(&mut game, GATE, p0, ZoneType::Battlefield);
    if swamp_maker_first {
        put(&mut game, SWAMP_MAKER, p0, ZoneType::Battlefield);
        put(&mut game, MOUNTAIN_MOON, p0, ZoneType::Battlefield);
    } else {
        put(&mut game, MOUNTAIN_MOON, p0, ZoneType::Battlefield);
        put(&mut game, SWAMP_MAKER, p0, ZoneType::Battlefield);
    }
    apply_continuous_effects(&mut game);
    let card = game.card(gate);
    assert!(card.type_line.has_subtype("Mountain"));
    assert!(!card.type_line.has_subtype("Gate"));
    (
        card.type_line.has_subtype("Swamp"),
        card.type_line.has_subtype("Mountain"),
    )
}

#[test]
fn land_type_statics_apply_in_timestamp_order() {
    assert_eq!(land_types_under_both_statics(true), (false, true));
    assert_eq!(land_types_under_both_statics(false), (true, true));
}

const EVERY_TYPE_AURA: &str = "Name:Every Type Banner\nManaCost:2\nTypes:Artifact\nS:Mode$ Continuous | Affected$ Creature.Human+YouCtrl | AddAllCreatureTypes$ True | Description$ Humans you control are every creature type.\nOracle:";
const GATE_BREAKER: &str = "Name:Gate Breaker\nManaCost:2\nTypes:Artifact\nS:Mode$ Continuous | Affected$ Land | RemoveType$ Gate | Description$ Lands aren't Gates.\nOracle:";

#[test]
fn a_static_adds_all_creature_types_and_removes_a_named_type() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let captain = put(&mut game, BONE_CAPTAIN, p0, ZoneType::Battlefield);
    let gate = put(&mut game, GATE, p0, ZoneType::Battlefield);
    put(&mut game, EVERY_TYPE_AURA, p0, ZoneType::Battlefield);
    put(&mut game, GATE_BREAKER, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert!(game.card(captain).type_line.all_creature_types);
    assert_eq!(game.card(captain).power(), 2);
    assert!(game.card(gate).type_line.is_land());
    assert!(!game.card(gate).type_line.has_subtype("Gate"));
}

const GREEN_BEAR: &str = "Name:Green Bear\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const BONE_CURSE: &str = "Name:Bone Curse\nManaCost:2 B\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | SetColor$ Black | Description$ Creatures you control are black.\nOracle:";
const DAWN_BANNER: &str = "Name:Dawn Banner\nManaCost:2\nTypes:Artifact\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddColor$ White | Description$ Creatures you control are white in addition to their other colors.\nOracle:";

#[test]
fn color_statics_and_an_animate_apply_in_timestamp_order() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let bear = put(&mut game, GREEN_BEAR, p0, ZoneType::Battlefield);
    put(&mut game, BONE_CURSE, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert_eq!(game.card(bear).color, ColorSet::BLACK);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        bear,
        "DB$ Animate | Defined$ Targeted | Colors$ Blue | OverwriteColors$ True",
        bear,
    );
    put(&mut game, DAWN_BANNER, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert_eq!(game.card(bear).color, ColorSet::BLUE.union(ColorSet::WHITE));
    let mut agents = pass_agents();
    game_loop.step_cleanup(&mut game, &mut agents);
    apply_continuous_effects(&mut game);
    assert_eq!(
        game.card(bear).color,
        ColorSet::BLACK.union(ColorSet::WHITE)
    );
}

const LAND_WAKER: &str = "Name:Land Waker\nManaCost:3 G\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Land.YouCtrl | AddType$ Creature | SetPower$ 3 | SetToughness$ 3 | Description$ Lands you control are 3/3 creatures.\nOracle:";

#[test]
fn an_earthbend_sets_power_over_an_older_set_power_static() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    put(&mut game, LAND_WAKER, p0, ZoneType::Battlefield);
    let sanctuary = put(&mut game, SANCTUARY, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert_eq!(game.card(sanctuary).power(), 3);
    let captain = put(&mut game, BONE_CAPTAIN, p0, ZoneType::Battlefield);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        captain,
        "DB$ Earthbend | Num$ 1",
        sanctuary,
    );
    apply_continuous_effects(&mut game);
    let card = game.card(sanctuary);
    assert_eq!((card.power(), card.toughness()), (1, 1));
}

#[test]
fn a_creature_that_died_keeps_its_animated_color_in_its_last_known_information() {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let bear = put(&mut game, GREEN_BEAR, p0, ZoneType::Battlefield);
    let mut game_loop = GameLoop::new(2);
    resolve(
        &mut game,
        &mut game_loop,
        bear,
        "DB$ Animate | Defined$ Targeted | Colors$ Blue | OverwriteColors$ True",
        bear,
    );
    apply_continuous_effects(&mut game);
    game.copy_last_state();
    game.move_card(bear, ZoneType::Graveyard, p0);
    assert_eq!(game.card(bear).color, ColorSet::GREEN);
    let lki = manabrew_engine::lki::battlefield_lki_card(&game, bear).expect("lki");
    assert_eq!(lki.color, ColorSet::BLUE);
}
