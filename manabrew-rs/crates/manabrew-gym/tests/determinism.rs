use std::sync::{Arc, OnceLock};

use manabrew_gym::data::repo_root;
use manabrew_gym::encode::CARD_FEATURES;
use manabrew_gym::{
    Action, Decision, DecisionKind, EndReason, EnvConfig, GameEnv, GameSpec, GymData, Limits,
    Observation, Opponent, Outcome, RandomPolicy, Step, VecEnv,
};

fn survey() -> &'static (Arc<GymData>, Vec<[usize; 2]>) {
    static DATA: OnceLock<(Arc<GymData>, Vec<[usize; 2]>)> = OnceLock::new();
    DATA.get_or_init(|| {
        let (data, matchups) = GymData::survey(&repo_root()).expect("survey data");
        (Arc::new(data), matchups)
    })
}

fn config() -> EnvConfig {
    EnvConfig {
        max_turns: 10,
        ..EnvConfig::default()
    }
}

fn specs() -> Vec<GameSpec> {
    let (_, matchups) = survey();
    (0..4u64)
        .map(|i| GameSpec {
            seed: 11 + i,
            decks: matchups[(i as usize * 7) % matchups.len()],
            learners: [true, i % 2 == 0],
        })
        .collect()
}

fn feature(name: &str) -> usize {
    CARD_FEATURES.iter().position(|&n| n == name).expect(name)
}

struct Trace {
    actions: Vec<Action>,
    observations: Vec<Observation>,
    outcome: Outcome,
}

fn play(spec: GameSpec, mut answer: impl FnMut(&Decision) -> Action) -> Trace {
    let mut env = GameEnv::new(Arc::clone(&survey().0), config());
    let mut actions = Vec::new();
    let mut observations = Vec::new();
    let mut step = env.reset(spec);
    loop {
        match step {
            Step::Decision(decision) => {
                let action = answer(&decision);
                observations.push(*decision.observation.expect("observation"));
                actions.push(action.clone());
                step = env.step(action).expect("valid action");
            }
            Step::Done(outcome) => {
                return Trace {
                    actions,
                    observations,
                    outcome,
                }
            }
        }
    }
}

fn same_game(a: &Outcome, b: &Outcome) {
    assert_eq!(
        (
            a.winner,
            &a.reason,
            a.turns,
            a.life,
            a.decisions,
            a.checksum,
            a.observes
        ),
        (
            b.winner,
            &b.reason,
            b.turns,
            b.life,
            b.decisions,
            b.checksum,
            b.observes
        )
    );
}

#[test]
fn same_seeds_and_actions_replay_identically_on_one_and_many_envs() {
    let specs = specs();
    let shapes = config().encoder.expect("encoder").shapes();
    let traces: Vec<Trace> = specs
        .iter()
        .map(|&spec| {
            let mut policy = RandomPolicy::new(spec.seed, 2);
            play(spec, |d| policy.act(&d.kind))
        })
        .collect();
    assert!(traces.iter().all(|t| !t.actions.is_empty()));
    for obs in traces.iter().flat_map(|t| &t.observations) {
        let lens = [
            obs.card_ints.len(),
            obs.card_floats.len(),
            obs.card_mask.len(),
            obs.global.len(),
            obs.stack_ints.len(),
            obs.stack_floats.len(),
            obs.stack_mask.len(),
            obs.candidate_ints.len(),
            obs.candidate_floats.len(),
            obs.candidate_mask.len(),
            obs.decision.len(),
        ];
        for (len, (name, [rows, cols])) in lens.into_iter().zip(shapes) {
            assert_eq!(len, rows * cols, "{name}");
        }
        for row in (0..obs.card_mask.len()).filter(|&r| obs.card_mask[r]) {
            let features = &obs.card_floats[row * CARD_FEATURES.len()..];
            assert!(
                features[feature("zone_hand")] == 0.0 || features[feature("owner_self")] == 1.0
            );
        }
        let shown = obs.candidate_mask.iter().filter(|&&m| m).count();
        assert_eq!(
            shown,
            (obs.decision[3] as usize).min(obs.candidate_mask.len())
        );
    }

    for (&spec, trace) in specs.iter().zip(&traces) {
        let mut log = trace.actions.iter();
        let replay = play(spec, |_| log.next().expect("logged action").clone());
        same_game(&replay.outcome, &trace.outcome);
        assert_eq!(replay.actions, trace.actions);
        assert!(replay.observations == trace.observations);
    }

    let mut env = VecEnv::new(Arc::clone(&survey().0), config(), specs.len());
    let mut cursor = vec![0; specs.len()];
    let mut outcomes: Vec<Option<Outcome>> = vec![None; specs.len()];
    let mut results = env.reset_all(&specs);
    loop {
        let mut actions = Vec::new();
        for (e, result) in results {
            match result.expect("valid action") {
                Step::Decision(decision) => {
                    assert!(
                        decision.observation.as_deref() == Some(&traces[e].observations[cursor[e]])
                    );
                    actions.push((e, traces[e].actions[cursor[e]].clone()));
                    cursor[e] += 1;
                }
                Step::Done(outcome) => outcomes[e] = Some(outcome),
            }
        }
        if actions.is_empty() {
            break;
        }
        results = env.step_all(actions);
    }
    for (outcome, trace) in outcomes.iter().zip(&traces) {
        same_game(outcome.as_ref().expect("finished"), &trace.outcome);
    }
}

