use forge_carddb::parse_card_script;
use forge_foundation::{CardTypeLine, ColorSet, ManaCost, PhaseType, ZoneType};
/// Integration tests for Zone Change Effects (Issue #13):
/// ChangeZone, ChangeZoneAll, Sacrifice, SacrificeAll
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::{CardInstance, CounterType};
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::game_rng::GameRng;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

// ── Card constructors ────────────────────────────────────────────────

fn make_grizzly_bears(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Grizzly Bears".to_string(),
        owner,
        CardTypeLine::parse("Creature - Bear"),
        ManaCost::parse("1 G"),
        ColorSet::GREEN,
        Some(2),
        Some(2),
        vec![],
        vec![],
    )
}

fn make_forest(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Forest".to_string(),
        owner,
        CardTypeLine::parse("Basic Land - Forest"),
        ManaCost::no_cost(),
        ColorSet::COLORLESS,
        None,
        None,
        vec![],
        vec![],
    )
}

fn make_mountain(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Mountain".to_string(),
        owner,
        CardTypeLine::parse("Basic Land - Mountain"),
        ManaCost::no_cost(),
        ColorSet::COLORLESS,
        None,
        None,
        vec![],
        vec![],
    )
}

fn make_test_source(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Test Source".to_string(),
        owner,
        CardTypeLine::parse("Artifact"),
        ManaCost::no_cost(),
        ColorSet::COLORLESS,
        None,
        None,
        vec![],
        vec![],
    )
}

fn effect_source(game: &mut GameState, controller: PlayerId) -> CardId {
    let source = game.create_card(make_test_source(controller));
    game.move_card(source, ZoneType::Command, controller);
    source
}

/// Build a minimal 2-agent PassAgent slice for tests that don't care about choices.
fn pass_agents() -> Vec<Box<dyn PlayerAgent>> {
    vec![Box::new(PassAgent), Box::new(PassAgent)]
}

struct ReverseShuffleRng;

impl GameRng for ReverseShuffleRng {
    fn shuffle_cards(&mut self, cards: &mut [CardId]) {
        cards.reverse();
    }

    fn next_int(&mut self, _bound: i32) -> i32 {
        0
    }
}

/// Push a fake non-permanent spell entry for testing effect resolution.
fn push_effect_entry(
    game: &mut GameState,
    controller: PlayerId,
    ability_text: &str,
    target_card: Option<CardId>,
    target_player: Option<PlayerId>,
    source: Option<CardId>,
) {
    let mut sa = SpellAbility::new_simple(source, controller, ability_text);
    sa.target_chosen.target_card = target_card;
    sa.target_chosen.target_player = target_player;
    let entry = StackEntry {
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
    game.stack.push(entry);
}

// ── Test 1: Bounce to Hand (Battlefield → Hand) ──────────────────────

/// ChangeZone Battlefield→Hand on a targeted creature (bounce effect like Unsummon).
#[test]
fn test_bounce_to_hand() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Put a creature on Bob's battlefield
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);

    // Alice casts an Unsummon-like effect targeting Bob's creature
    let ability = "SP$ ChangeZone | Origin$ Battlefield | Destination$ Hand | ValidTgts$ Creature";
    push_effect_entry(&mut game, p0, ability, Some(bears), None, None);

    // Create a fake source card for the effect (not important, just needs to exist)
    // Actually, the effect resolves the stack and moves to graveyard, but there's no source card.
    // We need to wrap this as a triggered/activated ability so it doesn't try to move a card
    // from source. Let's use an activated ability entry instead.
    // Clear the stack and use is_activated_ability = true
    game.stack.pop();

    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    sa.target_chosen.target_card = Some(bears);
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.game_rng = Box::new(ReverseShuffleRng);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Creature should now be in Bob's hand
    assert_eq!(
        game.card(bears).zone,
        ZoneType::Hand,
        "Bounced creature should be in Bob's hand"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        0,
        "Bob's battlefield should be empty"
    );
    assert_eq!(
        game.zone(ZoneType::Hand, p1).len(),
        1,
        "Bob should have 1 card in hand (bounced creature)"
    );
}

// ── Test 2: Exile Permanent (Battlefield → Exile) ────────────────────

/// ChangeZone Battlefield→Exile on a targeted creature (Swords to Plowshares style).
#[test]
fn test_exile_permanent() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Put a creature on Bob's battlefield
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);

    let ability = "SP$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Creature";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    sa.target_chosen.target_card = Some(bears);
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    assert_eq!(
        game.card(bears).zone,
        ZoneType::Exile,
        "Exiled creature should be in Exile zone"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        0,
        "Bob's battlefield should be empty"
    );
}

// ── Test 3: Reanimate (Graveyard → Battlefield) ──────────────────────

/// ChangeZone Graveyard→Battlefield on a creature in the graveyard (Animate Dead style).
#[test]
fn test_reanimate() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Put a creature in Bob's graveyard
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Graveyard, p1);

    // Alice reanimates it onto the battlefield
    let ability = "SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature.inZoneGraveyard";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    sa.target_chosen.target_card = Some(bears);
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Without explicit gain-control text, the card returns under its owner's control.
    assert_eq!(
        game.card(bears).zone,
        ZoneType::Battlefield,
        "Reanimated creature should be on the battlefield"
    );
    assert_eq!(
        game.zone(ZoneType::Graveyard, p1).len(),
        0,
        "Bob's graveyard should be empty after reanimation"
    );
    // It returns under Bob's (owner's) control.
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        1,
        "The reanimated creature should be on Bob's battlefield (owner controls it)"
    );
}

// ── Test 4: Raise Dead (Graveyard → Hand) ────────────────────────────

/// ChangeZone Graveyard→Hand on a creature in the graveyard (Raise Dead style).
#[test]
fn test_raise_dead() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let _p1 = PlayerId(1);

    // Put a creature in Alice's graveyard
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Graveyard, p0);

    // Alice casts Raise Dead targeting her own creature
    let ability = "SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand | ValidTgts$ Creature.inZoneGraveyard";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    sa.target_chosen.target_card = Some(bears);
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    assert_eq!(
        game.card(bears).zone,
        ZoneType::Hand,
        "Raised creature should be in hand"
    );
    assert_eq!(
        game.zone(ZoneType::Graveyard, p0).len(),
        0,
        "Alice's graveyard should be empty"
    );
    assert_eq!(
        game.zone(ZoneType::Hand, p0).len(),
        1,
        "Alice should have 1 card in hand"
    );
}

