//! Random self-play throughput: `selfplay <deck1> <deck2> <games> <max_turns> <play_weight> [first_seed]`.
//! Threads follow `RAYON_NUM_THREADS`. The checksum folds every game's winner, final turn and
//! life totals, so a change that should not alter play must leave it unchanged.
use std::time::Instant;

use manabrew_engine::agent::{
    ActivatableAction, ManaAbilityOption, ManaCostAction, PlayerAgent, PriorityActionSpace,
    TargetChoice,
};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::game_rng::GameRng;
use manabrew_engine::game_runtime::GameRuntime;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use parity::runner::{load_data, DEFAULT_DECKS_DIRS};
use parity::runtime::PARITY_THREAD_STACK_SIZE;
use parity::utils::decks::{build_deck_from_templates, resolve_deck_spec};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Default, Clone, Copy)]
struct Counts {
    priority_calls: u64,
    decisions: u64,
}

struct RandomAgent {
    rng: StdRng,
    weight: u32,
    streak: u32,
    last_replacement: Option<(Option<CardId>, String)>,
    counts: std::rc::Rc<std::cell::Cell<Counts>>,
}

impl RandomAgent {
    fn bump(&self, decision: bool) {
        let mut c = self.counts.get();
        c.priority_calls += u64::from(!decision);
        c.decisions += u64::from(decision);
        self.counts.set(c);
    }

    fn pick<T: Copy>(&mut self, v: &[T]) -> Option<T> {
        if v.len() > 1 {
            self.bump(true);
        }
        if v.is_empty() {
            None
        } else {
            Some(v[self.rng.gen_range(0..v.len())])
        }
    }
}

impl PlayerAgent for RandomAgent {
    fn mulligan_decision(&mut self, _p: PlayerId, _h: &[CardId], _m: u32) -> bool {
        true
    }

