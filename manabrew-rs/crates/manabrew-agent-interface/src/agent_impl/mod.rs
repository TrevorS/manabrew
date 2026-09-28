use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use forge_foundation::{ManaAtom, ZoneType};
use manabrew_engine::agent::notification::GameNotification;
use manabrew_engine::agent::{
    BinaryChoiceKind, CombatCostAction, GameEntity, ManaCostAction, PlayerAgent,
    PriorityActionSpace, RollSwapChoice, TargetChoice,
};
use manabrew_engine::card::CounterType;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::player_action::AbilityRef;
use manabrew_engine::player::actions::PlayerAction as EnginePlayerAction;

use crate::game_log_event::GameLogEntryDto;
use crate::game_snapshot_event::GameSnapshotEventDto;
use crate::game_view_dto::{
    card_to_dto_for_viewer, shows_command_cards, shows_in_zones, stack_source_ability_text,
    CardDto, GameViewDto, GameViewDtoExt,
};
use crate::ids_codec::{card_id_str, parse_card_id, parse_player_id, player_id_str};
use crate::mana_action_id::{mana_ability_actions, parse_tap_action_id};
use crate::prompt::*;
use manabrew_engine::agent::{DecisionContext, PriorityContext};

mod choices;
mod combat;
mod costs;
mod library;
mod targeting;

/// Match a mana symbol letter (e.g. "U") or a full color name (e.g. "Blue")
/// against a list of color strings.  Handles the Blue/U mismatch where the
/// mana symbol "U" doesn't match the first character of "Blue".
pub(crate) fn find_matching_color<'a>(
    pending: &str,
    colors: impl Iterator<Item = &'a String>,
) -> Option<String> {
    let mana_to_name: &[(&str, &str)] = &[
        ("W", "White"),
        ("U", "Blue"),
        ("B", "Black"),
        ("R", "Red"),
        ("G", "Green"),
        ("C", "Colorless"),
    ];
    colors
        .into_iter()
        .find(|c| {
            // Direct case-insensitive match (covers both "U"=="U" and "Blue"=="Blue")
            c.eq_ignore_ascii_case(pending)
            // Mana symbol → full name lookup (covers "U" matching "Blue")
            || mana_to_name.iter().any(|(sym, name)| {
                pending.eq_ignore_ascii_case(sym) && c.eq_ignore_ascii_case(name)
            })
            // Full name → mana symbol lookup (covers "Blue" matching "U")
            || mana_to_name.iter().any(|(sym, name)| {
                pending.eq_ignore_ascii_case(name) && c.eq_ignore_ascii_case(sym)
            })
        })
        .cloned()
}

pub(crate) fn parse_express_mana_choice(color: Option<&str>) -> Option<u16> {
    color
        .map(|color| ManaAtom::from_name(&color.to_ascii_lowercase()))
        .filter(|&atom| atom != 0)
}

pub struct StateSummary {
    pub turn: u32,
    pub shows_command_cards: bool,
}

/// Answers the prompts a `PromptAgent` builds.
///
pub trait Responder {
    fn respond(&mut self, prompt: AgentPrompt) -> ClientToServerMessage;
    fn present(&mut self, _message: &AgentMessage) {}
    fn present_state(&mut self, view: &Arc<GameViewDto>) {
        self.present(&AgentMessage::State(StateUpdate {
            game_view: GameViewDto::clone(view),
        }));
    }
    /// A responder that defers views gets a summary of each state update, and the view of the
    /// last one just before it answers a prompt.
    fn defers_views(&self) -> bool {
        false
    }
    fn present_state_summary(&mut self, _summary: StateSummary) {}
    fn await_ack(&mut self) -> ClientToServerMessage {
        ClientToServerMessage::Response {
            prompt_id: 0,
            action: PromptOutput::DiceRolled(DiceRolledOutput::DiceRolledAcknowledged),
        }
    }
    fn send_log(&mut self, _entry: GameLogEntryDto) {}
    fn send_snapshot(&mut self, _snapshot: GameSnapshotEventDto) {}
    fn observe_state(&mut self, _game: &GameState, _mana_pools: &[ManaPool]) {}
    fn reads_game(&self) -> bool {
        false
    }
    fn respond_to_game(
        &mut self,
        _context: DecisionContext<'_>,
        _viewer: PlayerId,
        prompt: AgentPrompt,
    ) -> ClientToServerMessage {
        self.respond(prompt)
    }
    fn notify(&mut self, _context: DecisionContext<'_>, _event: &GameNotification) {}
    fn observe_reveal(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        _cards: &[CardId],
        _zone: ZoneType,
        _owner: PlayerId,
    ) {
    }
    fn hand_off_at_turn_start(&mut self, _game: &GameState) -> Option<Box<dyn PlayerAgent>> {
        None
    }
}

pub(crate) struct Live<'a> {
    pub(crate) game: &'a GameState,
    mana_pools: Option<&'a [ManaPool]>,
    view: OnceCell<Arc<GameViewDto>>,
}

impl<'a> Live<'a> {
    pub(crate) fn new(context: DecisionContext<'a>) -> Self {
        Live {
            game: context.game,
            mana_pools: context.mana_pools,
            view: OnceCell::new(),
        }
    }
}

#[derive(Clone, Default)]
enum OwedView {
    #[default]
    None,
    Latest,
    Held(Arc<GameViewDto>),
}

pub struct PromptAgent<R: Responder> {
    pub player_id: PlayerId,
    pub game_id: String,
    pub responder: R,
    pending_prompt: Option<AgentPrompt>,
    mana_pools: Vec<ManaPool>,
    owed_view: OwedView,
    pub(crate) pending_restore_checkpoint: Option<u64>,
    pub pass_until: Option<manabrew_engine::agent::PassUntilTarget>,
    conceded: bool,
    next_prompt_id: u32,
    pub(crate) targeting_cancellable: bool,
    pub(crate) targeting_cancelled: bool,
    pub(crate) targeting_optional: bool,
}

impl<R: Responder> PromptAgent<R> {
    pub fn new(player_id: PlayerId, game_id: String, responder: R) -> Self {
        Self {
            player_id,
            game_id,
            responder,
            pending_prompt: None,
            mana_pools: Vec::new(),
            owed_view: OwedView::None,
            pending_restore_checkpoint: None,
            pass_until: None,
            conceded: false,
            next_prompt_id: 0,
            targeting_cancellable: false,
            targeting_cancelled: false,
            targeting_optional: false,
        }
    }

    pub fn fork_with<S: Responder>(&self, responder: S) -> PromptAgent<S> {
        PromptAgent {
            player_id: self.player_id,
            game_id: self.game_id.clone(),
            responder,
            pending_prompt: self.pending_prompt.clone(),
            mana_pools: self.mana_pools.clone(),
            owed_view: self.owed_view.clone(),
            pending_restore_checkpoint: self.pending_restore_checkpoint,
            pass_until: self.pass_until,
            conceded: self.conceded,
            next_prompt_id: self.next_prompt_id,
            targeting_cancellable: self.targeting_cancellable,
            targeting_cancelled: self.targeting_cancelled,
            targeting_optional: self.targeting_optional,
        }
    }

