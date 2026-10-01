use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
use manabrew_engine::agent::GameEntity;
use manabrew_engine::card::{CardInstance, CounterType};
use manabrew_engine::game::GameState;
use manabrew_engine::game_entity_counter_table::GameEntityCounterTable;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::trigger::TriggerHandler;

fn bears(game: &mut GameState) -> CardId {
    let card = game.create_card(CardInstance::new(
        CardId(0),
        "Grizzly Bears".to_string(),
        PlayerId(0),
        CardTypeLine::parse("Creature - Bear"),
        ManaCost::parse("1 G"),
        ColorSet::GREEN,
        Some(2),
        Some(2),
        vec![],
        vec![],
    ));
    game.move_card(card, ZoneType::Battlefield, PlayerId(0));
    card
}

fn add_p1p1(game: &mut GameState, handler: &mut TriggerHandler, card: CardId, amount: i32) {
    let mut table = GameEntityCounterTable::default();
    table.put(
        Some(PlayerId(0)),
        GameEntity::Card(card),
        CounterType::P1P1,
        amount,
    );
    table.replace_counter_effect(game, Some(handler), None, None, true, Default::default());
}

#[test]
fn counters_stop_at_i32_max_and_unheard_counter_events_are_not_held() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let card = bears(&mut game);
    let mut handler = TriggerHandler::new();
    add_p1p1(&mut game, &mut handler, card, 1 << 30);
    add_p1p1(&mut game, &mut handler, card, 1 << 30);
    assert_eq!(game.card(card).counters[&CounterType::P1P1], i32::MAX);
    assert!(handler.waiting_trigger_count() < 16);
}

#[test]
fn counters_wrap_past_i32_max_as_java_ints_do_when_mirroring_forge() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = true;
    let card = bears(&mut game);
    let mut handler = TriggerHandler::new();
    add_p1p1(&mut game, &mut handler, card, 1 << 30);
    add_p1p1(&mut game, &mut handler, card, 1 << 30);
    assert_eq!(game.card(card).counters[&CounterType::P1P1], i32::MIN);
}
