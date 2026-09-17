//! `parity sweep`: Rust-only random play over a card list, built to say why a
//! game did not finish.
//!
//! Cards are grouped by colour into decks of a few cards each. Every deck runs
//! in its own child process (`parity sweep-deck`), so a game that never returns
//! can be sampled and killed without taking the run down with it, and one
//! panic cannot leak state into later games. Each game ends with a status:
//!
//! - `OK`       finished or reached the turn limit
//! - `PANIC`    with the message and location
//! - `LOOP`     the same game state came back at a decision point over and over
//! - `BUDGET`   the decision budget ran out without a repeated state
//! - `TIMEOUT`  the game noticed it was past its wall-clock budget
//! - `KILLED`   the child stopped answering; the parent sampled and killed it
//!
//! Every status other than `OK` comes with a command that replays that game.
//! Agent seeds derive from the deck contents and the seed, not from a deck's
//! position in the run, so the replay does not need the rest of the run.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use forge_foundation::ZoneType;
use manabrew_engine::agent::{
    ActivatableAction, ManaAbilityOption, ManaCostAction, PlayerAgent, PriorityActionSpace,
    TargetChoice,
};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::game_runtime::GameRuntime;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::deck_generator::{format_inline, parse_inline, DeckSpec};
use crate::runner::LoadedData;
use crate::script_index::read_card_list;
use crate::utils::decks::build_deck_from_spec;

