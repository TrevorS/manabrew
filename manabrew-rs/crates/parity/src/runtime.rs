//! Shared runtime for executing parity matchups.
//!
//! Keep Rust/Java scheduling here so CLI, CI/server mode, and debugger tooling
//! do not grow separate execution semantics.

use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use crate::java_bridge::{
    JavaBridge, JavaBridgeConfig, JavaBridgeError, JavaMatchupData, JavaServer, JavaServerConfig,
};
use crate::java_cache::{truncate_log, JavaCache};
use crate::parity_compare::compare_matchup;
use crate::protocol::{GameTrace, MatchupResult, MatchupStatus, ParityLogEntry};
use crate::runner::{self, LoadedData, RunConfig};

pub const PARITY_THREAD_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Run Rust-side and Java-side work concurrently.
///
/// Rust runs on a scoped background thread with the parity stack size; Java runs
/// on the current thread so callers can pass `&mut JavaServer` without extra
/// locking. CLI and debugger side-by-side runs should use this helper instead
/// of hand-rolling their own thread shape.
#[allow(clippy::type_complexity)]
pub fn run_parallel<RustFn, JavaFn, RustOut, JavaOut>(
    rust_thread_name: &str,
    rust_fn: RustFn,
    java_fn: JavaFn,
) -> Result<(Result<RustOut, String>, Result<JavaOut, String>), String>
where
    RustFn: FnOnce() -> Result<RustOut, String> + Send,
    JavaFn: FnOnce() -> Result<JavaOut, String>,
    RustOut: Send,
{
    std::thread::scope(|scope| {
        let rust_handle = std::thread::Builder::new()
            .name(rust_thread_name.to_string())
            .stack_size(PARITY_THREAD_STACK_SIZE)
            .spawn_scoped(scope, rust_fn)
            .map_err(|err| format!("failed to spawn {rust_thread_name} thread: {err}"))?;

        let java_result = java_fn();
        let rust_result = rust_handle
            .join()
            .map_err(|_| format!("{rust_thread_name} thread panicked"))?;
        Ok((rust_result, java_result))
    })
}

/// A pool of JavaServer instances. Servers start on first use, up to `capacity`,
/// so a run served entirely from the Java cache never starts a JVM.
pub struct JavaServerPool {
    state: Mutex<PoolState>,
    available: Condvar,
    config: JavaServerConfig,
    capacity: usize,
}

struct PoolState {
    idle: Vec<JavaServer>,
    live: usize,
}

struct PooledServer<'a> {
    pool: &'a JavaServerPool,
    server: Option<JavaServer>,
}

impl Drop for PooledServer<'_> {
    fn drop(&mut self) {
        let Some(mut server) = self.server.take() else {
            return;
        };
        let mut state = self.pool.state.lock().unwrap_or_else(|e| e.into_inner());
        if server.is_alive() {
            state.idle.push(server);
        } else {
            state.live -= 1;
        }
        drop(state);
        self.pool.available.notify_one();
    }
}

