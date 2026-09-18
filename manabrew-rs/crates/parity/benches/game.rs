//! The Rust side of one parity game, as the gates run it: deterministic agents, parity
//! snapshots and the callback log, no Java. `cargo bench -p parity --bench game`.

use criterion::{criterion_group, criterion_main, Criterion};
use manabrew_engine::mana::ActionSpaceManaProbe;
use parity::deterministic_agent::VerboseMode;
use parity::runner::{load_data, run_with_data, RunConfig};

fn workspace_path(relative: &str) -> String {
    format!("{}/../../../{relative}", env!("CARGO_MANIFEST_DIR"))
}

fn config(deck1: &str, deck2: &str, seed: u64) -> RunConfig {
    RunConfig {
        deck1: deck1.to_string(),
        deck2: deck2.to_string(),
        seed,
        max_turns: 30,
        cards_dir: None,
        decks_dir: Some(workspace_path("parity_decks")),
        verbose: VerboseMode::Off,
        prefer_actions: false,
        deep: false,
        loose_parity: false,
        log_snapshots: true,
        java_heap: String::new(),
        variant: "Constructed".to_string(),
        commanders: Vec::new(),
        full_log: false,
        live_log: None,
        callback_compare: false,
        localize: false,
        mana_probe: ActionSpaceManaProbe::ComputerUtilMana,
    }
}

fn games(c: &mut Criterion) {
    if std::env::var_os("CARDSET_ARCHIVE").is_none() {
        std::env::set_var(
            "CARDSET_ARCHIVE",
            workspace_path("src-tauri/resources/cardset.rkyv"),
        );
    }
    let data = load_data(None, false).expect("card data");
    let mut group = c.benchmark_group("game_30_turns");
    group.sample_size(10);
    for (name, deck1, deck2, seed) in [
        (
            "landfall_vs_izzet_s17",
            "meta_mono_green_landfall",
            "meta_izzet_spellementals",
            17,
        ),
        (
            "landfall_mirror_s17",
            "meta_mono_green_landfall",
            "meta_mono_green_landfall",
            17,
        ),
    ] {
        let config = config(deck1, deck2, seed);
        group.bench_function(name, |b| {
            b.iter(|| run_with_data(&config, &data).expect("game runs"))
        });
    }
    group.finish();
}

criterion_group!(benches, games);
criterion_main!(benches);