/// ChangeZone Graveyard->Library with Shuffle$ True must actually shuffle the
/// destination library, matching Java's ChangeZoneEffect behavior.
#[test]
fn test_graveyard_to_library_with_shuffle() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);

    let forest = game.create_card(make_forest(p0));
    let mountain_one = game.create_card(make_mountain(p0));
    let mountain_two = game.create_card(make_mountain(p0));
    let bears = game.create_card(make_grizzly_bears(p0));

    game.move_card(forest, ZoneType::Library, p0);
    game.move_card(mountain_one, ZoneType::Library, p0);
    game.move_card(mountain_two, ZoneType::Library, p0);
    game.move_card(bears, ZoneType::Graveyard, p0);

    let expected_without_shuffle = vec![forest, mountain_one, mountain_two, bears];

    let ability =
        "SP$ ChangeZone | Origin$ Graveyard | Destination$ Library | Defined$ Self | Shuffle$ True";
    let mut sa = SpellAbility::new_simple(Some(bears), p0, ability);
    sa.is_activated = true;
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.game_rng = Box::new(ReverseShuffleRng);
    game_loop.resolve_stack(&mut game, &mut agents);

    assert_eq!(
        game.card(bears).zone,
        ZoneType::Library,
        "The moved card should end up in the library"
    );
    assert_ne!(
        game.zone(ZoneType::Library, p0).cards,
        expected_without_shuffle,
        "Shuffle$ True should not leave the library in simple append order"
    );
}

// ── Test 5: ChangeZoneAll Board Wipe (Battlefield → Exile) ───────────

/// ChangeZoneAll Battlefield→Exile moves all creatures off the battlefield.
#[test]
fn test_change_zone_all_board_wipe() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Put creatures on both battlefields
    let alice_bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(alice_bears, ZoneType::Battlefield, p0);

    let bob_bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bob_bears, ZoneType::Battlefield, p1);

    // Also put a land on Alice's battlefield — it should NOT be exiled
    let alice_forest = game.create_card(make_forest(p0));
    game.move_card(alice_forest, ZoneType::Battlefield, p0);

    // Exile all creatures (Cataclysm-style)
    let ability =
        "SP$ ChangeZoneAll | Origin$ Battlefield | Destination$ Exile | ValidCards$ Creature";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Both creatures should be exiled
    assert_eq!(
        game.card(alice_bears).zone,
        ZoneType::Exile,
        "Alice's creature should be in Exile"
    );
    assert_eq!(
        game.card(bob_bears).zone,
        ZoneType::Exile,
        "Bob's creature should be in Exile"
    );
    // Land should remain on battlefield
    assert_eq!(
        game.card(alice_forest).zone,
        ZoneType::Battlefield,
        "Land should remain on the battlefield"
    );
    // Both players' battlefields should only have land (Alice has 1, Bob has 0)
    assert_eq!(
        game.zone(ZoneType::Battlefield, p0).len(),
        1,
        "Alice's battlefield should have only the land"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        0,
        "Bob's battlefield should be empty"
    );
}

// ── Test 6: Sacrifice Self (SacValid$ Self) ───────────────────────────

/// Sacrifice with SacValid$=Self sacrifices the source card.
#[test]
fn test_sacrifice_self() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let _p1 = PlayerId(1);

    // Put a creature on Alice's battlefield as the source of the ability
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);

    // The creature sacrifices itself (like a Loxodon Warhammer echo)
    let ability = "SP$ Sacrifice | SacValid$ Self";
    let mut sa = SpellAbility::new_simple(Some(bears), p0, ability); // source is the creature that sacrifices
    sa.is_activated = true;
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    assert_eq!(
        game.card(bears).zone,
        ZoneType::Graveyard,
        "Self-sacrificed creature should be in graveyard"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p0).len(),
        0,
        "Alice's battlefield should be empty"
    );
}

// ── Test 7: SacrificeAll Creatures ────────────────────────────────────

/// SacrificeAll moves all creatures from battlefield to graveyard.
#[test]
fn test_sacrifice_all_creatures() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Put creatures on both battlefields
    let alice_bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(alice_bears, ZoneType::Battlefield, p0);

    let bob_bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bob_bears, ZoneType::Battlefield, p1);

    // Put a land on Alice's battlefield — it should survive
    let alice_forest = game.create_card(make_forest(p0));
    game.move_card(alice_forest, ZoneType::Battlefield, p0);

    // Sacrifice all creatures (Overwhelming Splendor style)
    let ability = "SP$ SacrificeAll | ValidCards$ Creature";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Both creatures should be in their owners' graveyards
    assert_eq!(
        game.card(alice_bears).zone,
        ZoneType::Graveyard,
        "Alice's creature should be in graveyard"
    );
    assert_eq!(
        game.card(bob_bears).zone,
        ZoneType::Graveyard,
        "Bob's creature should be in graveyard"
    );
    // Land survives
    assert_eq!(
        game.card(alice_forest).zone,
        ZoneType::Battlefield,
        "Land should still be on battlefield"
    );
    // Graveyard counts
    assert_eq!(
        game.zone(ZoneType::Graveyard, p0).len(),
        1,
        "Alice's graveyard should have 1 card"
    );
    assert_eq!(
        game.zone(ZoneType::Graveyard, p1).len(),
        1,
        "Bob's graveyard should have 1 card"
    );
}

// ── Test 8: Sacrifice Defined$ Player — each player sacrifices ────────

/// Sacrifice with Defined$ Player (Innocent Blood) makes BOTH players sacrifice a creature.
#[test]
fn test_sacrifice_each_player() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Both players have a creature on the battlefield
    let alice_bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(alice_bears, ZoneType::Battlefield, p0);

    let bob_bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bob_bears, ZoneType::Battlefield, p1);

    // Innocent Blood: each player sacrifices a creature
    let ability = "SP$ Sacrifice | SacValid$ Creature | Defined$ Player";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Both creatures must have been sacrificed
    assert_eq!(
        game.card(alice_bears).zone,
        ZoneType::Graveyard,
        "Alice's creature should be sacrificed (Defined$ Player affects each player)"
    );
    assert_eq!(
        game.card(bob_bears).zone,
        ZoneType::Graveyard,
        "Bob's creature should be sacrificed (Defined$ Player affects each player)"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p0).len(),
        0,
        "Alice's battlefield should be empty"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        0,
        "Bob's battlefield should be empty"
    );
}

// ── Test 9: Tuck to Library Bottom (LibraryPosition$ -1) ─────────────