const LOOP_REPEATS: u32 = 200;

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
struct CardStat {
    offered: u64,
    cast: u64,
    failed: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct GameRecord {
    seed: u64,
    status: String,
    turns: u32,
    decisions: u32,
    secs: f64,
    detail: String,
    cards: BTreeMap<String, CardStat>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum ChildEvent {
    Start { seed: u64 },
    End(GameRecord),
}

#[derive(Default)]
struct Shared {
    names: Vec<String>,
    cards: BTreeMap<String, CardStat>,
    decisions: u32,
    turn: u32,
    stop: Option<(&'static str, String)>,
    seen: HashMap<u64, u32>,
    seen_turn: u32,
    recent: Vec<String>,
}

struct Agent {
    shared: std::sync::Arc<Mutex<Shared>>,
    pending: Option<CardId>,
    rng: StdRng,
    streak: u32,
    started: Instant,
    decision_budget: u32,
    wall_budget: Duration,
}

fn fingerprint(game: &GameState, pools: &[ManaPool]) -> u64 {
    let mut h = DefaultHasher::new();
    game.turn.turn_number.hash(&mut h);
    format!("{:?}", game.turn.phase).hash(&mut h);
    game.stack.len().hash(&mut h);
    for (index, player) in game.players.iter().enumerate() {
        let pid = PlayerId(index as u32);
        player.life.hash(&mut h);
        player.poison_counters.hash(&mut h);
        for zone in [
            ZoneType::Hand,
            ZoneType::Library,
            ZoneType::Graveyard,
            ZoneType::Exile,
        ] {
            game.zone(zone, pid).len().hash(&mut h);
        }
        for &cid in game.cards_in_zone(ZoneType::Battlefield, pid) {
            let card = game.card(cid);
            cid.index().hash(&mut h);
            card.tapped.hash(&mut h);
            card.damage.hash(&mut h);
            card.power().hash(&mut h);
            card.toughness().hash(&mut h);
            for (counter, count) in &card.counters {
                format!("{counter:?}").hash(&mut h);
                count.hash(&mut h);
            }
        }
        if let Some(pool) = pools.get(index) {
            (
                pool.white(),
                pool.blue(),
                pool.black(),
                pool.red(),
                pool.green(),
                pool.colorless(),
            )
                .hash(&mut h);
        }
    }
    h.finish()
}

impl Agent {
    fn pick<T: Copy>(&mut self, options: &[T]) -> Option<T> {
        if options.is_empty() {
            None
        } else {
            Some(options[self.rng.gen_range(0..options.len())])
        }
    }

    fn name(&self, id: CardId) -> String {
        self.shared
            .lock()
            .unwrap()
            .names
            .get(id.index())
            .cloned()
            .unwrap_or_else(|| "?".to_string())
    }

    fn stat(&self, id: CardId, update: impl FnOnce(&mut CardStat)) {
        let name = self.name(id);
        update(self.shared.lock().unwrap().cards.entry(name).or_default());
    }
}

impl PlayerAgent for Agent {
    fn snapshot_state(&mut self, game: &GameState, pools: &[ManaPool]) {
        let mut shared = self.shared.lock().unwrap();
        if game.cards.len() > shared.names.len() {
            let known = shared.names.len();
            shared
                .names
                .extend(game.cards[known..].iter().map(|c| c.card_name.clone()));
        }
        shared.turn = game.turn.turn_number;
        if shared.stop.is_some() {
            return;
        }
        if shared.seen_turn != game.turn.turn_number {
            shared.seen_turn = game.turn.turn_number;
            shared.seen.clear();
        }
        let repeats = {
            let count = shared.seen.entry(fingerprint(game, pools)).or_insert(0);
            *count += 1;
            *count
        };
        if repeats >= LOOP_REPEATS {
            let recent = shared.recent.join(" > ");
            shared.stop = Some((
                "LOOP",
                format!(
                    "turn {} {:?}: the same state came back {repeats} times; last actions: {recent}",
                    game.turn.turn_number, game.turn.phase
                ),
            ));
        }
    }

    fn mulligan_decision(&mut self, _p: PlayerId, _h: &[CardId], _m: u32) -> bool {
        true
    }

    fn choose_action(
        &mut self,
        _player: PlayerId,
        space: Option<&PriorityActionSpace>,
        request: &mut dyn FnMut() -> PriorityActionSpace,
    ) -> PlayerAction {
        {
            let mut shared = self.shared.lock().unwrap();
            shared.decisions += 1;
            if shared.stop.is_none() && shared.decisions >= self.decision_budget {
                shared.stop = Some((
                    "BUDGET",
                    format!(
                        "{} decisions without a repeated state",
                        self.decision_budget
                    ),
                ));
            }
            if shared.stop.is_none() && self.started.elapsed() > self.wall_budget {
                shared.stop = Some((
                    "TIMEOUT",
                    format!(
                        "past {}s at turn {}",
                        self.wall_budget.as_secs(),
                        shared.turn
                    ),
                ));
            }
            if shared.stop.is_some() {
                return PlayerAction::Concede;
            }
        }
        let owned;
        let space = match space {
            Some(space) => space,
            None => {
                owned = request();
                &owned
            }
        };
        if let Some(pending) = self.pending.take() {
            if space.playable.iter().any(|po| po.card_id == pending) {
                self.stat(pending, |s| s.failed += 1);
            }
        }
        let activatable: Vec<&ActivatableAction> = space
            .activatable
            .iter()
            .filter(|a| !a.is_mana_ability)
            .collect();
        let options = space.playable.len() + activatable.len();
        if options == 0 {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        for playable in &space.playable {
            self.stat(playable.card_id, |s| s.offered += 1);
        }
        if self.streak >= 30 {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        let roll = self.rng.gen_range(0..(options as u32 * 3 + 1));
        if roll == options as u32 * 3 {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        self.streak += 1;
        let index = (roll / 3) as usize;
        let (card_id, action) = if index < space.playable.len() {
            let playable = space.playable[index];
            self.stat(playable.card_id, |s| s.cast += 1);
            self.pending = Some(playable.card_id);
            (playable.card_id, PlayerAction::CastSpell(playable))
        } else {
            let ability = activatable[index - space.playable.len()];
            (
                ability.card_id,
                PlayerAction::ActivateAbility(AbilityRef {
                    card_id: ability.card_id,
                    ability_index: ability.ability_index,
                }),
            )
        };
        let label = match &action {
            PlayerAction::CastSpell(_) => format!("cast {}", self.name(card_id)),
            _ => format!("activate {}", self.name(card_id)),
        };
        let mut shared = self.shared.lock().unwrap();
        shared.recent.push(label);
        if shared.recent.len() > 6 {
            shared.recent.remove(0);
        }
        action
    }

    fn choose_attackers(
        &mut self,
        _p: PlayerId,
        available: &[CardId],
        defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        if defenders.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        for &attacker in available {
            if self.rng.gen_bool(0.5) {
                out.push((attacker, defenders[self.rng.gen_range(0..defenders.len())]));
            }
        }
        out
    }

    fn choose_blockers(
        &mut self,
        _p: PlayerId,
        attackers: &[CardId],
        blockers: &[CardId],
        max: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        let mut out = Vec::new();
        if attackers.is_empty() {
            return out;
        }
        for &blocker in blockers {
            if max.is_some_and(|m| out.len() >= m) {
                break;
            }
            if self.rng.gen_bool(0.5) {
                out.push((blocker, attackers[self.rng.gen_range(0..attackers.len())]));
            }
        }
        out
    }

    fn choose_targets_for(
        &mut self,
        sa: &mut SpellAbility,
        game: &GameState,
        pools: &[ManaPool],
    ) -> bool {
        manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, pools)
    }

    fn choose_target_player(
        &mut self,
        _p: PlayerId,
        valid: &[PlayerId],
        _sa: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        self.pick(valid)
    }

    fn choose_target_card(
        &mut self,
        _p: PlayerId,
        valid: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        self.pick(valid)
    }

    fn choose_target_any(
        &mut self,
        _p: PlayerId,
        players: &[PlayerId],
        cards: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        let total = players.len() + cards.len();
        if total == 0 {
            return TargetChoice::None;
        }
        let index = self.rng.gen_range(0..total);
        if index < players.len() {
            TargetChoice::Player(players[index])
        } else {
            TargetChoice::Card(cards[index - players.len()])
        }
    }

    fn choose_land_or_spell(&mut self, _p: PlayerId) -> Option<bool> {
        Some(self.rng.gen_bool(0.5))
    }

    fn pay_mana_cost(
        &mut self,
        _p: PlayerId,
        _card: CardId,
        _name: &str,
        _cost: &str,
        _cost_display: &str,
        _cost_colors: &str,
        _can_pay: bool,
        _auto_resolvable: bool,
        _sources: &[CardId],
        _options: &[ManaAbilityOption],
        _tappable: &[CardId],
        _untappable: &[CardId],
        _pool: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
}

struct GameLimits {
    max_turns: u32,
    decision_budget: u32,
    wall_budget: Duration,
}

fn deck_seed(deck: &DeckSpec, seed: u64) -> u64 {
    let mut h = DefaultHasher::new();
    format_inline(deck).hash(&mut h);
    seed.hash(&mut h);
    h.finish()
}

thread_local! {
    static LAST_PANIC_LOCATION: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn play_game(data: &LoadedData, deck: &DeckSpec, seed: u64, limits: &GameLimits) -> GameRecord {
    let started = Instant::now();
    let shared = std::sync::Arc::new(Mutex::new(Shared::default()));
    let base = deck_seed(deck, seed);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let db = &data.db;
        let mut game = GameState::new(&["P1", "P2"], 20);
        build_deck_from_spec(&mut game, db, PlayerId(0), deck, false);
        build_deck_from_spec(&mut game, db, PlayerId(1), deck, false);
        let mut game_loop = GameLoop::new(2);
        game_loop.set_provide_priority_action_space(false);
        for (name, template) in &data.token_templates {
            game_loop.register_token(name.clone(), template.clone());
        }
        game_loop.token_art_variants = db.token_art_variants().clone().into_iter().collect();
        game_loop.token_fallback = db.token_fallback().clone().into_iter().collect();
        game_loop.edition_dates = db.edition_dates().clone().into_iter().collect();
        let agents: Vec<Box<dyn PlayerAgent>> = (0..2u64)
            .map(|player| {
                Box::new(Agent {
                    shared: std::sync::Arc::clone(&shared),
                    pending: None,
                    rng: StdRng::seed_from_u64(base.wrapping_add(player + 1)),
                    streak: 0,
                    started,
                    decision_budget: limits.decision_budget,
                    wall_budget: limits.wall_budget,
                }) as Box<dyn PlayerAgent>
            })
            .collect();
        let mut runtime = GameRuntime::from_parts(game, game_loop, agents);
        let mut rng = StdRng::seed_from_u64(base);
        runtime.run(&mut rng, limits.max_turns);
    }));
    manabrew_engine::census::flush();

    let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    let (status, detail) = match (&outcome, &shared.stop) {
        (Err(payload), _) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            let location = LAST_PANIC_LOCATION.with(|l| l.borrow().clone());
            ("PANIC".to_string(), format!("{location} {message}"))
        }
        (Ok(()), Some((status, detail))) => (status.to_string(), detail.clone()),
        (Ok(()), None) => ("OK".to_string(), String::new()),
    };
    GameRecord {
        seed,
        status,
        turns: shared.turn,
        decisions: shared.decisions,
        secs: started.elapsed().as_secs_f64(),
        detail: detail.chars().take(400).collect(),
        cards: if outcome.is_ok() {
            shared.cards.clone()
        } else {
            BTreeMap::new()
        },
    }
}

fn option<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn number<T: std::str::FromStr>(args: &[String], flag: &str, default: T) -> T {
    option(args, flag)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn limits(args: &[String]) -> GameLimits {
    GameLimits {
        max_turns: number(args, "--max-turns", 30),
        decision_budget: number(args, "--decision-budget", 20_000),
        wall_budget: Duration::from_secs(number(args, "--game-secs", 60)),
    }
}

/// `parity sweep-deck --deck <inline> --seeds a,b,c [limits] [--census-out F]`
/// One JSON line per event on stdout. Also the replay command for one game.
pub fn run_deck_cli(args: &[String], data: &LoadedData) -> i32 {
    let Some(deck) = option(args, "--deck") else {
        eprintln!("usage: parity sweep-deck --deck <inline spec> --seeds 0,1,2 [--max-turns N]");
        return 2;
    };
    let deck = match parse_inline(deck.strip_prefix("inline:").unwrap_or(deck)) {
        Ok(deck) => deck,
        Err(e) => {
            eprintln!("sweep-deck: {e}");
            return 2;
        }
    };
    let seeds: Vec<u64> = option(args, "--seeds")
        .unwrap_or("0")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let limits = limits(args);
    if option(args, "--census-out").is_some() {
        manabrew_engine::census::enable();
    }
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        LAST_PANIC_LOCATION.with(|l| *l.borrow_mut() = location);
    }));

    let stdout = std::io::stdout();
    let mut bad = 0;
    for seed in seeds {
        let mut out = stdout.lock();
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string(&ChildEvent::Start { seed }).unwrap()
        );
        let _ = out.flush();
        drop(out);
        let record = play_game(data, &deck, seed, &limits);
        bad += i32::from(record.status != "OK");
        let mut out = stdout.lock();
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string(&ChildEvent::End(record)).unwrap()
        );
        let _ = out.flush();
    }
    if let Some(path) = option(args, "--census-out") {
        if let Err(e) = crate::census_report::write_census(Path::new(path)) {
            eprintln!("sweep-deck: {path}: {e}");
            return 2;
        }
    }
    i32::from(bad > 0)
}

