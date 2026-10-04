use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_foundation::ZoneType;
use manabrew_engine::agent::PlayerAgent;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::game_runtime::GameRuntime;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ActionSpaceManaProbe;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde_json::Value;

use crate::deterministic_agent::VerboseMode;
use crate::java_random::{DrawTape, JavaGameRng, JavaRandom};
use crate::parity_card_map::ParityCardMap;
use crate::protocol::{ParityLogEntry, StateSnapshot};
use crate::runner::{CapturingAgent, LoadedData};
use crate::utils::decks::build_deck_from_templates;

const READY_TIMEOUT: Duration = Duration::from_secs(300);
const END_TIMEOUT: Duration = Duration::from_secs(120);
const STDERR_TAIL: usize = 40;

#[derive(Debug, Clone)]
pub struct ForgeConfig {
    pub jar: PathBuf,
    pub forge_home: PathBuf,
    pub heap: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desync {
    pub kind: String,
    pub detail: String,
}

fn desync(kind: &str, detail: String) -> ! {
    if std::env::var("FORGE_LOCKSTEP_BT").is_ok_and(|v| v == "1") {
        eprintln!(
            "[lockstep] {kind}: {detail}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
    }
    resume_unwind(Box::new(Desync {
        kind: kind.to_string(),
        detail,
    }))
}

enum JavaMessage {
    Ready,
    Need(u64),
    Snapshot(Box<StateSnapshot>),
    End(Value),
    Fatal(String),
}

pub struct ForgeJvm {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<JavaMessage, String>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl Drop for ForgeJvm {
    fn drop(&mut self) {
        self.kill();
    }
}

impl ForgeJvm {
    pub fn spawn(config: &ForgeConfig) -> Result<ForgeJvm, String> {
        let mut child = Command::new(crate::java_bridge::resolve_java_bin(false))
            .arg(format!("-Xmx{}", config.heap))
            .arg("-XX:+DisableExplicitGC")
            .arg("-Dforge.synchronous=true")
            .arg("-Dfile.encoding=UTF-8")
            .arg("-Dsun.stdout.encoding=UTF-8")
            .arg("-Dsun.stderr.encoding=UTF-8")
            .arg("-jar")
            .arg(&config.jar)
            .arg("--lockstep-server")
            .arg("--forge-home")
            .arg(&config.forge_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn java: {e}"))?;
        let stdin = child.stdin.take().ok_or("java stdin")?;
        let stdout = child.stdout.take().ok_or("java stdout")?;
        let stderr_pipe = child.stderr.take().ok_or("java stderr")?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let tail = Arc::clone(&stderr);
        let mut dump = std::env::var("FORGE_LOCKSTEP_STDERR")
            .ok()
            .and_then(|path| std::fs::File::create(path).ok());
        std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                if let Some(file) = dump.as_mut() {
                    let _ = writeln!(file, "{line}");
                }
                let mut tail = tail.lock().unwrap();
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        let (tx, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let message = match line {
                    Ok(line) => match parse_message(&line) {
                        Some(message) => Ok(message),
                        None => continue,
                    },
                    Err(e) => Err(format!("java stdout: {e}")),
                };
                let failed = message.is_err();
                if tx.send(message).is_err() || failed {
                    return;
                }
            }
            let _ = tx.send(Err("java closed stdout".to_string()));
        });
        let mut jvm = ForgeJvm {
            child,
            stdin,
            messages,
            stderr,
        };
        match jvm.messages.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(JavaMessage::Ready)) => Ok(jvm),
            Ok(Ok(JavaMessage::Fatal(e))) => Err(format!("java: {e}")),
            other => {
                let tail = jvm.stderr_tail();
                jvm.kill();
                Err(format!(
                    "java did not start ({}): {tail}",
                    match other {
                        Err(_) => "timeout".to_string(),
                        Ok(Err(e)) => e,
                        Ok(Ok(_)) => "unexpected message".to_string(),
                    }
                ))
            }
        }
    }