    fn build_prompt(
        &mut self,
        live: &Live<'_>,
        inner: PromptInput,
        source: Option<CardId>,
    ) -> AgentPrompt {
        self.next_prompt_id += 1;
        let source_card = source.and_then(|card_id| self.source_card(live, card_id));
        let source_ability_text = source
            .filter(|_| source_card.is_some())
            .and_then(|card_id| {
                let entry = live
                    .game
                    .stack
                    .iter()
                    .filter(|entry| entry.spell_ability.source == Some(card_id))
                    .last()?;
                stack_source_ability_text(live.game, &entry.spell_ability, self.player_id)
            });
        AgentPrompt {
            prompt_id: self.next_prompt_id,
            deciding_player_id: player_id_str(self.player_id),
            source_card,
            source_ability_text,
            input: inner,
        }
    }

    pub(crate) fn send_prompt(
        &mut self,
        live: &Live<'_>,
        inner: PromptInput,
        source: Option<CardId>,
    ) {
        let prompt = self.build_prompt(live, inner, source);
        self.emit_state(live);
        self.responder
            .present(&AgentMessage::Prompt(prompt.clone()));
        self.pending_prompt = Some(prompt);
    }

    pub(crate) fn recv_action(&mut self, live: &Live<'_>) -> PromptOutput {
        let prompt = self
            .pending_prompt
            .take()
            .expect("recv_action called without a pending prompt");
        if self.conceded {
            return Self::default_pass();
        }
        self.present_owed_view(live);
        loop {
            let context =
                DecisionContext::new(live.game, live.mana_pools.unwrap_or(&self.mana_pools));
            match self
                .responder
                .respond_to_game(context, self.player_id, prompt.clone())
            {
                ClientToServerMessage::Response { prompt_id: 0, .. } => {
                    return Self::default_pass();
                }
                ClientToServerMessage::Response { prompt_id, action } => {
                    if prompt_id != prompt.prompt_id {
                        self.reject(
                            &prompt,
                            ProtocolErrorCode::StalePrompt,
                            format!(
                                "response for prompt {prompt_id}, open prompt is {}",
                                prompt.prompt_id
                            ),
                        );
                        continue;
                    }
                    match prompt.input.validate_response(&action) {
                        Ok(()) => return action,
                        Err(ResponseViolation::WrongPromptType) => {
                            self.reject(
                                &prompt,
                                ProtocolErrorCode::WrongPromptType,
                                "response output does not match the prompt type".to_string(),
                            );
                        }
                        Err(ResponseViolation::UnknownActionId(id)) => {
                            self.reject(
                                &prompt,
                                ProtocolErrorCode::UnknownActionId,
                                format!("action id {id:?} was not advertised by the prompt"),
                            );
                        }
                        Err(ResponseViolation::CancelNotAllowed) => {
                            self.reject(
                                &prompt,
                                ProtocolErrorCode::CancelNotAllowed,
                                "this prompt is not cancellable".to_string(),
                            );
                        }
                        Err(ResponseViolation::IllegalAssignment(pair)) => {
                            self.reject(&prompt, ProtocolErrorCode::IllegalAssignment, pair);
                        }
                    }
                }
                ClientToServerMessage::Directive { directive } => {
                    self.handle_directive(directive);
                    return Self::default_pass();
                }
            }
        }
    }

    fn default_pass() -> PromptOutput {
        PromptOutput::ChooseAction(ChooseActionOutput::Pass {
            until: None,
            exhaust_stack: false,
        })
    }

    fn reject(&mut self, prompt: &AgentPrompt, code: ProtocolErrorCode, message: String) {
        self.responder.present(&AgentMessage::Error(ProtocolError {
            code,
            message,
            prompt_id: Some(prompt.prompt_id),
        }));
        self.responder
            .present(&AgentMessage::Prompt(prompt.clone()));
    }

    fn handle_directive(&mut self, directive: DirectiveInput) {
        match directive {
            DirectiveInput::Concede => self.conceded = true,
        }
    }

    pub(crate) fn present_prompt(
        &mut self,
        live: &Live<'_>,
        inner: PromptInput,
        source: Option<CardId>,
    ) {
        let prompt = self.build_prompt(live, inner, source);
        self.emit_state(live);
        self.responder.present(&AgentMessage::Prompt(prompt));
    }

    pub(crate) fn emit_state(&mut self, live: &Live<'_>) {
        if self.responder.defers_views() {
            self.owed_view = OwedView::Latest;
            self.responder.present_state_summary(StateSummary {
                turn: live.game.turn.turn_number,
                shows_command_cards: shows_command_cards(live.game),
            });
        } else {
            let view = self.latest_view(live).clone();
            self.responder.present_state(&view);
        }
    }

    fn present_owed_view(&mut self, live: &Live<'_>) {
        let view = match std::mem::take(&mut self.owed_view) {
            OwedView::None => return,
            OwedView::Latest if self.responder.reads_game() => return,
            OwedView::Latest => self.latest_view(live).clone(),
            OwedView::Held(view) => view,
        };
        self.responder.present_state(&view);
    }

    pub(crate) fn hold_owed_view(&mut self, live: &Live<'_>) {
        if !self.responder.reads_game() && matches!(self.owed_view, OwedView::Latest) {
            self.owed_view = OwedView::Held(self.latest_view(live).clone());
        }
    }

    pub(crate) fn latest_view<'l>(&self, live: &'l Live<'_>) -> &'l Arc<GameViewDto> {
        live.view.get_or_init(|| {
            Arc::new(GameViewDto::from_engine(
                live.game,
                live.mana_pools.unwrap_or(&self.mana_pools),
                self.player_id,
                &self.game_id,
            ))
        })
    }

    pub(crate) fn emit_display(&mut self, event: DisplayEvent) {
        self.responder.present(&AgentMessage::Display(event));
    }

    pub(crate) fn source_card(&self, live: &Live<'_>, card_id: CardId) -> Option<CardDto> {
        let game = live.game;
        game.cards
            .get(card_id.index())
            .filter(|card| card.id == card_id)?;
        Some(card_to_dto_for_viewer(game, card_id, Some(self.player_id)))
    }

    pub(crate) fn shown_card(&self, live: &Live<'_>, card_id: CardId) -> Option<CardDto> {
        live.game
            .cards
            .get(card_id.index())
            .filter(|card| card.id == card_id)?;
        shows_in_zones(live.game, card_id, self.player_id)
            .then(|| card_to_dto_for_viewer(live.game, card_id, Some(self.player_id)))
    }

    pub(crate) fn player_name(&self, live: &Live<'_>, player_id: PlayerId) -> Option<String> {
        live.game
            .player_order
            .contains(&player_id)
            .then(|| live.game.player(player_id).name.clone())
    }