impl JavaServerPool {
    pub fn lazy(capacity: usize, config: JavaServerConfig) -> Self {
        Self {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
            }),
            available: Condvar::new(),
            config,
            capacity: capacity.max(1),
        }
    }

    /// Spawn N server instances up front.
    pub fn spawn(n: usize, config: &JavaServerConfig) -> Result<Self, JavaBridgeError> {
        let pool = Self::lazy(n, config.clone());
        let mut servers = Vec::with_capacity(n);
        for i in 0..n {
            if config.verbose {
                eprintln!("[parity] Spawning Java worker {}/{}", i + 1, n);
            }
            servers.push(JavaServer::spawn(config)?);
        }
        {
            let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
            state.live = servers.len();
            state.idle = servers;
        }
        Ok(pool)
    }

    pub fn servers_started(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).live
    }

    fn checkout(&self) -> Result<PooledServer<'_>, JavaBridgeError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            while let Some(mut server) = state.idle.pop() {
                if server.is_alive() {
                    return Ok(PooledServer {
                        pool: self,
                        server: Some(server),
                    });
                }
                state.live -= 1;
            }
            if state.live < self.capacity {
                state.live += 1;
                drop(state);
                if self.config.verbose {
                    eprintln!("[parity] Spawning Java worker on demand");
                }
                return match JavaServer::spawn(&self.config) {
                    Ok(server) => Ok(PooledServer {
                        pool: self,
                        server: Some(server),
                    }),
                    Err(err) => {
                        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                        state.live -= 1;
                        drop(state);
                        self.available.notify_one();
                        Err(err)
                    }
                };
            }
            state = self
                .available
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Run a matchup on any available server with streaming snapshot comparison.
    pub fn run_matchup_streaming<F>(
        &self,
        deck1: &str,
        deck2: &str,
        seed: u64,
        max_turns: u32,
        prefer_actions: bool,
        deep: bool,
        variant: &str,
        commanders: &[String],
        verbose_turns: Option<String>,
        on_snapshot: F,
    ) -> Result<JavaMatchupData, JavaBridgeError>
    where
        F: FnMut(usize, &ParityLogEntry) -> bool,
    {
        let mut pooled = self.checkout()?;
        let server = pooled.server.as_mut().expect("checked-out server");
        server.run_matchup_streaming(
            deck1,
            deck2,
            seed,
            max_turns,
            prefer_actions,
            deep,
            variant,
            commanders,
            verbose_turns,
            on_snapshot,
        )
    }

    /// Run a matchup and collect all snapshots/decisions.
    pub fn run_matchup(
        &self,
        deck1: &str,
        deck2: &str,
        seed: u64,
        max_turns: u32,
        prefer_actions: bool,
        deep: bool,
        variant: &str,
        commanders: &[String],
        verbose_turns: Option<String>,
    ) -> Result<JavaMatchupData, JavaBridgeError> {
        self.run_matchup_streaming(
            deck1,
            deck2,
            seed,
            max_turns,
            prefer_actions,
            deep,
            variant,
            commanders,
            verbose_turns,
            |_, _| true,
        )
    }

    /// Shutdown all servers in parallel.
    pub fn shutdown(self) {
        let state = self.state.into_inner().unwrap_or_else(|e| e.into_inner());
        let handles: Vec<_> = state
            .idle
            .into_iter()
            .map(|server| std::thread::spawn(move || server.shutdown()))
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

pub struct RuntimeMatchup {
    pub result: MatchupResult,
    pub duration_ms: u64,
    pub cache_hit: bool,
    pub stages: StageTimings,
}

#[derive(Clone, Copy, Default)]
pub struct StageTimings {
    pub rust_ms: u64,
    pub java_ms: u64,
    pub compare_ms: u64,
}

impl RuntimeMatchup {
    fn finish(
        result: MatchupResult,
        start: Instant,
        cache_hit: bool,
        stages: StageTimings,
    ) -> Self {
        Self {
            result,
            duration_ms: start.elapsed().as_millis() as u64,
            cache_hit,
            stages,
        }
    }
}

pub struct ParityRuntime<'a> {
    data: &'a LoadedData,
}

impl<'a> ParityRuntime<'a> {
    pub fn new(data: &'a LoadedData) -> Self {
        Self { data }
    }

    pub fn run_rust_trace(&self, config: &RunConfig) -> Result<GameTrace, String> {
        std::thread::scope(|s| {
            let rust_handle = std::thread::Builder::new()
                .name("parity-rust".to_string())
                .stack_size(PARITY_THREAD_STACK_SIZE)
                .spawn_scoped(s, || runner::run_with_data(config, self.data))
                .expect("Failed to spawn Rust parity thread");
            rust_handle
                .join()
                .expect("Rust engine thread panicked")
                .map_err(|e| e.to_string())
        })
    }

    pub fn run_rust_only(&self, config: &RunConfig) -> MatchupResult {
        match runner::run_with_data(config, self.data) {
            Ok(trace) => build_rust_only_result(config, trace),
            Err(e) => MatchupResult::error(config, format!("Rust engine error: {e}")),
        }
    }

