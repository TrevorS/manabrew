use std::hash::{Hash, Hasher};
use std::panic::Location;
use std::sync::{Arc, Mutex};

use crate::card::Card;
use crate::game::{GameState, LAYER_KEY_FIELDS};

type Site = &'static Location<'static>;

#[derive(Debug)]
pub struct LayerStatsSnapshot {
    cards: Vec<Arc<Card>>,
    hashes: [u64; LAYER_KEY_FIELDS.len()],
}

#[derive(Default)]
struct Stats {
    calls: u64,
    skipped: u64,
    no_key: u64,
    ran: u64,
    scalars_moved: u64,
    all_noop: u64,
    cards_only_real: u64,
    changed_cards: [u64; 4],
    moved: [u64; LAYER_KEY_FIELDS.len()],
    real: [u64; LAYER_KEY_FIELDS.len()],
    sites: crate::HashMap<(usize, bool, Site), u64>,
    scalars: crate::HashMap<&'static str, u64>,
    scalars_only: crate::HashMap<Vec<&'static str>, u64>,
    blockers: crate::HashMap<Vec<(usize, Site)>, u64>,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

fn out_path() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var("manabrew_engine_LAYER_SKIP_STATS").ok())
        .as_deref()
}

fn debug_hash<T: std::fmt::Debug + ?Sized>(value: &T) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    format!("{value:?}").hash(&mut hasher);
    hasher.finish()
}

fn field_hashes(game: &GameState) -> [u64; LAYER_KEY_FIELDS.len()] {
    [
        0,
        debug_hash(&*game.players),
        debug_hash(&game.zone_store_snapshot()),
        debug_hash(&*game.stack),
        debug_hash(&*game.last_state_battlefield),
        debug_hash(&*game.last_state_battlefield_combat_lki),
        debug_hash(&*game.change_zone_lki_info),
        debug_hash(&*game.counter_added_this_turn),
        debug_hash(&*game.left_battlefield_this_turn),
        debug_hash(&*game.left_graveyard_this_turn),
        debug_hash(&*game.damage_this_turn_lki),
    ]
}

fn changed_cards(before: &[Arc<Card>], after: &[Arc<Card>]) -> usize {
    after.len().abs_diff(before.len())
        + before
            .iter()
            .zip(after)
            .filter(|(b, a)| !Arc::ptr_eq(b, a) && format!("{b:?}") != format!("{a:?}"))
            .count()
}

pub fn capture(game: &GameState) -> Option<Arc<LayerStatsSnapshot>> {
    out_path()?;
    Some(Arc::new(LayerStatsSnapshot {
        cards: game.cards.to_vec(),
        hashes: field_hashes(game),
    }))
}

pub fn record(game: &GameState, skipped: bool) {
    let Some(path) = out_path() else {
        return;
    };
    let mut guard = STATS.lock().expect("layer stats");
    let stats = guard.get_or_insert_with(Stats::default);
    stats.calls += 1;
    if skipped {
        stats.skipped += 1;
    } else {
        match (&game.layer_key_after_pass.0, &game.layer_key_after_pass.1) {
            (Some(before), Some(snapshot)) => {
                let after = game.layer_key();
                let (moved, scalars) = before.moved_fields(&after);
                let writers = game.layer_key_writers();
                let hashes = field_hashes(game);
                stats.ran += 1;
                if scalars {
                    stats.scalars_moved += 1;
                    let names = before.moved_scalars(&after);
                    for name in &names {
                        *stats.scalars.entry(name).or_default() += 1;
                    }
                    if !moved.iter().any(|&m| m) {
                        *stats.scalars_only.entry(names).or_default() += 1;
                    }
                }
                let mut blockers = Vec::new();
                let mut real_fields = Vec::new();
                let mut cards = 0;
                for (index, &field_moved) in moved.iter().enumerate() {
                    if !field_moved {
                        continue;
                    }
                    stats.moved[index] += 1;
                    let real = if index == 0 {
                        cards = changed_cards(&snapshot.cards, &game.cards);
                        cards > 0
                    } else {
                        snapshot.hashes[index] != hashes[index]
                    };
                    if real {
                        stats.real[index] += 1;
                        real_fields.push(index);
                    }
                    if let Some(site) = writers[index] {
                        *stats.sites.entry((index, real, site)).or_default() += 1;
                        if !real {
                            blockers.push((index, site));
                        }
                    }
                }
                if !scalars && real_fields.is_empty() {
                    stats.all_noop += 1;
                    *stats.blockers.entry(blockers).or_default() += 1;
                }
                if !scalars && real_fields == [0] {
                    stats.cards_only_real += 1;
                    stats.changed_cards[match cards {
                        1 => 0,
                        2..=3 => 1,
                        4..=10 => 2,
                        _ => 3,
                    }] += 1;
                }
            }
            _ => {
                stats.no_key += 1;
            }
        }
    }
    if stats.calls.is_multiple_of(2000) {
        write_report(stats, path);
    }
}

fn write_report(stats: &Stats, path: &str) {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "calls {} skipped {} ran {} no_key {} scalars_moved {} skippable_if_noop_borrows_guarded {}",
        stats.calls, stats.skipped, stats.ran, stats.no_key, stats.scalars_moved, stats.all_noop
    );
    let _ = writeln!(
        out,
        "cards_only_real {} changed cards 1:{} 2-3:{} 4-10:{} 11+:{}",
        stats.cards_only_real,
        stats.changed_cards[0],
        stats.changed_cards[1],
        stats.changed_cards[2],
        stats.changed_cards[3]
    );
    for (index, name) in LAYER_KEY_FIELDS.iter().enumerate() {
        let _ = writeln!(
            out,
            "field {name}: moved {} real {} noop {}",
            stats.moved[index],
            stats.real[index],
            stats.moved[index] - stats.real[index]
        );
    }
    let mut scalars: Vec<_> = stats.scalars.iter().collect();
    scalars.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in scalars {
        let _ = writeln!(out, "scalar {name}: {count}");
    }
    let mut scalars_only: Vec<_> = stats.scalars_only.iter().collect();
    scalars_only.sort_by(|a, b| b.1.cmp(a.1));
    for (names, count) in scalars_only.iter().take(10) {
        let _ = writeln!(out, "scalars-only pass {count}: {}", names.join(" "));
    }
    let mut sites: Vec<_> = stats.sites.iter().collect();
    sites.sort_by(|a, b| b.1.cmp(a.1));
    for ((index, real, site), count) in sites.iter().take(30) {
        let _ = writeln!(
            out,
            "site {count} {} {} {}:{}",
            LAYER_KEY_FIELDS[*index],
            if *real { "real" } else { "noop" },
            site.file(),
            site.line()
        );
    }
    let mut blockers: Vec<_> = stats.blockers.iter().collect();
    blockers.sort_by(|a, b| b.1.cmp(a.1));
    for (set, count) in blockers.iter().take(15) {
        let names: Vec<String> = set
            .iter()
            .map(|(index, site)| {
                format!(
                    "{}@{}:{}",
                    LAYER_KEY_FIELDS[*index],
                    site.file(),
                    site.line()
                )
            })
            .collect();
        let _ = writeln!(out, "noop-only pass {count}: {}", names.join(" "));
    }
    let _ = std::fs::write(path, out);
}
