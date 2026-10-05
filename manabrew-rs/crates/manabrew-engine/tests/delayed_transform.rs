use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::PlayerId;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const SHIFTER: &str = "Name:Day Shifter\nManaCost:1 G\nTypes:Creature Human\nPT:2/2\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Shifter\nManaCost:no cost\nTypes:Creature Wolf\nPT:4/4\nOracle:";

#[test]
fn a_delayed_transform_made_before_its_host_transformed_does_nothing() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let rules = parse_card_script(SHIFTER).expect("script");
    let shifter = game.create_card(CardInstance::from_rules(&rules, p0));
    game.move_card(shifter, ZoneType::Battlefield, p0);
    game.card_mut(shifter).transform_count = 1;

    let mut sa = SpellAbility::new_simple(
        Some(shifter),
        p0,
        "DB$ SetState | Defined$ Self | Mode$ Transform | StoredTransform$ 0",
    );
    sa.is_trigger = true;
    let mut entry = StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    };
    entry.store_transform(&game);
    game.stack.push(entry);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    let card = game.card(shifter);
    assert!(!card.is_transformed);
    assert_eq!(card.transform_count, 1);
}
