use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::StackEntry;

const LILIANA: &str = "Name:Liliana, Dreadhorde General\nManaCost:4 B B\nTypes:Legendary Planeswalker Liliana\nLoyalty:6\nA:AB$ ChooseCard | Cost$ SubCounter<9/LOYALTY> | Planeswalker$ True | Ultimate$ True | Defined$ Opponent | Choices$ Permanent | ChooseEach$ Artifact & Battle & Creature & Enchantment & Land & Planeswalker | ControlledByPlayer$ Chooser | Mandatory$ True | Reveal$ True | SubAbility$ SacAllOthers | SpellDescription$ Each opponent chooses a permanent they control of each permanent type and sacrifices the rest.\nSVar:SacAllOthers:DB$ SacrificeAll | ValidCards$ Permanent.nonChosenCard+OppCtrl | SubAbility$ DBCleanup\nSVar:DBCleanup:DB$ Cleanup | ClearChosenCard$ True\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const BEAR: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const STONE: &str = "Name:Glass Stone\nManaCost:2\nTypes:Artifact\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn liliana_s_ultimate_keeps_one_permanent_of_each_type() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let liliana = put(&mut game, LILIANA, p0, ZoneType::Battlefield);
    for script in [FOREST, FOREST, BEAR, BEAR, STONE, STONE] {
        put(&mut game, script, p1, ZoneType::Battlefield);
    }
    let ability = game.card(liliana).activated_abilities[0]
        .ability_text
        .clone();
    let mut sa = manabrew_engine::spellability::build_spell_ability(&game, liliana, &ability, p0);
    sa.is_activated = true;
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
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    let mut kept: Vec<String> = game
        .zone(ZoneType::Battlefield, p1)
        .cards
        .iter()
        .map(|&id| game.card(id).card_name.clone())
        .collect();
    kept.sort();
    assert_eq!(kept, vec!["Forest", "Glass Stone", "Grizzly Bears"]);
}
