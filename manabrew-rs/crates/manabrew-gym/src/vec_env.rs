use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use crate::data::GymData;
use crate::decision::{Action, ActionError};
use crate::game_env::{EnvConfig, Envelope, GameSpec, Message, Step, Worker};

pub type StepResult = (usize, Result<Step, ActionError>);

pub struct VecEnv {
    data: Arc<GymData>,
    config: EnvConfig,
    workers: Vec<Option<Worker>>,
    generations: Vec<u64>,
    awaiting: Vec<bool>,
    done: Vec<bool>,
    events_tx: Sender<Envelope>,
    events_rx: Receiver<Envelope>,
}

impl VecEnv {
    pub fn new(data: Arc<GymData>, config: EnvConfig, num_envs: usize) -> VecEnv {
        let (events_tx, events_rx) = mpsc::channel();
        VecEnv {
            data,
            config,
            workers: (0..num_envs).map(|_| None).collect(),
            generations: vec![0; num_envs],
            awaiting: vec![false; num_envs],
            done: vec![true; num_envs],
            events_tx,
            events_rx,
        }
    }

    pub fn num_envs(&self) -> usize {
        self.workers.len()
    }

    pub fn config(&self) -> &EnvConfig {
        &self.config
    }

    pub fn data(&self) -> &Arc<GymData> {
        &self.data
    }

    pub fn reset(&mut self, env: usize, spec: GameSpec) {
        self.workers[env] = None;
        self.generations[env] += 1;
        self.workers[env] = Some(Worker::spawn(
            env,
            self.generations[env],
            Arc::clone(&self.data),
            self.config,
            spec,
            self.events_tx.clone(),
        ));
        self.awaiting[env] = true;
        self.done[env] = false;
    }

    pub fn send(&mut self, env: usize, action: Action) {
        assert!(
            !self.awaiting[env] && !self.done[env],
            "env {env} has no open decision"
        );
        let sent = self.workers[env]
            .as_ref()
            .is_some_and(|worker| worker.send(action));
        assert!(sent, "env {env} game thread is gone");
        self.awaiting[env] = true;
    }

    pub fn recv(&mut self, min: usize) -> Vec<StepResult> {
        let min = min.min(self.pending());
        let mut out = Vec::new();
        while out.len() < min {
            let envelope = self.events_rx.recv().expect("events channel open");
            self.accept(envelope, &mut out);
        }
        while let Ok(envelope) = self.events_rx.try_recv() {
            self.accept(envelope, &mut out);
        }
        out
    }

    pub fn pending(&self) -> usize {
        self.awaiting.iter().filter(|&&a| a).count()
    }

    pub fn reset_all(&mut self, specs: &[GameSpec]) -> Vec<StepResult> {
        for (env, &spec) in specs.iter().enumerate() {
            self.reset(env, spec);
        }
        self.collect(specs.len())
    }

    pub fn step_all(&mut self, actions: Vec<(usize, Action)>) -> Vec<StepResult> {
        let n = actions.len();
        for (env, action) in actions {
            self.send(env, action);
        }
        self.collect(n)
    }

    fn collect(&mut self, n: usize) -> Vec<StepResult> {
        let mut out = self.recv(n);
        out.sort_by_key(|(env, _)| *env);
        out
    }

    fn accept(&mut self, (env, generation, message): Envelope, out: &mut Vec<StepResult>) {
        if generation != self.generations[env] {
            return;
        }
        self.awaiting[env] = false;
        match message {
            Message::Step(step) => {
                if matches!(step, Step::Done(_)) {
                    self.done[env] = true;
                }
                out.push((env, Ok(step)));
            }
            Message::Rejected(error) => out.push((env, Err(error))),
        }
    }
}

impl Drop for VecEnv {
    fn drop(&mut self) {
        for worker in self.workers.iter_mut().flatten() {
            worker.stop();
        }
    }
}