struct DeckJob {
    index: usize,
    cards: Vec<String>,
    spec: DeckSpec,
}

fn build_decks(data: &LoadedData, names: &[String], chunk: usize) -> (Vec<DeckJob>, usize) {
    let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    let mut missing = 0usize;
    for name in names {
        let Some(rules) = data.db.get_by_card_name(name) else {
            missing += 1;
            continue;
        };
        let type_line = &rules.main_part.type_line;
        if type_line.is_land() && type_line.is_basic() {
            continue;
        }
        let mut colors: Vec<String> = rules
            .color_identity
            .iter()
            .map(|c| c.basic_land_type().to_string())
            .collect();
        colors.sort();
        groups.entry(colors).or_default().push(name.clone());
    }
    let mut decks = Vec::new();
    for (lands, mut cards) in groups {
        cards.sort();
        cards.dedup();
        let lands = if lands.is_empty() {
            vec!["Plains".to_string()]
        } else {
            lands
        };
        for chunk_cards in cards.chunks(chunk.max(1)) {
            let mut spec: DeckSpec = chunk_cards.iter().map(|n| (n.clone(), 4)).collect();
            for (i, land) in lands.iter().enumerate() {
                let count = 20 / lands.len() + usize::from(i < 20 % lands.len());
                spec.push((land.clone(), count));
            }
            decks.push(DeckJob {
                index: decks.len(),
                cards: chunk_cards.to_vec(),
                spec,
            });
        }
    }
    (decks, missing)
}