    pub(crate) fn card_ids(cards: &[CardId]) -> Vec<String> {
        cards.iter().map(|&c| card_id_str(c)).collect()
    }

    pub(crate) fn player_ids(players: &[PlayerId]) -> Vec<String> {
        players.iter().map(|&p| player_id_str(p)).collect()
    }

    pub(crate) fn attack_targets_to_dtos(
        defenders: &[DefenderId],
    ) -> Vec<crate::prompt::AttackTargetDto> {
        use crate::prompt::AttackTargetKind;
        defenders
            .iter()
            .map(|d| match d {
                DefenderId::Player(pid) => crate::prompt::AttackTargetDto {
                    id: format!("player-{}", pid.0),
                    label: format!("Player {}", pid.0),
                    kind: AttackTargetKind::Player,
                },
                // The Rust engine can't distinguish a planeswalker from a
                // battle yet; approximate any permanent target as a walker.
                DefenderId::Permanent(cid) => crate::prompt::AttackTargetDto {
                    id: format!("card-{}", cid.0),
                    label: format!("Permanent {}", cid.0),
                    kind: AttackTargetKind::Planeswalker,
                },
            })
            .collect()
    }

    fn alt_cost_kind(alt: manabrew_engine::spellability::AlternativeCost) -> AlternativeCostKind {
        use manabrew_engine::spellability::AlternativeCost as A;
        match alt {
            A::Flashback => AlternativeCostKind::Flashback,
            A::Spectacle => AlternativeCostKind::Spectacle,
            A::Evoke => AlternativeCostKind::Evoke,
            A::Dash => AlternativeCostKind::Dash,
            A::Blitz => AlternativeCostKind::Blitz,
            A::Escape => AlternativeCostKind::Escape,
            A::Overload => AlternativeCostKind::Overload,
            A::Madness => AlternativeCostKind::Madness,
            A::Foretell => AlternativeCostKind::Foretell,
            A::Emerge => AlternativeCostKind::Emerge,
            A::Suspend => AlternativeCostKind::Suspend,
            A::Morph => AlternativeCostKind::Morph,
            A::Megamorph => AlternativeCostKind::Megamorph,
            A::Bestow => AlternativeCostKind::Bestow,
            A::Warp => AlternativeCostKind::Warp,
            A::SacrificeAlt => AlternativeCostKind::SacrificeAlt,
            A::Plot => AlternativeCostKind::Plot,
            A::Awaken => AlternativeCostKind::Awaken,
            A::Disturb => AlternativeCostKind::Disturb,
            A::Harmonize => AlternativeCostKind::Harmonize,
            A::Freerunning => AlternativeCostKind::Freerunning,
            A::Impending => AlternativeCostKind::Impending,
            A::Mayhem => AlternativeCostKind::Mayhem,
            A::MTMtE => AlternativeCostKind::MTMtE,
            A::Mutate => AlternativeCostKind::Mutate,
            A::Prowl => AlternativeCostKind::Prowl,
            A::Sneak => AlternativeCostKind::Sneak,
            A::Surge => AlternativeCostKind::Surge,
            A::WebSlinging => AlternativeCostKind::WebSlinging,
            A::Plotted => AlternativeCostKind::Plotted,
        }
    }

    fn play_mode_dto(mode: &manabrew_engine::agent::PlayCardMode) -> (PlayCardMode, String) {
        use manabrew_engine::agent::PlayCardMode as E;
        match mode {
            E::Normal => (PlayCardMode::Normal, "Cast normally".to_string()),
            E::BackFaceLand => (
                PlayCardMode::BackFaceLand,
                "Play back face as land".to_string(),
            ),
            E::RoomRightSplit => (PlayCardMode::RoomRightSplit, "Cast right room".to_string()),
            E::Secondary => (PlayCardMode::Secondary, "Cast secondary face".to_string()),
            E::StaticAlternative => (
                PlayCardMode::StaticAlternative,
                "Cast with alternative cost".to_string(),
            ),
            E::ForetellExile => (
                PlayCardMode::ForetellExile,
                "Foretell (exile face-down)".to_string(),
            ),
            E::UnlockDoor => (PlayCardMode::UnlockDoor, "Unlock door".to_string()),
            E::MayPlay(_) => (
                PlayCardMode::StaticAlternative,
                "Cast by paying its alternative cost".to_string(),
            ),
            E::Alternative(alt) => (
                PlayCardMode::Alternative {
                    cost: Self::alt_cost_kind(*alt),
                },
                format!("Cast with {alt:?}"),
            ),
        }
    }

    fn play_mode_key(mode: &manabrew_engine::agent::PlayCardMode) -> String {
        use manabrew_engine::agent::PlayCardMode as E;
        match mode {
            E::Normal => "normal".to_string(),
            E::BackFaceLand => "backFaceLand".to_string(),
            E::RoomRightSplit => "roomRightSplit".to_string(),
            E::Secondary => "secondary".to_string(),
            E::StaticAlternative => "staticAlternative".to_string(),
            E::ForetellExile => "foretellExile".to_string(),
            E::UnlockDoor => "unlockDoor".to_string(),
            E::Alternative(alt) => format!("alternative:{}", format!("{alt:?}").to_lowercase()),
            E::MayPlay(None) => "mayPlay".to_string(),
            E::MayPlay(Some(alt)) => format!("mayPlay:{}", format!("{alt:?}").to_lowercase()),
        }
    }

    pub(crate) fn parse_defender_id(id: &str, possible: &[DefenderId]) -> Option<DefenderId> {
        if let Some(rest) = id.strip_prefix("player-") {
            let idx: u32 = rest.parse().ok()?;
            possible
                .iter()
                .find(|d| matches!(d, DefenderId::Player(p) if p.0 == idx))
                .copied()
        } else if let Some(rest) = id.strip_prefix("card-") {
            let idx: u32 = rest.parse().ok()?;
            possible
                .iter()
                .find(|d| matches!(d, DefenderId::Permanent(c) if c.0 == idx))
                .copied()
        } else {
            None
        }
    }

