use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::card::card_copy_service::{copy_card, get_lki_copy};
use manabrew_engine::card::card_factory::copy_spell_ability;
use manabrew_engine::card::valid_filter::MatchContext;
use manabrew_engine::card::CardInstance;
use manabrew_engine::event::RunParams;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::keyword::Keyword;
use manabrew_engine::spellability::target_restrictions::{
    filter_spells_by_type, get_all_candidates_spells,
};
use manabrew_engine::spellability::{
    build_spell_ability, build_spell_ability_from_host_card, matches_valid_sa, SpellAbility,
    StackEntry,
};

const IMMORTAL_SUN: &str = "Name:The Immortal Sun\nManaCost:6\nTypes:Legendary Artifact\nS:Mode$ CantBeActivated | Activator$ Player | ValidCard$ Planeswalker | ValidSA$ Activated.Loyalty | Description$ Players can't activate planeswalkers' loyalty abilities.\nOracle:";
const PLANESWALKER: &str = "Name:Test Walker\nManaCost:2 U U\nTypes:Legendary Planeswalker Jace\nLoyalty:3\nA:AB$ Draw | Cost$ AddCounter<1/LOYALTY> | Planeswalker$ True | NumCards$ 1 | SpellDescription$ Draw a card.\nA:AB$ Mana | Cost$ T | Produced$ U | SpellDescription$ Add {U}.\nA:AB$ Draw | Cost$ 1 T | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";
const CYCLER: &str = "Name:Test Cycler\nManaCost:1 W\nTypes:Creature Human\nPT:2/2\nK:Cycling:2\nA:AB$ Draw | Cost$ 1 T | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";
const WARDED: &str =
    "Name:Test Warded\nManaCost:1 U\nTypes:Creature Spirit\nPT:2/2\nK:Ward:2\nOracle:";
const RESONATOR: &str = "Name:Strionic Resonator\nManaCost:2\nTypes:Artifact\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const ORNITHOPTER: &str =
    "Name:Ornithopter\nManaCost:0\nTypes:Artifact Creature Thopter\nPT:0/2\nOracle:";
const SHOCK: &str = "Name:Shock\nManaCost:R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn ability(game: &GameState, card: CardId, index: usize) -> SpellAbility {
    let text = game.card(card).activated_abilities[index]
        .ability_text
        .clone();
    build_spell_ability(game, card, &text, game.card(card).controller)
}

fn matches(game: &GameState, filter: &str, sa: &SpellAbility, source: CardId) -> bool {
    let host = sa.source.map(|id| game.card(id));
    matches_valid_sa(filter, sa, host, MatchContext::new(game.card(source), game))
}

fn push(game: &mut GameState, sa: SpellAbility) -> u32 {
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
    })
}

fn spell(card: CardId, controller: PlayerId) -> SpellAbility {
    let mut sa = SpellAbility::new_simple(Some(card), controller, "");
    sa.is_spell = true;
    sa
}

fn trigger(card: CardId, controller: PlayerId) -> SpellAbility {
    let mut sa = SpellAbility::new_simple(Some(card), controller, "DB$ Draw");
    sa.is_trigger = true;
    sa
}

#[test]
fn the_immortal_sun_matches_only_loyalty_abilities() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sun = put(&mut game, IMMORTAL_SUN, p0, ZoneType::Battlefield);
    let walker = put(&mut game, PLANESWALKER, p0, ZoneType::Battlefield);

    assert!(matches(
        &game,
        "Activated.Loyalty",
        &ability(&game, walker, 0),
        sun
    ));
    assert!(!matches(
        &game,
        "Activated.Loyalty",
        &ability(&game, walker, 2),
        sun
    ));
}

#[test]
fn overwhelming_splendor_leaves_mana_and_loyalty_abilities_alone() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let walker = put(&mut game, PLANESWALKER, p0, ZoneType::Battlefield);
    let filter = "Activated.!ManaAbility+!Loyalty";

    assert!(!matches(&game, filter, &ability(&game, walker, 0), walker));
    assert!(!matches(&game, filter, &ability(&game, walker, 1), walker));
    assert!(matches(&game, filter, &ability(&game, walker, 2), walker));
}