struct DeckOutcome {
    records: Vec<GameRecord>,
    census: Vec<PathBuf>,
}

fn sample_process(pid: u32, out: &Path) -> bool {
    Command::new("sample")
        .args([&pid.to_string(), "1", "-file"])
        .arg(out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run_deck_in_children(
    exe: &Path,
    job: &DeckJob,
    seeds: &[u64],
    args: &[String],
    out_dir: &Path,
    census: bool,
) -> DeckOutcome {
    let game_limits = limits(args);
    let stall =
        game_limits.wall_budget + Duration::from_secs(number(args, "--kill-after-secs", 30));
    let inline = format_inline(&job.spec);
    let mut remaining: Vec<u64> = seeds.to_vec();
    let mut outcome = DeckOutcome {
        records: Vec::new(),
        census: Vec::new(),
    };

    while !remaining.is_empty() {
        let seeds_arg = remaining
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut command = Command::new(exe);
        command
            .arg("sweep-deck")
            .args(["--deck", &inline, "--seeds", &seeds_arg])
            .args(["--max-turns", &game_limits.max_turns.to_string()])
            .args([
                "--decision-budget",
                &game_limits.decision_budget.to_string(),
            ])
            .args([
                "--game-secs",
                &game_limits.wall_budget.as_secs().to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if census {
            let path = out_dir.join(format!(
                "census-{}-{}.json",
                job.index,
                outcome.census.len()
            ));
            command.arg("--census-out").arg(&path);
            outcome.census.push(path);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                for seed in remaining.drain(..) {
                    outcome.records.push(GameRecord {
                        seed,
                        status: "KILLED".into(),
                        turns: 0,
                        decisions: 0,
                        secs: 0.0,
                        detail: format!("could not start the child: {e}"),
                        cards: BTreeMap::new(),
                    });
                }
                break;
            }
        };
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel::<ChildEvent>();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(event) = serde_json::from_str::<ChildEvent>(&line) {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        });

        let mut in_progress: Option<u64> = None;
        loop {
            match rx.recv_timeout(stall) {
                Ok(ChildEvent::Start { seed }) => in_progress = Some(seed),
                Ok(ChildEvent::End(record)) => {
                    remaining.retain(|s| *s != record.seed);
                    in_progress = None;
                    outcome.records.push(record);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.wait();
                    if let Some(seed) = in_progress.take() {
                        remaining.retain(|s| *s != seed);
                        outcome.records.push(GameRecord {
                            seed,
                            status: "KILLED".into(),
                            turns: 0,
                            decisions: 0,
                            secs: 0.0,
                            detail: "the child exited mid-game (abort, stack overflow or signal)"
                                .into(),
                            cards: BTreeMap::new(),
                        });
                    } else if !remaining.is_empty() {
                        let seed = remaining.remove(0);
                        outcome.records.push(GameRecord {
                            seed,
                            status: "KILLED".into(),
                            turns: 0,
                            decisions: 0,
                            secs: 0.0,
                            detail: "the child exited before starting this game".into(),
                            cards: BTreeMap::new(),
                        });
                    }
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let seed = in_progress.take().unwrap_or_else(|| remaining[0]);
                    let sample =
                        out_dir.join(format!("stall-deck{}-seed{seed}.sample.txt", job.index));
                    let sampled = sample_process(child.id(), &sample);
                    let _ = child.kill();
                    let _ = child.wait();
                    remaining.retain(|s| *s != seed);
                    outcome.records.push(GameRecord {
                        seed,
                        status: "KILLED".into(),
                        turns: 0,
                        decisions: 0,
                        secs: stall.as_secs_f64(),
                        detail: if sampled {
                            format!(
                                "no decision for {}s; stack sample in {}",
                                stall.as_secs(),
                                sample.display()
                            )
                        } else {
                            format!("no decision for {}s; no stack sample", stall.as_secs())
                        },
                        cards: BTreeMap::new(),
                    });
                    break;
                }
            }
        }
        let _ = reader.join();
    }
    outcome.records.sort_by_key(|r| r.seed);
    outcome
}

fn merge_census(files: &[PathBuf], out: &Path) -> std::io::Result<()> {
    use crate::census_report::{CensusFile, CensusParam, CensusUnhandled};
    let mut params: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
    let mut unhandled: BTreeMap<(String, String), u64> = BTreeMap::new();
    for file in files {
        let Ok(bytes) = std::fs::read(file) else {
            continue;
        };
        let Ok(census) = serde_json::from_slice::<CensusFile>(&bytes) else {
            continue;
        };
        for p in census.params {
            let entry = params.entry((p.owner, p.key)).or_default();
            entry.0 += p.present;
            entry.1 += p.read;
        }
        for u in census.unhandled {
            *unhandled.entry((u.kind, u.detail)).or_default() += u.count;
        }
        let _ = std::fs::remove_file(file);
    }
    let merged = CensusFile {
        params: params
            .into_iter()
            .map(|((owner, key), (present, read))| CensusParam {
                owner,
                key,
                present,
                read,
            })
            .collect(),
        unhandled: unhandled
            .into_iter()
            .map(|((kind, detail), count)| CensusUnhandled {
                kind,
                detail,
                count,
            })
            .collect(),
    };
    std::fs::write(out, serde_json::to_vec(&merged)?)
}

/// `parity sweep --cards-file F --out-dir D [--games 10] [--chunk 10] [--jobs N]
///               [--max-turns 30] [--decision-budget 20000] [--game-secs 60]
///               [--kill-after-secs 30] [--census-out F]`
pub fn run_cli(args: &[String], data: &LoadedData) -> i32 {
    let (Some(cards_file), Some(out_dir)) =
        (option(args, "--cards-file"), option(args, "--out-dir"))
    else {
        eprintln!(
            "usage: parity sweep --cards-file FILE --out-dir DIR [--games 10] [--chunk 10] [--jobs N] \
             [--max-turns 30] [--decision-budget 20000] [--game-secs 60] [--kill-after-secs 30] \
             [--census-out FILE]"
        );
        return 2;
    };
    let names = match read_card_list(Path::new(cards_file)) {
        Ok(names) => names,
        Err(e) => {
            eprintln!("sweep: {e}");
            return 2;
        }
    };
    let out_dir = PathBuf::from(out_dir);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("sweep: {}: {e}", out_dir.display());
        return 2;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("sweep: cannot find my own executable: {e}");
            return 2;
        }
    };
    let games: u64 = number(args, "--games", 10);
    let seeds: Vec<u64> = (0..games).collect();
    let (decks, missing) = build_decks(data, &names, number(args, "--chunk", 10));
    let census_out = option(args, "--census-out").map(PathBuf::from);
    let jobs: usize = number(args, "--jobs", rayon::current_num_threads());
    eprintln!(
        "sweep: {} cards ({missing} not in the database), {} decks, {games} games each, {jobs} at a time",
        names.len(),
        decks.len()
    );

    let pool = match rayon::ThreadPoolBuilder::new().num_threads(jobs).build() {
        Ok(pool) => pool,
        Err(e) => {
            eprintln!("sweep: {e}");
            return 2;
        }
    };
    let done = AtomicUsize::new(0);
    let started = Instant::now();
    let outcomes: Vec<DeckOutcome> = pool.install(|| {
        decks
            .par_iter()
            .map(|job| {
                let outcome =
                    run_deck_in_children(&exe, job, &seeds, args, &out_dir, census_out.is_some());
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(25) || n == decks.len() {
                    eprintln!(
                        "sweep: {n}/{} decks, {:.0}s",
                        decks.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
                outcome
            })
            .collect()
    });

    let limits = limits(args);
    let mut games_tsv = String::from("deck\tseed\tstatus\tturns\tdecisions\tsecs\tdetail\n");
    let mut cards_tsv = String::from(
        "card\tdeck\tgames_ok\tgames_bad\toffered_per_game\tcast_per_game\tfailed_per_game\n",
    );
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut repro = String::new();
    let (mut never_offered, mut never_cast, mut mostly_failed) = (0usize, 0usize, 0usize);
    for (job, outcome) in decks.iter().zip(&outcomes) {
        let mut totals: BTreeMap<&str, CardStat> = BTreeMap::new();
        let (mut ok, mut bad) = (0u64, 0u64);
        for record in &outcome.records {
            *statuses.entry(record.status.clone()).or_default() += 1;
            games_tsv.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{:.2}\t{}\n",
                job.index,
                record.seed,
                record.status,
                record.turns,
                record.decisions,
                record.secs,
                record.detail.replace(['\t', '\n'], " ")
            ));
            if record.status == "OK" {
                ok += 1;
            } else {
                bad += 1;
                let class: String = format!("{} {}", record.status, record.detail)
                    .chars()
                    .take(120)
                    .collect();
                problems
                    .entry(class)
                    .or_default()
                    .push(format!("deck {} seed {}", job.index, record.seed));
                repro.push_str(&format!(
                    "# deck {} seed {}: {} {}\nparity sweep-deck --deck \"{}\" --seeds {} --max-turns {} --decision-budget {} --game-secs {}\n",
                    job.index,
                    record.seed,
                    record.status,
                    record.detail.replace('\n', " "),
                    format_inline(&job.spec),
                    record.seed,
                    limits.max_turns,
                    limits.decision_budget,
                    limits.wall_budget.as_secs(),
                ));
            }
            for (name, stat) in &record.cards {
                if let Some(card) = job.cards.iter().find(|c| *c == name) {
                    let total = totals.entry(card.as_str()).or_default();
                    total.offered += stat.offered;
                    total.cast += stat.cast;
                    total.failed += stat.failed;
                }
            }
        }
        let per_game = ok.max(1) as f64;
        for card in &job.cards {
            let stat = totals.get(card.as_str()).copied().unwrap_or_default();
            if ok > 0 && stat.offered == 0 {
                never_offered += 1;
            }
            if ok > 0 && stat.offered > 0 && stat.cast == 0 {
                never_cast += 1;
            }
            if stat.cast >= 4 && stat.failed * 2 > stat.cast {
                mostly_failed += 1;
            }
            cards_tsv.push_str(&format!(
                "{card}\t{}\t{ok}\t{bad}\t{:.2}\t{:.2}\t{:.2}\n",
                job.index,
                stat.offered as f64 / per_game,
                stat.cast as f64 / per_game,
                stat.failed as f64 / per_game
            ));
        }
    }

    let write = |name: &str, text: &str| {
        if let Err(e) = std::fs::write(out_dir.join(name), text) {
            eprintln!("sweep: {name}: {e}");
        }
    };
    write("games.tsv", &games_tsv);
    write("cards.tsv", &cards_tsv);
    write("repro.txt", &repro);
    if let Some(census_out) = &census_out {
        let files: Vec<PathBuf> = outcomes.iter().flat_map(|o| o.census.clone()).collect();
        if let Err(e) = merge_census(&files, census_out) {
            eprintln!("sweep: {}: {e}", census_out.display());
        }
    }

    println!(
        "games by status: {statuses:?} in {:.0}s",
        started.elapsed().as_secs_f64()
    );
    println!(
        "cards never offered: {never_offered}; offered and never cast: {never_cast}; \
         casts that mostly failed: {mostly_failed}"
    );
    let mut classes: Vec<_> = problems.into_iter().collect();
    classes.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    for (class, games) in classes.iter().take(number(args, "--top", 25)) {
        println!("  {:>4}  {class}  (first: {})", games.len(), games[0]);
    }
    println!(
        "wrote games.tsv, cards.tsv and repro.txt (a replay command per bad game) to {}",
        out_dir.display()
    );
    i32::from(statuses.keys().any(|s| s != "OK"))
}
