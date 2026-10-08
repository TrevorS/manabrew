use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::StackEntry;

const SPELL_WATCHER: &str = "Name:Spell Watcher\nManaCost:1 U\nTypes:Creature Wizard\nPT:1/1\nT:Mode$ SpellCast | ValidCard$ Instant,Sorcery | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever you cast an instant or sorcery spell, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";
const FREE_LIFE: &str = "Name:Free Life\nManaCost:1 W\nTypes:Instant\nA:SP$ GainLife | LifeAmount$ 2 | Defined$ You | SpellDescription$ You gain 2 life.\nOracle:";
const SPREE_LIFE: &str = "Name:Spree Life\nManaCost:U\nTypes:Instant\nK:Spree\nA:SP$ Charm | Choices$ DBGain | MinCharmNum$ 1 | CharmNum$ 1\nSVar:DBGain:DB$ GainLife | ModeCost$ 2 | LifeAmount$ 10 | Defined$ You | SpellDescription$ You gain 10 life.\nOracle:";
const FREE_CASTER: &str = "Name:Free Caster\nManaCost:5 R R\nTypes:Sorcery\nA:SP$ Play | Valid$ Card.IsRemembered | ValidSA$ Spell | ValidZone$ Exile | WithoutManaCost$ True | Amount$ All | Controller$ You | Optional$ True | SpellDescription$ Cast any number of them without paying their mana costs.\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn a_cancelled_free_cast_keeps_the_triggers_of_an_earlier_one() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, SPELL_WATCHER, p0, ZoneType::Battlefield);
    let free = put(&mut game, FREE_LIFE, p0, ZoneType::Exile);
    let spree = put(&mut game, SPREE_LIFE, p0, ZoneType::Exile);
    let caster = put(&mut game, FREE_CASTER, p0, ZoneType::Stack);
    game.card_mut(caster).add_remembered_card(free);
    game.card_mut(caster).add_remembered_card(spree);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut sa = manabrew_engine::spellability::build_spell_ability(
        &game,
        caster,
        "SP$ Play | Valid$ Card.IsRemembered | ValidSA$ Spell | ValidZone$ Exile | WithoutManaCost$ True | Amount$ All | Controller$ You | Optional$ True",
        p0,
    );
    sa.is_spell = true;
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: Some(ZoneType::Hand),
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(free).zone, ZoneType::Graveyard);
    assert_eq!(game.card(spree).zone, ZoneType::Exile);
    assert_eq!(game.player(p0).life, 23);
}

#[test]
fn a_free_cast_plays_a_card_face_down_in_exile_from_its_original_state() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let free = put(&mut game, FREE_LIFE, p0, ZoneType::Exile);
    game.card_mut(free).set_face_down(true);
    let other = put(&mut game, FREE_LIFE, p0, ZoneType::Exile);
    let caster = put(&mut game, FREE_CASTER, p0, ZoneType::Stack);
    game.card_mut(caster).add_remembered_card(free);
    game.card_mut(caster).add_remembered_card(other);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut sa = manabrew_engine::spellability::build_spell_ability(
        &game,
        caster,
        "SP$ Play | Valid$ Card.IsRemembered | ValidSA$ Spell | ValidZone$ Exile | WithoutManaCost$ True | Amount$ All | Controller$ You | Optional$ True",
        p0,
    );
    sa.is_spell = true;
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: Some(ZoneType::Hand),
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(free).zone, ZoneType::Graveyard);
    assert!(!game.card(free).face_down);
    assert_eq!(game.player(p0).life, 24);
}

const PALE_LAND: &str = "Name:Pale Land\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ W | SpellDescription$ Add {W}.\nOracle:";
const BIG_BOLT: &str = "Name:Big Bolt\nManaCost:4 R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 5 | SpellDescription$ CARDNAME deals 5 damage to any target.\nOracle:";

#[test]
fn a_mana_value_limited_play_reads_a_face_down_exiled_card_from_its_original_state() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let land = put(&mut game, PALE_LAND, p0, ZoneType::Exile);
    let big = put(&mut game, BIG_BOLT, p0, ZoneType::Exile);
    let small = put(&mut game, FREE_LIFE, p0, ZoneType::Exile);
    for card in [land, big, small] {
        game.card_mut(card).set_face_down(true);
    }
    let caster = put(&mut game, FREE_CASTER, p0, ZoneType::Stack);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let sa = manabrew_engine::spellability::build_spell_ability(
        &game,
        caster,
        "SP$ Play | Valid$ Card.IsRemembered | ValidSA$ Spell.cmcLE4 | ValidZone$ Exile | WithoutManaCost$ True | Optional$ True",
        p0,
    );
    let spells = |card| {
        manabrew_engine::ability::ability_utils::get_spells_from_play_effect(
            &game,
            card,
            p0,
            false,
            Some(("Spell.cmcLE4", &sa)),
        )
        .len()
    };
    assert_eq!(spells(land), 0);
    assert_eq!(spells(big), 0);
    assert_eq!(spells(small), 1);
}
