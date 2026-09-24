use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use forge_foundation::PhaseType;
use manabrew_engine::agent::PlayerAgent;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::game_rng::GameRng;
use manabrew_engine::game_runtime::GameRuntime;
use manabrew_engine::ids::{CardId, PlayerId};
use parity::runtime::PARITY_THREAD_STACK_SIZE;
use parity::utils::decks::build_deck_from_spec;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::data::GymData;
use crate::decision::{Action, ActionError, Decision};
use crate::encode::EncoderConfig;
use crate::learner_agent::{LearnerAgent, Link, Stalled};
use crate::random_agent::Opponent;
use crate::vec_env::VecEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_decisions: u32,
    pub max_turn_decisions: u32,
    pub max_turn_calls: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvConfig {
    pub max_turns: u32,
    pub limits: Limits,
    pub opponent: Opponent,
    pub encoder: Option<EncoderConfig>,
}

impl Default for EnvConfig {
    fn default() -> Self {
        EnvConfig {
            max_turns: 40,
            limits: Limits {
                max_decisions: 10_000,
                max_turn_decisions: 1_000,
                max_turn_calls: 5_000,
            },
            opponent: Opponent::Random { play_weight: 1 },
            encoder: Some(EncoderConfig::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSpec {
    pub seed: u64,
    pub decks: [usize; 2],
    pub learners: [bool; 2],
}

#[derive(Debug, Clone)]
pub enum Step {
    Decision(Decision),
    Done(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    GameOver,
    TurnCap,
    DecisionCap,
    TurnDecisionCap,
    Stalled,
    Aborted,
    EnginePanic(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub winner: Option<PlayerId>,
    pub reason: EndReason,
    pub turns: u32,
    pub phase: Option<PhaseType>,
    pub life: [i32; 2],
    pub decisions: u32,
    pub checksum: u64,
    pub blocked: Duration,
    pub encoding: Duration,
    pub observes: u32,
}

pub(crate) enum Message {
    Step(Step),
    Rejected(ActionError),
}

pub(crate) type Envelope = (usize, u64, Message);

pub(crate) struct Worker {
    actions: Option<Sender<Action>>,
    abort: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn spawn(
        env: usize,
        generation: u64,
        data: Arc<GymData>,
        config: EnvConfig,
        spec: GameSpec,
        events: Sender<Envelope>,
    ) -> Worker {
        let (actions, actions_rx) = mpsc::channel();
        let abort = Arc::new(AtomicBool::new(false));
        let thread_abort = Arc::clone(&abort);
        let handle = thread::Builder::new()
            .name(format!("gym-{env}"))
            .stack_size(PARITY_THREAD_STACK_SIZE)
            .spawn(move || {
                let link = Rc::new(Link::new(
                    env,
                    generation,
                    events,
                    actions_rx,
                    thread_abort,
                    config.limits,
                ));
                let outcome = catch_unwind(AssertUnwindSafe(|| play(&data, &config, &spec, &link)))
                    .unwrap_or_else(|panic| Outcome {
                        winner: None,
                        reason: if panic.is::<Stalled>() {
                            EndReason::Stalled
                        } else {
                            EndReason::EnginePanic(panic_message(panic))
                        },
                        turns: link.turn(),
                        phase: link.phase(),
                        life: [0, 0],
                        decisions: link.decisions(),
                        checksum: 0,
                        blocked: link.blocked(),
                        encoding: link.encoding(),
                        observes: link.observes(),
                    });
                link.send(Message::Step(Step::Done(outcome)));
            })
            .expect("spawn game thread");
        Worker {
            actions: Some(actions),
            abort,
            handle: Some(handle),
        }
    }

    pub(crate) fn send(&self, action: Action) -> bool {
        self.actions
            .as_ref()
            .is_some_and(|actions| actions.send(action).is_ok())
    }

    pub(crate) fn stop(&mut self) {
        self.abort.store(true, Ordering::Relaxed);
        self.actions = None;
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct SeededGameRng(StdRng);

impl GameRng for SeededGameRng {
    fn shuffle_cards(&mut self, cards: &mut [CardId]) {
        use rand::seq::SliceRandom;
        cards.shuffle(&mut self.0);
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        self.0.gen_range(0..bound)
    }
}

fn play(data: &GymData, config: &EnvConfig, spec: &GameSpec, link: &Rc<Link>) -> Outcome {
    let mut game = GameState::new(&["P1", "P2"], 20);
    for (p, &deck) in spec.decks.iter().enumerate() {
        build_deck_from_spec(
            &mut game,
            &data.loaded.db,
            PlayerId(p as u32),
            &data.decks[deck].cards,
            false,
        );
    }
    let mut gl = GameLoop::new(2);
    gl.set_provide_priority_action_space(false);
    gl.game_rng = Box::new(SeededGameRng(StdRng::seed_from_u64(
        spec.seed ^ 0x9e37_79b9_7f4a_7c15,
    )));
    gl.set_abort_signal(Arc::clone(&link.abort));
    data.loaded.share_token_data(&mut gl);
    let agents: Vec<Box<dyn PlayerAgent>> = (0..2u32)
        .map(|p| {
            let player = PlayerId(p);
            if spec.learners[p as usize] {
                Box::new(LearnerAgent::new(
                    player,
                    Rc::clone(link),
                    config.encoder,
                    spec.seed,
                )) as Box<dyn PlayerAgent>
            } else {
                config.opponent.build(spec.seed, player)
            }
        })
        .collect();
    let mut rt = GameRuntime::from_parts(game, gl, agents);
    let winner = rt.run(&mut StdRng::seed_from_u64(spec.seed), config.max_turns);
    let game = rt.game();
    let reason = link
        .stop_reason()
        .unwrap_or(if link.abort.load(Ordering::Relaxed) {
            EndReason::Aborted
        } else if game.game_over {
            EndReason::GameOver
        } else {
            EndReason::TurnCap
        });
    Outcome {
        winner,
        reason,
        turns: game.turn.turn_number,
        phase: Some(game.turn.phase),
        life: [game.player(PlayerId(0)).life, game.player(PlayerId(1)).life],
        decisions: link.decisions(),
        checksum: state_checksum(game),
        blocked: link.blocked(),
        encoding: link.encoding(),
        observes: link.observes(),
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

pub fn state_checksum(game: &GameState) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    game.winner.hash(&mut h);
    game.turn.turn_number.hash(&mut h);
    for player in &game.players {
        player.life.hash(&mut h);
        player.poison_counters.hash(&mut h);
    }
    for card in &game.cards {
        (card.zone as u8).hash(&mut h);
        card.controller.hash(&mut h);
        card.tapped.hash(&mut h);
        card.damage.hash(&mut h);
        card.card_name.hash(&mut h);
        for (kind, count) in &card.counters {
            std::mem::discriminant(kind).hash(&mut h);
            count.hash(&mut h);
        }
    }
    h.finish()
}

pub struct GameEnv {
    inner: VecEnv,
}

impl GameEnv {
    pub fn new(data: Arc<GymData>, config: EnvConfig) -> GameEnv {
        GameEnv {
            inner: VecEnv::new(data, config, 1),
        }
    }

    pub fn reset(&mut self, spec: GameSpec) -> Step {
        self.inner.reset(0, spec);
        match self.inner.recv(1).pop() {
            Some((_, Ok(step))) => step,
            _ => unreachable!("reset yields a step"),
        }
    }

    pub fn step(&mut self, action: Action) -> Result<Step, ActionError> {
        self.inner.send(0, action);
        self.inner.recv(1).pop().expect("step yields a result").1
    }
}
