use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
use manabrew_engine::agent::{PassAgent, PlayerAgent};
use manabrew_engine::card::Card;
use manabrew_engine::game::{GameState, TypeRegistry};
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;

fn load_types() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
}

fn card(game: &mut GameState, name: &str, type_line: &str, zone: ZoneType) -> CardId {
    let p0 = PlayerId(0);
    let card = game.create_card(Card::new(
        CardId(0),
        name.to_string(),
        p0,
        CardTypeLine::parse(type_line),
        ManaCost::parse("1 G"),
        ColorSet::GREEN,
        Some(1),
        Some(1),
        vec![],
        vec![],
    ));
    game.move_card(card, zone, p0);
    card
}

fn reunion_board(elves: usize) -> (GameState, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let reunion = card(&mut game, "Celestial Reunion", "Sorcery", ZoneType::Hand);
    for _ in 0..elves {
        card(
            &mut game,
            "Llanowar Elves",
            "Creature - Elf Druid",
            ZoneType::Battlefield,
        );
    }
    card(
        &mut game,
        "Goblin Guide",
        "Creature - Goblin Scout",
        ZoneType::Battlefield,
    );
    game.card_mut(reunion)
        .set_chosen_type(Some("Elf".to_string()), None, true);
    (game, reunion)
}

fn behold_elves(game: &GameState, reunion: CardId) -> Option<Vec<CardId>> {
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let pools = vec![ManaPool::default(), ManaPool::default()];
    GameLoop::choose_behold_cards(
        game,
        &mut agents,
        &pools,
        PlayerId(0),
        reunion,
        "Creature.ChosenType",
        2,
    )
}

#[test]
fn a_chosen_type_behold_is_paid_with_creatures_of_that_type() {
    load_types();
    let (game, reunion) = reunion_board(2);

    let picks = behold_elves(&game, reunion).expect("two Elves");
    assert_eq!(picks.len(), 2);
    assert!(picks
        .iter()
        .all(|&cid| game.card(cid).type_line.has_subtype("Elf")));
}

#[test]
fn a_chosen_type_behold_short_of_creatures_fails() {
    load_types();
    let (game, reunion) = reunion_board(1);

    assert!(behold_elves(&game, reunion).is_none());
}