    pub(crate) fn recv_card_choice_or_first(
        &mut self,
        live: &Live<'_>,
        valid: &[CardId],
    ) -> Option<CardId> {
        match self.recv_action(live) {
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::Cancel) => {
                self.targeting_cancelled = true;
                None
            }
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::BoardTargets { chosen }) => {
                chosen.into_iter().find_map(|r| match r.kind {
                    TargetKind::Card => parse_card_id(&r.id),
                    _ => None,
                })
            }
            _ => valid.first().copied(),
        }
    }

    pub(crate) fn recv_player_choice_or_first(
        &mut self,
        live: &Live<'_>,
        valid: &[PlayerId],
    ) -> Option<PlayerId> {
        match self.recv_action(live) {
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::Cancel) => {
                self.targeting_cancelled = true;
                None
            }
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::BoardTargets { chosen }) => {
                chosen.into_iter().find_map(|r| match r.kind {
                    TargetKind::Player => parse_player_id(&r.id),
                    _ => None,
                })
            }
            _ => valid.first().copied(),
        }
    }

    pub(crate) fn recv_spell_choice_or_first(
        &mut self,
        live: &Live<'_>,
        valid: &[u32],
    ) -> Option<u32> {
        match self.recv_action(live) {
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::Cancel) => {
                self.targeting_cancelled = true;
                None
            }
            PromptOutput::ChooseBoardTargets(ChooseBoardTargetsOutput::BoardTargets { chosen }) => {
                chosen.into_iter().find_map(|r| match r.kind {
                    TargetKind::Spell => crate::ids_codec::parse_stack_id(&r.id),
                    _ => None,
                })
            }
            _ => valid.first().copied(),
        }
    }
}