#[test]
fn cycling_matches_the_ability_its_keyword_built() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let cycler = put(&mut game, CYCLER, p0, ZoneType::Hand);
    let abilities = &game.card(cycler).activated_abilities;
    let cycling = abilities
        .iter()
        .position(|ab| ab.ability_text.contains("Discard<1/CARDNAME>"))
        .expect("cycling ability");
    let draw = abilities
        .iter()
        .position(|ab| ab.ability_text.starts_with("AB$ Draw | Cost$ 1 T"))
        .expect("draw ability");

    assert!(matches(
        &game,
        "Activated.Cycling",
        &ability(&game, cycler, cycling),
        cycler
    ));
    assert!(!matches(
        &game,
        "Activated.Cycling",
        &ability(&game, cycler, draw),
        cycler
    ));
}

#[test]
fn a_negated_restriction_negates_its_properties_too() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let mine = put(&mut game, BEARS, p0, ZoneType::Hand);
    let theirs = put(&mut game, BEARS, p1, ZoneType::Hand);

    assert!(!matches(&game, "!Spell.YouCtrl", &spell(mine, p0), mine));
    assert!(matches(&game, "!Spell.YouCtrl", &spell(theirs, p1), mine));
}

#[test]
fn strionic_resonator_targets_only_triggers_its_controller_controls() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let resonator = put(&mut game, RESONATOR, p0, ZoneType::Battlefield);
    let my_host = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let their_host = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let mine = push(&mut game, trigger(my_host, p0));
    push(&mut game, trigger(their_host, p1));

    let candidates = get_all_candidates_spells(&game);
    assert_eq!(
        filter_spells_by_type(&game, p0, Some(resonator), &candidates, "Triggered.YouCtrl"),
        vec![mine]
    );
}

#[test]
fn radiate_copies_only_a_spell_with_one_target() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let shock = put(&mut game, SHOCK, p1, ZoneType::Stack);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let mut one_target =
        build_spell_ability(&game, shock, &game.card(shock).abilities[0].clone(), p1);
    one_target.target_chosen.target_card = Some(bears);
    let mut two_targets = one_target.clone();
    two_targets.target_chosen.target_player = Some(p0);

    assert!(matches(&game, "Spell.numTargets EQ1", &one_target, shock));
    assert!(!matches(&game, "Spell.numTargets EQ1", &two_targets, shock));
}

#[test]
fn consign_to_memory_counters_colorless_spells_and_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let consign = put(&mut game, RESONATOR, p0, ZoneType::Battlefield);
    let thopter = put(&mut game, ORNITHOPTER, p1, ZoneType::Stack);
    let bears = put(&mut game, BEARS, p1, ZoneType::Stack);
    let host = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let filter = "Spell.Colorless,Triggered";

    assert!(matches(&game, filter, &spell(thopter, p1), consign));
    assert!(!matches(&game, filter, &spell(bears, p1), consign));
    assert!(matches(&game, filter, &trigger(host, p1), consign));
}

#[test]
fn a_keyword_tag_survives_copies_and_lki() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let cycler = put(&mut game, CYCLER, p0, ZoneType::Hand);
    let index = game
        .card(cycler)
        .activated_abilities
        .iter()
        .position(|ab| ab.ability_text.contains("Discard<1/CARDNAME>"))
        .expect("cycling ability");
    let text = game.card(cycler).activated_abilities[index]
        .ability_text
        .clone();

    let lki = get_lki_copy(game.card(cycler));
    assert!(build_spell_ability_from_host_card(&lki, &text, p0).is_cycling());
    let copy = copy_card(game.card(cycler), false, None, None);
    let copied_text = &copy.activated_abilities[index].ability_text;
    assert!(build_spell_ability_from_host_card(&copy, copied_text, p0).is_cycling());
    let cycling = ability(&game, cycler, index);
    assert!(copy_spell_ability(&mut game, &cycling, p0).is_cycling());

    let warded = put(&mut game, WARDED, p0, ZoneType::Battlefield);
    let (trigger_index, trigger) = game
        .card(warded)
        .triggers
        .iter()
        .enumerate()
        .find(|(_, trigger)| trigger.execute.starts_with("TrigWard"))
        .expect("ward trigger");
    let ward = trigger.clone().build_triggered_spell_ability(
        &game,
        warded,
        p0,
        trigger_index,
        &RunParams::default(),
    );
    assert!(ward.is_keyword(Keyword::Ward));
    assert!(copy_spell_ability(&mut game, &ward, p0).is_keyword(Keyword::Ward));
}
