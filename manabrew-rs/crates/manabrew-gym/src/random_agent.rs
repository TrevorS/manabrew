use manabot::BotResponder;
use manabrew_agent_interface::agent_impl::{PromptAgent, Responder};
use manabrew_agent_interface::game_log_event::GameLogEntryDto;
use manabrew_agent_interface::game_snapshot_event::GameSnapshotEventDto;
use manabrew_agent_interface::prompt::{
    AgentMessage, AgentPrompt, ClientToServerMessage, PromptInput,
};
use manabrew_engine::agent::{
    ActivatableAction, ManaAbilityOption, ManaCostAction, PlayerAgent, PriorityActionSpace,
    TargetChoice,
};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::decision::{Action, DecisionKind, LAND_OR_SPELL};
use crate::game_env::Limits;
use crate::learner_agent::Stalled;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opponent {
    Random { play_weight: u32 },
    SimpleAi,
}

impl Opponent {
    pub(crate) fn build(
        self,
        seed: u64,
        player: PlayerId,
        limits: &Limits,
    ) -> Box<dyn PlayerAgent> {
        match self {
            Opponent::Random { play_weight } => Box::new(RandomAgent::new(
                seed * 2 + u64::from(player.0) + 1,
                play_weight,
            )),
            Opponent::SimpleAi => Box::new(PromptAgent::new(
                player,
                String::new(),
                CappedResponder {
                    inner: BotResponder::default(),
                    prompts: 0,
                    max_prompts: limits.max_opponent_prompts,
                    payment_streak: 0,
                },
            )),
        }
    }
}

const MAX_PAYMENT_STREAK: u32 = 50;

struct CappedResponder<R> {
    inner: R,
    prompts: u32,
    max_prompts: u32,
    payment_streak: u32,
}

impl<R: Responder> Responder for CappedResponder<R> {
    fn respond(&mut self, prompt: AgentPrompt) -> ClientToServerMessage {
        self.prompts += 1;
        self.payment_streak = match prompt.input {
            PromptInput::PayManaCost(_) => self.payment_streak + 1,
            _ => 0,
        };
        if self.prompts > self.max_prompts || self.payment_streak > MAX_PAYMENT_STREAK {
            std::panic::resume_unwind(Box::new(Stalled));
        }
        self.inner.respond(prompt)
    }

    fn present(&mut self, message: &AgentMessage) {
        self.inner.present(message);
    }

    fn await_ack(&mut self) -> ClientToServerMessage {
        self.inner.await_ack()
    }

    fn send_log(&mut self, entry: GameLogEntryDto) {
        self.inner.send_log(entry);
    }

    fn send_snapshot(&mut self, snapshot: GameSnapshotEventDto) {
        self.inner.send_snapshot(snapshot);
    }
}

pub struct RandomAgent {
    rng: StdRng,
    weight: u32,
    streak: u32,
    last_replacement: Option<(Option<CardId>, String)>,
}

impl RandomAgent {
    pub fn new(seed: u64, play_weight: u32) -> Self {
        RandomAgent {
            rng: StdRng::seed_from_u64(seed),
            weight: play_weight,
            streak: 0,
            last_replacement: None,
        }
    }

    fn pick<T: Copy>(&mut self, v: &[T]) -> Option<T> {
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

pub struct RandomPolicy {
    rng: StdRng,
    weight: u32,
    streak: u32,
}

impl RandomPolicy {
    pub fn new(seed: u64, play_weight: u32) -> Self {
        RandomPolicy {
            rng: StdRng::seed_from_u64(seed),
            weight: play_weight,
            streak: 0,
        }
    }

    pub fn act(&mut self, kind: &DecisionKind) -> Action {
        match kind {
            DecisionKind::Priority { options } => {
                let n = options.len() as u32 - 1;
                let r = self.rng.gen_range(0..(n * self.weight + 1));
                if self.streak >= 30 || r == n * self.weight {
                    self.streak = 0;
                    return Action::Choose(0);
                }
                self.streak += 1;
                Action::Choose(1 + (r / self.weight) as usize)
            }
            DecisionKind::LandOrSpell => Action::Choose(self.rng.gen_range(0..LAND_OR_SPELL.len())),
            DecisionKind::AbilityToPlay { abilities } => {
                Action::Choose(self.rng.gen_range(0..abilities.len()))
            }
            DecisionKind::Target { candidates, .. } => {
                Action::Choose(self.rng.gen_range(0..candidates.len()))
            }
            DecisionKind::Attackers { legal, .. } => Action::Assign(self.assign(legal, None)),
            DecisionKind::Blockers { legal, max, .. } => Action::Assign(self.assign(legal, *max)),
            DecisionKind::Cards {
                cards, min, max, ..
            } => Action::Select(self.subset(cards.len(), *min, *max)),
            DecisionKind::Modes {
                descriptions,
                min,
                max,
                ..
            } => Action::Select(self.subset(descriptions.len(), *min, *max)),
            DecisionKind::Confirm { .. } => Action::Confirm(self.rng.gen_bool(0.5)),
            DecisionKind::Mulligan { .. } => Action::Confirm(true),
        }
    }

    fn assign(&mut self, legal: &[Vec<usize>], max: Option<usize>) -> Vec<Option<usize>> {
        let mut picked = 0;
        legal
            .iter()
            .map(|allowed| {
                if allowed.is_empty() || max.is_some_and(|m| picked >= m) || !self.rng.gen_bool(0.5)
                {
                    return None;
                }
                picked += 1;
                Some(allowed[self.rng.gen_range(0..allowed.len())])
            })
            .collect()
    }

    fn subset(&mut self, len: usize, min: usize, max: usize) -> Vec<usize> {
        let count = self.rng.gen_range(min..=max);
        rand::seq::index::sample(&mut self.rng, len, count).into_vec()
    }
}
