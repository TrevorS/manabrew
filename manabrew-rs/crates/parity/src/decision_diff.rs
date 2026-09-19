//! What happened first: the first decision the two engines disagree on, and
//! what each engine did between two snapshots.
//!
//! A snapshot divergence says which state differs, usually a turn or more after
//! the cause. The decision sequence says where the engines parted: the first
//! callback that one engine asked and the other did not, or the first action
//! space that offered different options. The last decision both agreed on is
//! kept as context, because it usually names the card.

use std::collections::BTreeMap;

use crate::protocol::{
    CallbackRecord, CardSnapshot, Divergence, ParityLogEntry, PlayerSnapshot, StateSnapshot,
};

/// Callbacks both engines log at the same points. Anything else is logged by
/// one engine only and would misalign the sequences.
///
/// `pay_cost_to_prevent_effect` is left out on purpose: Java pays inside the
/// controller call and logs the row after the nested payment prompts, with the
/// payment result, and also logs `false` when the cost cannot be paid; Rust
/// logs `can_pay` before it pays and asks nothing when it cannot. Neither side
/// draws RNG for the row, and the nested prompts are compared on their own.
pub const COMPARED_CALLBACKS: &[&str] = &[
    "$ACTION_SPACE",
    "assign_combat_damage",
    "choose_action",
    "choose_attackers",
    "choose_binary",
    "choose_blockers",
    "choose_card_name",
    "choose_cards_for_effect",
    "choose_cards_for_zone_change",
    "choose_cards_to_bottom",
    "choose_color",
    "choose_colors",
    "choose_counter_type",
    "choose_damage_assignment_order",
    "choose_delve",
    "choose_dice_to_reroll",
    "choose_dig",
    "choose_discard",
    "choose_keyword_for_pump",
    "choose_mode",
    "choose_number",
    "choose_number_for_keyword_cost",
    "choose_number_from_list",
    "choose_optional_trigger",
    "choose_reorder_library",
    "choose_roll_swap_value",
    "choose_roll_to_ignore",
    "choose_roll_to_modify",
    "choose_roll_to_swap",
    "choose_sacrifice",
    "choose_scry",
    "choose_single_card_for_zone_change",
    "choose_single_entity_for_effect",
    "choose_single_replacement_effect",
    "choose_spell_abilities_for_effect",
    "choose_surveil",
    "choose_target_spell",
    "choose_targets_for",
    "choose_type",
    "confirm_action",
    "confirm_payment",
    "confirm_replacement_effect",
    "enlist_attackers",
    "exert_attackers",
    "flip_coin_call",
    "help_pay_assist",
    "pay_combat_cost",
    "pay_mana_cost",
    "specify_mana_combo",
];

/// Callbacks whose outcome names the cards on offer in the same order on both
/// sides, so a different card sequence is a different option set or pick.
const OUTCOME_COMPARED: &[&str] = &["$ACTION_SPACE", "choose_action"];