/// ChangeZone Battlefield→Library with LibraryPosition$=-1 places the card at the bottom.
#[test]
fn test_tuck_to_library_bottom() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Give Alice some cards in her library first
    let library_card1 = game.create_card(make_mountain(p0));
    game.move_card(library_card1, ZoneType::Library, p0);
    let library_card2 = game.create_card(make_forest(p0));
    game.move_card(library_card2, ZoneType::Library, p0);

    // Put a creature on Bob's battlefield that will get tucked
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);

    // Alice casts Condemn on Bob's attacking creature — puts it on bottom of its owner's library
    let ability = "SP$ ChangeZone | Origin$ Battlefield | Destination$ Library | LibraryPosition$ -1 | ValidTgts$ Creature";
    let mut sa = SpellAbility::new_simple(Some(effect_source(&mut game, p0)), p0, ability);
    sa.is_activated = true;
    sa.target_chosen.target_card = Some(bears);
    let entry = StackEntry {
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
    game.stack.push(entry);

    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);

    // Creature should be in Bob's library (its owner)
    assert_eq!(
        game.card(bears).zone,
        ZoneType::Library,
        "Tucked creature should be in Bob's library"
    );
    assert_eq!(
        game.zone(ZoneType::Battlefield, p1).len(),
        0,
        "Bob's battlefield should be empty"
    );

    // The tucked card should be at the bottom (index 0 in our internal representation)
    // Bob's library only has the bears card
    let bob_library = &game.zone(ZoneType::Library, p1).cards;
    assert_eq!(bob_library.len(), 1, "Bob's library should have 1 card");
    assert_eq!(
        bob_library[0], bears,
        "Tucked creature should be at bottom of Bob's library (index 0)"
    );

    // Alice's library should be untouched
    assert_eq!(
        game.zone(ZoneType::Library, p0).len(),
        2,
        "Alice's library should still have 2 cards"
    );
}

#[test]
fn mana_spent_to_cast_belongs_to_the_object_that_was_cast() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);
    game.card_mut(bears).set_paying_mana_to_cast(vec![8, 16]);
    game.card_mut(bears).set_colors_spent_to_cast(24);
    let count = |game: &GameState| {
        manabrew_engine::svar::resolve_count_svar("Count$CastTotalManaSpent", game, bears, p0)
    };
    assert_eq!(count(&game), 2);
    game.move_card(bears, ZoneType::Exile, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);
    assert_eq!(count(&game), 0);
    assert_eq!(game.card(bears).colors_spent_to_cast, 0);
}

const GRAVE_WATCHER: &str = "Name:Grave Watcher\nManaCost:B\nTypes:Creature Elemental\nPT:1/1\nT:Mode$ ChangesZoneAll | ValidCards$ Permanent.YouOwn+!token | Origin$ Any | Destination$ Graveyard | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever one or more permanent cards are put into your graveyard, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";
const DEATH_WARDEN: &str = "Name:Death Warden\nManaCost:2 B B\nTypes:Creature Wolf\nPT:4/3\nR:Event$ Moved | ActiveZones$ Battlefield | Origin$ Battlefield | Destination$ Graveyard | ValidLKI$ Card.Creature+OppCtrl | ReplaceWith$ DBExile | Description$ If a creature an opponent controls would die, exile it instead.\nSVar:DBExile:DB$ ChangeZone | Hidden$ True | Origin$ All | Destination$ Exile | Defined$ ReplacedCard\nOracle:";

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

#[test]
fn a_card_returned_by_a_later_sub_ability_does_not_see_an_earlier_sub_abilitys_moves() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);
    let watcher = put(&mut game, GRAVE_WATCHER, p0, ZoneType::Graveyard);
    let source = effect_source(&mut game, p0);
    let mut sa = SpellAbility::new_simple(
        Some(source),
        p0,
        "DB$ SacrificeAll | ValidCards$ Creature.YouCtrl",
    );
    sa.append_sub_ability(SpellAbility::new_simple(
        Some(source),
        p0,
        "DB$ ChangeZoneAll | ChangeType$ Elemental.YouOwn | Origin$ Graveyard | Destination$ Battlefield",
    ));
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
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(bears).zone, ZoneType::Graveyard);
    assert_eq!(game.card(watcher).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p0).life, 20);
}

const CONSUL_WATCH: &str = "Name:Consul Watch\nManaCost:W\nTypes:Enchantment\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.OppCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever a creature an opponent controls enters, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const SNARE: &str = "Name:Snare\nManaCost:2 W\nTypes:Artifact\nOracle:";
const TORMENT: &str = "Name:Torment\nManaCost:B\nTypes:Sorcery\nSVar:DBTorment:DB$ LoseLife | Defined$ Player.Opponent | LifeAmount$ 4 | UnlessCost$ Sac<1/Permanent.nonLand> | UnlessPayer$ Player.Opponent\nOracle:";

#[test]
fn a_permanent_sacrificed_for_a_later_unless_cost_still_sees_a_creature_enter() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let snare = put(&mut game, SNARE, p0, ZoneType::Battlefield);
    let watch = put(&mut game, CONSUL_WATCH, p0, ZoneType::Battlefield);
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ ChangeZone | Defined$ Targeted | Origin$ Battlefield | Destination$ Exile | Duration$ UntilHostLeavesPlay",
        Some(bears),
        None,
        Some(snare),
    );
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(bears).zone, ZoneType::Exile);
    let torment = put(&mut game, TORMENT, p1, ZoneType::Command);
    push_effect_entry(
        &mut game,
        p1,
        "DB$ Repeat | MaxRepeat$ 2 | RepeatSubAbility$ DBTorment",
        None,
        None,
        Some(torment),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert_eq!(game.card(snare).zone, ZoneType::Graveyard);
    assert_eq!(game.card(watch).zone, ZoneType::Graveyard);
    assert_eq!(game.card(bears).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p0).life, 21);
}

#[test]
fn a_replacement_on_a_permanent_destroyed_by_the_same_effect_still_applies() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    main_phase(&mut game, p0);
    let warden = put(&mut game, DEATH_WARDEN, p0, ZoneType::Battlefield);
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ DestroyAll | ValidCards$ Creature",
        None,
        None,
        Some(source),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(warden).zone, ZoneType::Graveyard);
    assert_eq!(game.card(bears).zone, ZoneType::Exile);
}

const DYING_WITNESS: &str = "Name:Dying Witness\nManaCost:1 W\nTypes:Creature Human\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Execute$ TrigGain | TriggerDescription$ When this creature dies, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";

fn wipe_under_death_warden(effect: &str) -> (GameState, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p1);
    put(&mut game, DEATH_WARDEN, p0, ZoneType::Battlefield);
    let witness = put(&mut game, DYING_WITNESS, p1, ZoneType::Battlefield);
    let source = effect_source(&mut game, p1);
    push_effect_entry(&mut game, p1, effect, None, None, Some(source));
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.resolve_stack(&mut game, &mut agents);
    (game, witness)
}