    /// Run against a single long-lived Java server. Rust and Java execute in parallel.
    pub fn run_with_server(&self, config: &RunConfig, server: &mut JavaServer) -> MatchupResult {
        let (rust_result, java_result) = match run_parallel(
            "parity-rust",
            || runner::run_with_data(config, self.data),
            || {
                server
                    .run_matchup(
                        &config.deck1,
                        &config.deck2,
                        config.seed,
                        config.max_turns,
                        config.prefer_actions,
                        config.deep,
                        &config.variant,
                        &config.commanders,
                        config.verbose.to_java_arg(),
                    )
                    .map_err(|err| err.to_string())
            },
        ) {
            Ok(result) => result,
            Err(err) => return MatchupResult::error(config, err),
        };
        compare_results(config, rust_result, java_result, "Java server error")
    }

    /// Run against a Java server while streaming Rust and Java log entries.
    /// Rust and Java execute in parallel with the same scheduling as CLI.
    pub fn run_with_server_streaming<R, J>(
        &self,
        config: &RunConfig,
        server: &mut JavaServer,
        rust_thread_name: &str,
        mut on_rust_entry: R,
        mut on_java_entry: J,
    ) -> Result<(GameTrace, JavaMatchupData), String>
    where
        R: FnMut(ParityLogEntry) + Send + 'static,
        J: FnMut(&ParityLogEntry) -> bool,
    {
        let (entry_tx, entry_rx) = std::sync::mpsc::channel::<ParityLogEntry>();
        let (rust_result, java_result) = run_parallel(
            rust_thread_name,
            || {
                let forwarder = std::thread::spawn(move || {
                    while let Ok(entry) = entry_rx.recv() {
                        on_rust_entry(entry);
                    }
                });
                let result = runner::run_with_data_streaming(config, self.data, Some(entry_tx));
                forwarder
                    .join()
                    .map_err(|_| "rust trace forwarder panicked".to_string())?;
                result
            },
            || {
                server
                    .run_matchup_streaming(
                        &config.deck1,
                        &config.deck2,
                        config.seed,
                        config.max_turns,
                        config.prefer_actions,
                        config.deep,
                        &config.variant,
                        &config.commanders,
                        config.verbose.to_java_arg(),
                        |_, entry| on_java_entry(entry),
                    )
                    .map_err(|err| err.to_string())
            },
        )?;

        Ok((rust_result?, java_result?))
    }

    /// Run against a Java server pool. Rust and Java execute in parallel.
    pub fn run_with_pool(&self, config: &RunConfig, pool: &JavaServerPool) -> MatchupResult {
        let (rust_result, java_result) = match run_parallel(
            "parity-rust",
            || runner::run_with_data(config, self.data),
            || {
                pool.run_matchup(
                    &config.deck1,
                    &config.deck2,
                    config.seed,
                    config.max_turns,
                    config.prefer_actions,
                    config.deep,
                    &config.variant,
                    &config.commanders,
                    config.verbose.to_java_arg(),
                )
                .map_err(|err| err.to_string())
            },
        ) {
            Ok(result) => result,
            Err(err) => return MatchupResult::error(config, err),
        };
        compare_results(config, rust_result, java_result, "Java server error")
    }

    /// Run with a one-shot Java subprocess fallback. Rust and Java execute in parallel.
    pub fn run_oneshot(&self, config: &RunConfig, jar_path: &Path) -> MatchupResult {
        let (rust_result, java_result) = match run_parallel(
            "parity-rust",
            || runner::run_with_data(config, self.data),
            || {
                let bridge_config = JavaBridgeConfig {
                    jar_path: jar_path.to_path_buf(),
                    seed: config.seed,
                    max_turns: config.max_turns,
                    deck1: config.deck1.clone(),
                    deck2: config.deck2.clone(),
                    forge_home: None,
                    decks_dir: config.decks_dir.clone(),
                    verbose: config.verbose.is_any(),
                    prefer_actions: config.prefer_actions,
                    deep: config.deep,
                    java_heap: config.java_heap.clone(),
                    verbose_turns: config.verbose.to_java_arg(),
                };
                JavaBridge::new(bridge_config)
                    .run()
                    .map_err(|err| err.to_string())
            },
        ) {
            Ok(result) => result,
            Err(err) => return MatchupResult::error(config, err),
        };
        compare_results(config, rust_result, java_result, "Java engine error")
    }

