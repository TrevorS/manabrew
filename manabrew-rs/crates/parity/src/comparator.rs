use crate::protocol::{CardSnapshot, Divergence, PlayerSnapshot, StateSnapshot};

pub fn compare(index: usize, rust: &StateSnapshot, java: &StateSnapshot) -> Vec<Divergence> {
    let mut divs = Vec::new();
    let turn = rust.turn;
    let phase = rust.phase.clone();

    if rust.turn != java.turn {
        divs.push(divergence(
            index, turn, &phase, "turn", &rust.turn, &java.turn,
        ));
    }
    if rust.phase != java.phase {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "phase",
            &rust.phase,
            &java.phase,
        ));
    }
    if rust.active_player != java.active_player {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "active_player",
            &rust.active_player,
            &java.active_player,
        ));
    }
    if rust.priority_player != java.priority_player {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "priority_player",
            &rust.priority_player,
            &java.priority_player,
        ));
    }
    if rust.game_over != java.game_over {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "game_over",
            &rust.game_over,
            &java.game_over,
        ));
    }
    if rust.winner != java.winner {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "winner",
            &format!("{:?}", rust.winner),
            &format!("{:?}", java.winner),
        ));
    }
    if rust.stack != java.stack {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "stack",
            &format!("{:?}", rust.stack),
            &format!("{:?}", java.stack),
        ));
    }

    if rust.monarch != java.monarch {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "monarch",
            &format!("{:?}", rust.monarch),
            &format!("{:?}", java.monarch),
        ));
    }
    if rust.initiative != java.initiative {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "initiative",
            &format!("{:?}", rust.initiative),
            &format!("{:?}", java.initiative),
        ));
    }
    if rust.day_night != java.day_night {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "day_night",
            &rust.day_night,
            &java.day_night,
        ));
    }
    if rust.game_rng_calls != java.game_rng_calls {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "game_rng_calls",
            &rust.game_rng_calls,
            &java.game_rng_calls,
        ));
    }
    if rust.agent_rng_calls != java.agent_rng_calls {
        divs.push(divergence(
            index,
            turn,
            &phase,
            "agent_rng_calls",
            &rust.agent_rng_calls,
            &java.agent_rng_calls,
        ));
    }

    // Per-player comparison
    let max_players = rust.players.len().max(java.players.len());
    for i in 0..max_players {
        let prefix = format!("players[{i}]");
        match (rust.players.get(i), java.players.get(i)) {
            (Some(rp), Some(jp)) => {
                compare_players(&mut divs, index, turn, &phase, &prefix, rp, jp);
            }
            (Some(_), None) => {
                divs.push(divergence(
                    index,
                    turn,
                    &phase,
                    &format!("{prefix}.exists"),
                    &"present",
                    &"missing",
                ));
            }
            (None, Some(_)) => {
                divs.push(divergence(
                    index,
                    turn,
                    &phase,
                    &format!("{prefix}.exists"),
                    &"missing",
                    &"present",
                ));
            }
            (None, None) => {}
        }
    }

    divs
}

fn compare_players(
    divs: &mut Vec<Divergence>,
    index: usize,
    turn: u32,
    phase: &str,
    prefix: &str,
    rust: &PlayerSnapshot,
    java: &PlayerSnapshot,
) {
    macro_rules! cmp_field {
        ($field:ident) => {
            if rust.$field != java.$field {
                divs.push(divergence(
                    index,
                    turn,
                    phase,
                    &format!("{}.{}", prefix, stringify!($field)),
                    &rust.$field,
                    &java.$field,
                ));
            }
        };
    }

    cmp_field!(name);
    cmp_field!(life);
    cmp_field!(poison);
    cmp_field!(lands_played);
    cmp_field!(has_lost);
    cmp_field!(has_won);
    cmp_field!(library_size);
    cmp_field!(speed);
    if rust.counters != java.counters {
        divs.push(divergence(
            index,
            turn,
            phase,
            &format!("{prefix}.counters"),
            &format!("{:?}", rust.counters),
            &format!("{:?}", java.counters),
        ));
    }
    if rust.mana_pool != java.mana_pool {
        divs.push(divergence(
            index,
            turn,
            phase,
            &format!("{prefix}.mana_pool"),
            &format!("{:?}", rust.mana_pool),
            &format!("{:?}", java.mana_pool),
        ));
    }

    // Diagnostic: compare library top order (ordered, not sorted) so silent
    // library-order divergences surface early. `library_top` is the first few
    // cards in draw order on each side.
    if rust.library_top != java.library_top {
        divs.push(crate::protocol::Divergence {
            snapshot_index: index,
            turn,
            phase: phase.to_string(),
            field: format!("{prefix}.library_top"),
            rust_value: format!("{:?}", rust.library_top),
            java_value: format!("{:?}", java.library_top),
        });
    }

    // Zone comparisons (sorted card name lists)
    compare_name_list(
        divs,
        index,
        turn,
        phase,
        &format!("{prefix}.graveyard"),
        &rust.graveyard,
        &java.graveyard,
    );
    compare_name_list(
        divs,
        index,
        turn,
        phase,
        &format!("{prefix}.hand"),
        &rust.hand,
        &java.hand,
    );
    compare_name_list(
        divs,
        index,
        turn,
        phase,
        &format!("{prefix}.exile"),
        &rust.exile,
        &java.exile,
    );

    // Battlefield comparison
    compare_battlefield(
        divs,
        index,
        turn,
        phase,
        &format!("{prefix}.battlefield"),
        &rust.battlefield,
        &java.battlefield,
    );
}