#[test]
fn a_creature_destroyed_into_exile_by_a_replacement_does_not_die() {
    let (game, witness) = wipe_under_death_warden("DB$ DestroyAll | ValidCards$ Creature");
    assert_eq!(game.card(witness).zone, ZoneType::Exile);
    assert_eq!(game.player(PlayerId(1)).life, 20);
}

#[test]
fn a_creature_sacrificed_into_exile_by_a_replacement_does_not_die() {
    let (game, witness) =
        wipe_under_death_warden("DB$ SacrificeAll | ValidCards$ Creature.YouCtrl");
    assert_eq!(game.card(witness).zone, ZoneType::Exile);
    assert_eq!(game.player(PlayerId(1)).life, 20);
}

const FOOD_GLUTTON: &str = "Name:Food Glutton\nManaCost:2 G\nTypes:Creature Elemental\nPT:2/2\nS:Mode$ Continuous | Affected$ Creature.Other | AffectedZone$ Battlefield | AddType$ Artifact & Food | Description$ Other creatures are Food artifacts in addition to their other types.\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Food | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever a Food is put into a graveyard from the battlefield, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";

#[test]
fn a_look_back_trigger_sees_every_card_its_own_wipe_destroys() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    put(&mut game, FOOD_GLUTTON, p0, ZoneType::Battlefield);
    for _ in 0..2 {
        let bears = game.create_card(make_grizzly_bears(p1));
        game.move_card(bears, ZoneType::Battlefield, p1);
    }
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ DestroyAll | ValidCards$ Creature",
        None,
        None,
        Some(source),
    );
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.resolve_stack(&mut game, &mut agents);

    assert_eq!(game.player(p0).life, 22);
}

const BRASS_BAUBLE: &str = "Name:Brass Bauble\nManaCost:0\nTypes:Artifact\nOracle:";
const RELIC_WATCHER: &str = "Name:Relic Watcher\nManaCost:1 W\nTypes:Creature Human\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Artifact.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever an artifact you control enters, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";
const KIN_WATCHER: &str = "Name:Kin Watcher\nManaCost:1 G\nTypes:Creature Elf\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.Other+YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever another creature you control enters, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";

fn return_remembered(game: &mut GameState, controller: PlayerId, cards: &[CardId]) {
    let source = effect_source(game, controller);
    for &card in cards {
        game.card_mut(source).add_remembered_card(card);
    }
    push_effect_entry(
        game,
        controller,
        "DB$ ChangeZone | Defined$ Remembered | Origin$ Graveyard | Destination$ Battlefield",
        None,
        None,
        Some(source),
    );
}

#[test]
fn a_permanent_returned_with_an_artifact_sees_the_artifact_enter() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let bauble = put(&mut game, BRASS_BAUBLE, p0, ZoneType::Graveyard);
    let watcher = put(&mut game, RELIC_WATCHER, p0, ZoneType::Graveyard);
    return_remembered(&mut game, p0, &[bauble, watcher]);
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(bauble).zone, ZoneType::Battlefield);
    assert_eq!(game.card(watcher).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p0).life, 21);
}

#[test]
fn creatures_returned_by_one_effect_each_see_the_other_enter() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let first = put(&mut game, KIN_WATCHER, p0, ZoneType::Graveyard);
    let second = put(&mut game, KIN_WATCHER, p0, ZoneType::Graveyard);
    return_remembered(&mut game, p0, &[first, second]);
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(first).zone, ZoneType::Battlefield);
    assert_eq!(game.card(second).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p0).life, 22);
}

const THIEF_OF_BLOOD: &str = "Name:Thief of Blood\nManaCost:4 B B\nTypes:Creature Vampire\nPT:1/1\nK:Flying\nK:ETBReplacement:Other:DBRemoveCounterAll\nSVar:DBRemoveCounterAll:DB$ RemoveCounter | Defined$ Valid Permanent | CounterType$ All | SubAbility$ DBPutCounters | RememberAmount$ True | SpellDescription$ As CARDNAME enters, remove all counters from all permanents. CARDNAME enters with a +1/+1 counter on it for each counter removed this way.\nSVar:DBPutCounters:DB$ PutCounter | ETB$ True | Defined$ Self | CounterType$ P1P1 | CounterNum$ X | SubAbility$ DBCleanup\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nSVar:X:Count$RememberedNumber\nOracle:";

fn kin_watcher_life_after_a_batch_with_a_replacement(mirror_forge_bugs: bool) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = mirror_forge_bugs;
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let early = game.create_card(make_grizzly_bears(p0));
    game.move_card(early, ZoneType::Graveyard, p0);
    let thief = put(&mut game, THIEF_OF_BLOOD, p0, ZoneType::Graveyard);
    let watcher = put(&mut game, KIN_WATCHER, p0, ZoneType::Graveyard);
    let late = game.create_card(make_grizzly_bears(p0));
    game.move_card(late, ZoneType::Graveyard, p0);
    return_remembered(&mut game, p0, &[early, thief, watcher, late]);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.resolve_stack(&mut game, &mut agents);
    for card in [early, thief, watcher, late] {
        assert_eq!(game.card(card).zone, ZoneType::Battlefield);
    }
    game.player(p0).life
}

#[test]
fn a_creature_returned_in_a_batch_sees_every_other_creature_in_it_enter() {
    assert_eq!(kin_watcher_life_after_a_batch_with_a_replacement(false), 23);
}

#[test]
fn forge_collects_a_batch_arrival_before_the_watcher_enters_when_a_replacement_resolves() {
    assert_eq!(kin_watcher_life_after_a_batch_with_a_replacement(true), 22);
}

const GRAVE_LEAVE_WATCHER: &str = "Name:Grave Leave Watcher\nManaCost:R W\nTypes:Creature Bird\nPT:2/1\nT:Mode$ ChangesZoneAll | ValidCards$ Card.YouOwn | Origin$ Graveyard | Destination$ Any | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever one or more cards leave your graveyard, you gain 3 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 3 | Defined$ You\nOracle:";

#[test]
fn a_graveyard_leave_trigger_sees_the_graveyard_exiled_with_its_host() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let watcher = put(&mut game, GRAVE_LEAVE_WATCHER, p0, ZoneType::Battlefield);
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Graveyard, p0);
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ ChangeZoneAll | ChangeType$ Creature.YouOwn | Origin$ Battlefield,Graveyard | Destination$ Exile",
        None,
        None,
        Some(source),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(watcher).zone, ZoneType::Exile);
    assert_eq!(game.card(bears).zone, ZoneType::Exile);
    assert_eq!(game.player(p0).life, 23);
}