    fn send(&mut self, line: &str) -> bool {
        writeln!(self.stdin, "{line}").is_ok() && self.stdin.flush().is_ok()
    }

    fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn quit(mut self) {
        self.send("{\"t\":\"quit\"}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.kill();
    }
}

fn parse_message(line: &str) -> Option<JavaMessage> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(match value.get("t")?.as_str()? {
        "ready" => JavaMessage::Ready,
        "need" => JavaMessage::Need(value["k"].as_u64()?),
        "snap" => JavaMessage::Snapshot(Box::new(serde_json::from_value(value["s"].clone()).ok()?)),
        "end" => JavaMessage::End(value),
        "fatal" => JavaMessage::Fatal(value["error"].as_str().unwrap_or("").to_string()),
        _ => return None,
    })
}

struct Link {
    jvm: ForgeJvm,
    shared: JavaRandom,
    sent: u64,
    pending: VecDeque<JavaMessage>,
    snapshots: Vec<StateSnapshot>,
    end: Option<Value>,
    timeout: Duration,
    seats: Vec<Weak<RefCell<JavaRandom>>>,
}

impl Link {
    fn sync(&self) {
        for seat in &self.seats {
            if let Some(rng) = seat.upgrade() {
                if let Ok(mut rng) = rng.try_borrow_mut() {
                    rng.call_count = self.shared.call_count;
                    rng.api_call_count = self.shared.api_call_count;
                }
            }
        }
    }

    fn poll(&mut self) {
        while let Ok(message) = self.jvm.messages.try_recv() {
            match message {
                Ok(JavaMessage::Snapshot(snapshot)) => self.snapshots.push(*snapshot),
                Ok(JavaMessage::End(end)) => self.end = Some(end),
                Ok(JavaMessage::Need(_)) | Ok(JavaMessage::Ready) => {}
                Ok(message) => self.pending.push_back(message),
                Err(e) => desync("crash", format!("{e}: {}", self.jvm.stderr_tail())),
            }
        }
    }
}

struct SeatTape {
    link: Rc<RefCell<Link>>,
}

impl DrawTape for SeatTape {
    fn counts(&self) -> (u64, u64) {
        let link = self.link.borrow();
        (link.shared.call_count, link.shared.api_call_count)
    }

    fn draw(&mut self, bound: i32) -> i32 {
        let mut link = self.link.borrow_mut();
        let value = link.shared.next_int(bound);
        let line = format!("{{\"t\":\"d\",\"b\":{bound},\"v\":{value},\"f\":1}}");
        if !link.jvm.send(&line) {
            desync(
                "crash",
                format!("java stdin closed: {}", link.jvm.stderr_tail()),
            );
        }
        link.sent += 1;
        link.sync();
        value
    }
}