    fn choose_action(
        &mut self,
        _p: PlayerId,
        space: Option<&PriorityActionSpace>,
        request: &mut dyn FnMut() -> PriorityActionSpace,
    ) -> PlayerAction {
        self.bump(false);
        self.last_replacement = None;
        let owned;
        let s = match space {
            Some(s) => s,
            None => {
                owned = request();
                &owned
            }
        };
        let acts: Vec<&ActivatableAction> = s
            .activatable
            .iter()
            .filter(|a| !a.is_mana_ability)
            .collect();
        let n = s.playable.len() + acts.len();
        if n == 0 {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        self.bump(true);
        if self.streak >= 30 {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        let r = self.rng.gen_range(0..(n as u32 * self.weight + 1));
        if r == n as u32 * self.weight {
            self.streak = 0;
            return PlayerAction::PassPriority;
        }
        self.streak += 1;
        let i = (r / self.weight) as usize;
        if i < s.playable.len() {
            PlayerAction::CastSpell(s.playable[i])
        } else {
            let a = acts[i - s.playable.len()];
            PlayerAction::ActivateAbility(AbilityRef {
                card_id: a.card_id,
                ability_index: a.ability_index,
            })
        }
    }

    fn choose_attackers(
        &mut self,
        _p: PlayerId,
        avail: &[CardId],
        defs: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        self.bump(true);
        if defs.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        for &a in avail {
            if self.rng.gen_bool(0.5) {
                out.push((a, defs[self.rng.gen_range(0..defs.len())]));
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
        self.bump(true);
        let mut out = Vec::new();
        if attackers.is_empty() {
            return out;
        }
        for &b in blockers {
            if max.is_some_and(|m| out.len() >= m) {
                break;
            }
            if self.rng.gen_bool(0.5) {
                out.push((b, attackers[self.rng.gen_range(0..attackers.len())]));
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
        vp: &[PlayerId],
        vc: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        let n = vp.len() + vc.len();
        if n == 0 {
            return TargetChoice::None;
        }
        if n > 1 {
            self.bump(true);
        }
        let i = self.rng.gen_range(0..n);
        if i < vp.len() {
            TargetChoice::Player(vp[i])
        } else {
            TargetChoice::Card(vc[i - vp.len()])
        }
    }

    fn confirm_replacement_effect(
        &mut self,
        _p: PlayerId,
        question: &str,
        _d: &str,
        source: Option<CardId>,
    ) -> bool {
        let prompt = Some((source, question.to_string()));
        let repeated = self.last_replacement == prompt;
        self.last_replacement = prompt;
        !repeated
    }

    fn choose_land_or_spell(&mut self, _p: PlayerId) -> Option<bool> {
        Some(self.rng.gen_bool(0.5))
    }

    fn pay_mana_cost(
        &mut self,
        _p: PlayerId,
        _c: CardId,
        _n: &str,
        _mc: &str,
        _mcd: &str,
        _mcc: &str,
        _cp: bool,
        _ar: bool,
        _rs: &[CardId],
        _mo: &[ManaAbilityOption],
        _tl: &[CardId],
        _ul: &[CardId],
        _pool: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
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

struct GameResult {
    winner: Option<PlayerId>,
    turns: u32,
    lives: [i32; 2],
    counts: Counts,
    setup_ns: u128,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (d1, d2) = (&args[1], &args[2]);
    let games: u64 = args[3].parse().expect("games");
    let max_turns: u32 = args[4].parse().expect("max_turns");
    let weight: u32 = args[5].parse().expect("play_weight");
    let first_seed: u64 = args.get(6).map_or(0, |s| s.parse().expect("first_seed"));

    rayon::ThreadPoolBuilder::new()
        .stack_size(PARITY_THREAD_STACK_SIZE)
        .build_global()
        .expect("rayon pool");
    let data = load_data(None, false).expect("load_data");
    let s1 = resolve_deck_spec(d1, DEFAULT_DECKS_DIRS).expect("deck1");
    let s2 = resolve_deck_spec(d2, DEFAULT_DECKS_DIRS).expect("deck2");

    let t0 = Instant::now();
    let results: Vec<GameResult> = (first_seed..first_seed + games)
        .into_par_iter()
        .map(|seed| {
            let setup = Instant::now();
            let mut game = GameState::new(&["P1", "P2"], 20);
            let templates = &data.card_templates;
            build_deck_from_templates(&mut game, templates, &data.db, PlayerId(0), &s1, false);
            build_deck_from_templates(&mut game, templates, &data.db, PlayerId(1), &s2, false);
            let mut gl = GameLoop::new(2);
            gl.set_provide_priority_action_space(false);
            gl.game_rng = Box::new(SeededGameRng(StdRng::seed_from_u64(
                seed ^ 0x9e37_79b9_7f4a_7c15,
            )));
            data.share_token_data(&mut gl);
            let counts = std::rc::Rc::new(std::cell::Cell::new(Counts::default()));
            let agents: Vec<Box<dyn PlayerAgent>> = (0..2u64)
                .map(|p| {
                    Box::new(RandomAgent {
                        rng: StdRng::seed_from_u64(seed * 2 + p + 1),
                        weight,
                        streak: 0,
                        last_replacement: None,
                        counts: std::rc::Rc::clone(&counts),
                    }) as Box<dyn PlayerAgent>
                })
                .collect();
            let mut rt = GameRuntime::from_parts(game, gl, agents);
            let setup_ns = setup.elapsed().as_nanos();
            let mut rng = StdRng::seed_from_u64(seed);
            let winner = rt.run(&mut rng, max_turns);
            let game = rt.game();
            GameResult {
                winner,
                turns: game.turn.turn_number,
                lives: [game.player(PlayerId(0)).life, game.player(PlayerId(1)).life],
                counts: counts.get(),
                setup_ns,
            }
        })
        .collect();
    let wall = t0.elapsed().as_secs_f64();

    let n = results.len() as f64;
    let decisions: u64 = results.iter().map(|r| r.counts.decisions).sum();
    let priority: u64 = results.iter().map(|r| r.counts.priority_calls).sum();
    let finished = results.iter().filter(|r| r.winner.is_some()).count() as f64;
    let turns: u64 = results.iter().map(|r| u64::from(r.turns)).sum();
    let setup_ms = results.iter().map(|r| r.setup_ns).sum::<u128>() as f64 / 1e6 / n;
    let checksum = results.iter().enumerate().fold(0u64, |acc, (i, r)| {
        let w = r.winner.map_or(9, |p| p.0 as u64);
        let v = w * 1_000_003
            + u64::from(r.turns) * 1_009
            + (r.lives[0] + 1000) as u64 * 31
            + (r.lives[1] + 1000) as u64;
        acc.wrapping_mul(1_000_000_007).wrapping_add(v ^ i as u64)
    });
    println!(
        "wall={wall:.2}s games/s={:.2} decisions/s={:.0} priority_calls/s={:.0} setup_ms/game={setup_ms:.2}",
        n / wall,
        decisions as f64 / wall,
        priority as f64 / wall
    );
    println!(
        "finished={:.1}% mean_turns={:.1} checksum={checksum:016x}",
        finished * 100.0 / n,
        turns as f64 / n
    );
}