const TAPPING_SHRINE: &str = "Name:Tapping Shrine\nManaCost:W\nTypes:Enchantment\nR:Event$ Moved | ValidCard$ Creature.OppCtrl | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Creatures your opponents control enter tapped.\nSVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard\nOracle:";

#[test]
fn a_replacement_host_returned_by_an_earlier_sub_ability_no_longer_applies() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    main_phase(&mut game, p1);
    let shrine = put(&mut game, TAPPING_SHRINE, p0, ZoneType::Battlefield);
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Graveyard, p1);
    let source = effect_source(&mut game, p1);
    game.card_mut(source).add_remembered_card(shrine);
    let mut sa = SpellAbility::new_simple(
        Some(source),
        p1,
        "DB$ ChangeZone | Defined$ Remembered | Origin$ Battlefield | Destination$ Hand",
    );
    sa.append_sub_ability(SpellAbility::new_simple(
        Some(source),
        p1,
        "DB$ ChangeZoneAll | ChangeType$ Creature.YouOwn | Origin$ Graveyard | Destination$ Battlefield",
    ));
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
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(shrine).zone, ZoneType::Hand);
    assert_eq!(game.card(bears).zone, ZoneType::Battlefield);
    assert!(!game.card(bears).tapped);
}

const RUDE_CLASS: &str = "Name:Rude Class\nManaCost:1 R\nTypes:Enchantment Class\nK:Class:2:1 R:AddTrigger$ TriggerDiscard\nSVar:TriggerDiscard:Mode$ Discarded | ValidCard$ Card.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigDamage | Secondary$ True | TriggerDescription$ Whenever you discard a card, this Class deals 2 damage to each opponent.\nSVar:TrigDamage:DB$ DealDamage | Defined$ Player.Opponent | NumDmg$ 2\nOracle:";

#[test]
fn a_class_that_leaves_the_battlefield_returns_at_level_one() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let class = put(&mut game, RUDE_CLASS, p0, ZoneType::Battlefield);
    game.card_mut(class).set_class_level(2);

    game.move_card(class, ZoneType::Exile, p0);
    game.move_card(class, ZoneType::Battlefield, p0);

    assert_eq!(game.card(class).class_level, 1);
}

#[test]
fn a_goaded_creature_that_leaves_the_battlefield_returns_ungoaded() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);
    game.card_mut(bears).add_goad(p1);

    game.move_card(bears, ZoneType::Exile, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);

    assert_eq!(game.card(bears).goaded_by, None);
}

#[test]
fn a_regeneration_shield_does_not_follow_a_creature_out_of_the_battlefield() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);
    game.card_mut(bears).regeneration_shields = 1;

    game.move_card(bears, ZoneType::Hand, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);

    assert_eq!(game.card(bears).regeneration_shields, 0);
}

const SEARCH_WATCHER: &str = "Name:Search Watcher\nManaCost:1 U\nTypes:Creature Bird\nPT:1/1\nT:Mode$ SearchedLibrary | ValidPlayer$ Player.Opponent | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever an opponent searches their library, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";

fn watcher_life_after(ability: &str) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p1);
    put(&mut game, SEARCH_WATCHER, p0, ZoneType::Battlefield);
    for _ in 0..3 {
        let forest = game.create_card(make_forest(p1));
        game.move_card(forest, ZoneType::Library, p1);
    }
    let source = effect_source(&mut game, p1);
    push_effect_entry(&mut game, p1, ability, None, None, Some(source));
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.resolve_stack(&mut game, &mut agents);
    game.player(p0).life
}

#[test]
fn moving_a_defined_card_out_of_a_library_is_not_a_search() {
    assert_eq!(
        watcher_life_after(
            "DB$ ChangeZone | Defined$ TopOfLibrary | Origin$ Library | Destination$ Hand"
        ),
        20
    );
    assert_eq!(
        watcher_life_after("DB$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeType$ Card | ChangeNum$ 1"),
        21
    );
}

const MIMIC: &str = "Name:Mimic\nManaCost:2 U\nTypes:Creature Shapeshifter\nPT:1/1\nOracle:";

#[test]
fn a_copied_replacement_applies_after_the_copy_dies_earlier_in_the_batch() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let mimic = put(&mut game, MIMIC, p0, ZoneType::Battlefield);
    put(&mut game, DEATH_WARDEN, p0, ZoneType::Graveyard);
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard",
        None,
        None,
        Some(mimic),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(mimic).card_name, "Death Warden");

    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ DestroyAll | ValidCards$ Creature",
        None,
        None,
        Some(source),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);

    assert_eq!(game.card(bears).zone, ZoneType::Exile);
}

const DISGUISED_SPY: &str = "Name:Disguised Spy\nManaCost:1 U\nTypes:Creature Merfolk Detective\nPT:1/1\nK:Disguise:1 U\nOracle:";

#[test]
fn an_animate_on_a_face_down_creature_ends_on_its_face_up_characteristics() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let spy = put(&mut game, DISGUISED_SPY, p0, ZoneType::Battlefield);
    manabrew_engine::card::card_factory_util::turn_face_down_with_state(game.card_mut(spy));
    push_effect_entry(
        &mut game,
        p0,
        "DB$ Animate | Defined$ Self | Power$ 3 | Toughness$ 4 | Types$ Hero | RemoveCreatureTypes$ True",
        None,
        None,
        Some(spy),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(spy).state_base_power(), 3);

    game.card_mut(spy).turn_face_up();
    assert_eq!(game.card(spy).state_base_power(), 3);
    let timestamp = game.card(spy).animate_state.as_ref().unwrap().records[0].timestamp;
    manabrew_engine::phase::PhaseCommand::RestoreAnimate {
        card: spy,
        timestamp,
    }
    .run(&mut game, &mut ReverseShuffleRng);

    assert_eq!(game.card(spy).state_base_power(), 1);
    assert!(game.card(spy).type_line.to_string().contains("Merfolk"));
}

const CASE_FILE: &str = "Name:Case File\nManaCost:1 W\nTypes:Enchantment Case\nSVar:Solved:DB$ AlterAttribute | Defined$ Self | Attributes$ Solved\nOracle:";

#[test]
fn a_solved_case_that_leaves_the_battlefield_returns_unsolved_and_can_solve_again() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let case = put(&mut game, CASE_FILE, p0, ZoneType::Battlefield);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ AlterAttribute | Defined$ Self | Attributes$ Solved",
        None,
        None,
        Some(case),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert!(game.card(case).is_solved());

    game.move_card(case, ZoneType::Hand, p0);
    game.move_card(case, ZoneType::Battlefield, p0);

    assert!(!game.card(case).is_solved());
    assert_eq!(
        game.card(case).get_s_var("Solved"),
        Some("DB$ AlterAttribute | Defined$ Self | Attributes$ Solved")
    );
}