pub struct LockstepGame<'a> {
    pub data: &'a LoadedData,
    pub decks: [&'a [(String, usize)]; 2],
    pub seed: u64,
    pub max_turns: u32,
    pub log: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockstepEnd {
    GameOver,
    TurnCap,
    GuardMatched(String),
    GuardUnverified(String),
    JavaCrash(String),
    JavaRunawayMatched(String),
    JavaTimeout(String),
    Desync(Desync),
}

pub struct LockstepOutcome {
    pub game: GameState,
    pub winner: Option<PlayerId>,
    pub end: LockstepEnd,
    pub forge_winner: i64,
    pub rust_snapshots: Vec<StateSnapshot>,
    pub java_snapshots: Vec<StateSnapshot>,
    pub rust_log: Vec<ParityLogEntry>,
}

fn inline_spec(cards: &[(String, usize)]) -> String {
    format!(
        "inline:{}",
        cards
            .iter()
            .map(|(name, count)| format!("{name}*{count}"))
            .collect::<Vec<_>>()
            .join("|")
    )
}

fn new_game(
    setup: &LockstepGame,
    abort: &Arc<AtomicBool>,
) -> (GameState, GameLoop, Rc<RefCell<JavaRandom>>) {
    let mut game = GameState::new(&["Player1", "Player2"], 20);
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    game.mirror_forge_bugs = true;
    for (seat, deck) in setup.decks.iter().enumerate() {
        build_deck_from_templates(
            &mut game,
            &setup.data.card_templates,
            &setup.data.db,
            PlayerId(seat as u32),
            deck,
            false,
        );
    }
    let mut game_loop = GameLoop::new(2);
    game_loop.set_provide_priority_action_space(false);
    game_loop.set_abort_signal(Arc::clone(abort));
    setup.data.share_token_data(&mut game_loop);
    let game_rng = Rc::new(RefCell::new({
        let mut rng = JavaRandom::new(setup.seed as i64);
        rng.label = "game";
        rng
    }));
    {
        let mut shuffle_rng = game_rng.borrow_mut();
        for &pid in &game.player_order.clone() {
            let mut library: Vec<CardId> = game.cards_in_zone(ZoneType::Library, pid).to_vec();
            library.sort_by(|a, b| {
                game.cards[a.index()]
                    .card_name
                    .cmp(&game.cards[b.index()].card_name)
            });
            shuffle_rng.shuffle(&mut library);
            library.reverse();
            game.replace_zone_cards(ZoneType::Library, pid, library);
        }
    }
    let players = game.player_order.len() as i32;
    game_rng.borrow_mut().next_int(players);
    for &pid in &game.player_order.clone() {
        game.draw_cards(pid, 7);
    }
    game_loop.game_rng = Box::new(JavaGameRng(Rc::clone(&game_rng)));
    (game, game_loop, game_rng)
}

fn agent_rng(seed: u64, tape: Box<dyn DrawTape>) -> Rc<RefCell<JavaRandom>> {
    let mut rng = JavaRandom::new(seed as i64);
    rng.label = "agent";
    rng.tape = Some(tape);
    Rc::new(RefCell::new(rng))
}

fn compare_snapshots(
    rust: &[StateSnapshot],
    java: &[StateSnapshot],
    from: usize,
) -> Option<Desync> {
    for (index, (r, j)) in rust.iter().zip(java).enumerate().skip(from) {
        if let Some(div) = crate::comparator::compare(index, r, j).into_iter().next() {
            return Some(Desync {
                kind: "state".to_string(),
                detail: format!(
                    "turn {} {}: rust {} java {}",
                    div.turn, div.field, div.rust_value, div.java_value
                ),
            });
        }
    }
    None
}

fn rust_snapshots(log: &Mutex<Vec<ParityLogEntry>>) -> Vec<StateSnapshot> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|entry| match entry {
            ParityLogEntry::Snapshot(snapshot) => Some(snapshot.clone()),
            _ => None,
        })
        .collect()
}

pub fn guard_end(
    rust: &StateSnapshot,
    java_guard: Option<&StateSnapshot>,
    java_end: Option<&Value>,
    sent: u64,
) -> Result<(), String> {
    let java_end = java_end.ok_or("java did not end the game")?;
    if let Some(found) = java_desync(java_end) {
        return Err(found.detail);
    }
    let consumed = java_end["consumed"].as_u64().unwrap_or(0);
    if consumed != sent {
        return Err(format!("java consumed {consumed} of rust's {sent} draws"));
    }
    let mut java = java_guard
        .ok_or("java logged no snapshot at its guard")?
        .clone();
    java.game_over = rust.game_over;
    java.winner = rust.winner;
    for (java_player, rust_player) in java.players.iter_mut().zip(&rust.players) {
        java_player.has_won = rust_player.has_won;
        java_player.has_lost = rust_player.has_lost;
    }
    let fields: Vec<String> = crate::comparator::compare(0, rust, &java)
        .into_iter()
        .take(6)
        .map(|d| format!("{} rust {} java {}", d.field, d.rust_value, d.java_value))
        .collect();
    if fields.is_empty() {
        Ok(())
    } else {
        Err(fields.join("; "))
    }
}

fn last_logged_snapshot(log: &Path) -> Option<StateSnapshot> {
    std::fs::read_to_string(log)
        .ok()?
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| entry.get("players").is_some())
        .and_then(|entry| serde_json::from_value(entry).ok())
}

