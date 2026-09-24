use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    CardOrStackTarget, ManaAbilityOption, ManaCostAction, PlayerAgent, PriorityActionSpace,
    TargetChoice,
};
use manabrew_engine::combat::{self, combat_util, DefenderId};
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::decision::{
    AbilityOption, Action, CardPurpose, ConfirmPurpose, Decision, DecisionKind, PriorityOption,
    TargetOption, LAND_OR_SPELL,
};
use crate::encode::{Encoder, EncoderConfig};
use crate::game_env::{EndReason, Envelope, Limits, Message, Step};

pub(crate) struct Link {
    env: usize,
    generation: u64,
    events: Sender<Envelope>,
    actions: Receiver<Action>,
    pub(crate) abort: Arc<AtomicBool>,
    limits: Limits,
    state: RefCell<LinkState>,
}

#[derive(Default)]
struct LinkState {
    decisions: u32,
    turn: u32,
    turn_decisions: u32,
    stop: Option<EndReason>,
    blocked: Duration,
    encoding: Duration,
    observes: u32,
}

impl Link {
    pub(crate) fn new(
        env: usize,
        generation: u64,
        events: Sender<Envelope>,
        actions: Receiver<Action>,
        abort: Arc<AtomicBool>,
        limits: Limits,
    ) -> Link {
        Link {
            env,
            generation,
            events,
            actions,
            abort,
            limits,
            state: RefCell::default(),
        }
    }

    pub(crate) fn send(&self, message: Message) -> bool {
        self.events
            .send((self.env, self.generation, message))
            .is_ok()
    }

    pub(crate) fn stop_reason(&self) -> Option<EndReason> {
        self.state.borrow().stop.clone()
    }

    pub(crate) fn decisions(&self) -> u32 {
        self.state.borrow().decisions
    }

    pub(crate) fn blocked(&self) -> Duration {
        self.state.borrow().blocked
    }

    pub(crate) fn encoding(&self) -> Duration {
        self.state.borrow().encoding
    }

    pub(crate) fn observes(&self) -> u32 {
        self.state.borrow().observes
    }

    fn add_encoding(&self, started: Instant) {
        self.state.borrow_mut().encoding += started.elapsed();
    }

    fn add_observe(&self, started: Instant) {
        let mut state = self.state.borrow_mut();
        state.encoding += started.elapsed();
        state.observes += 1;
    }

    fn stopped(&self) -> bool {
        self.state.borrow().stop.is_some()
    }

    fn stop(&self, reason: EndReason) {
        self.state.borrow_mut().stop.get_or_insert(reason);
        self.abort.store(true, Ordering::Relaxed);
    }

    fn observe_turn(&self, turn: u32) {
        let mut state = self.state.borrow_mut();
        if state.turn != turn {
            state.turn = turn;
            state.turn_decisions = 0;
        }
    }

    fn admit(&self) -> bool {
        if self.stopped() {
            return false;
        }
        if self.abort.load(Ordering::Relaxed) {
            self.stop(EndReason::Aborted);
            return false;
        }
        let reason = {
            let state = self.state.borrow();
            if state.decisions >= self.limits.max_decisions {
                Some(EndReason::DecisionCap)
            } else if state.turn_decisions >= self.limits.max_turn_decisions {
                Some(EndReason::TurnDecisionCap)
            } else {
                None
            }
        };
        if let Some(reason) = reason {
            self.stop(reason);
            return false;
        }
        let mut state = self.state.borrow_mut();
        state.decisions += 1;
        state.turn_decisions += 1;
        true
    }

    fn exchange(&self, decision: Decision) -> Option<Action> {
        let kind = decision.kind.clone();
        let started = Instant::now();
        let mut answer = None;
        if self.send(Message::Step(Step::Decision(decision))) {
            while let Ok(action) = self.actions.recv() {
                match kind.validate(&action) {
                    Ok(()) => {
                        answer = Some(action);
                        break;
                    }
                    Err(error) => {
                        if !self.send(Message::Rejected(error)) {
                            break;
                        }
                    }
                }
            }
        }
        self.state.borrow_mut().blocked += started.elapsed();
        if answer.is_none() {
            self.stop(EndReason::Aborted);
        }
        answer
    }
}

