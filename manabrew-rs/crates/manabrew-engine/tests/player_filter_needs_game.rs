use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::combat_util::can_attack_defender;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::staticability::static_ability_cant_be_cast::cant_play_land_ability;
use manabrew_engine::trigger::TriggerHandler;

const DANDAN: &str = "Name:Dandân\nManaCost:U U\nTypes:Creature Fish\nPT:4/1\nS:Mode$ CantAttack | ValidCard$ Card.Self | UnlessDefender$ controlsIsland | Description$ CARDNAME can't attack unless defending player controls an Island.\nOracle:Dandân can't attack unless defending player controls an Island.";
const ISLAND: &str =
    "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:({T}: Add {U}.)";
const FOREST: &str =
    "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:({T}: Add {G}.)";
const LAND_LOCK: &str = "Name:Land Lock\nManaCost:2\nTypes:Enchantment\nS:Mode$ CantPlayLand | Player$ Player.IsRemembered | Description$ This player can't play land cards this turn.\nOracle:";
const ARCHER: &str =
    "Name:Arbalest Elite\nManaCost:2 W W\nTypes:Creature Human Archer\nPT:2/3\nOracle:";
const STAY_TAPPED: &str = "Name:Stay Tapped\nManaCost:2\nTypes:Enchantment\nR:Event$ Untap | ValidCard$ Card.IsRemembered | ValidStepTurnToController$ Player.Activator | Layer$ CantHappen | Description$ EFFECTSOURCE doesn't untap during your next untap step.\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn dandan_attacks_a_player_who_controls_an_island() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let dandan = put(&mut game, DANDAN, p0, ZoneType::Battlefield);
    game.card_mut(dandan).summoning_sick = false;

    assert!(!can_attack_defender(&game, dandan, DefenderId::Player(p1)));
    put(&mut game, ISLAND, p1, ZoneType::Battlefield);
    assert!(can_attack_defender(&game, dandan, DefenderId::Player(p1)));
}

#[test]
fn pardic_miner_stops_only_the_remembered_player_from_playing_lands() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let effect = put(&mut game, LAND_LOCK, p0, ZoneType::Battlefield);
    game.card_mut(effect).remembered_players.push(p1);
    let alice_land = put(&mut game, FOREST, p0, ZoneType::Hand);
    let bob_land = put(&mut game, FOREST, p1, ZoneType::Hand);

    assert!(cant_play_land_ability(&game, game.card(bob_land), p1));
    assert!(!cant_play_land_ability(&game, game.card(alice_land), p0));
}

fn tapped_archer_under_its_effect(game: &mut GameState) -> CardId {
    let p0 = PlayerId(0);
    let archer = put(game, ARCHER, p0, ZoneType::Battlefield);
    game.card_mut(archer).tapped = true;
    let effect = put(game, STAY_TAPPED, p0, ZoneType::Battlefield);
    game.card_mut(effect).remembered_cards.push(archer);
    archer
}

#[test]
fn doesnt_untap_holds_during_its_controllers_untap_step() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let archer = tapped_archer_under_its_effect(&mut game);

    assert!(!game.untap_during_untap_step(archer, PlayerId(0), &mut TriggerHandler::new()));
    assert!(game.card(archer).tapped);
}

#[test]
fn doesnt_untap_lets_go_during_another_players_untap_step() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let archer = tapped_archer_under_its_effect(&mut game);

    assert!(game.untap_during_untap_step(archer, PlayerId(1), &mut TriggerHandler::new()));
    assert!(!game.card(archer).tapped);
}