fn java_desync(end: &Value) -> Option<Desync> {
    if let Some(text) = end["desync"].as_str() {
        let (kind, detail) = text.split_once(": ").unwrap_or(("java", text));
        return Some(Desync {
            kind: kind.to_string(),
            detail: format!("java: {detail}"),
        });
    }
    end["error"].as_str().map(|error| Desync {
        kind: "crash".to_string(),
        detail: format!("java game error: {error}"),
    })
}

type SeatAgents = (Vec<Box<dyn PlayerAgent>>, Vec<Weak<RefCell<JavaRandom>>>);

struct SeatParts {
    shared_log: Arc<Mutex<Vec<ParityLogEntry>>>,
    card_uses: Arc<Mutex<BTreeMap<String, usize>>>,
    snapshot_index: Arc<Mutex<usize>>,
    parity_map: Arc<ParityCardMap>,
    decisions: Arc<AtomicU32>,
}

impl SeatParts {
    fn new() -> Self {
        Self {
            shared_log: Arc::new(Mutex::new(Vec::new())),
            card_uses: Arc::new(Mutex::new(BTreeMap::new())),
            snapshot_index: Arc::new(Mutex::new(0)),
            parity_map: Arc::new(ParityCardMap::default()),
            decisions: Arc::new(AtomicU32::new(0)),
        }
    }

    fn agents(
        &self,
        setup: &LockstepGame,
        abort: &Arc<AtomicBool>,
        game_rng: &Rc<RefCell<JavaRandom>>,
        mut tape: impl FnMut() -> Box<dyn DrawTape>,
    ) -> SeatAgents {
        let mut seats = Vec::new();
        let agents = (0..2u32)
            .map(|p| {
                let rng = agent_rng(setup.seed, tape());
                seats.push(Rc::downgrade(&rng));
                Box::new(CapturingAgent::new(
                    PlayerId(p),
                    VerboseMode::Off,
                    false,
                    Arc::clone(&self.shared_log),
                    Arc::clone(&self.card_uses),
                    Arc::clone(&self.snapshot_index),
                    None,
                    None,
                    rng,
                    Rc::clone(game_rng),
                    Arc::clone(&self.parity_map),
                    p == 0,
                    false,
                    false,
                    Arc::clone(abort),
                    Arc::clone(&self.decisions),
                )) as Box<dyn PlayerAgent>
            })
            .collect();
        (agents, seats)
    }
}

struct ReplayStop;

struct ReplayDraws {
    shared: JavaRandom,
    sent: u64,
    stop_at: u64,
    seats: Vec<Weak<RefCell<JavaRandom>>>,
}

struct ReplayTape(Rc<RefCell<ReplayDraws>>);

impl DrawTape for ReplayTape {
    fn counts(&self) -> (u64, u64) {
        let draws = self.0.borrow();
        (draws.shared.call_count, draws.shared.api_call_count)
    }

    fn draw(&mut self, bound: i32) -> i32 {
        let mut draws = self.0.borrow_mut();
        if draws.sent == draws.stop_at {
            drop(draws);
            resume_unwind(Box::new(ReplayStop));
        }
        let value = draws.shared.next_int(bound);
        draws.sent += 1;
        for seat in &draws.seats {
            if let Some(rng) = seat.upgrade() {
                if let Ok(mut rng) = rng.try_borrow_mut() {
                    rng.call_count = draws.shared.call_count;
                    rng.api_call_count = draws.shared.api_call_count;
                }
            }
        }
        value
    }
}