    /// Run a matchup with Java-cache support. Cache hits still run Rust for comparison.
    pub fn run_cached(
        &self,
        config: &RunConfig,
        pool: &JavaServerPool,
        cache: Option<&JavaCache>,
    ) -> RuntimeMatchup {
        let start = Instant::now();
        let mut stages = StageTimings::default();

        if let Some(cached_java) = cache.and_then(|c| c.get(config)) {
            let rust_start = Instant::now();
            let rust_trace = self.run_rust_trace(config);
            stages.rust_ms = rust_start.elapsed().as_millis() as u64;
            let rust_trace = match rust_trace {
                Ok(t) => t,
                Err(e) => {
                    let result = MatchupResult::error(config, format!("Rust engine error: {e}"));
                    return RuntimeMatchup::finish(result, start, false, stages);
                }
            };
            let compare_start = Instant::now();
            let mut result = compare_and_attach_coverage(config, rust_trace, &cached_java);
            stages.compare_ms = compare_start.elapsed().as_millis() as u64;
            self.localize(config, pool, cache, &mut result);
            return RuntimeMatchup::finish(result, start, true, stages);
        }

        let java_turns = cache.map_or(config.max_turns, |c| c.java_turns(config));
        let (rust_timed, java_timed) = match run_parallel(
            "parity-rust",
            || {
                let rust_start = Instant::now();
                let trace = runner::run_with_data(config, self.data);
                Ok((trace, rust_start.elapsed().as_millis() as u64))
            },
            || {
                let java_start = Instant::now();
                let data = pool
                    .run_matchup(
                        &config.deck1,
                        &config.deck2,
                        config.seed,
                        java_turns,
                        config.prefer_actions,
                        config.deep,
                        &config.variant,
                        &config.commanders,
                        config.verbose.to_java_arg(),
                    )
                    .map_err(|err| err.to_string());
                Ok((data, java_start.elapsed().as_millis() as u64))
            },
        ) {
            Ok(result) => result,
            Err(err) => {
                let result = MatchupResult::error(config, err);
                return RuntimeMatchup::finish(result, start, false, stages);
            }
        };
        let (rust_result, java_result) = match (rust_timed, java_timed) {
            (Ok((rust_result, rust_ms)), Ok((java_result, java_ms))) => {
                stages.rust_ms = rust_ms;
                stages.java_ms = java_ms;
                (rust_result, java_result)
            }
            (Err(err), _) | (_, Err(err)) => {
                let result = MatchupResult::error(config, err);
                return RuntimeMatchup::finish(result, start, false, stages);
            }
        };

        let java_result = match java_result {
            Ok(data) if java_turns > config.max_turns => {
                if let (Some(c), Ok(_)) = (cache, &rust_result) {
                    let long_config = RunConfig {
                        max_turns: java_turns,
                        ..config.clone()
                    };
                    let _ = c.put(&long_config, &data);
                }
                Ok(JavaMatchupData {
                    log: truncate_log(data.log, config.max_turns),
                })
            }
            other => other,
        };
        let compare_start = Instant::now();
        let compared =
            compare_results_with_java_data(config, rust_result, java_result, "Java server error");
        stages.compare_ms = compare_start.elapsed().as_millis() as u64;
        let (mut result, java_data) = match compared {
            Ok(value) => value,
            Err(result) => return RuntimeMatchup::finish(result, start, false, stages),
        };

        if result.status != MatchupStatus::Error && java_turns == config.max_turns {
            if let Some(c) = cache {
                let _ = c.put(config, &java_data);
            }
        }

        self.localize(config, pool, cache, &mut result);
        RuntimeMatchup::finish(result, start, false, stages)
    }

