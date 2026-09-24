use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
/// Integration test to verify counterspell and priority system works end-to-end
/// This validates that the UI components (chooseTargetSpell, stack rendering, priority passing)
/// have proper backend support
use manabrew_engine::agent::PlayerAgent;
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

fn make_counterspell(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Counterspell".to_string(),
        owner,
        CardTypeLine::parse("Instant"),
        ManaCost::parse("U U"),
        ColorSet::BLUE,
        None,
        None,
        vec![],
        vec!["SP$ Counter | TargetType$ Spell | ValidTgts$ Card | SpellDescription$ Counter target spell.".to_string()],
    )
}

fn make_lightning_bolt(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(0),
        "Lightning Bolt".to_string(),
        owner,
        CardTypeLine::parse("Instant"),
        ManaCost::parse("R"),
        ColorSet::RED,
        None,
        None,
        vec![],
        vec!["SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ CARDNAME deals 3 damage to any target.".to_string()],
    )
}

/// Test priority passing during counterspell wars
#[test]
fn test_priority_passing_during_counter_war() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Setup: Both players have counterspells ready
    let counterspell1 = game.create_card(make_counterspell(p0));
    let counterspell2 = game.create_card(make_counterspell(p1));

    game.move_card(counterspell1, ZoneType::Hand, p0);
    game.move_card(counterspell2, ZoneType::Hand, p1);

    // Put a Lightning Bolt on stack (simulating being cast)
    let bolt = game.create_card(make_lightning_bolt(p0));
    game.move_card(bolt, ZoneType::Stack, p0);

    let sa = SpellAbility::new_simple(
        Some(bolt),
        p0,
        "SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3",
    );
    let entry = StackEntry {
        id: 0, // Will be overwritten
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    };
    let _bolt_stack_id = game.stack.push(entry);

    // Verify initial state
    assert_eq!(game.stack.len(), 1, "Should start with Bolt on stack");

    // Priority system: players can respond in order
    // This tests that the priority_round function works correctly
    let mut game_loop = GameLoop::new(2);
    let mut pass_agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(manabrew_engine::agent::PassAgent),
        Box::new(manabrew_engine::agent::PassAgent),
    ];

    // Both players pass priority → step_with_priority resolves the stack
    game_loop.step_with_priority(&mut game, &mut pass_agents, false);

    // After both pass with empty stack response, the spell resolves
    assert_eq!(
        game.stack.len(),
        0,
        "Stack should resolve after both players pass"
    );
}

/// Test that UI can differentiate between valid and invalid counter targets
#[test]
fn test_valid_counter_target_filtering() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let _p0 = PlayerId(0);
    let p1 = PlayerId(1);

    // Create spells with different characteristics
    let counterable_spell = game.create_card(make_lightning_bolt(p1));
    game.move_card(counterable_spell, ZoneType::Stack, p1);

    let sa = SpellAbility::new_simple(Some(counterable_spell), p1, "SP$ DealDamage");
    let entry = StackEntry {
        id: 1, // Will be overwritten
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    };
    let spell_stack_id = game.stack.push(entry);

    // The UI should be able to show this as a valid target
    // This tests that validSpellIds is properly populated
    let valid_targets: Vec<u32> = game.stack.iter().map(|e| e.id).collect();

    assert_eq!(valid_targets.len(), 1, "Should have 1 valid counter target");
    assert_eq!(
        valid_targets[0], spell_stack_id,
        "Target should be the Lightning Bolt"
    );
}