pub fn state_at_draw(
    setup: &LockstepGame,
    abort: &Arc<AtomicBool>,
    draws: u64,
) -> Option<StateSnapshot> {
    let (game, game_loop, game_rng) = new_game(setup, abort);
    let replay = Rc::new(RefCell::new(ReplayDraws {
        shared: {
            let mut rng = JavaRandom::new(setup.seed as i64);
            rng.label = "agent";
            rng
        },
        sent: 0,
        stop_at: draws,
        seats: Vec::new(),
    }));
    let parts = SeatParts::new();
    let (agents, seats) = parts.agents(setup, abort, &game_rng, || {
        Box::new(ReplayTape(Rc::clone(&replay)))
    });
    replay.borrow_mut().seats = seats;
    crate::parity_log::set_sink(Arc::new(Mutex::new(Vec::new())));
    let mut runtime = GameRuntime::from_parts(game, game_loop, agents);
    let run = catch_unwind(AssertUnwindSafe(|| {
        runtime.run_opening_hand_actions();
        parts
            .parity_map
            .initialize_from_opening_state(runtime.game());
        let mut rng = StdRng::seed_from_u64(setup.seed);
        while !runtime.game().game_over
            && runtime.game().turn.turn_number <= setup.max_turns
            && !abort.load(Ordering::Relaxed)
        {
            runtime.run_turn(&mut rng);
        }
    }));
    crate::parity_log::clear_sink();
    match run {
        Err(stop) if stop.is::<ReplayStop>() => {
            Some(crate::snapshot::snapshot_game(runtime.game(), &[]))
        }
        _ => None,
    }
}

pub fn guard_unverified(
    detail: &str,
    java_end: Option<&Value>,
    timeout: Duration,
) -> Option<LockstepEnd> {
    java_end.is_none().then(|| {
        LockstepEnd::GuardUnverified(format!(
            "{detail} | java did not reach its guard within {}s",
            timeout.as_secs()
        ))
    })
}

pub fn java_timeout(
    rust: &[StateSnapshot],
    java: &[StateSnapshot],
    timeout: Duration,
) -> Result<LockstepEnd, Desync> {
    match compare_snapshots(rust, java, 0) {
        Some(found) => Err(found),
        None => Ok(LockstepEnd::JavaTimeout(format!(
            "java did not end the game within {}s; its {} of rust's {} turn snapshots agree",
            timeout.as_secs(),
            java.len(),
            rust.len()
        ))),
    }
}

pub fn java_error_end(
    java: &Value,
    detail: String,
    earlier: Option<Desync>,
    rust_at_draw: impl FnOnce(u64) -> Option<StateSnapshot>,
) -> Result<LockstepEnd, Desync> {
    if let Some(earlier) = earlier {
        return Err(Desync {
            kind: earlier.kind,
            detail: format!("{} (then {detail})", earlier.detail),
        });
    }
    if !java["error"]
        .as_str()
        .is_some_and(|error| error.contains("RunawayGameException"))
    {
        return Ok(LockstepEnd::JavaCrash(detail));
    }
    let consumed = java["consumed"].as_u64().unwrap_or(0);
    rust_at_draw(consumed)
        .ok_or_else(|| format!("rust ended before java's draw {consumed}"))
        .and_then(|rust| runaway_end(&rust, java))
        .map(|()| LockstepEnd::JavaRunawayMatched(detail.clone()))
        .map_err(|mismatch| Desync {
            kind: "runaway".to_string(),
            detail: format!("{detail} | state at the runaway cap: {mismatch}"),
        })
}

pub fn runaway_end(rust: &StateSnapshot, java_end: &Value) -> Result<(), String> {
    let mut java: StateSnapshot = java_end
        .get("snapshot")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .ok_or("java logged no snapshot at its runaway cap")?;
    java.game_over = rust.game_over;
    java.winner = rust.winner;
    for (java_player, rust_player) in java.players.iter_mut().zip(&rust.players) {
        java_player.has_won = rust_player.has_won;
        java_player.has_lost = rust_player.has_lost;
    }
    let fields: Vec<String> = crate::comparator::compare(0, rust, &java)
        .into_iter()
        .filter(|d| !d.field.ends_with("rng_calls"))
        .take(6)
        .map(|d| format!("{} rust {} java {}", d.field, d.rust_value, d.java_value))
        .collect();
    if fields.is_empty() {
        Ok(())
    } else {
        Err(fields.join("; "))
    }
}