    fn localize(
        &self,
        config: &RunConfig,
        pool: &JavaServerPool,
        cache: Option<&JavaCache>,
        result: &mut MatchupResult,
    ) {
        if !config.localize || config.deep || result.status != MatchupStatus::Fail {
            return;
        }
        let deep_config = RunConfig {
            deep: true,
            localize: false,
            ..config.clone()
        };
        let deep = self.run_cached(&deep_config, pool, cache).result;
        if let Some(divergence) = &deep.first_divergence {
            result.localized_detail = crate::decision_diff::describe_window(
                &deep.rust_log,
                &deep.java_log,
                divergence.snapshot_index,
            );
        }
        result.localized = deep.first_divergence;
    }
}

fn build_rust_only_result(config: &RunConfig, trace: GameTrace) -> MatchupResult {
    let snapshots = trace.snapshot_vec();
    let finished_turn = snapshots
        .last()
        .and_then(|s| if s.game_over { Some(s.turn) } else { None });
    let skip_reason = guard_abort_reason(&trace.log);
    MatchupResult {
        deck1: config.deck1.clone(),
        deck2: config.deck2.clone(),
        seed: config.seed,
        status: MatchupStatus::Pass,
        snapshots_compared: snapshots.len(),
        divergence_count: 0,
        first_divergence: None,
        divergences: vec![],
        localized: None,
        localized_detail: vec![],
        decision: None,
        error_message: None,
        skip_reason,
        rust_snapshot: None,
        java_snapshot: None,
        covered_cards: trace.covered_cards,
        card_uses: trace.card_uses,
        rust_log: trace.log,
        java_log: vec![],
        finished_turn,
    }
}

fn guard_abort_reason(log: &[ParityLogEntry]) -> Option<String> {
    log.iter().find_map(|entry| match entry {
        ParityLogEntry::Decision(decision) if decision.kind == "$PARITY_GUARD" => {
            Some(format!("ABORTED AT TURN {}", decision.turn))
        }
        _ => None,
    })
}

fn compare_results(
    config: &RunConfig,
    rust_result: Result<GameTrace, String>,
    java_result: Result<JavaMatchupData, String>,
    java_error_prefix: &str,
) -> MatchupResult {
    match compare_results_with_java_data(config, rust_result, java_result, java_error_prefix) {
        Ok((result, _)) => result,
        Err(result) => result,
    }
}

#[allow(clippy::result_large_err)]
fn compare_results_with_java_data(
    config: &RunConfig,
    rust_result: Result<GameTrace, String>,
    java_result: Result<JavaMatchupData, String>,
    java_error_prefix: &str,
) -> Result<(MatchupResult, JavaMatchupData), MatchupResult> {
    let rust_trace = match rust_result {
        Ok(trace) => trace,
        Err(e) => {
            return Err(MatchupResult::error(
                config,
                format!("Rust engine error: {e}"),
            ));
        }
    };
    let java_data = match java_result {
        Ok(data) => data,
        Err(e) => {
            return Err(MatchupResult::error(
                config,
                format!("{java_error_prefix}: {e}"),
            ));
        }
    };
    let result = compare_and_attach_coverage(config, rust_trace, &java_data);
    Ok((result, java_data))
}

fn compare_and_attach_coverage(
    config: &RunConfig,
    rust_trace: GameTrace,
    java_data: &JavaMatchupData,
) -> MatchupResult {
    if config.log_snapshots {
        dump_snapshot_timeline(&rust_trace.snapshot_vec(), &java_data.snapshot_vec());
    }
    let mut result = compare_matchup(config, &rust_trace, java_data);
    result.covered_cards = rust_trace.covered_cards;
    result.card_uses = rust_trace.card_uses;
    result
}