#[derive(Default)]
struct CombatOptions {
    attack: Vec<(CardId, Vec<DefenderId>)>,
    block: Vec<(CardId, Vec<CardId>)>,
}

impl CombatOptions {
    fn observe(&mut self, game: &GameState, player: PlayerId) {
        self.attack.clear();
        self.block.clear();
        let active = game.active_player();
        match game.turn.phase {
            PhaseType::CombatDeclareAttackers if active == player => {
                let defenders = combat::get_possible_defenders(game, player);
                for attacker in combat::get_available_attackers(game, player) {
                    let allowed = defenders
                        .iter()
                        .copied()
                        .filter(|&d| combat_util::can_attack_defender(game, attacker, d))
                        .collect();
                    self.attack.push((attacker, allowed));
                }
            }
            PhaseType::CombatDeclareBlockers if active != player => {
                let attackers: Vec<CardId> = game
                    .cards_in_zone(ZoneType::Battlefield, active)
                    .iter()
                    .copied()
                    .filter(|&c| game.card(c).attacking_player == Some(player))
                    .collect();
                for blocker in combat::get_available_blockers(game, player) {
                    let allowed = attackers
                        .iter()
                        .copied()
                        .filter(|&a| combat::can_creature_block(game, blocker, a))
                        .collect();
                    self.block.push((blocker, allowed));
                }
            }
            _ => {}
        }
    }

    fn attack_legal(&self, attackers: &[CardId], defenders: &[DefenderId]) -> Vec<Vec<usize>> {
        attackers
            .iter()
            .map(|a| {
                let allowed = self
                    .attack
                    .iter()
                    .find(|(c, _)| c == a)
                    .map(|(_, allowed)| allowed)
                    .filter(|allowed| !allowed.is_empty());
                (0..defenders.len())
                    .filter(|&d| allowed.is_none_or(|allowed| allowed.contains(&defenders[d])))
                    .collect()
            })
            .collect()
    }

    fn block_legal(&self, blockers: &[CardId], attackers: &[CardId]) -> Vec<Vec<usize>> {
        blockers
            .iter()
            .map(|b| {
                let allowed = self.block.iter().find(|(c, _)| c == b).map(|(_, a)| a);
                (0..attackers.len())
                    .filter(|&a| allowed.is_none_or(|allowed| allowed.contains(&attackers[a])))
                    .collect()
            })
            .collect()
    }
}

pub(crate) struct LearnerAgent {
    player: PlayerId,
    link: Rc<Link>,
    encoder: Option<Encoder>,
    turn: u32,
    combat: CombatOptions,
    last_replacement: Option<(Option<CardId>, String)>,
    rng: StdRng,
}

impl LearnerAgent {
    pub(crate) fn new(
        player: PlayerId,
        link: Rc<Link>,
        encoder: Option<EncoderConfig>,
        seed: u64,
    ) -> LearnerAgent {
        LearnerAgent {
            player,
            link,
            encoder: encoder.map(Encoder::new),
            turn: 0,
            combat: CombatOptions::default(),
            last_replacement: None,
            rng: StdRng::seed_from_u64(seed * 2 + u64::from(player.0) + 1),
        }
    }

    fn ask(&mut self, kind: DecisionKind) -> Option<Action> {
        if !self.link.admit() {
            return None;
        }
        let started = Instant::now();
        let observation = self
            .encoder
            .as_ref()
            .map(|e| Box::new(e.observation(&kind)));
        self.link.add_encoding(started);
        self.link.exchange(Decision {
            player: self.player,
            turn: self.turn,
            kind,
            observation,
        })
    }

    fn choose_target(
        &mut self,
        candidates: Vec<TargetOption>,
        source: Option<CardId>,
    ) -> Option<TargetOption> {
        if candidates.len() < 2 {
            return candidates.first().copied();
        }
        let kind = DecisionKind::Target {
            candidates: candidates.clone(),
            source,
        };
        match self.ask(kind) {
            Some(Action::Choose(i)) => Some(candidates[i]),
            _ => Some(candidates[0]),
        }
    }