pub fn play(
    slot: &mut Option<ForgeJvm>,
    config: &ForgeConfig,
    setup: &LockstepGame,
    abort: &Arc<AtomicBool>,
) -> Result<LockstepOutcome, String> {
    let jvm = match slot.take() {
        Some(jvm) => jvm,
        None => ForgeJvm::spawn(config)?,
    };
    let mut start = serde_json::json!({
        "t": "start",
        "deck1": inline_spec(setup.decks[0]),
        "deck2": inline_spec(setup.decks[1]),
        "seed": setup.seed,
        "max_turns": setup.max_turns,
    });
    if let Some(log) = &setup.log {
        start["log"] = Value::String(log.to_string_lossy().to_string());
    }
    let link = Rc::new(RefCell::new(Link {
        jvm,
        shared: {
            let mut rng = JavaRandom::new(setup.seed as i64);
            rng.label = "agent";
            rng
        },
        sent: 0,
        pending: VecDeque::new(),
        snapshots: Vec::new(),
        end: None,
        timeout: config.timeout,
        seats: Vec::new(),
    }));
    if !link.borrow_mut().jvm.send(&start.to_string()) {
        let mut link = Rc::try_unwrap(link).ok().unwrap().into_inner();
        link.jvm.kill();
        return Err("java stdin closed".to_string());
    }

    let (game, game_loop, game_rng) = new_game(setup, abort);
    let parts = SeatParts::new();
    let shared_log = Arc::clone(&parts.shared_log);
    let parity_map = Arc::clone(&parts.parity_map);
    let (agents, seats) = parts.agents(setup, abort, &game_rng, || {
        Box::new(SeatTape {
            link: Rc::clone(&link),
        })
    });
    link.borrow_mut().seats = seats;

    let choice_log = Arc::new(Mutex::new(Vec::new()));
    crate::parity_log::set_sink(Arc::clone(&choice_log));
    let mut runtime = GameRuntime::from_parts(game, game_loop, agents);
    let mut compared = 0;
    let run = catch_unwind(AssertUnwindSafe(|| {
        runtime.run_opening_hand_actions();
        parity_map.initialize_from_opening_state(runtime.game());
        let mut rng = StdRng::seed_from_u64(setup.seed);
        while !runtime.game().game_over
            && runtime.game().turn.turn_number <= setup.max_turns
            && !abort.load(Ordering::Relaxed)
        {
            runtime.run_turn(&mut rng);
            link.borrow_mut().poll();
            let rust = rust_snapshots(&shared_log);
            let link = link.borrow();
            if let Some(found) = compare_snapshots(&rust, &link.snapshots, compared) {
                drop(link);
                resume_unwind(Box::new(found));
            }
            compared = rust.len().min(link.snapshots.len());
        }
    }));

    let mut end = match run {
        Ok(()) => None,
        Err(panic) => match panic.downcast::<Desync>() {
            Ok(found) => Some(*found),
            Err(panic) => {
                crate::parity_log::clear_sink();
                drop(runtime);
                finish_java(&link, Finish::Abort);
                *slot = release(link);
                resume_unwind(panic);
            }
        },
    };
    if end.is_none() && abort.load(Ordering::Relaxed) {
        let guard = shared_log
            .lock()
            .unwrap()
            .iter()
            .find_map(|entry| match entry {
                ParityLogEntry::Decision(d) if d.kind == "$PARITY_GUARD" => Some(d.choice.clone()),
                _ => None,
            });
        end = Some(match guard {
            Some(choice) => Desync {
                kind: "guard".to_string(),
                detail: format!("parity guard: {choice}"),
            },
            None => Desync {
                kind: "abort".to_string(),
                detail: "the game was aborted".to_string(),
            },
        });
    }
    crate::parity_log::clear_sink();
    let game_over = runtime.game().game_over;
    let guard_tripped = end.as_ref().is_some_and(|found| found.kind == "guard");
    let finish = match (&end, game_over) {
        (Some(_), _) if guard_tripped => Finish::CatchUp,
        (Some(_), _) => Finish::Abort,
        (None, true) => Finish::GameOver,
        (None, false) => Finish::TurnCap,
    };
    let (java_end, overran, timed_out) = finish_java(&link, finish);
    if let Some(found) = end.as_mut() {
        let link = link.borrow();
        if let Some(earlier) = compare_snapshots(&rust_snapshots(&shared_log), &link.snapshots, 0) {
            if earlier.detail != found.detail {
                *found = Desync {
                    kind: earlier.kind,
                    detail: format!("{} (then {}: {})", earlier.detail, found.kind, found.detail),
                };
            }
        }
    }
    let mut special = end
        .as_ref()
        .filter(|found| found.kind == "guard")
        .and_then(|found| {
            guard_unverified(&found.detail, java_end.as_ref(), link.borrow().timeout)
        });
    let mut guard_mismatch = None;
    if let Some(found) = end
        .as_ref()
        .filter(|found| found.kind == "guard" && special.is_none())
    {
        let mut rust = rust_snapshots(&shared_log);
        let rust_guard = rust.pop();
        let java_guard = setup.log.as_deref().and_then(last_logged_snapshot);
        let turns_match = rust.len() == link.borrow().snapshots.len();
        let compared = rust_guard
            .ok_or_else(|| "rust logged no snapshot at its guard".to_string())
            .and_then(|rust_guard| {
                guard_end(
                    &rust_guard,
                    java_guard.as_ref(),
                    java_end.as_ref(),
                    link.borrow().sent,
                )
            });
        match compared {
            Ok(()) if turns_match => {
                special = Some(LockstepEnd::GuardMatched(found.detail.clone()))
            }
            Ok(()) => {
                guard_mismatch = Some(format!(
                    "rust took {} turn snapshots, java {}",
                    rust.len(),
                    link.borrow().snapshots.len()
                ))
            }
            Err(mismatch) => guard_mismatch = Some(mismatch),
        }
    }
    if special.is_some() {
        end = None;
    }
    if let (Some(found), Some(mismatch)) = (end.as_mut(), guard_mismatch) {
        found.detail = format!("{} | guard comparison: {mismatch}", found.detail);
    }
    if let (Some(path), Some(java)) = (&setup.log, &java_end) {
        let _ = std::fs::write(path.with_extension("end.json"), java.to_string());
    }
    if let (Some(found), Some(java)) = (end.as_mut(), java_end.as_ref()) {
        if let Some(snapshot) = java
            .get("snapshot")
            .and_then(|s| serde_json::from_value::<StateSnapshot>(s.clone()).ok())
        {
            let rust = crate::snapshot::snapshot_game(runtime.game(), &[]);
            let fields: Vec<String> = crate::comparator::compare(0, &rust, &snapshot)
                .into_iter()
                .filter(|d| !d.field.ends_with("rng_calls"))
                .take(6)
                .map(|d| format!("{} rust {} java {}", d.field, d.rust_value, d.java_value))
                .collect();
            found.detail = format!("{} | state at desync: {}", found.detail, fields.join("; "));
        }
    }
    let rust = rust_snapshots(&shared_log);
    if end.is_none() && overran && special.is_none() {
        end = Some(Desync {
            kind: "sequence".to_string(),
            detail: "java kept playing after rust's game ended".to_string(),
        });
    }
    if end.is_none() && special.is_none() {
        end = match &java_end {
            None if timed_out => {
                let link = link.borrow();
                match java_timeout(&rust, &link.snapshots, link.timeout) {
                    Ok(kind) => {
                        special = Some(kind);
                        None
                    }
                    Err(found) => Some(found),
                }
            }
            None => Some(Desync {
                kind: "sequence".to_string(),
                detail: "java did not end the game with rust".to_string(),
            }),
            Some(java) => {
                java_desync(java).filter(|d| !(finish == Finish::TurnCap && d.kind == "abort"))
            }
        };
    }
    if let Some(java) = java_end.as_ref().filter(|java| {
        java["error"].is_string() && end.as_ref().is_some_and(|found| found.kind == "crash")
    }) {
        let detail = end.take().map(|found| found.detail).unwrap_or_default();
        let earlier = compare_snapshots(&rust, &link.borrow().snapshots, 0);
        match java_error_end(java, detail, earlier, |draws| {
            state_at_draw(setup, abort, draws)
        }) {
            Ok(kind) => special = Some(kind),
            Err(found) => end = Some(found),
        }
    }
    if end.is_none() && special.is_none() {
        let link = link.borrow();
        end = compare_snapshots(&rust, &link.snapshots, 0).or_else(|| {
            (rust.len() != link.snapshots.len()).then(|| Desync {
                kind: "sequence".to_string(),
                detail: format!(
                    "rust took {} turn snapshots, java {}",
                    rust.len(),
                    link.snapshots.len()
                ),
            })
        });
    }
    let winner = runtime.game().winner;
    let forge_winner = java_end
        .as_ref()
        .and_then(|e| e["winner"].as_i64())
        .unwrap_or(-1);
    if end.is_none()
        && special.is_none()
        && game_over
        && forge_winner != winner.map_or(-1, |w| i64::from(w.0))
    {
        end = Some(Desync {
            kind: "sequence".to_string(),
            detail: format!("rust winner {winner:?}, java winner {forge_winner}"),
        });
    }
    let sent = link.borrow().sent;
    if let Some(consumed) = java_end
        .as_ref()
        .and_then(|e| e["consumed"].as_u64())
        .filter(|&consumed| end.is_none() && special.is_none() && consumed < sent)
    {
        end = Some(Desync {
            kind: "sequence".to_string(),
            detail: format!("java ended the game after {consumed} of rust's {sent} draws"),
        });
    }
    let java_snapshots = std::mem::take(&mut link.borrow_mut().snapshots);
    let GameRuntime { game, agents, .. } = runtime;
    drop(agents);
    *slot = release(link);
    let rust_log = std::mem::take(&mut *shared_log.lock().unwrap());
    Ok(LockstepOutcome {
        game,
        winner,
        end: match (end, special) {
            (Some(found), _) => LockstepEnd::Desync(found),
            (None, Some(special)) => special,
            (None, None) if game_over => LockstepEnd::GameOver,
            (None, None) => LockstepEnd::TurnCap,
        },
        forge_winner,
        rust_snapshots: rust,
        java_snapshots,
        rust_log,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    GameOver,
    TurnCap,
    CatchUp,
    Abort,
}

fn finish_java(link: &Rc<RefCell<Link>>, finish: Finish) -> (Option<Value>, bool, bool) {
    let mut link = link.borrow_mut();
    let mut overran = finish == Finish::GameOver && !link.pending.is_empty();
    if let Some(end) = link.end.take() {
        return (Some(end), overran, false);
    }
    let mut aborted = finish == Finish::Abort;
    if aborted {
        link.jvm.send("{\"t\":\"abort\"}");
    }
    let deadline = Instant::now() + if aborted { END_TIMEOUT } else { link.timeout };
    while Instant::now() < deadline {
        match link.jvm.messages.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(JavaMessage::End(end))) => return (Some(end), overran, false),
            Ok(Ok(JavaMessage::Snapshot(snapshot))) => link.snapshots.push(*snapshot),
            Ok(Ok(JavaMessage::Need(k))) => {
                if !aborted && k >= link.sent {
                    overran |= finish == Finish::GameOver;
                    link.jvm.send("{\"t\":\"abort\"}");
                    aborted = true;
                }
            }
            Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => return (None, overran, false),
        }
    }
    link.jvm.kill();
    (None, overran, !aborted)
}

fn release(link: Rc<RefCell<Link>>) -> Option<ForgeJvm> {
    let mut link = Rc::try_unwrap(link).ok()?.into_inner();
    match link.jvm.child.try_wait() {
        Ok(None) => Some(link.jvm),
        _ => {
            link.jvm.kill();
            None
        }
    }
}