fn library_after(ability: &str, target_bears: bool) -> (Vec<CardId>, Vec<CardId>) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    for card in [make_forest(p0), make_mountain(p0), make_mountain(p0)] {
        let id = game.create_card(card);
        game.move_card(id, ZoneType::Library, p0);
    }
    let bears = game.create_card(make_grizzly_bears(p0));
    game.move_card(bears, ZoneType::Battlefield, p0);
    let before = game.zone(ZoneType::Library, p0).cards.clone();
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        ability,
        target_bears.then_some(bears),
        None,
        Some(source),
    );
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.game_rng = Box::new(ReverseShuffleRng);
    game_loop.resolve_stack(&mut game, &mut agents);
    (before, game.zone(ZoneType::Library, p0).cards.clone())
}

#[test]
fn a_targeted_creature_put_into_its_library_with_shuffle_shuffles_that_library() {
    let (_, unshuffled) = library_after(
        "SP$ ChangeZone | ValidTgts$ Creature | ValidOrigin$ Battlefield | Destination$ Library | LibraryPosition$ 0",
        true,
    );
    let (_, shuffled) = library_after(
        "SP$ ChangeZone | ValidTgts$ Creature | ValidOrigin$ Battlefield | Destination$ Library | LibraryPosition$ 0 | Shuffle$ True",
        true,
    );
    let mut expected = unshuffled.clone();
    expected.reverse();
    assert_eq!(shuffled, expected);
}

#[test]
fn a_library_search_onto_the_library_that_finds_nothing_still_shuffles() {
    let (before, after) = library_after(
        "SP$ ChangeZone | Origin$ Library | Destination$ Library | LibraryPosition$ 0 | ChangeType$ Planeswalker",
        false,
    );
    let mut expected = before.clone();
    expected.reverse();
    assert_eq!(after, expected);
}

const BARBS_WATCH: &str = "Name:Barbs Watch\nManaCost:1 R\nTypes:Creature Lizard\nPT:2/1\nT:Mode$ DamageAll | ValidTarget$ Opponent | CombatDamage$ False | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever one or more opponents are dealt noncombat damage, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";

#[test]
fn a_damage_all_trigger_sees_noncombat_damage_to_an_opponent() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    put(&mut game, BARBS_WATCH, p0, ZoneType::Battlefield);
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ DealDamage | ValidTgts$ Player | NumDmg$ 2",
        None,
        Some(p1),
        Some(source),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);

    assert_eq!(game.player(p1).life, 18);
    assert_eq!(game.player(p0).life, 21);
}

const TYPED_PASSAGE: &str = "Name:Typed Passage\nManaCost:no cost\nTypes:Land\nS:Mode$ Continuous | AffectedDefined$ Self | AddType$ Mountain | RemoveLandTypes$ True | Description$ This land is a Mountain.\nOracle:";

#[test]
fn an_earthbended_land_stays_a_creature_under_its_own_land_type_static() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let passage = put(&mut game, TYPED_PASSAGE, p0, ZoneType::Battlefield);
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ Earthbend | Num$ 2",
        Some(passage),
        None,
        Some(source),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);
    let card = game.card(passage);
    assert!(card.type_line.has_subtype("Mountain"));
    assert!(card.type_line.is_creature());
    assert_eq!(card.power(), 2);
}

const FLASHBACK_BOLT: &str = "Name:Flashback Bolt\nManaCost:R\nTypes:Instant\nK:Flashback:1 R\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";
const HILL_OGRE: &str = "Name:Hill Ogre\nManaCost:3 R\nTypes:Creature Ogre\nPT:4/4\nOracle:";

fn exile_top_card_face_down(script: &str) -> (GameState, CardId, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let card = put(&mut game, script, p0, ZoneType::Library);
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ Dig | DigNum$ 1 | ChangeNum$ All | DestinationZone$ Exile | ExileFaceDown$ True | NoReveal$ True",
        None,
        None,
        Some(source),
    );
    GameLoop::new(2).resolve_stack(&mut game, &mut pass_agents());
    assert_eq!(game.card(card).zone, ZoneType::Exile);
    (game, card, source)
}

fn matches(game: &GameState, valid: &str, card: CardId, source: CardId) -> bool {
    manabrew_engine::card::valid_filter::matches_valid_card_in_game(
        valid,
        game.card(card),
        game.card(source),
        game,
    )
}

#[test]
fn a_card_exiled_face_down_is_a_nameless_creature_without_its_keywords() {
    let (game, bolt, source) = exile_top_card_face_down(FLASHBACK_BOLT);
    assert!(game.card(bolt).face_down);
    assert!(!matches(
        &game,
        "Card.withFlashback+inZoneExile",
        bolt,
        source
    ));
    assert!(matches(&game, "Creature.inZoneExile", bolt, source));
    assert!(!matches(&game, "Instant", bolt, source));
}

#[test]
fn a_face_down_exiled_card_that_moves_to_the_graveyard_is_face_up() {
    let (mut game, bolt, source) = exile_top_card_face_down(FLASHBACK_BOLT);
    game.move_card(bolt, ZoneType::Graveyard, PlayerId(0));
    assert!(!game.card(bolt).face_down);
    assert!(matches(&game, "Instant.withFlashback", bolt, source));
}

fn put_face_down_exiled_ogre_onto_the_battlefield(ability: &str) -> (bool, i32) {
    let (mut game, ogre, source) = exile_top_card_face_down(HILL_OGRE);
    push_effect_entry(
        &mut game,
        PlayerId(0),
        ability,
        Some(ogre),
        None,
        Some(source),
    );
    GameLoop::new(2).resolve_stack(&mut game, &mut pass_agents());
    assert_eq!(game.card(ogre).zone, ZoneType::Battlefield);
    (game.card(ogre).face_down, game.card(ogre).power())
}

#[test]
fn a_face_down_exiled_card_put_onto_the_battlefield_enters_face_up() {
    assert_eq!(
        put_face_down_exiled_ogre_onto_the_battlefield(
            "DB$ ChangeZone | Origin$ Exile | Destination$ Battlefield | ValidTgts$ Card"
        ),
        (false, 4)
    );
}

#[test]
fn a_face_down_exiled_card_put_onto_the_battlefield_face_down_stays_face_down() {
    assert_eq!(
        put_face_down_exiled_ogre_onto_the_battlefield(
            "DB$ ChangeZone | Origin$ Exile | Destination$ Battlefield | ValidTgts$ Card | FaceDown$ True"
        ),
        (true, 2)
    );
}