    fn choose_cards(
        &mut self,
        purpose: CardPurpose,
        cards: &[CardId],
        min: usize,
        max: usize,
        source: Option<CardId>,
    ) -> Vec<CardId> {
        let max = max.min(cards.len());
        let min = min.min(max);
        if max == 0 || min == cards.len() {
            return cards[..min].to_vec();
        }
        let kind = DecisionKind::Cards {
            purpose,
            cards: cards.to_vec(),
            min,
            max,
            source,
        };
        match self.ask(kind) {
            Some(Action::Select(picks)) => picks.iter().map(|&i| cards[i]).collect(),
            _ => cards[..min].to_vec(),
        }
    }

    fn confirm(
        &mut self,
        purpose: ConfirmPurpose,
        text: &str,
        source: Option<CardId>,
        default: bool,
    ) -> bool {
        let kind = DecisionKind::Confirm {
            purpose,
            text: text.to_string(),
            source,
        };
        match self.ask(kind) {
            Some(Action::Confirm(yes)) => yes,
            _ => default,
        }
    }
}

impl PlayerAgent for LearnerAgent {
    fn snapshot_state(&mut self, game: &GameState, mana_pools: &[ManaPool]) {
        self.turn = game.turn.turn_number;
        self.link.observe_turn(self.turn);
        if self.link.stopped() {
            return;
        }
        if let Some(encoder) = &mut self.encoder {
            let started = Instant::now();
            encoder.observe(game, mana_pools, self.player);
            self.link.add_observe(started);
        }
        self.combat.observe(game, self.player);
    }

    fn mulligan_decision(&mut self, _player: PlayerId, _hand: &[CardId], _count: u32) -> bool {
        true
    }

    fn choose_action(
        &mut self,
        _player: PlayerId,
        space: Option<&PriorityActionSpace>,
        request: &mut dyn FnMut() -> PriorityActionSpace,
    ) -> PlayerAction {
        self.last_replacement = None;
        let owned;
        let space = match space {
            Some(s) => s,
            None => {
                owned = request();
                &owned
            }
        };
        let mut options = vec![PriorityOption::Pass];
        options.extend(space.playable.iter().map(|&p| PriorityOption::Play(p)));
        options.extend(
            space
                .activatable
                .iter()
                .filter(|a| !a.is_mana_ability)
                .map(|a| {
                    PriorityOption::Activate(AbilityRef {
                        card_id: a.card_id,
                        ability_index: a.ability_index,
                    })
                }),
        );
        if options.len() == 1 {
            return PlayerAction::PassPriority;
        }
        let kind = DecisionKind::Priority {
            options: options.clone(),
        };
        match self.ask(kind) {
            Some(Action::Choose(i)) => options[i].to_player_action(),
            _ => PlayerAction::PassPriority,
        }
    }

    fn choose_land_or_spell(&mut self, _player: PlayerId) -> Option<bool> {
        match self.ask(DecisionKind::LandOrSpell) {
            Some(Action::Choose(i)) => LAND_OR_SPELL[i],
            _ => None,
        }
    }

    fn get_ability_to_play(
        &mut self,
        _player: PlayerId,
        abilities: &[SpellAbility],
    ) -> Option<usize> {
        if abilities.len() < 2 {
            return (!abilities.is_empty()).then_some(0);
        }
        let kind = DecisionKind::AbilityToPlay {
            abilities: abilities
                .iter()
                .map(|sa| AbilityOption {
                    source: sa.source,
                    description: sa.ir.spell_description_text.clone().unwrap_or_default(),
                })
                .collect(),
        };
        match self.ask(kind) {
            Some(Action::Choose(i)) => Some(i),
            _ => Some(0),
        }
    }

