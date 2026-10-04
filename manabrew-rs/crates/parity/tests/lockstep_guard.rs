use parity::lockstep::{
    guard_end, guard_unverified, java_error_end, java_timeout, Desync, LockstepEnd,
};
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

#[test]
fn a_guard_java_never_reached_in_the_catch_up_is_unverified_not_failing() {
    let unverified = guard_unverified(
        "parity guard: 101 copies",
        None,
        std::time::Duration::from_secs(300),
    );
    assert!(
        matches!(&unverified, Some(LockstepEnd::GuardUnverified(detail)) if detail.contains("within 300s")),
        "{unverified:?}"
    );
    assert_eq!(
        guard_unverified(
            "parity guard",
            Some(&java_end(127)),
            std::time::Duration::from_secs(300)
        ),
        None
    );
}

fn java_error(error: &str, consumed: u64) -> Value {
    let mut end = java_end(consumed);
    end["error"] = Value::String(error.to_string());
    end["snapshot"] = serde_json::to_value(snapshot(true, None, 10, false)).expect("snapshot");
    end
}

#[test]
fn a_java_crash_after_agreeing_turns_is_its_own_kind() {
    let end = java_error("java.lang.IndexOutOfBoundsException", 90);
    assert_eq!(
        java_error_end(&end, "java game error".to_string(), None, |_| None),
        Ok(LockstepEnd::JavaCrash("java game error".to_string()))
    );
    let earlier = Desync {
        kind: "state".to_string(),
        detail: "turn 9 life".to_string(),
    };
    let failure = java_error_end(&end, "java game error".to_string(), Some(earlier), |_| None)
        .expect_err("earlier divergence");
    assert_eq!(failure.kind, "state");
}

#[test]
fn a_java_runaway_cap_is_compared_with_rust_at_the_same_draw() {
    let end = java_error("forge.game.Game$RunawayGameException: Runaway game", 236);
    let mut asked = None;
    let matched = java_error_end(&end, "runaway".to_string(), None, |draws| {
        asked = Some(draws);
        Some(snapshot(false, None, 10, false))
    });
    assert_eq!(asked, Some(236));
    assert_eq!(
        matched,
        Ok(LockstepEnd::JavaRunawayMatched("runaway".to_string()))
    );
    let differs = java_error_end(&end, "runaway".to_string(), None, |_| {
        Some(snapshot(false, None, 7, false))
    })
    .expect_err("life differs");
    assert_eq!(differs.kind, "runaway");
    assert!(differs.detail.contains("life"), "{}", differs.detail);
    assert!(java_error_end(&end, "runaway".to_string(), None, |_| None).is_err());
}

#[test]
fn a_java_timeout_after_agreeing_turns_is_listed_not_failing() {
    let rust = vec![
        snapshot(false, None, 20, false),
        snapshot(false, None, 18, false),
        snapshot(false, None, 15, false),
    ];
    let java = rust[..2].to_vec();
    let timeout = std::time::Duration::from_secs(300);
    let ended = java_timeout(&rust, &java, timeout);
    assert!(
        matches!(&ended, Ok(LockstepEnd::JavaTimeout(detail)) if detail.contains("2 of rust's 3")),
        "{ended:?}"
    );
    let mut differing = java.clone();
    differing[1].players[0].life -= 1;
    let failure = java_timeout(&rust, &differing, timeout).expect_err("a differing turn");
    assert_eq!(failure.kind, "state");
}