/// The `name@id` of every option in an action-space or chosen-action outcome,
/// in order; an option without a card (`PASS`, `PassPriority`) is kept whole.
///
/// The action kind and `ability_index` are left out because the engines render
/// them differently: Java prints any ability that is not an activated ability
/// (Plot's special action) as `CastSpell`, and its `ability_index` counts the
/// abilities currently possible while Rust's counts the card's ability list.
pub(crate) fn action_cards(outcome: &str) -> Vec<&str> {
    outcome
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(" | ")
        .map(|option| {
            let Some(start) = option.find("card: ") else {
                return option.trim();
            };
            let card = &option[start + "card: ".len()..];
            let Some(at) = card.find('@') else {
                return option.trim();
            };
            let digits = card[at + 1..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            &card[..at + 1 + digits]
        })
        .collect()
}

/// Keep in sync with `DeterministicController`: `confirmAction`,
/// `confirmPayment` and `chooseBinary` write two rows under one name, the pick
/// (carrying the choice args) and then the description. Rust writes one.
const JAVA_PICK_ROW_CALLBACKS: &[&str] = &["choose_binary", "confirm_action", "confirm_payment"];

pub fn is_java_pick_row(record: &CallbackRecord) -> bool {
    JAVA_PICK_ROW_CALLBACKS.contains(&record.name.as_str()) && !record.args.is_empty()
}

/// Prompts Forge raises even when there is exactly one option: `ReplacementHandler.run`
/// asks which replacement effect to apply, and `CountersPutEffect.chooseTypeFromList`
/// asks which counter type. A one-option row offers no choice and draws no RNG,
/// and the engines reach it for different sets of rules (Rust applies
/// `K:etbCounter` and the stun untap rule inline), so it is left out of the
/// compared sequence. A pick among two or more stays in.
const FORCED_CHOICE_CALLBACKS: &[&str] =
    &["choose_counter_type", "choose_single_replacement_effect"];

pub fn is_forced_choice(record: &CallbackRecord) -> bool {
    FORCED_CHOICE_CALLBACKS.contains(&record.name.as_str())
        && matches!(record.args.as_slice(), [only] if only.choices == Some(1))
}

fn compared(log: &[ParityLogEntry], java: bool) -> Vec<&CallbackRecord> {
    log.iter()
        .filter_map(ParityLogEntry::as_callback)
        .filter(|record| COMPARED_CALLBACKS.contains(&record.name.as_str()))
        .filter(|record| !(java && is_java_pick_row(record)))
        .filter(|record| !is_forced_choice(record))
        .collect()
}

fn describe(record: &CallbackRecord) -> String {
    let outcome: String = record.outcome.chars().take(200).collect();
    format!("P{} {} -> {outcome}", record.player, record.name)
}

/// Keep in sync with `HarnessCostPlumbing.visit(CostDiscard)`: Java asks for a
/// discard paid as a cost through `chooseCardsForEffect`, Rust through
/// `choose_discard`. Both agents sort the pool the same way and make the same
/// `pick_many_unique` draw, so the two names are one decision.
pub fn canonical_name(name: &str) -> &str {
    match name {
        "choose_discard" => "choose_cards_for_effect",
        other => other,
    }
}

fn same_decision(rust: &CallbackRecord, java: &CallbackRecord) -> bool {
    if rust.player != java.player || canonical_name(&rust.name) != canonical_name(&java.name) {
        return false;
    }
    if OUTCOME_COMPARED.contains(&rust.name.as_str())
        && action_cards(&rust.outcome) != action_cards(&java.outcome)
    {
        return false;
    }
    true
}

/// The first decision the engines disagree on, in game order. `subject` holds
/// the last decision they agreed on.
pub fn first_decision_divergence(
    rust_log: &[ParityLogEntry],
    java_log: &[ParityLogEntry],
) -> Option<Divergence> {
    let rust = compared(rust_log, false);
    let java = compared(java_log, true);
    let shared = rust.len().min(java.len());
    let position = (0..shared)
        .find(|&i| !same_decision(rust[i], java[i]))
        .or_else(|| (rust.len() != java.len()).then_some(shared))?;
    let anchor = rust.get(position).or_else(|| java.get(position))?;
    Some(Divergence {
        snapshot_index: anchor.snapshot_index,
        turn: anchor.turn,
        phase: anchor.phase.clone(),
        field: format!("decision[{position}]"),
        rust_value: rust
            .get(position)
            .map(|r| describe(r))
            .unwrap_or_else(|| "no further decision".to_string()),
        java_value: java
            .get(position)
            .map(|j| describe(j))
            .unwrap_or_else(|| "no further decision".to_string()),
        subject: position
            .checked_sub(1)
            .and_then(|previous| rust.get(previous))
            .map(|r| format!("after {}", describe(r))),
    })
}

fn multiset_diff(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let mut counts: BTreeMap<&str, i32> = BTreeMap::new();
    for name in before {
        *counts.entry(name.as_str()).or_default() -= 1;
    }
    for name in after {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for (name, count) in counts {
        for _ in 0..count.max(0) {
            added.push(name.to_string());
        }
        for _ in 0..(-count).max(0) {
            removed.push(name.to_string());
        }
    }
    (added, removed)
}

fn zone_delta(out: &mut Vec<String>, player: u32, zone: &str, before: &[String], after: &[String]) {
    let (added, removed) = multiset_diff(before, after);
    if !added.is_empty() {
        out.push(format!("P{player} +{zone}: {}", added.join(", ")));
    }
    if !removed.is_empty() {
        out.push(format!("P{player} -{zone}: {}", removed.join(", ")));
    }
}

fn card_delta(out: &mut Vec<String>, player: u32, before: &CardSnapshot, after: &CardSnapshot) {
    let name = &after.name;
    if before.tapped != after.tapped {
        let verb = if after.tapped { "tapped" } else { "untapped" };
        out.push(format!("P{player} {verb} {name}"));
    }
    if before.counters != after.counters {
        out.push(format!(
            "P{player} {name} counters {:?} -> {:?}",
            before.counters, after.counters
        ));
    }
    if before.damage != after.damage {
        out.push(format!(
            "P{player} {name} damage {} -> {}",
            before.damage, after.damage
        ));
    }
    if (before.power, before.toughness) != (after.power, after.toughness) {
        out.push(format!(
            "P{player} {name} P/T {:?}/{:?} -> {:?}/{:?}",
            before.power, before.toughness, after.power, after.toughness
        ));
    }
    if before.attached_to != after.attached_to {
        out.push(format!(
            "P{player} {name} attached {:?} -> {:?}",
            before.attached_to, after.attached_to
        ));
    }
    if before.keywords != after.keywords {
        out.push(format!(
            "P{player} {name} keywords {:?} -> {:?}",
            before.keywords, after.keywords
        ));
    }
    if before.types != after.types {
        out.push(format!(
            "P{player} {name} types {:?} -> {:?}",
            before.types, after.types
        ));
    }
}

fn player_delta(out: &mut Vec<String>, before: &PlayerSnapshot, after: &PlayerSnapshot) {
    let player = after.index;
    if before.life != after.life {
        out.push(format!("P{player} life {} -> {}", before.life, after.life));
    }
    if before.poison != after.poison {
        out.push(format!(
            "P{player} poison {} -> {}",
            before.poison, after.poison
        ));
    }
    if before.library_size != after.library_size {
        out.push(format!(
            "P{player} library {} -> {}",
            before.library_size, after.library_size
        ));
    }
    if before.counters != after.counters {
        out.push(format!(
            "P{player} counters {:?} -> {:?}",
            before.counters, after.counters
        ));
    }
    if before.mana_pool != after.mana_pool {
        out.push(format!(
            "P{player} mana pool {:?} -> {:?}",
            before.mana_pool, after.mana_pool
        ));
    }
    zone_delta(out, player, "hand", &before.hand, &after.hand);
    zone_delta(
        out,
        player,
        "graveyard",
        &before.graveyard,
        &after.graveyard,
    );
    zone_delta(out, player, "exile", &before.exile, &after.exile);
    let names = |cards: &[CardSnapshot]| cards.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    zone_delta(
        out,
        player,
        "battlefield",
        &names(&before.battlefield),
        &names(&after.battlefield),
    );
    let mut remaining: Vec<&CardSnapshot> = before.battlefield.iter().collect();
    for card in &after.battlefield {
        if let Some(position) = remaining.iter().position(|b| b.name == card.name) {
            card_delta(out, player, remaining.remove(position), card);
        }
    }
}

/// What changed between two snapshots of one engine, as readable lines.
pub fn describe_delta(before: &StateSnapshot, after: &StateSnapshot) -> Vec<String> {
    let mut out = Vec::new();
    if before.stack != after.stack {
        out.push(format!("stack {:?} -> {:?}", before.stack, after.stack));
    }
    for (b, a) in before.players.iter().zip(&after.players) {
        player_delta(&mut out, b, a);
    }
    out
}

fn window_decisions(
    log: &[ParityLogEntry],
    java: bool,
    from_snapshot: usize,
    to_snapshot: usize,
) -> Vec<String> {
    compared(log, java)
        .into_iter()
        .filter(|record| {
            record.snapshot_index >= from_snapshot
                && record.snapshot_index <= to_snapshot
                && record.name != "$ACTION_SPACE"
        })
        .map(describe)
        .collect()
}

/// For a divergence at snapshot `index` of a (usually `--deep`) run: what each
/// engine did since the previous snapshot, and the decisions it took there.
pub fn describe_window(
    rust_log: &[ParityLogEntry],
    java_log: &[ParityLogEntry],
    index: usize,
) -> Vec<String> {
    let snapshots = |log: &[ParityLogEntry]| -> Vec<StateSnapshot> {
        log.iter()
            .filter_map(ParityLogEntry::as_snapshot)
            .cloned()
            .collect()
    };
    let (rust, java) = (snapshots(rust_log), snapshots(java_log));
    let mut out = Vec::new();
    let Some(previous) = index.checked_sub(1) else {
        return out;
    };
    for (label, snaps, log) in [("Rust", &rust, rust_log), ("Java", &java, java_log)] {
        let (Some(before), Some(after)) = (snaps.get(previous), snaps.get(index)) else {
            continue;
        };
        let delta = describe_delta(before, after);
        out.push(format!(
            "{label} since the last matching point [T{} {}]: {}",
            before.turn,
            before.phase,
            if delta.is_empty() {
                "no state change".to_string()
            } else {
                delta.join("; ")
            }
        ));
        let decisions = window_decisions(log, label == "Java", previous, index);
        if !decisions.is_empty() {
            out.push(format!(
                "{label} decisions there: {}",
                decisions.join(" | ")
            ));
        }
    }
    let events = window_events(java_log, previous, index);
    if !events.is_empty() {
        out.push(format!("Forge events there: {}", events.join(" | ")));
    }
    out
}

/// Forge game events logged after snapshot `from` and before snapshot `to`.
fn window_events(log: &[ParityLogEntry], from: usize, to: usize) -> Vec<String> {
    let mut seen = 0usize;
    let mut out = Vec::new();
    for entry in log {
        match entry {
            ParityLogEntry::Snapshot(_) => seen += 1,
            ParityLogEntry::Event(event) if seen > from && seen <= to => {
                out.push(format!("{}: {}", event.kind, event.text));
            }
            _ => {}
        }
    }
    out
}
