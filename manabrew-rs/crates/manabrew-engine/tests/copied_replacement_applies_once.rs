use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};

const SUPERIOR_SPIDER_MAN: &str = "Name:Superior Spider-Man\nManaCost:2 U B\nTypes:Legendary Creature Spider Human Hero\nPT:4/4\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | NewName$ Superior Spider-Man | AddTypes$ Spider & Human & Hero | SetPower$ 4 | SetToughness$ 4 | RememberCloneOrigin$ True | SubAbility$ DBImmediateTrig | SpellDescription$ Mind Swap — You may have CARDNAME enter as a copy of any creature card in a graveyard, except his name is Superior Spider-Man and he's a 4/4 Spider Human Hero in addition to his other types. When you do, exile that card.\nSVar:DBImmediateTrig:DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ GE1 | Execute$ TrigExile | RememberObjects$ RememberedCard | SubAbility$ DBCleanup | TriggerDescription$ When you do, exile that card.\nSVar:TrigExile:DB$ ChangeZone | Defined$ DelayTriggerRememberedLKI | Origin$ Graveyard | Destination$ Exile\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nOracle:Mind Swap — You may have Superior Spider-Man enter as a copy of any creature card in a graveyard, except his name is Superior Spider-Man and he's a 4/4 Spider Human Hero in addition to his other types. When you do, exile that card.";

fn enter_copying_another_superior_spider_man(mirror_forge_bugs: bool) -> (GameState, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = mirror_forge_bugs;
    let rules = parse_card_script(SUPERIOR_SPIDER_MAN).expect("card script should parse");
    let in_graveyard = game.create_card(CardInstance::from_rules(&rules, PlayerId(1)));
    game.move_card(in_graveyard, ZoneType::Graveyard, PlayerId(1));
    let entering = game.create_card(CardInstance::from_rules(&rules, PlayerId(0)));
    game.move_card(entering, ZoneType::Hand, PlayerId(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    game.move_card_with_agents(entering, ZoneType::Battlefield, PlayerId(0), &mut agents);
    (game, entering)
}

#[test]
fn a_copied_mind_swap_applies_once_to_the_event() {
    let (game, entering) = enter_copying_another_superior_spider_man(false);
    assert_eq!(game.card(entering).zone, ZoneType::Battlefield);
    assert_eq!(game.card(entering).card_name, "Superior Spider-Man");
}

#[test]
#[should_panic(expected = "replacement recursion deeper than")]
fn forge_offering_the_copied_mind_swap_again_panics_instead_of_overflowing() {
    enter_copying_another_superior_spider_man(true);
}