/// Print all Rust and Java snapshots side-by-side so we can see exactly what
/// each engine checkpointed and when.
pub fn dump_snapshot_timeline(
    rust_snapshots: &[crate::protocol::StateSnapshot],
    java_snapshots: &[crate::protocol::StateSnapshot],
) {
    fn fmt_snap(idx: usize, s: &crate::protocol::StateSnapshot) -> String {
        format!(
            "{:>4}  T{} {} P{} prio{}",
            idx, s.turn, s.phase, s.active_player, s.priority_player
        )
    }

    let max_len = rust_snapshots.len().max(java_snapshots.len());
    let rust_lines: Vec<String> = rust_snapshots
        .iter()
        .enumerate()
        .map(|(i, s)| fmt_snap(i, s))
        .collect();
    let java_lines: Vec<String> = java_snapshots
        .iter()
        .enumerate()
        .map(|(i, s)| fmt_snap(i, s))
        .collect();
    let col_w = rust_lines
        .iter()
        .map(|s| s.len())
        .max()
        .unwrap_or(20)
        .max(28);

    eprintln!();
    eprintln!(
        "{:col_w$} |   #   Java snapshots",
        "  #   Rust snapshots",
        col_w = col_w
    );
    eprintln!("{:-<col_w$}-+-{:-<28}", "", "", col_w = col_w);
    for i in 0..max_len {
        let left = rust_lines.get(i).map(String::as_str).unwrap_or("");
        let right = java_lines.get(i).map(String::as_str).unwrap_or("");
        eprintln!("{left:col_w$} | {right}");
    }
    eprintln!(
        "Rust: {} snapshots, Java: {} snapshots",
        rust_snapshots.len(),
        java_snapshots.len()
    );
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::{build_rust_only_result, guard_abort_reason};
    use crate::deterministic_agent::VerboseMode;
    use crate::protocol::{
        ChoiceLogEntry, DecisionRecord, GameTrace, MatchupStatus, ParityLogEntry,
    };
    use crate::runner::RunConfig;

    fn test_config() -> RunConfig {
        RunConfig {
            deck1: "red_burn".to_string(),
            deck2: "green_stompy".to_string(),
            seed: 42,
            max_turns: 10,
            cards_dir: None,
            decks_dir: None,
            verbose: VerboseMode::Off,
            prefer_actions: false,
            deep: false,
            loose_parity: false,
            log_snapshots: false,
            java_heap: "512m".to_string(),
            variant: "Constructed".to_string(),
            commanders: vec![],
            full_log: false,
            live_log: None,
            callback_compare: false,
            localize: false,
            mana_probe: Default::default(),
        }
    }

    fn guard_decision(turn: u32) -> ParityLogEntry {
        ParityLogEntry::Decision(DecisionRecord {
            turn,
            phase: "Main1".to_string(),
            deciding_player: 0,
            kind: "$PARITY_GUARD".to_string(),
            options: vec![ChoiceLogEntry {
                name: "guard".to_string(),
                choices: None,
                outcome: "abort".to_string(),
                rng_call_count: None,
            }],
            choice: "abort".to_string(),
            timestamp_ms: 0,
        })
    }

    #[test]
    fn guard_abort_reason_reports_turn() {
        assert_eq!(
            guard_abort_reason(&[guard_decision(7)]),
            Some("ABORTED AT TURN 7".to_string())
        );
    }

    #[test]
    fn rust_only_result_preserves_trace_logs_and_guard_skip_reason() {
        let config = test_config();
        let trace = GameTrace {
            seed: config.seed,
            deck1: config.deck1.clone(),
            deck2: config.deck2.clone(),
            max_turns: config.max_turns,
            variant: config.variant.clone(),
            commanders: vec![],
            log: vec![guard_decision(3)],
            covered_cards: vec!["Lightning Bolt".to_string()],
            card_uses: Default::default(),
        };

        let result = build_rust_only_result(&config, trace);

        assert_eq!(result.status, MatchupStatus::Pass);
        assert_eq!(result.skip_reason, Some("ABORTED AT TURN 3".to_string()));
        assert_eq!(result.rust_log.len(), 1);
        assert!(result.java_log.is_empty());
        assert_eq!(result.covered_cards, vec!["Lightning Bolt".to_string()]);
    }
}
