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

    // RNG call counts are compared last: they diverge whenever the engines make
    // different decisions, so reporting them first masks the game-state
    // difference that caused the divergence.
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
            subject: None,
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

    // Both lists are sorted by name. Pair cards by name so one extra permanent
    // is reported once instead of shifting every later card onto a stranger.
    let mut only_rust: Vec<&str> = Vec::new();
    let mut only_java: Vec<&str> = Vec::new();
    let mut pairs: Vec<(usize, &CardSnapshot, &CardSnapshot)> = Vec::new();
    let (mut ri, mut ji) = (0usize, 0usize);
    while ri < rust.len() || ji < java.len() {
        match (rust.get(ri), java.get(ji)) {
            (Some(rc), Some(jc)) => match rc.name.cmp(&jc.name) {
                std::cmp::Ordering::Equal => {
                    pairs.push((ri, rc, jc));
                    ri += 1;
                    ji += 1;
                }
                std::cmp::Ordering::Less => {
                    only_rust.push(&rc.name);
                    ri += 1;
                }
                std::cmp::Ordering::Greater => {
                    only_java.push(&jc.name);
                    ji += 1;
                }
            },
            (Some(rc), None) => {
                only_rust.push(&rc.name);
                ri += 1;
            }
            (None, Some(jc)) => {
                only_java.push(&jc.name);
                ji += 1;
            }
            (None, None) => break,
        }
    }
    if !only_rust.is_empty() || !only_java.is_empty() {
        divs.push(divergence(
            index,
            turn,
            phase,
            &format!("{prefix}.cards"),
            &format!("only here: {only_rust:?}"),
            &format!("only here: {only_java:?}"),
        ));
    }

    for (i, rc, jc) in pairs {
        let card_prefix = format!("{prefix}[{i}]");
        let mut card_divs: Vec<Divergence> = Vec::new();
        macro_rules! cmp_card {
            ($field:ident) => {
                if rc.$field != jc.$field {
                    card_divs.push(divergence(
                        index,
                        turn,
                        phase,
                        &format!("{}.{}", card_prefix, stringify!($field)),
                        &format!("{:?}", rc.$field),
                        &format!("{:?}", jc.$field),
                    ));
                }
            };
        }
        cmp_card!(tapped);
        cmp_card!(power);
        cmp_card!(toughness);
        cmp_card!(damage);
        // Only compare summoning_sick for creatures (power is Some).
        // Non-creature permanents (lands, artifacts, enchantments) have
        // summoning sickness tracked differently between the Java and
        // Rust engines — Java may retain sickness=true for a land that
        // entered the battlefield on a previous turn via MayPlay/
        // graveyard play, while Rust clears it at the next new_turn().
        // Since summoning sickness has no gameplay effect for non-
        // creatures (CR 302.6), we skip the comparison to avoid false
        // divergences.
        if rc.power.is_some() || jc.power.is_some() {
            cmp_card!(summoning_sick);
        }
        cmp_card!(counters);
        cmp_card!(controller);
        cmp_card!(types);
        cmp_card!(keywords);
        cmp_card!(attached_to);
        cmp_card!(token);
        cmp_card!(face_down);
        for mut div in card_divs {
            div.subject = Some(rc.name.clone());
            divs.push(div);
        }
    }
}

/// Field names the comparator can report, with indices normalised to `[i]`.
/// A gate baseline records this list, so a divergence on a field added later
/// is labelled "newly compared" instead of "regressed". Keep in sync with
/// `compare`, `compare_players` and `compare_battlefield`.
pub const COMPARED_FIELDS: &[&str] = &[
    "turn",
    "phase",
    "active_player",
    "priority_player",
    "game_over",
    "winner",
    "stack",
    "monarch",
    "initiative",
    "day_night",
    "players[i].exists",
    "players[i].name",
    "players[i].life",
    "players[i].poison",
    "players[i].lands_played",
    "players[i].has_lost",
    "players[i].has_won",
    "players[i].library_size",
    "players[i].speed",
    "players[i].counters",
    "players[i].mana_pool",
    "players[i].library_top",
    "players[i].graveyard",
    "players[i].hand",
    "players[i].exile",
    "players[i].battlefield.count",
    "players[i].battlefield.cards",
    "players[i].battlefield[i].tapped",
    "players[i].battlefield[i].power",
    "players[i].battlefield[i].toughness",
    "players[i].battlefield[i].damage",
    "players[i].battlefield[i].summoning_sick",
    "players[i].battlefield[i].counters",
    "players[i].battlefield[i].controller",
    "players[i].battlefield[i].types",
    "players[i].battlefield[i].keywords",
    "players[i].battlefield[i].attached_to",
    "players[i].battlefield[i].token",
    "players[i].battlefield[i].face_down",
    "game_rng_calls",
    "agent_rng_calls",
    "snapshot.exists",
];

/// `players[1].battlefield[12].keywords` -> `players[i].battlefield[i].keywords`.
pub fn normalize_field(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '[' {
            let mut digits = String::new();
            while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                digits.push(d);
                chars.next();
            }
            if !digits.is_empty() && chars.peek() == Some(&']') {
                out.push('i');
            } else {
                out.push_str(&digits);
            }
        }
    }
    out
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
        subject: None,
    }
}
