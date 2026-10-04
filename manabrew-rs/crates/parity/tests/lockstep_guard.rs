use parity::lockstep::guard_end;
use parity::protocol::StateSnapshot;
use serde_json::{json, Value};

fn snapshot(game_over: bool, winner: Option<u32>, life: i32, has_won: bool) -> StateSnapshot {
    let player = |index: u32, won: bool| {
        json!({
            "name": format!("Player{}", index + 1),
            "index": index,
            "life": life,
            "poison": 0,
            "lands_played": 1,
            "has_lost": false,
            "has_won": won,
            "battlefield": [],
            "graveyard": ["Llanowar Elves"],
            "hand": ["Forest"],
            "exile": [],
            "library_size": 30,
        })
    };
    serde_json::from_value(json!({
        "turn": 22,
        "phase": "Main2",
        "active_player": 1,
        "priority_player": 1,
        "game_over": game_over,
        "winner": winner,
        "players": [player(0, has_won), player(1, false)],
        "stack": [],
        "agent_rng_calls": 127,
        "game_rng_calls": 40,
    }))
    .expect("snapshot")
}

fn java_end(consumed: u64) -> Value {
    json!({"t": "end", "winner": 1, "turn": 22, "error": null, "desync": null, "consumed": consumed})
}

#[test]
fn java_stopped_at_its_own_guard_on_rusts_draws_matches_despite_its_result() {
    let rust = snapshot(false, None, 10, false);
    let java = snapshot(true, Some(0), 10, true);
    assert_eq!(
        guard_end(&rust, Some(&java), Some(&java_end(127)), 127),
        Ok(())
    );
}

#[test]
fn a_perturbed_java_state_at_the_guard_is_a_failure() {
    let rust = snapshot(false, None, 10, false);
    let java = snapshot(false, None, 9, false);
    let failure = guard_end(&rust, Some(&java), Some(&java_end(127)), 127).expect_err("life");
    assert!(failure.contains("life"), "{failure}");
}

#[test]
fn a_java_end_off_rusts_draws_or_without_a_guard_snapshot_is_a_failure() {
    let rust = snapshot(false, None, 10, false);
    let java = snapshot(false, None, 10, false);
    assert!(guard_end(&rust, Some(&java), Some(&java_end(126)), 127).is_err());
    assert!(guard_end(&rust, None, Some(&java_end(127)), 127).is_err());
    assert!(guard_end(&rust, Some(&java), None, 127).is_err());
    let desynced = json!({"t": "end", "consumed": 127, "desync": "state: differs", "error": null});
    assert!(guard_end(&rust, Some(&java), Some(&desynced), 127).is_err());
}

fn boros_temur_guard() -> (StateSnapshot, StateSnapshot, Value, u64) {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/guard_boros_mobilize_temur_ferocious_1.json"
    ))
    .expect("fixture");
    (
        serde_json::from_value(fixture["rust_guard"].clone()).expect("rust guard"),
        serde_json::from_value(fixture["java_guard"].clone()).expect("java guard"),
        fixture["java_end"].clone(),
        fixture["sent"].as_u64().expect("sent"),
    )
}

#[test]
fn the_boros_temur_llanowar_guard_game_matches_at_both_guards() {
    let (rust, java, end, sent) = boros_temur_guard();
    assert_eq!(guard_end(&rust, Some(&java), Some(&end), sent), Ok(()));
}

#[test]
fn the_boros_temur_llanowar_guard_game_fails_with_a_perturbed_java_board() {
    let (rust, mut java, end, sent) = boros_temur_guard();
    let elves = java.players[1]
        .battlefield
        .iter()
        .position(|card| card.name == "Llanowar Elves")
        .expect("an elf");
    java.players[1].battlefield.remove(elves);
    assert!(guard_end(&rust, Some(&java), Some(&end), sent).is_err());
    let (rust, mut java, end, sent) = boros_temur_guard();
    java.players[0].life -= 1;
    assert!(guard_end(&rust, Some(&java), Some(&end), sent).is_err());
}