#[test]
fn a_heisted_card_is_face_down_in_exile() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let bolt = put(&mut game, FLASHBACK_BOLT, p1, ZoneType::Library);
    let source = effect_source(&mut game, p0);
    push_effect_entry(&mut game, p0, "DB$ Heist", None, Some(p1), Some(source));
    GameLoop::new(2).resolve_stack(&mut game, &mut pass_agents());
    assert_eq!(game.card(bolt).zone, ZoneType::Exile);
    assert!(game.card(bolt).face_down);
    assert!(matches(&game, "Creature", bolt, source));
}

fn earthbended_forest_after_a_copied_holding_cell_leaves(
    mirror_forge_bugs: bool,
) -> (ZoneType, i32, bool) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = mirror_forge_bugs;
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let cell = put(&mut game, SNARE, p0, ZoneType::Battlefield);
    let original = put(&mut game, SNARE, p0, ZoneType::Graveyard);
    let forest = game.create_card(make_forest(p1));
    game.move_card(forest, ZoneType::Battlefield, p1);
    let bender = put(&mut game, CONSUL_WATCH, p1, ZoneType::Battlefield);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_effect_entry(
        &mut game,
        p1,
        "DB$ Earthbend | Num$ 2",
        Some(forest),
        None,
        Some(bender),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    let mut exile = SpellAbility::new_simple(
        Some(cell),
        p0,
        "DB$ ChangeZone | Defined$ Targeted | Origin$ Battlefield | Destination$ Exile | Duration$ UntilHostLeavesPlay",
    );
    exile.target_chosen.target_card = Some(forest);
    exile.original_host = Some(original);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: exile,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(forest).zone, ZoneType::Battlefield);
    assert!(!game.card(forest).is_creature());
    push_effect_entry(
        &mut game,
        p1,
        "DB$ Earthbend | Num$ 2",
        Some(forest),
        None,
        Some(bender),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(forest).counter_count(&CounterType::P1P1), 2);
    game.move_card(cell, ZoneType::Graveyard, p0);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    let card = game.card(forest);
    (
        card.zone,
        card.counter_count(&CounterType::P1P1),
        card.is_creature(),
    )
}

#[test]
fn a_land_returned_from_a_copied_exile_stays_put_when_the_copy_leaves() {
    assert_eq!(
        earthbended_forest_after_a_copied_holding_cell_leaves(false),
        (ZoneType::Battlefield, 2, true)
    );
}

#[test]
fn forge_moves_a_land_returned_from_a_copied_exile_again_when_the_copy_leaves() {
    assert_eq!(
        earthbended_forest_after_a_copied_holding_cell_leaves(true),
        (ZoneType::Battlefield, 0, false)
    );
}

const LEDGER_PRIEST: &str = "Name:Ledger Priest\nManaCost:W B G\nTypes:Creature Cleric\nPT:3/3\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Any | ValidCard$ Card.Self | Execute$ TrigGain | TriggerDescription$ When this leaves the battlefield, you gain life equal to the mana value of the exiled card.\nSVar:TrigExile:DB$ ChangeZone | Origin$ Graveyard | Destination$ Exile | ValidTgts$ Card | TgtPrompt$ Choose target card in a graveyard | RememberChanged$ True\nSVar:Bounce:DB$ ChangeZone | Defined$ Self | Origin$ Battlefield | Destination$ Hand | SubAbility$ DBExileAll\nSVar:DBExileAll:DB$ ChangeZoneAll | ChangeType$ Creature.OppOwn | Origin$ Graveyard | Destination$ Exile | RememberChanged$ True | SubAbility$ TrigGain\nSVar:TrigGain:DB$ GainLife | LifeAmount$ X | SubAbility$ DBCleanup\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nSVar:X:Remembered$CardManaCost\nOracle:";
const TEST_OX: &str = "Name:Test Ox\nManaCost:2 W\nTypes:Creature Ox\nPT:2/4\nOracle:";

fn push_host_ability(
    game: &mut GameState,
    host: CardId,
    svar: &str,
    trigger: bool,
    target: Option<CardId>,
) {
    let text = game.card(host).get_s_var(svar).expect("svar").to_string();
    let controller = game.card(host).controller;
    let mut sa = manabrew_engine::spellability::build_spell_ability(game, host, &text, controller);
    if trigger {
        sa.is_trigger = true;
        sa.trigger_source = Some(host);
        sa.trigger_source_zone_timestamp = Some(game.card(host).zone_timestamp);
    }
    sa.target_chosen.target_card = target;
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
}

#[test]
fn an_enters_trigger_of_a_bounced_host_does_not_remember_on_the_recast_card() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let priest = put(&mut game, LEDGER_PRIEST, p0, ZoneType::Battlefield);
    let first = put(&mut game, TEST_OX, p1, ZoneType::Graveyard);
    let second = put(&mut game, TEST_OX, p1, ZoneType::Graveyard);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_host_ability(&mut game, priest, "TrigExile", true, Some(first));
    game.move_card(priest, ZoneType::Hand, p0);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(first).zone, ZoneType::Exile);
    assert!(game.card(priest).remembered_cards.is_empty());
    game.move_card(priest, ZoneType::Battlefield, p0);
    push_host_ability(&mut game, priest, "TrigExile", true, Some(second));
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(priest).remembered_cards, vec![second]);
}

#[test]
fn a_remember_chain_reads_and_clears_the_host_that_moved_during_its_resolution() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let priest = put(&mut game, LEDGER_PRIEST, p0, ZoneType::Battlefield);
    let ox = put(&mut game, TEST_OX, p1, ZoneType::Graveyard);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_host_ability(&mut game, priest, "Bounce", false, None);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(priest).zone, ZoneType::Hand);
    assert_eq!(game.card(ox).zone, ZoneType::Exile);
    assert_eq!(game.player(p0).life, 23);
    assert!(game.card(priest).remembered_cards.is_empty());
}

#[test]
fn a_leaves_the_battlefield_trigger_reads_what_its_host_remembered() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let priest = put(&mut game, LEDGER_PRIEST, p0, ZoneType::Battlefield);
    let ox = put(&mut game, TEST_OX, p1, ZoneType::Graveyard);
    let source = effect_source(&mut game, p1);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_host_ability(&mut game, priest, "TrigExile", true, Some(ox));
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(priest).remembered_cards, vec![ox]);
    push_effect_entry(
        &mut game,
        p1,
        "DB$ Destroy | Defined$ Targeted",
        Some(priest),
        None,
        Some(source),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(priest).zone, ZoneType::Graveyard);
    assert_eq!(game.player(p0).life, 23);
}