    fn choose_attackers(
        &mut self,
        _player: PlayerId,
        available: &[CardId],
        defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        let legal = self.combat.attack_legal(available, defenders);
        if legal.iter().all(Vec::is_empty) {
            return Vec::new();
        }
        let kind = DecisionKind::Attackers {
            attackers: available.to_vec(),
            defenders: defenders.to_vec(),
            legal,
        };
        match self.ask(kind) {
            Some(Action::Assign(slots)) => available
                .iter()
                .zip(slots)
                .filter_map(|(&a, slot)| slot.map(|d| (a, defenders[d])))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn choose_blockers(
        &mut self,
        _player: PlayerId,
        attackers: &[CardId],
        available: &[CardId],
        max: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        let legal = self.combat.block_legal(available, attackers);
        if max == Some(0) || legal.iter().all(Vec::is_empty) {
            return Vec::new();
        }
        let kind = DecisionKind::Blockers {
            attackers: attackers.to_vec(),
            blockers: available.to_vec(),
            legal,
            max,
        };
        match self.ask(kind) {
            Some(Action::Assign(slots)) => available
                .iter()
                .zip(slots)
                .filter_map(|(&b, slot)| slot.map(|a| (b, attackers[a])))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn choose_targets_for(
        &mut self,
        sa: &mut SpellAbility,
        game: &GameState,
        mana_pools: &[ManaPool],
    ) -> bool {
        manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, mana_pools)
    }

    fn choose_target_player(
        &mut self,
        _player: PlayerId,
        valid: &[PlayerId],
        sa: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        let candidates = valid.iter().map(|&p| TargetOption::Player(p)).collect();
        match self.choose_target(candidates, sa.and_then(|sa| sa.source)) {
            Some(TargetOption::Player(p)) => Some(p),
            _ => None,
        }
    }

    fn choose_target_card(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        let candidates = valid.iter().map(|&c| TargetOption::Card(c)).collect();
        match self.choose_target(candidates, sa.and_then(|sa| sa.source)) {
            Some(TargetOption::Card(c)) => Some(c),
            _ => None,
        }
    }

    fn choose_target_any(
        &mut self,
        _player: PlayerId,
        valid_players: &[PlayerId],
        valid_cards: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        let candidates = valid_players
            .iter()
            .map(|&p| TargetOption::Player(p))
            .chain(valid_cards.iter().map(|&c| TargetOption::Card(c)))
            .collect();
        match self.choose_target(candidates, sa.and_then(|sa| sa.source)) {
            Some(TargetOption::Player(p)) => TargetChoice::Player(p),
            Some(TargetOption::Card(c)) => TargetChoice::Card(c),
            _ => TargetChoice::None,
        }
    }

    fn choose_target_card_or_stack(
        &mut self,
        _player: PlayerId,
        cards: &[CardId],
        stack: &[(u32, CardId)],
        sa: Option<&SpellAbility>,
    ) -> CardOrStackTarget {
        let candidates = cards
            .iter()
            .map(|&c| TargetOption::Card(c))
            .chain(stack.iter().map(|&(id, _)| TargetOption::Stack(id)))
            .collect();
        match self.choose_target(candidates, sa.and_then(|sa| sa.source)) {
            Some(TargetOption::Card(c)) => CardOrStackTarget::Card(c),
            Some(TargetOption::Stack(id)) => CardOrStackTarget::Stack(id),
            _ => CardOrStackTarget::None,
        }
    }

    fn choose_target_spell(
        &mut self,
        _player: PlayerId,
        valid: &[u32],
        source: Option<CardId>,
    ) -> Option<u32> {
        let candidates = valid.iter().map(|&id| TargetOption::Stack(id)).collect();
        match self.choose_target(candidates, source) {
            Some(TargetOption::Stack(id)) => Some(id),
            _ => None,
        }
    }

    fn choose_target_cards(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
        sa: &SpellAbility,
    ) -> Vec<CardId> {
        self.choose_cards(CardPurpose::Target, valid, min, max, sa.source)
    }

    fn choose_cards_for_effect(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        self.choose_cards(CardPurpose::Effect, valid, min, max, None)
    }

    fn choose_dig(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        valid: &[CardId],
        max: usize,
        optional: bool,
    ) -> Vec<CardId> {
        let min = if optional { 0 } else { max };
        self.choose_cards(CardPurpose::Dig, valid, min, max, None)
    }

    fn choose_sacrifice(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        source: Option<CardId>,
    ) -> Option<CardId> {
        self.choose_cards(CardPurpose::Sacrifice, valid, 1, 1, source)
            .first()
            .copied()
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _player: PlayerId,
        min: usize,
        max: usize,
        valid: &[CardId],
        source: Option<CardId>,
    ) -> Vec<CardId> {
        self.choose_cards(CardPurpose::Sacrifice, valid, min, max, source)
    }

    fn choose_discard(&mut self, _player: PlayerId, hand: &[CardId], num: usize) -> Vec<CardId> {
        self.choose_cards(CardPurpose::Discard, hand, num, num, None)
    }

    fn choose_discard_any_number(
        &mut self,
        _player: PlayerId,
        hand: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        self.choose_cards(CardPurpose::Discard, hand, min, max, None)
    }

    fn choose_random_discard(
        &mut self,
        _player: PlayerId,
        hand: &[CardId],
        num: usize,
    ) -> Vec<CardId> {
        rand::seq::index::sample(&mut self.rng, hand.len(), num.min(hand.len()))
            .into_iter()
            .map(|i| hand[i])
            .collect()
    }

    fn choose_tap_type_for_cost(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min_total_power: i32,
        card_powers: &[(CardId, i32)],
        _card_sort_powers: &[(CardId, i32)],
        _sa: Option<&SpellAbility>,
    ) -> Vec<CardId> {
        let mut by_power: Vec<(CardId, i32)> = card_powers
            .iter()
            .copied()
            .filter(|(c, _)| valid.contains(c))
            .collect();
        by_power.sort_by_key(|&(_, power)| std::cmp::Reverse(power));
        let mut total = 0;
        let mut chosen = Vec::new();
        for (card, power) in by_power {
            if total >= min_total_power {
                break;
            }
            total += power;
            chosen.push(card);
        }
        chosen
    }

    fn choose_mode(
        &mut self,
        _player: PlayerId,
        descriptions: &[String],
        min: usize,
        max: usize,
        source: Option<CardId>,
    ) -> Vec<usize> {
        let max = max.min(descriptions.len());
        let min = min.min(max);
        if max == 0 || min == descriptions.len() {
            return (0..min).collect();
        }
        let kind = DecisionKind::Modes {
            descriptions: descriptions.to_vec(),
            min,
            max,
            source,
        };
        match self.ask(kind) {
            Some(Action::Select(picks)) => picks,
            _ => (0..min).collect(),
        }
    }

    fn choose_optional_trigger(
        &mut self,
        _player: PlayerId,
        description: &str,
        source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        self.confirm(ConfirmPurpose::OptionalTrigger, description, source, true)
    }

    fn confirm_replacement_effect(
        &mut self,
        _player: PlayerId,
        question: &str,
        _effect_description: &str,
        source: Option<CardId>,
    ) -> bool {
        let prompt = Some((source, question.to_string()));
        if self.last_replacement == prompt {
            return false;
        }
        self.last_replacement = prompt;
        self.confirm(ConfirmPurpose::Replacement, question, source, true)
    }

    fn confirm_action(
        &mut self,
        _player: PlayerId,
        mode: Option<&str>,
        message: &str,
        _options: &[String],
        source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        self.confirm(
            ConfirmPurpose::Action(mode.map(str::to_string)),
            message,
            source,
            false,
        )
    }

    fn pay_mana_cost(
        &mut self,
        _player: PlayerId,
        _card_id: CardId,
        _card_name: &str,
        _mana_cost: &str,
        _mana_cost_display: &str,
        _mana_cost_checkpoint: &str,
        _can_confirm_from_pool: bool,
        _allow_reserved_source_reuse: bool,
        _reserved_sacrifices: &[CardId],
        _mana_ability_options: &[ManaAbilityOption],
        _tappable_lands: &[CardId],
        _untappable_lands: &[CardId],
        _mana_pool: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
}
