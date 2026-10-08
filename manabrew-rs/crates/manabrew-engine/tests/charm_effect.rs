use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const COPY_HOST: &str = "Name:Copy Host\nManaCost:2\nTypes:Artifact\nSVar:DBReturn:DB$ ChangeZone | Defined$ OriginalHost | Origin$ Graveyard | Destination$ Hand | SpellDescription$ Return the original to its owner's hand.\nOracle:";
const ORIGINAL: &str = "Name:Original\nManaCost:2\nTypes:Artifact\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn a_charm_mode_of_a_copied_trait_keeps_the_original_host() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let host = put(&mut game, COPY_HOST, p0, ZoneType::Battlefield);
    let original = put(&mut game, ORIGINAL, p0, ZoneType::Graveyard);
    let mut charm = SpellAbility::new_simple(Some(host), p0, "DB$ Charm | Choices$ DBReturn");
    charm.set_original_host(original);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: charm,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(original).zone, ZoneType::Hand);
}