#[test]
fn caps_and_stalls_end_games_as_draws_and_bad_actions_are_rejected() {
    let spec = specs()[0];
    let config = EnvConfig {
        limits: Limits {
            max_decisions: 10_000,
            max_turn_decisions: 3,
            max_turn_calls: 5_000,
            max_opponent_prompts: 20_000,
        },
        ..config()
    };
    let mut env = GameEnv::new(Arc::clone(&survey().0), config);
    let mut policy = RandomPolicy::new(spec.seed, 2);
    let mut step = env.reset(spec);
    assert!(matches!(&step, Step::Decision(d) if matches!(d.kind, DecisionKind::Mulligan { .. })));
    let mut rejected = false;
    let outcome = loop {
        match step {
            Step::Decision(decision) => {
                if !rejected {
                    assert!(env.step(Action::Choose(usize::MAX)).is_err());
                    rejected = true;
                }
                step = env.step(policy.act(&decision.kind)).expect("valid action");
            }
            Step::Done(outcome) => break outcome,
        }
    };
    assert!(rejected);
    assert_eq!(outcome.reason, EndReason::TurnDecisionCap);
    assert_eq!(outcome.winner, None);

    let stalling = EnvConfig {
        limits: Limits {
            max_turn_calls: 20,
            ..config.limits
        },
        ..config
    };
    let mut env = GameEnv::new(Arc::clone(&survey().0), stalling);
    let mut step = env.reset(spec);
    let outcome = loop {
        match step {
            Step::Decision(decision) => {
                step = env.step(policy.act(&decision.kind)).expect("valid action")
            }
            Step::Done(outcome) => break outcome,
        }
    };
    assert_eq!(outcome.reason, EndReason::Stalled);
    assert!(outcome.phase.is_some());

    let mut env = GameEnv::new(Arc::clone(&survey().0), config);
    assert!(matches!(env.reset(spec), Step::Decision(_)));
    drop(env);
}

#[test]
fn a_learner_plays_a_game_against_the_simple_ai() {
    let spec = GameSpec {
        learners: [true, false],
        ..specs()[1]
    };
    let config = EnvConfig {
        opponent: Opponent::SimpleAi,
        ..config()
    };
    let mut env = GameEnv::new(Arc::clone(&survey().0), config);
    let mut policy = RandomPolicy::new(spec.seed, 2);
    let mut step = env.reset(spec);
    let outcome = loop {
        match step {
            Step::Decision(decision) => {
                step = env.step(policy.act(&decision.kind)).expect("valid action")
            }
            Step::Done(outcome) => break outcome,
        }
    };
    assert!(matches!(
        outcome.reason,
        EndReason::GameOver | EndReason::TurnCap
    ));
}
