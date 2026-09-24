use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};

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

/// Test basic control change functionality
#[test]
fn test_basic_control_change() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0); // Alice
    let p1 = PlayerId(1); // Bob

    // Bob has a Grizzly Bears on the battlefield
    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);
    game.card_mut(bears).summoning_sick = false;

    // Verify initial state
    let bears_card = game.card(bears);
    assert_eq!(
        bears_card.controller, p1,
        "Bears initially controlled by Bob"
    );
    assert_eq!(bears_card.owner, p1, "Bears owned by Bob");

    // Change control to Alice
    game.change_controller(bears, p0);

    // Verify new state
    let bears_card = game.card(bears);
    assert_eq!(bears_card.controller, p0, "Bears now controlled by Alice");
    assert_eq!(bears_card.owner, p1, "Bears still owned by Bob");

    // Verify zone tracking
    let alice_battlefield = game.zone(ZoneType::Battlefield, p0);
    let bob_battlefield = game.zone(ZoneType::Battlefield, p1);

    assert_eq!(alice_battlefield.len(), 1, "Alice should have Bears");
    assert_eq!(bob_battlefield.len(), 0, "Bob should have no creatures");
}
