use std::cell::RefCell;
use std::rc::Rc;

use rand::SeedableRng;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

use crate::common_test_agent::{CallbackEvent, RecordingAgent, RecordingState};

const LEYLINE_OF_HOPE: &str = "Name:Leyline of Hope\nManaCost:2 W W\nTypes:Enchantment\nR:Event$ GainLife | ActiveZones$ Battlefield | ValidPlayer$ You | ReplaceWith$ GainLife | AILogic$ DoubleLife | Description$ If you would gain life, you gain that much life plus 1 instead.\nSVar:GainLife:DB$ ReplaceEffect | VarName$ LifeGained | VarValue$ X\nSVar:X:ReplaceCount$LifeGained/Plus.1\nOracle:";
const SOURCE: &str = "Name:Life Source\nManaCost:no cost\nTypes:Artifact\nOracle:";
const LIFELINK_BEAR: &str =
    "Name:Lifelink Bear\nManaCost:1 W\nTypes:Creature Bear\nPT:2/2\nK:Lifelink\nOracle:";
const LIFELINK_SOURCE: &str =
    "Name:Lifelink Source\nManaCost:no cost\nTypes:Artifact\nK:Lifelink\nOracle:";

fn put(game: &mut GameState, script: &str, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, PlayerId(0)));
    game.move_card(card, zone, PlayerId(0));
    card
}

fn replacement_choices_resolving(source_script: &str, ability: &str) -> (i32, Vec<usize>) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.turn.active_player = PlayerId(0);
    game.turn.phase = PhaseType::Main1;
    put(&mut game, LEYLINE_OF_HOPE, ZoneType::Battlefield);
    put(&mut game, LEYLINE_OF_HOPE, ZoneType::Battlefield);
    let source = put(&mut game, source_script, ZoneType::Battlefield);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(Some(source), PlayerId(0), ability),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let state = Rc::new(RefCell::new(RecordingState::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(RecordingAgent::new(Rc::clone(&state))),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);
    let choices: Vec<usize> = state
        .borrow()
        .events
        .iter()
        .filter_map(|event| match event {
            CallbackEvent::ChooseReplacement(options) => Some(options.len()),
            _ => None,
        })
        .collect();
    (game.player(PlayerId(0)).life, choices)
}

#[test]
fn two_leylines_of_hope_ask_their_controller_which_applies_first() {
    assert_eq!(
        replacement_choices_resolving(SOURCE, "DB$ GainLife | LifeAmount$ 1 | Defined$ You"),
        (23, vec![2])
    );
    assert_eq!(
        replacement_choices_resolving(
            LIFELINK_SOURCE,
            "DB$ DealDamage | NumDmg$ 2 | Defined$ Player.Opponent"
        ),
        (24, vec![2])
    );
}

#[test]
fn two_leylines_of_hope_ask_which_applies_first_to_combat_lifelink() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    put(&mut game, LEYLINE_OF_HOPE, ZoneType::Battlefield);
    put(&mut game, LEYLINE_OF_HOPE, ZoneType::Battlefield);
    let bear = put(&mut game, LIFELINK_BEAR, ZoneType::Battlefield);
    game.card_mut(bear).set_summoning_sick(false);
    for player in [PlayerId(0), PlayerId(1)] {
        for _ in 0..3 {
            let rules = parse_card_script(LIFELINK_BEAR).expect("script");
            let card = game.create_card(CardInstance::from_rules(&rules, player));
            game.move_card(card, ZoneType::Library, player);
        }
    }
    game.turn.active_player = PlayerId(0);
    let state = Rc::new(RefCell::new(RecordingState {
        attacks: true,
        ..Default::default()
    }));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(RecordingAgent::new(Rc::clone(&state))),
        Box::new(PassAgent),
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    GameLoop::new(2).run_turn(&mut game, &mut agents, &mut rng);
    assert_eq!(game.player(PlayerId(1)).life, 18);
    assert_eq!(game.player(PlayerId(0)).life, 24);
    let choices: Vec<usize> = state
        .borrow()
        .events
        .iter()
        .filter_map(|event| match event {
            CallbackEvent::ChooseReplacement(options) => Some(options.len()),
            _ => None,
        })
        .collect();
    assert_eq!(choices, vec![2]);
}
