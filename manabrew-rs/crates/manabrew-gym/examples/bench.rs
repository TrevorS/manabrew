//! VecEnv throughput on the survey decks:
//! `bench <envs> <games> <learner_seats 0|1|2> <play_weight> [first_seed] [matchup|-1] [encode 0|1]`.
//! Learner seats are answered by `RandomPolicy` on the calling thread; with 0 learner seats every
//! seat is the in-thread `RandomAgent`, and the checksum matches the selfplay example's for the
//! same deck pair and seeds.
use std::time::{Duration, Instant};

use manabrew_gym::data::repo_root;
use manabrew_gym::encode::vocab::UNKNOWN;
use manabrew_gym::encode::{CARD_INTS, GLOBAL_FEATURES};
use manabrew_gym::{
    DecisionKind, EncoderConfig, EndReason, EnvConfig, GameSpec, GymData, Opponent, Outcome,
    RandomPolicy, Step, VecEnv,
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let envs: usize = args[1].parse().expect("envs");
    let games: u64 = args[2].parse().expect("games");
    let learner_seats: usize = args[3].parse().expect("learner seats");
    let weight: u32 = args[4].parse().expect("play_weight");
    let first_seed: u64 = args.get(5).map_or(0, |s| s.parse().expect("first_seed"));
    let matchup: i64 = args.get(6).map_or(-1, |s| s.parse().expect("matchup"));
    let encode = args.get(7).is_none_or(|s| s != "0");

    let (data, matchups) = GymData::survey(&repo_root()).expect("survey data");
    let data = std::sync::Arc::new(data);
    let config = EnvConfig {
        opponent: Opponent::Random {
            play_weight: weight,
        },
        encoder: encode.then(EncoderConfig::default),
        ..EnvConfig::default()
    };
    let spec = |game: u64| GameSpec {
        seed: first_seed + game,
        decks: matchups[if matchup < 0 {
            game as usize % matchups.len()
        } else {
            matchup as usize
        }],
        learners: [learner_seats > 0, learner_seats > 1],
    };

    let mut env = VecEnv::new(data, config, envs);
    let mut policies: Vec<RandomPolicy> = Vec::new();
    let mut running: Vec<u64> = Vec::new();
    let mut outcomes: Vec<Option<Outcome>> = vec![None; games as usize];
    let mut next = 0u64;
    let mut decisions = 0u64;
    let mut by_kind = [0u64; DecisionKind::COUNT];
    let (mut rows, mut unknown_rows, mut truncated) = (0u64, 0u64, 0u64);
    let mut policy_time = Duration::ZERO;
    let global = |name: &str| GLOBAL_FEATURES.iter().position(|&n| n == name).expect(name);
    let (dropped_cards, dropped_candidates) =
        (global("cards_dropped"), global("candidates_dropped"));
    let t0 = Instant::now();
    for e in 0..envs.min(games as usize) {
        env.reset(e, spec(next));
        policies.push(RandomPolicy::new(first_seed + next, weight));
        running.push(next);
        next += 1;
    }
    while env.pending() > 0 {
        for (e, result) in env.recv(1) {
            match result.expect("valid action") {
                Step::Decision(decision) => {
                    decisions += 1;
                    by_kind[decision.kind.id()] += 1;
                    if let Some(obs) = &decision.observation {
                        for (row, &shown) in obs.card_mask.iter().enumerate() {
                            rows += u64::from(shown);
                            unknown_rows +=
                                u64::from(shown && obs.card_ints[row * CARD_INTS.len()] == UNKNOWN);
                        }
                        truncated += u64::from(
                            obs.global[dropped_cards] > 0.0 || obs.global[dropped_candidates] > 0.0,
                        );
                    }
                    let started = Instant::now();
                    let action = policies[e].act(&decision.kind);
                    policy_time += started.elapsed();
                    env.send(e, action);
                }
                Step::Done(outcome) => {
                    outcomes[running[e] as usize] = Some(outcome);
                    if next < games {
                        env.reset(e, spec(next));
                        policies[e] = RandomPolicy::new(first_seed + next, weight);
                        running[e] = next;
                        next += 1;
                    }
                }
            }
        }
    }
    let wall = t0.elapsed().as_secs_f64();

    let outcomes: Vec<Outcome> = outcomes.into_iter().map(Option::unwrap).collect();
    let n = outcomes.len() as f64;
    let turns: u64 = outcomes.iter().map(|o| u64::from(o.turns)).sum();
    let finished = outcomes.iter().filter(|o| o.winner.is_some()).count() as f64;
    let blocked: Duration = outcomes.iter().map(|o| o.blocked).sum();
    let encoding: Duration = outcomes.iter().map(|o| o.encoding).sum();
    let observes: u64 = outcomes.iter().map(|o| u64::from(o.observes)).sum();
    let count = |r: &dyn Fn(&EndReason) -> bool| outcomes.iter().filter(|o| r(&o.reason)).count();
    let checksum = outcomes.iter().enumerate().fold(0u64, |acc, (i, r)| {
        let w = r.winner.map_or(9, |p| p.0 as u64);
        let v = w * 1_000_003
            + u64::from(r.turns) * 1_009
            + (r.life[0] + 1000) as u64 * 31
            + (r.life[1] + 1000) as u64;
        acc.wrapping_mul(1_000_000_007).wrapping_add(v ^ i as u64)
    });
    let per_decision = |d: Duration| d.as_secs_f64() * 1e6 / decisions.max(1) as f64;
    println!(
        "envs={envs} learners={learner_seats} encode={encode} wall={wall:.2}s games/s={:.2} decisions/s={:.0} decisions/game={:.0}",
        n / wall,
        decisions as f64 / wall,
        decisions as f64 / n
    );
    println!(
        "blocked_us/decision={:.1} encode_us/decision={:.1} observes/decision={:.1} policy_us/decision={:.2}",
        per_decision(blocked),
        per_decision(encoding),
        observes as f64 / decisions.max(1) as f64,
        per_decision(policy_time)
    );
    let selfplay_kinds = [0, 3, 4, 5].iter().map(|&k| by_kind[k]).sum::<u64>();
    println!(
        "selfplay_comparable_decisions/s={:.0} by_kind={by_kind:?} unknown_vocab_rows={:.3}% truncated_decisions={truncated}",
        selfplay_kinds as f64 / wall,
        unknown_rows as f64 * 100.0 / rows.max(1) as f64
    );
    println!(
        "finished={:.1}% mean_turns={:.1} turn_cap={} decision_caps={} panics={} checksum={checksum:016x}",
        finished * 100.0 / n,
        turns as f64 / n,
        count(&|r| *r == EndReason::TurnCap),
        count(&|r| matches!(r, EndReason::DecisionCap | EndReason::TurnDecisionCap)),
        count(&|r| matches!(r, EndReason::EnginePanic(_))),
    );
}
