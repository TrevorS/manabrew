use forge_carddb::parse_card_script;
use forge_foundation::{ColorSet, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::target_restrictions::can_be_targeted_by_sa;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const WARDEN: &str = "Name:Mono Warden\nManaCost:1 W\nTypes:Creature Human\nPT:2/2\nK:Hexproof:Card.MonoColor:monocolored\nOracle:";
const AVENGER: &str = "Name:Dying Avenger\nManaCost:R W\nTypes:Creature Human\nPT:2/2\nOracle:";
const DRAINER: &str = "Name:Drainer\nManaCost:B\nTypes:Creature Human\nPT:1/1\nSVar:TrigLoseLife:DB$ LoseLife | ValidTgts$ Opponent | LifeAmount$ 1 | SubAbility$ DBGainLife\nSVar:DBGainLife:DB$ GainLife | LifeAmount$ 1\nOracle:";
const CONSUL: &str = "Name:Consul\nManaCost:W\nTypes:Enchantment\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.OppCtrl | TriggerZones$ Battlefield | Execute$ TrigGainLife | TriggerDescription$ Whenever a creature an opponent controls enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const TURN_NINJA: &str = "Name:Turn Ninja\nManaCost:2\nTypes:Artifact\nS:Mode$ Continuous | Affected$ Card.Self | Condition$ PlayerTurn | AddType$ Creature | SetPower$ 1 | SetToughness$ 1\nA:AB$ ChangeZone | Cost$ 0 | ActivationZone$ Hand | Origin$ Hand | Destination$ Battlefield | Defined$ Self\nOracle:";
const SHIELD: &str = "Name:Shielder\nManaCost:W\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ You | AddKeyword$ Hexproof\nOracle:";

fn entry(spell_ability: SpellAbility) -> StackEntry {
    StackEntry {
        id: 0,
        spell_ability,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, ZoneType::Battlefield, owner);
    card
}

#[test]
fn a_dead_hosts_trigger_targets_as_the_object_it_was() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let warden = put(&mut game, WARDEN, p1);
    let avenger = put(&mut game, AVENGER, p0);
    let zone_timestamp = game.card(avenger).zone_timestamp;
    game.move_card(avenger, ZoneType::Graveyard, p0);
    game.card_mut(avenger).set_color(ColorSet::WHITE);
    let mut sa = SpellAbility::new_simple(Some(avenger), p0, "DB$ Destroy | ValidTgts$ Creature");
    sa.is_trigger = true;

    assert!(!can_be_targeted_by_sa(&game, warden, p0, &sa));
    sa.source_zone_timestamp = Some(zone_timestamp);
    assert!(can_be_targeted_by_sa(&game, warden, p0, &sa));
}

#[test]
fn a_live_hosts_trigger_targets_with_its_current_characteristics() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let warden = put(&mut game, WARDEN, p1);
    let avenger = put(&mut game, AVENGER, p0);
    let mut sa = SpellAbility::new_simple(Some(avenger), p0, "DB$ Destroy | ValidTgts$ Creature");
    sa.is_trigger = true;
    sa.source_zone_timestamp = Some(game.card(avenger).zone_timestamp);

    assert!(can_be_targeted_by_sa(&game, warden, p0, &sa));
    game.card_mut(avenger).set_color(ColorSet::WHITE);
    assert!(!can_be_targeted_by_sa(&game, warden, p0, &sa));
}

#[test]
fn a_drain_fizzles_when_its_target_player_gains_hexproof_before_it_resolves() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let drainer = put(&mut game, DRAINER, p0);
    let ability = game
        .card(drainer)
        .get_s_var("TrigLoseLife")
        .unwrap()
        .to_string();
    let mut sa = manabrew_engine::spellability::build_spell_ability(&game, drainer, &ability, p0);
    sa.is_trigger = true;
    sa.target_chosen.target_player = Some(p1);
    game.stack.push(entry(sa));
    put(&mut game, SHIELD, p1);
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    assert_eq!(game.player(p1).life, 20);
    assert_eq!(game.player(p0).life, 20);
}

#[test]
fn a_resolving_ability_reads_its_hosts_turn_condition_for_its_own_controller() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    game.turn.active_player = p0;
    game.turn.priority_player = p0;
    let consul = put(&mut game, CONSUL, p1);
    let rules = parse_card_script(TURN_NINJA).expect("script");
    let ninja = game.create_card(CardInstance::from_rules(&rules, p0));
    game.move_card(ninja, ZoneType::Hand, p0);
    let mut gain = SpellAbility::new_simple(
        Some(consul),
        p1,
        "DB$ GainLife | Defined$ You | LifeAmount$ 5",
    );
    gain.is_trigger = true;
    game.stack.push(entry(gain));
    let text = game.card(ninja).activated_abilities[0].ability_text.clone();
    let mut sa = manabrew_engine::spellability::build_spell_ability(&game, ninja, &text, p0);
    sa.is_activated = true;
    game.stack.push(entry(sa));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    while !game.stack.is_empty() {
        game_loop.step_with_priority(&mut game, &mut agents, false);
    }

    assert_eq!(game.card(ninja).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p1).life, 26);
}
