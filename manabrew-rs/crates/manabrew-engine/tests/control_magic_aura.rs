use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};

fn make_control_magic(owner: PlayerId) -> CardInstance {
    // Control Magic: 2UU - Enchant creature, you control enchanted creature
    let mut card = CardInstance::new(
        CardId(0),
        "Control Magic".to_string(),
        owner,
        CardTypeLine::parse("Enchantment - Aura"),
        ManaCost::parse("2 U U"),
        ColorSet::BLUE,
        None,
        None,
        vec![],
        vec![],
    );

    let sa = manabrew_engine::staticability::parse_static_ability(
        "S$ Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You",
    )
    .expect("static ability should parse");

    card.static_abilities.push(sa);
    card
}

fn make_grizzly_bears(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(1),
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

fn make_island(owner: PlayerId) -> CardInstance {
    CardInstance::new(
        CardId(2),
        "Island".to_string(),
        owner,
        CardTypeLine::parse("Basic Land - Island"),
        ManaCost::no_cost(),
        ColorSet::COLORLESS,
        None,
        None,
        vec![],
        vec![],
    )
}

/// Test that Control Magic grants control of enchanted creature
#[test]
fn test_control_magic_grants_control() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0); // Alice (casts Control Magic)
    let p1 = PlayerId(1); // Bob (owns Grizzly Bears)

    // Setup battlefield: Alice has 4 Islands, Bob has Grizzly Bears
    for _ in 0..4 {
        let island = game.create_card(make_island(p0));
        game.move_card(island, ZoneType::Battlefield, p0);
    }

    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);

    // Initial state: Bob controls the Bears
    let bears_initial = game.card(bears);
    assert_eq!(
        bears_initial.controller, p1,
        "Bears should initially be controlled by Bob"
    );

    // Put Control Magic on battlefield attached to Bears
    let control_magic = game.create_card(make_control_magic(p0));
    game.move_card(control_magic, ZoneType::Battlefield, p0);

    // Attach Control Magic to Grizzly Bears
    game.attach_to(control_magic, bears);

    // Verify attachment
    let aura = game.card(control_magic);
    assert_eq!(
        aura.attached_to,
        Some(bears),
        "Control Magic should be attached to Bears"
    );
    assert_eq!(
        aura.controller, p0,
        "Control Magic should be controlled by Alice"
    );

    let bears_card = game.card(bears);
    assert!(
        bears_card.attachments.contains(&control_magic),
        "Bears should have Control Magic attached"
    );

    println!("Before applying continuous effects:");
    println!("  Bears controller: {:?}", game.card(bears).controller);
    println!(
        "  Aura controller: {:?}",
        game.card(control_magic).controller
    );
    println!(
        "  Aura attached to: {:?}",
        game.card(control_magic).attached_to
    );

    // Apply static abilities (this should grant control, but doesn't due to missing layer 2)
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);

    println!("After applying continuous effects:");
    println!("  Bears controller: {:?}", game.card(bears).controller);

    // BUG: Control doesn't change because layer 2 (control-changing) is not implemented
    let bears_after = game.card(bears);
    if bears_after.controller != p0 {
        println!("BUG CONFIRMED: Control Magic didn't grant control!");
        println!(
            "Expected controller: {:?}, Actual controller: {:?}",
            p0, bears_after.controller
        );
        println!("\nReason: Layer 2 (control-changing) effects are not implemented in the static ability system.");
    }

    // This assertion will FAIL until layer 2 is implemented
    assert_eq!(
        bears_after.controller, p0,
        "BUG: Control Magic should grant control but doesn't - Layer 2 not implemented"
    );
}

#[test]
fn test_control_magic_attachment_tracking() {
    // Smaller test just to verify attachment mechanics work
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);

    let bears = game.create_card(make_grizzly_bears(p1));
    game.move_card(bears, ZoneType::Battlefield, p1);

    let control_magic = game.create_card(make_control_magic(p0));
    game.move_card(control_magic, ZoneType::Battlefield, p0);

    // Attach
    game.attach_to(control_magic, bears);

    // Verify the attachment
    let aura_after = game.card(control_magic);
    assert_eq!(aura_after.attached_to, Some(bears));

    let bears_after = game.card(bears);
    assert!(bears_after.attachments.contains(&control_magic));

    println!("✓ Attachment mechanics work correctly");
}