fn compare_name_list(
    divs: &mut Vec<Divergence>,
    index: usize,
    turn: u32,
    phase: &str,
    field: &str,
    rust: &[String],
    java: &[String],
) {
    if rust != java {
        divs.push(divergence(
            index,
            turn,
            phase,
            field,
            &format!("{rust:?}"),
            &format!("{java:?}"),
        ));
    }
}

fn compare_battlefield(
    divs: &mut Vec<Divergence>,
    index: usize,
    turn: u32,
    phase: &str,
    prefix: &str,
    rust: &[CardSnapshot],
    java: &[CardSnapshot],
) {
    // Both are sorted by name. Walk in lockstep.
    let max = rust.len().max(java.len());
    if rust.len() != java.len() {
        divs.push(divergence(
            index,
            turn,
            phase,
            &format!("{prefix}.count"),
            &rust.len(),
            &java.len(),
        ));
    }

    for i in 0..max {
        let card_prefix = format!("{prefix}[{i}]");
        match (rust.get(i), java.get(i)) {
            (Some(rc), Some(jc)) => {
                if rc.name != jc.name {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.name"),
                        &rc.name,
                        &jc.name,
                    ));
                }
                if rc.tapped != jc.tapped {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.tapped"),
                        &rc.tapped,
                        &jc.tapped,
                    ));
                }
                if rc.power != jc.power {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.power"),
                        &format!("{:?}", rc.power),
                        &format!("{:?}", jc.power),
                    ));
                }
                if rc.toughness != jc.toughness {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.toughness"),
                        &format!("{:?}", rc.toughness),
                        &format!("{:?}", jc.toughness),
                    ));
                }
                if rc.damage != jc.damage {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.damage"),
                        &rc.damage,
                        &jc.damage,
                    ));
                }
                // Only compare summoning_sick for creatures (power is Some).
                // Non-creature permanents (lands, artifacts, enchantments) have
                // summoning sickness tracked differently between the Java and
                // Rust engines — Java may retain sickness=true for a land that
                // entered the battlefield on a previous turn via MayPlay/
                // graveyard play, while Rust clears it at the next new_turn().
                // Since summoning sickness has no gameplay effect for non-
                // creatures (CR 302.6), we skip the comparison to avoid false
                // divergences.
                let is_creature = rc.power.is_some() || jc.power.is_some();
                if is_creature && rc.summoning_sick != jc.summoning_sick {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.summoning_sick"),
                        &rc.summoning_sick,
                        &jc.summoning_sick,
                    ));
                }
                if rc.counters != jc.counters {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.counters"),
                        &format!("{:?}", rc.counters),
                        &format!("{:?}", jc.counters),
                    ));
                }
                if rc.controller != jc.controller {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.controller"),
                        &rc.controller,
                        &jc.controller,
                    ));
                }
                if rc.types != jc.types {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.types"),
                        &format!("{:?}", rc.types),
                        &format!("{:?}", jc.types),
                    ));
                }
                if rc.keywords != jc.keywords {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.keywords"),
                        &format!("{:?}", rc.keywords),
                        &format!("{:?}", jc.keywords),
                    ));
                }
                if rc.attached_to != jc.attached_to {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.attached_to"),
                        &format!("{:?}", rc.attached_to),
                        &format!("{:?}", jc.attached_to),
                    ));
                }
                if rc.token != jc.token {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.token"),
                        &rc.token,
                        &jc.token,
                    ));
                }
                if rc.face_down != jc.face_down {
                    divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{card_prefix}.face_down"),
                        &rc.face_down,
                        &jc.face_down,
                    ));
                }
            }
            (Some(rc), None) => {
                divs.push(divergence(
                    index,
                    turn,
                    phase,
                    &format!("{card_prefix}.exists"),
                    &rc.name,
                    &"<missing>",
                ));
            }
            (None, Some(jc)) => {
                divs.push(divergence(
                    index,
                    turn,
                    phase,
                    &format!("{card_prefix}.exists"),
                    &"<missing>",
                    &jc.name,
                ));
            }
            (None, None) => {}
        }
    }
}

fn divergence<R: std::fmt::Display, J: std::fmt::Display>(
    snapshot_index: usize,
    turn: u32,
    phase: &str,
    field: &str,
    rust_value: &R,
    java_value: &J,
) -> Divergence {
    Divergence {
        snapshot_index,
        turn,
        phase: phase.to_string(),
        field: field.to_string(),
        rust_value: rust_value.to_string(),
        java_value: java_value.to_string(),
    }
}