const GRINDING_SAGA: &str = "Name:Grinding Saga\nManaCost:2 U\nTypes:Enchantment\nSVar:TrigRepeat:DB$ Repeat | RepeatSubAbility$ DBCleanAndGrind | MaxRepeat$ 3 | RepeatCheckSVar$ X | RepeatSVarCompare$ GE1\nSVar:DBCleanAndGrind:DB$ Cleanup | ClearRemembered$ True | SubAbility$ DBGrind\nSVar:DBGrind:DB$ Mill | NumCards$ 1 | RememberMilled$ True\nSVar:X:Count$RememberedSize\nOracle:";
const IMPRINTING_WARDEN: &str = "Name:Imprinting Warden\nManaCost:2 W U\nTypes:Creature Wizard\nPT:3/4\nSVar:ExileImprint:DB$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Creature | RememberLKI$ True | Imprint$ True | SubAbility$ DBReturn\nSVar:DBReturn:DB$ ChangeZone | Defined$ Imprinted | Origin$ Exile | Destination$ Battlefield\nOracle:";

#[test]
fn a_repeated_sub_ability_writes_the_remembered_list_its_repeat_check_reads() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let saga = put(&mut game, GRINDING_SAGA, p0, ZoneType::Battlefield);
    for _ in 0..5 {
        put(&mut game, TEST_OX, p0, ZoneType::Library);
    }
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_host_ability(&mut game, saga, "TrigRepeat", true, None);
    game.move_card(saga, ZoneType::Graveyard, p0);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.cards_in_zone(ZoneType::Library, p0).len(), 2);
}

#[test]
fn an_imprint_on_a_moved_host_is_what_its_sub_ability_reads() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let warden = put(&mut game, IMPRINTING_WARDEN, p0, ZoneType::Battlefield);
    let ox = put(&mut game, TEST_OX, p0, ZoneType::Battlefield);
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    push_host_ability(&mut game, warden, "ExileImprint", false, Some(ox));
    game.move_card(warden, ZoneType::Exile, p0);
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.card(ox).zone, ZoneType::Battlefield);
    assert!(game.card(warden).imprinted_cards.is_empty());
}

struct HeadsRng;

impl GameRng for HeadsRng {
    fn shuffle_cards(&mut self, _cards: &mut [CardId]) {}

    fn next_int(&mut self, bound: i32) -> i32 {
        bound - 1
    }
}

#[test]
fn a_coin_flip_sub_ability_skips_the_turns_of_the_player_its_parent_targeted() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    main_phase(&mut game, p0);
    let source = effect_source(&mut game, p0);
    game.card_mut(source).svars.insert(
        "DBSkipTurn".to_string(),
        "DB$ SkipTurn | NumTurns$ 1 | Defined$ Targeted".to_string(),
    );
    let mut agents = pass_agents();
    let mut game_loop = GameLoop::new(2);
    game_loop.game_rng = Box::new(HeadsRng);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ FlipCoin | ValidTgts$ Opponent | NoCall$ True | HeadsSubAbility$ DBSkipTurn",
        None,
        Some(p1),
        Some(source),
    );
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.player(p0).skip_turns, 0);
    assert_eq!(game.player(p1).skip_turns, 1);
}

const SAHEELIS_LATTICE: &str = "Name:Saheeli's Lattice\nManaCost:1 R\nTypes:Artifact\nK:Craft:4 R XMin1 ExileCtrlOrGrave<X/Dinosaur.Other>\nSVar:X:Count$xPaid\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Mastercraft Raptor\nManaCost:no cost\nColors:red\nTypes:Artifact Creature Dinosaur\nPT:*/4\nS:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | Description$ CARDNAME's power is equal to the total power of the exiled cards used to craft it.\nSVar:X:ExiledWith$CardPower\nOracle:";
const BIG_DINOSAUR: &str =
    "Name:Big Dinosaur\nManaCost:4 G\nTypes:Creature Dinosaur\nPT:5/5\nOracle:";
const SMALL_DINOSAUR: &str =
    "Name:Small Dinosaur\nManaCost:3 G\nTypes:Creature Dinosaur\nPT:4/4\nOracle:";

#[test]
fn a_crafted_card_counts_the_power_of_the_cards_exiled_to_craft_it() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let lattice = put(&mut game, SAHEELIS_LATTICE, p0, ZoneType::Battlefield);
    let big = put(&mut game, BIG_DINOSAUR, p0, ZoneType::Battlefield);
    let small = put(&mut game, SMALL_DINOSAUR, p0, ZoneType::Graveyard);
    for card in [lattice, big, small] {
        game.move_card(card, ZoneType::Exile, p0);
    }
    game.card_mut(lattice).paid_cost_exiled_cards = vec![big, small];
    push_effect_entry(
        &mut game,
        p0,
        "AB$ ChangeZone | Origin$ Exile | Destination$ Battlefield | Transformed$ True | Defined$ CorrectedSelf | Keyword$ Craft",
        None,
        None,
        Some(lattice),
    );
    let mut agents = pass_agents();
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);
    let card = game.card(lattice);
    assert_eq!(card.zone, ZoneType::Battlefield);
    assert_eq!(card.card_name, "Mastercraft Raptor");
    assert_eq!(card.exiled_cards, vec![big, small]);
    assert_eq!(card.power(), 9);
}

const SHELTERED_BY_GHOSTS: &str = "Name:Sheltered by Ghosts\nManaCost:1 W\nTypes:Enchantment Aura\nK:Enchant:Creature.YouCtrl:creature you control\nOracle:";

#[test]
fn a_dug_aura_with_nothing_to_enchant_stays_in_the_library_above_the_rest() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    main_phase(&mut game, p0);
    let forest = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
    for _ in 0..3 {
        put(&mut game, forest, p0, ZoneType::Library);
    }
    let aura = put(&mut game, SHELTERED_BY_GHOSTS, p0, ZoneType::Library);
    for _ in 0..2 {
        put(&mut game, forest, p0, ZoneType::Library);
    }
    let source = effect_source(&mut game, p0);
    push_effect_entry(
        &mut game,
        p0,
        "DB$ Dig | DigNum$ 7 | ChangeNum$ 2 | Optional$ True | ChangeValid$ Permanent.nonCreature+nonLand+cmcLE3 | DestinationZone$ Battlefield | RestRandomOrder$ True",
        None,
        None,
        Some(source),
    );
    GameLoop::new(2).resolve_stack(&mut game, &mut pass_agents());
    let library = game.cards_in_zone(ZoneType::Library, p0);
    assert_eq!(library.len(), 6);
    assert_eq!(library.last(), Some(&aura));
    assert_eq!(game.card(aura).zone, ZoneType::Library);
}