impl<R: Responder + 'static> PlayerAgent for PromptAgent<R> {
    fn choose_targets_for(
        &mut self,
        sa: &mut manabrew_engine::spellability::SpellAbility,
        game: &GameState,
        mana_pools: &[ManaPool],
    ) -> bool {
        self.targeting_cancelled = false;
        self.targeting_optional = sa
            .target_restrictions
            .as_ref()
            .is_some_and(|tr| tr.get_min_targets(game, sa) <= 0);
        let ok = manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, mana_pools);
        self.targeting_optional = false;
        ok && !self.targeting_cancelled
    }

    fn set_targeting_cancellable(&mut self, cancellable: bool) {
        self.targeting_cancellable = cancellable;
    }

    fn set_targeting_optional(&mut self, optional: bool) {
        self.targeting_optional = optional;
    }

    fn choose_new_targets_for(
        &mut self,
        sa: &mut manabrew_engine::spellability::SpellAbility,
        game: &GameState,
        mana_pools: &[ManaPool],
        optional: bool,
    ) -> bool {
        let old_targets = std::mem::take(&mut sa.target_chosen);
        let cancellable = std::mem::replace(&mut self.targeting_cancellable, optional);
        let chosen = self.choose_targets_for(sa, game, mana_pools);
        self.targeting_cancellable = cancellable;
        if !chosen {
            sa.target_chosen = old_targets;
        }
        chosen
    }

    fn get_pass_until(&self) -> Option<manabrew_engine::agent::PassUntilTarget> {
        self.pass_until
    }

    fn clear_pass_until(&mut self) {
        self.pass_until = None;
    }

    fn snapshot_state(&mut self, game: &GameState, mana_pools: &[ManaPool]) {
        self.responder.observe_state(game, mana_pools);
        self.mana_pools.clear();
        self.mana_pools.extend_from_slice(mana_pools);
    }

    fn mulligan_decision(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        mulligan_count: u32,
    ) -> bool {
        let live = Live::new(context);
        choices::mulligan_decision(self, &live, player, hand, mulligan_count)
    }

    fn mulligan_decision_send(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        mulligan_count: u32,
    ) {
        let live = Live::new(context);
        choices::mulligan_decision_send(self, &live, player, hand, mulligan_count);
    }

    fn mulligan_decision_recv(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        mulligan_count: u32,
    ) -> bool {
        let live = Live::new(context);
        choices::mulligan_decision_recv(self, &live, player, hand, mulligan_count)
    }

    fn choose_cards_to_bottom(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        count: usize,
    ) -> Vec<CardId> {
        let live = Live::new(context);
        choices::choose_cards_to_bottom(self, &live, player, hand, count)
    }

    fn choose_cards_to_bottom_send(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        count: usize,
    ) {
        let live = Live::new(context);
        choices::choose_cards_to_bottom_send(self, &live, player, hand, count);
    }

    fn choose_cards_to_bottom_recv(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        count: usize,
    ) -> Vec<CardId> {
        let live = Live::new(context);
        choices::choose_cards_to_bottom_recv(self, &live, player, hand, count)
    }

    fn choose_action(
        &mut self,
        _player: PlayerId,
        action_space: Option<&PriorityActionSpace>,
        priority: &mut dyn PriorityContext,
    ) -> EnginePlayerAction {
        if self.conceded {
            return EnginePlayerAction::Concede;
        }
        let requested_action_space;
        let action_space = match action_space {
            Some(action_space) => action_space,
            None => {
                requested_action_space = priority.action_space();
                &requested_action_space
            }
        };
        let playable = &action_space.playable;
        let untappable_lands = &action_space.untappable_lands;
        let _activatable = &action_space.activatable;
        let untappable_land_ids: Vec<String> =
            untappable_lands.iter().map(|&c| card_id_str(c)).collect();

        let mut actions: Vec<AvailableAction> = Vec::new();
        let mut cast_options: HashMap<String, manabrew_engine::agent::PlayOption> = HashMap::new();
        for play in playable.iter() {
            let card_id = card_id_str(play.card_id);
            let (mode, label) = Self::play_mode_dto(&play.mode);
            let key = Self::play_mode_key(&play.mode);
            let id = match play.alt_cost_index {
                0 => format!("cast:{card_id}:{key}"),
                index => format!("cast:{card_id}:{key}#{index}"),
            };
            cast_options.entry(id.clone()).or_insert(*play);
            actions.push(AvailableAction {
                id,
                kind: AvailableActionKind::Cast {
                    card_id: card_id.clone(),
                    mode,
                    label,
                },
            });
        }
        for a in action_space
            .activatable
            .iter()
            .chain(action_space.mana_abilities.iter())
        {
            let card_id = card_id_str(a.card_id);
            if a.is_mana_ability {
                actions.extend(
                    mana_ability_actions(
                        &card_id,
                        a.ability_index,
                        &a.description,
                        a.cost.clone(),
                        a.produced_mana.clone(),
                        a.produced_mana_amount,
                    )
                    .into_iter()
                    .map(|(id, info)| AvailableAction {
                        id,
                        kind: AvailableActionKind::ActivateAbility(info),
                    }),
                );
            } else {
                actions.push(AvailableAction {
                    id: format!("ability:{card_id}:{}", a.ability_index),
                    kind: AvailableActionKind::ActivateAbility(ActivatableAbilityInfo {
                        card_id,
                        ability_index: a.ability_index,
                        description: a.description.clone(),
                        cost: a.cost.clone(),
                        is_mana_ability: false,
                        is_class_level_up: Some(a.is_class_level_up),
                        produced_mana: None,
                    }),
                });
            }
        }
        for card_id in &untappable_land_ids {
            actions.push(AvailableAction {
                id: format!("untap:{card_id}"),
                kind: AvailableActionKind::UndoMana {
                    card_id: card_id.clone(),
                },
            });
        }
        let mut advertised = HashSet::new();
        actions.retain(|action| advertised.insert(action.id.clone()));

        let live = Live::new(priority.context());
        self.send_prompt(
            &live,
            PromptInput::ChooseAction(
                manabrew_protocol::prompts::choose_action::ChooseActionInput { actions },
            ),
            None,
        );
        let prompt = self
            .pending_prompt
            .take()
            .expect("choose_action called without a pending prompt");
        self.present_owed_view(&live);
        let context = DecisionContext::new(live.game, live.mana_pools.unwrap_or(&self.mana_pools));
        let action = match self
            .responder
            .respond_to_game(context, self.player_id, prompt)
        {
            ClientToServerMessage::Response { action, .. } => action,
            ClientToServerMessage::Directive {
                directive: DirectiveInput::Concede,
            } => return EnginePlayerAction::Concede,
        };
        match action {
            PromptOutput::ChooseAction(ChooseActionOutput::Act { action_id }) => {
                if let Some(&play) = cast_options.get(&action_id) {
                    EnginePlayerAction::CastSpell(play)
                } else if let Some(rest) = action_id.strip_prefix("tap:") {
                    let tap = parse_tap_action_id(rest);
                    match parse_card_id(tap.card_id) {
                        Some(cid) => EnginePlayerAction::ActivateMana(
                            cid,
                            tap.ability_index,
                            parse_express_mana_choice(tap.color),
                        ),
                        None => EnginePlayerAction::PassPriority,
                    }
                } else if let Some(rest) = action_id.strip_prefix("ability:") {
                    let (id_part, idx) = rest.split_once(':').unwrap_or((rest, ""));
                    match (parse_card_id(id_part), idx.parse::<usize>()) {
                        (Some(cid), Ok(ability_index)) => {
                            EnginePlayerAction::ActivateAbility(AbilityRef {
                                card_id: cid,
                                ability_index,
                            })
                        }
                        _ => EnginePlayerAction::PassPriority,
                    }
                } else if let Some(id_part) = action_id.strip_prefix("untap:") {
                    parse_card_id(id_part)
                        .map(EnginePlayerAction::UndoMana)
                        .unwrap_or(EnginePlayerAction::PassPriority)
                } else {
                    EnginePlayerAction::PassPriority
                }
            }
            PromptOutput::ChooseAction(ChooseActionOutput::Pass { until, .. }) => {
                self.pass_until = until.and_then(|u| {
                    Some(manabrew_engine::agent::PassUntilTarget {
                        player: crate::ids_codec::parse_player_id(&u.player_id)?,
                        phase: crate::game_view_dto::step_to_phase(u.phase),
                        through_combat: u.through_combat,
                    })
                });
                EnginePlayerAction::PassPriority
            }
            PromptOutput::ChooseAction(ChooseActionOutput::RestoreSnapshot { checkpoint_id }) => {
                self.pending_restore_checkpoint = Some(checkpoint_id);
                EnginePlayerAction::PassPriority
            }
            _ => EnginePlayerAction::PassPriority,
        }
    }

    fn choose_attackers(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        available: &[CardId],
        possible_defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        let live = Live::new(context);
        combat::choose_attackers(self, &live, player, available, possible_defenders)
    }

    fn choose_blockers(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attackers: &[CardId],
        available_blockers: &[CardId],
        max_blockers: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        let live = Live::new(context);
        combat::choose_blockers(
            self,
            &live,
            player,
            attackers,
            available_blockers,
            max_blockers,
        )
    }

    fn choose_damage_assignment_order(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attacker: CardId,
        blockers: &[CardId],
    ) -> Vec<CardId> {
        let live = Live::new(context);
        combat::choose_damage_assignment_order(self, &live, player, attacker, blockers)
    }

    fn assign_combat_damage(
        &mut self,
        game: &GameState,
        player: PlayerId,
        attacker: CardId,
        blockers_in_order: &[CardId],
        defender_id: Option<DefenderId>,
        damage_to_assign: i32,
    ) -> Vec<(Option<CardId>, i32)> {
        let live = Live::new(DecisionContext::game_only(game));
        let attacker_has_deathtouch = game.card(attacker).has_deathtouch();
        combat::choose_combat_damage_assignment(
            self,
            &live,
            player,
            attacker,
            blockers_in_order,
            defender_id,
            damage_to_assign,
            attacker_has_deathtouch,
        )
    }

    fn choose_target_player(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[PlayerId],
        sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<PlayerId> {
        let live = Live::new(context);
        let source = sa.and_then(|s| s.source);
        let intent = sa
            .map(crate::game_view_dto::targeting_intent_of)
            .unwrap_or(crate::game_view_dto::TargetingIntent::Hostile);
        let hostile = crate::game_view_dto::intent_is_hostile(intent);
        targeting::choose_target_player(self, &live, player, valid, source, hostile, intent)
    }

    fn choose_target_card(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<CardId> {
        let live = Live::new(context);
        let source = sa.and_then(|s| s.source);
        let intent = sa
            .map(crate::game_view_dto::targeting_intent_of)
            .unwrap_or(crate::game_view_dto::TargetingIntent::Hostile);
        let hostile = crate::game_view_dto::intent_is_hostile(intent);
        targeting::choose_target_card(self, &live, player, valid, source, hostile, intent)
    }

    fn choose_target_card_from_zone(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        zone: ZoneType,
        valid: &[CardId],
        sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<CardId> {
        let live = Live::new(context);
        let source = sa.and_then(|s| s.source);
        let intent = sa
            .map(crate::game_view_dto::targeting_intent_of)
            .unwrap_or(crate::game_view_dto::TargetingIntent::Hostile);
        let hostile = crate::game_view_dto::intent_is_hostile(intent);
        targeting::choose_target_card_from_zone(
            self, &live, player, zone, valid, source, hostile, intent,
        )
    }

    fn choose_target_any(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid_players: &[PlayerId],
        valid_cards: &[CardId],
        sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> TargetChoice {
        let live = Live::new(context);
        let source = sa.and_then(|s| s.source);
        let intent = sa
            .map(crate::game_view_dto::targeting_intent_of)
            .unwrap_or(crate::game_view_dto::TargetingIntent::Hostile);
        let hostile = crate::game_view_dto::intent_is_hostile(intent);
        targeting::choose_target_any(
            self,
            &live,
            player,
            valid_players,
            valid_cards,
            source,
            hostile,
            intent,
        )
    }

    fn choose_sacrifice(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        source: Option<CardId>,
    ) -> Option<CardId> {
        let live = Live::new(context);
        targeting::choose_sacrifice(self, &live, player, valid, source)
    }

    fn reveal_cards(
        &mut self,
        game: &GameState,
        player: PlayerId,
        cards: &[CardId],
        zone: ZoneType,
        owner: PlayerId,
        message_prefix: Option<&str>,
    ) {
        let live = Live::new(DecisionContext::game_only(game));
        self.responder
            .observe_reveal(game, player, cards, zone, owner);
        choices::reveal_cards(self, &live, game, cards, zone, owner, message_prefix)
    }

    fn choose_scry(
        &mut self,
        game: &GameState,
        player: PlayerId,
        source: Option<CardId>,
        cards: &[CardId],
    ) -> Vec<Vec<CardId>> {
        let live = Live::new(DecisionContext::game_only(game));
        library::choose_scry(self, &live, game, player, source, cards)
    }

    fn choose_surveil(
        &mut self,
        game: &GameState,
        player: PlayerId,
        source: Option<CardId>,
        cards: &[CardId],
    ) -> Vec<Vec<CardId>> {
        let live = Live::new(DecisionContext::game_only(game));
        library::choose_surveil(self, &live, game, player, source, cards)
    }

    fn choose_dig(
        &mut self,
        game: &GameState,
        player: PlayerId,
        valid: &[CardId],
        max: usize,
        optional: bool,
    ) -> Vec<CardId> {
        let live = Live::new(DecisionContext::game_only(game));
        library::choose_dig(self, &live, game, player, valid, max, optional)
    }

    fn choose_discard(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        num: usize,
    ) -> Vec<CardId> {
        let live = Live::new(context);
        choices::choose_discard(self, &live, player, hand, num)
    }

    fn choose_discard_any_number(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        let live = Live::new(context);
        choices::choose_discard_any_number(self, &live, player, hand, min, max)
    }

    fn choose_legend_keep(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        duplicates: &[CardId],
    ) -> CardId {
        let live = Live::new(context);
        choices::choose_legend_keep(self, &live, player, duplicates)
    }

    fn choose_target_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[u32],
        source: Option<CardId>,
    ) -> Option<u32> {
        let live = Live::new(context);
        targeting::choose_target_spell(self, &live, player, valid, source)
    }

    fn choose_mode(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        descriptions: &[String],
        min: usize,
        max: usize,
        source_card_id: Option<CardId>,
    ) -> Vec<usize> {
        let live = Live::new(context);
        choices::choose_mode(self, &live, player, descriptions, min, max, source_card_id)
    }

    fn choose_spell_abilities_for_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        abilities: &[manabrew_engine::spellability::SpellAbility],
        num: usize,
    ) -> Vec<usize> {
        let live = Live::new(context);
        choices::choose_spell_abilities_for_effect(self, &live, player, abilities, num)
    }

    fn get_ability_to_play(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        abilities: &[manabrew_engine::spellability::SpellAbility],
    ) -> Option<usize> {
        let live = Live::new(context);
        choices::get_ability_to_play(self, &live, player, abilities)
    }

    fn choose_single_entity_for_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[GameEntity],
        is_optional: bool,
    ) -> Option<GameEntity> {
        let live = Live::new(context);
        choices::choose_single_entity_for_effect(self, &live, player, valid, is_optional)
    }

    fn choose_target(
        &mut self,
        player: PlayerId,
        sa: &manabrew_engine::spellability::SpellAbility,
        all_targets: &[(usize, manabrew_engine::agent::GameObject)],
        game: &GameState,
    ) -> Option<usize> {
        use manabrew_engine::agent::GameObject;
        let descriptions: Vec<String> = all_targets
            .iter()
            .map(|&(_, target)| match target {
                GameObject::Entity(GameEntity::Card(card)) => game.card(card).card_name.clone(),
                GameObject::Entity(GameEntity::Player(player)) => game.player(player).name.clone(),
                GameObject::Spell(stack_id) => game
                    .stack
                    .find_by_id(stack_id)
                    .and_then(|si| si.spell_ability.source)
                    .map(|host| game.card(host).card_name.clone())
                    .unwrap_or_default(),
            })
            .collect();
        self.choose_mode(
            DecisionContext::game_only(game),
            player,
            &descriptions,
            1,
            1,
            sa.source,
        )
        .first()
        .copied()
    }

    fn choose_entities_for_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        candidates: &[GameEntity],
        min: usize,
        max: usize,
    ) -> Vec<GameEntity> {
        let live = Live::new(context);
        choices::choose_entities_for_effect(self, &live, player, candidates, min, max)
    }

    fn choose_single_replacement_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        descriptions: &[String],
        _hosts: &[CardId],
    ) -> usize {
        let live = Live::new(context);
        choices::choose_single_replacement_effect(self, &live, player, descriptions)
    }

    fn confirm_replacement_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        question: &str,
        effect_description: &str,
        source: Option<CardId>,
    ) -> bool {
        let live = Live::new(context);
        choices::confirm_replacement_effect(
            self,
            &live,
            player,
            question,
            effect_description,
            source,
        )
    }

    fn choose_optional_trigger(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        description: &str,
        source: Option<CardId>,
        api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let live = Live::new(context);
        choices::choose_optional_trigger(self, &live, player, description, source, api)
    }

    fn confirm_action(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        mode: Option<&str>,
        message: &str,
        options: &[String],
        source: Option<CardId>,
        api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let live = Live::new(context);
        choices::confirm_action(self, &live, player, mode, message, options, source, api)
    }

    fn confirm_payment(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        cost_kind: &str,
        message: &str,
        source: Option<CardId>,
        api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let live = Live::new(context);
        choices::confirm_payment(self, &live, player, cost_kind, message, source, api)
    }

    fn pay_cost_to_prevent_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        cost_kind: &str,
        message: &str,
        source: Option<CardId>,
        api: Option<manabrew_engine::ability::api_type::ApiType>,
        can_pay: bool,
        targets: &[manabrew_engine::agent::GameEntity],
        effect_text: &str,
    ) -> bool {
        let live = Live::new(context);
        choices::pay_cost_to_prevent_effect(
            self,
            &live,
            player,
            cost_kind,
            message,
            source,
            api,
            can_pay,
            targets,
            effect_text,
        )
    }

    fn choose_binary(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        question: &str,
        kind: BinaryChoiceKind,
        default_choice: Option<bool>,
        source: Option<CardId>,
        api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let live = Live::new(context);
        choices::choose_binary(
            self,
            &live,
            player,
            question,
            kind,
            default_choice,
            source,
            api,
        )
    }

    fn choose_phyrexian_pay_life(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        color: &str,
        source: Option<CardId>,
    ) -> bool {
        let live = Live::new(context);
        costs::choose_phyrexian_pay_life(self, &live, player, color, source)
    }

    fn choose_kicker(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        kicker_cost: &str,
        source: Option<CardId>,
    ) -> bool {
        let live = Live::new(context);
        costs::choose_kicker(self, &live, player, kicker_cost, source)
    }

    fn choose_buyback(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        buyback_cost: &str,
        source: Option<CardId>,
    ) -> bool {
        let live = Live::new(context);
        costs::choose_buyback(self, &live, player, buyback_cost, source)
    }

    fn choose_multikicker(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        cost: &str,
        max_kicks: u32,
        source: Option<CardId>,
    ) -> u32 {
        let live = Live::new(context);
        costs::choose_multikicker(self, &live, player, cost, max_kicks, source)
    }

    fn choose_replicate(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        cost: &str,
        max_replicates: u32,
        source: Option<CardId>,
    ) -> u32 {
        let live = Live::new(context);
        costs::choose_replicate(self, &live, player, cost, max_replicates, source)
    }

    fn choose_color(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid_colors: &[String],
    ) -> Option<String> {
        let live = Live::new(context);
        choices::choose_color(self, &live, player, valid_colors)
    }

    fn choose_colors(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid_colors: &[String],
        min: usize,
        max: usize,
    ) -> Vec<String> {
        let live = Live::new(context);
        choices::choose_colors(self, &live, player, valid_colors, min, max)
    }

    fn choose_cards_for_effect(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        let live = Live::new(context);
        choices::choose_cards_for_effect(self, &live, player, valid, min, max)
    }

    fn choose_single_card_for_zone_change(
        &mut self,
        game: &GameState,
        player: PlayerId,
        valid: &[CardId],
        select_prompt: &str,
        is_optional: bool,
    ) -> Option<CardId> {
        let live = Live::new(DecisionContext::game_only(game));
        choices::choose_single_card_for_zone_change(
            self,
            &live,
            game,
            player,
            valid,
            select_prompt,
            is_optional,
        )
    }

    fn choose_cards_for_zone_change(
        &mut self,
        game: &GameState,
        player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
        select_prompt: &str,
    ) -> Vec<CardId> {
        let live = Live::new(DecisionContext::game_only(game));
        choices::choose_cards_for_zone_change(
            self,
            &live,
            game,
            player,
            valid,
            min,
            max,
            select_prompt,
        )
    }

    fn choose_type(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        type_category: &str,
        valid_types: &[String],
    ) -> Option<String> {
        let live = Live::new(context);
        choices::choose_type(self, &live, player, type_category, valid_types)
    }

    fn choose_counter_type(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        options: &[CounterType],
        prompt: &str,
    ) -> Option<CounterType> {
        let live = Live::new(context);
        choices::choose_counter_type(self, &live, player, options, prompt)
    }

    fn choose_card_name(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid_names: &[String],
    ) -> Option<String> {
        let live = Live::new(context);
        choices::choose_card_name(self, &live, player, valid_names)
    }

    fn choose_number(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        source: Option<CardId>,
        title: &str,
        description: Option<&str>,
        min: i32,
        max: i32,
    ) -> Option<i32> {
        let live = Live::new(context);
        choices::choose_number(self, &live, player, source, title, description, min, max)
    }

    fn announce_requirements_x(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        source: Option<CardId>,
        min: u32,
        max: u32,
    ) -> u32 {
        if min >= max {
            return max;
        }
        let live = Live::new(context);
        choices::choose_number(
            self,
            &live,
            player,
            source,
            "Choose a value for X",
            None,
            min as i32,
            max as i32,
        )
        .map_or(max, |chosen| chosen.clamp(min as i32, max as i32) as u32)
    }

    fn choose_number_from_list(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        choices: &[i32],
        message: &str,
        source_card_id: Option<CardId>,
    ) -> Option<i32> {
        let live = Live::new(context);
        choices::choose_number_from_list(self, &live, player, choices, message, source_card_id)
    }

    fn choose_roll_to_ignore(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        rolls: &[i32],
        source: Option<CardId>,
    ) -> Option<i32> {
        let live = Live::new(context);
        choices::choose_roll_to_ignore(self, &live, player, rolls, source)
    }

    fn choose_roll_to_swap(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        rolls: &[i32],
        source: Option<CardId>,
    ) -> Option<i32> {
        let live = Live::new(context);
        choices::choose_roll_to_swap(self, &live, player, rolls, source)
    }

    fn choose_dice_to_reroll(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        rolls: &[i32],
        source: Option<CardId>,
    ) -> Vec<i32> {
        let live = Live::new(context);
        choices::choose_dice_to_reroll(self, &live, player, rolls, source)
    }

    fn choose_roll_to_modify(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        rolls: &[i32],
        source: Option<CardId>,
    ) -> Option<i32> {
        let live = Live::new(context);
        choices::choose_roll_to_modify(self, &live, player, rolls, source)
    }

    fn choose_roll_swap_value(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        current_result: i32,
        power: i32,
        toughness: i32,
        source: Option<CardId>,
    ) -> Option<RollSwapChoice> {
        let live = Live::new(context);
        choices::choose_roll_swap_value(
            self,
            &live,
            player,
            current_result,
            power,
            toughness,
            source,
        )
    }

    fn flip_coin_call(&mut self, context: DecisionContext<'_>, player: PlayerId) -> bool {
        let live = Live::new(context);
        choices::flip_coin_call(self, &live, player)
    }

    fn pay_combat_cost(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attacker: CardId,
        cost: i32,
        description: &str,
        mana_ability_options: &[manabrew_engine::agent::ManaAbilityOption],
        tappable_lands: &[CardId],
        untappable_lands: &[CardId],
        mana_pool_total: i32,
    ) -> CombatCostAction {
        let live = Live::new(context);
        combat::pay_combat_cost(
            self,
            &live,
            player,
            attacker,
            cost,
            description,
            mana_ability_options,
            tappable_lands,
            untappable_lands,
            mana_pool_total,
        )
    }

    fn choose_improvise(
        &mut self,
        _context: DecisionContext<'_>,
        player: PlayerId,
        untapped_artifacts: &[CardId],
        remaining_cost: &forge_foundation::ManaCost,
        source: Option<CardId>,
    ) -> Vec<CardId> {
        costs::choose_improvise(self, player, untapped_artifacts, remaining_cost, source)
    }

    fn choose_convoke(
        &mut self,
        _context: DecisionContext<'_>,
        player: PlayerId,
        untapped_creatures: &[CardId],
        remaining_cost: &forge_foundation::ManaCost,
        source: Option<CardId>,
    ) -> Vec<CardId> {
        costs::choose_convoke(self, player, untapped_creatures, remaining_cost, source)
    }

    fn pay_mana_cost(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        card_id: CardId,
        card_name: &str,
        mana_cost: &str,
        mana_cost_display: &str,
        _mana_cost_checkpoint: &str,
        can_confirm_from_pool: bool,
        _allow_reserved_source_reuse: bool,
        _reserved_sacrifices: &[CardId],
        mana_ability_options: &[manabrew_engine::agent::ManaAbilityOption],
        tappable_lands: &[CardId],
        untappable_lands: &[CardId],
        mana_pool: &ManaPool,
    ) -> ManaCostAction {
        let live = Live::new(context);
        costs::pay_mana_cost(
            self,
            &live,
            player,
            card_id,
            card_name,
            mana_cost,
            mana_cost_display,
            can_confirm_from_pool,
            mana_ability_options,
            tappable_lands,
            untappable_lands,
            mana_pool,
        )
    }

    fn await_display_ack(&mut self) {
        if self.conceded {
            return;
        }
        if let ClientToServerMessage::Directive { directive } = self.responder.await_ack() {
            self.handle_directive(directive);
        }
    }

    fn specify_mana_combo(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        available_colors: &[String],
        amount: usize,
        source: Option<CardId>,
        express_choice: Option<u16>,
    ) -> Vec<String> {
        let live = Live::new(context);
        costs::specify_mana_combo(
            self,
            &live,
            player,
            available_colors,
            amount,
            source,
            express_choice,
        )
    }

    fn exert_attackers(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attackers: &[CardId],
    ) -> Vec<CardId> {
        let live = Live::new(context);
        combat::exert_attackers(self, &live, player, attackers)
    }

    fn enlist_attackers(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attackers: &[CardId],
    ) -> Vec<CardId> {
        let live = Live::new(context);
        combat::enlist_attackers(self, &live, player, attackers)
    }

    fn choose_reorder_library(
        &mut self,
        game: &GameState,
        player: PlayerId,
        cards: &[CardId],
    ) -> Vec<CardId> {
        let live = Live::new(DecisionContext::game_only(game));
        library::choose_reorder_library(self, &live, game, player, cards)
    }

    fn order_move_to_zone_list(
        &mut self,
        game: &GameState,
        player: PlayerId,
        cards: &[CardId],
        destination: ZoneType,
    ) -> Vec<CardId> {
        let live = Live::new(DecisionContext::game_only(game));
        match destination {
            ZoneType::Hand | ZoneType::Graveyard => cards.to_vec(),
            _ => library::choose_reorder_library(self, &live, game, player, cards),
        }
    }

    fn help_pay_assist(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        card_name: &str,
        max_generic: u32,
    ) -> u32 {
        let live = Live::new(context);
        choices::help_pay_assist(self, &live, player, card_name, max_generic)
    }

    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        let live = Live::new(context);
        choices::choose_land_or_spell(self, &live, player)
    }

    fn supports_checkpoints(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn hand_off_at_turn_start(&mut self, game: &GameState) -> Option<Box<dyn PlayerAgent>> {
        self.responder.hand_off_at_turn_start(game)
    }

    fn enforces_block_requirements(&self) -> bool {
        true
    }

    fn auto_pay_floats_mana(&self) -> bool {
        true
    }

    fn notify(&mut self, context: DecisionContext<'_>, event: GameNotification) {
        let live = Live::new(context);
        self.responder.notify(context, &event);
        match event {
            GameNotification::Event(log_event) => {
                self.responder
                    .send_log(GameLogEntryDto::from_event(log_event));
            }
            GameNotification::CardPlayed {
                player,
                card_id,
                card_name,
                set_code,
            } => {
                let face_hidden = self
                    .source_card(&live, card_id)
                    .is_some_and(|card| card.is_face_down && card.identity.name.is_empty());
                self.emit_display(DisplayEvent::CardPlayed {
                    card_id: card_id_str(card_id),
                    card_name: if face_hidden {
                        String::new()
                    } else {
                        card_name
                    },
                    set_code: if face_hidden { String::new() } else { set_code },
                    player_id: player_id_str(player),
                });
                self.emit_state(&live);
            }
            GameNotification::TurnChanged {
                active_player,
                turn_number,
            } => {
                let player_id = player_id_str(active_player);
                let active_player_name = self
                    .player_name(&live, active_player)
                    .unwrap_or_else(|| format!("Player {}", active_player.0));
                self.responder.send_log(GameLogEntryDto::from_event(
                    manabrew_engine::agent::GameLogEvent::rule(format!(
                        "TURN {turn_number} — {active_player_name}"
                    ))
                    .with_player(active_player),
                ));
                self.emit_display(DisplayEvent::TurnChanged {
                    active_player_id: player_id,
                    active_player_name,
                    turn_number,
                });
                self.emit_state(&live);
            }
            GameNotification::PhaseChanged { .. } | GameNotification::StateChanged => {
                self.emit_state(&live);
            }
            GameNotification::PriorityChanged { .. } => {
                self.emit_state(&live);
            }
            GameNotification::FirstPlayerRoll {
                sides,
                rounds,
                winner,
            } => {
                let winner_id = player_id_str(winner);
                let mut entries = Vec::new();
                let last_round = rounds.len().saturating_sub(1);
                for (round_index, round) in rounds.into_iter().enumerate() {
                    for (pid, value) in round {
                        let id = player_id_str(pid);
                        let name = self.player_name(&live, pid).unwrap_or_else(|| id.clone());
                        entries.push(manabrew_protocol::prompts::dice_rolled::DiceRollEntry {
                            label: Some(name),
                            highlighted: round_index == last_round && id == winner_id,
                            player_id: Some(id),
                            round: round_index as u32,
                            natural_results: vec![value],
                            final_results: vec![value],
                            ignored_rolls: vec![],
                        });
                    }
                }
                self.present_prompt(
                    &live,
                    PromptInput::DiceRolled(
                        manabrew_protocol::prompts::dice_rolled::DiceRolledInput {
                            presentation: PromptPresentation {
                                title: "Roll for first player".to_string(),
                                description: None,
                                text: None,
                                targets: Vec::new(),
                            },
                            sides,
                            rolls: entries,
                        },
                    ),
                    None,
                );
            }
            GameNotification::DiceRolled {
                player,
                sides,
                natural_results,
                final_results,
                ignored_rolls,
                source_card_id,
            } => {
                self.present_prompt(
                    &live,
                    PromptInput::DiceRolled(
                        manabrew_protocol::prompts::dice_rolled::DiceRolledInput {
                            presentation: PromptPresentation {
                                title: "Dice roll".to_string(),
                                description: None,
                                text: None,
                                targets: Vec::new(),
                            },
                            sides,
                            rolls: vec![manabrew_protocol::prompts::dice_rolled::DiceRollEntry {
                                label: None,
                                player_id: Some(player_id_str(player)),
                                round: 0,
                                natural_results,
                                final_results,
                                ignored_rolls,
                                highlighted: false,
                            }],
                        },
                    ),
                    source_card_id,
                );
            }
            GameNotification::SnapshotCreated {
                checkpoint_id,
                label,
            } => {
                if !self.responder.reads_game() {
                    let view = GameViewDto::clone(self.latest_view(&live));
                    self.responder.send_snapshot(GameSnapshotEventDto::new(
                        checkpoint_id,
                        label,
                        view,
                    ));
                }
            }
            GameNotification::GameOver => {
                self.emit_state(&live);
                self.present_prompt(
                    &live,
                    PromptInput::GameOver(manabrew_protocol::prompts::game_over::GameOverInput {}),
                    None,
                );
            }
            GameNotification::ManaPaymentResolved { .. } => {}
            GameNotification::ActivatedAbilityPaymentFailed { .. }
            | GameNotification::SpellPaymentFailed { .. } => {
                self.emit_state(&live);
            }
        }
    }

    fn take_restore_request(&mut self) -> Option<u64> {
        self.pending_restore_checkpoint.take()
    }
}
