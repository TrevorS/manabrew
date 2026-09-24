use forge_carddb::parse_card_script;
use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};

fn make_control_magic(owner: PlayerId) -> CardInstance {
    let rules = parse_card_script(
        "Name:Control Magic\nManaCost:2 U U\nTypes:Enchantment Aura\nK:Enchant:Creature\nSVar:AttachAILogic:GainControl\nS:Mode$ Continuous | AffectedDefined$ Enchanted | GainControl$ You | Description$ You control enchanted creature.\nOracle:Enchant creature\\nYou control enchanted creature.",
    )
    .expect("card script should parse");
    CardInstance::from_rules(&rules, owner)
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

    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);

    assert_eq!(
        game.card(bears).controller,
        p0,
        "Control Magic should grant control of the enchanted creature"
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
