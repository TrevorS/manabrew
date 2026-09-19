//! Phase module — turn structure, phase handling, and untap logic.
//!
//! Mirrors Java's `forge.game.phase` package.

pub mod extra_phase;
pub mod extra_turn;
pub mod phase_handler;
pub mod phase_type;
pub mod untap;

use crate::HashMap;

use forge_foundation::PhaseType;
use serde::{Deserialize, Serialize};

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

// Re-exports
pub use extra_phase::ExtraPhase;
pub use extra_turn::ExtraTurn;
pub use phase_handler::PhaseHandler;

/// A phase command — a deferred action to execute at a phase boundary.
/// Mirrors Java's `GameCommand` callbacks stored in `Phase`.
///
/// In Java, these are `Runnable`-like objects that modify game state.
/// In Rust, we represent them as an enum of known command types,
/// since we can't store closures in serializable state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PhaseCommand {
    AddController {
        player: PlayerId,
        controller: PlayerId,
        combat: bool,
    },
    RemoveController {
        player: PlayerId,
        controller: PlayerId,
    },
    /// `GameAction.exileEffect`: the effect card leaves the command zone.
    ExileEffect {
        effect: CardId,
    },
    RemoveKeyword {
        card: CardId,
        keyword: String,
    },
    RemoveGoad {
        card: CardId,
        player: PlayerId,
    },
}

impl PhaseCommand {
    pub fn run(self, game: &mut GameState) {
        match self {
            PhaseCommand::AddController {
                player,
                controller,
                combat,
            } => {
                if crate::player::has_lost(game, controller) {
                    return;
                }
                crate::player::add_controller(game, player, controller);
                let release = PhaseCommand::RemoveController { player, controller };
                if combat {
                    game.end_of_combat.add_until(None, release);
                } else {
                    game.cleanup.add_until(None, release);
                }
            }
            PhaseCommand::RemoveController { player, controller } => {
                crate::player::remove_controller(game, player, controller);
            }
            PhaseCommand::RemoveKeyword { card, keyword } => {
                game.card_mut(card).remove_changed_card_keywords(&keyword);
            }
            PhaseCommand::RemoveGoad { card, player } => {
                game.card_mut(card).remove_goad(player);
            }
            PhaseCommand::ExileEffect { effect } => {
                if game.card(effect).zone == forge_foundation::ZoneType::Command {
                    let controller = game.card(effect).controller;
                    game.remove_card_from_zone(
                        forge_foundation::ZoneType::Command,
                        controller,
                        effect,
                    );
                    game.card_mut(effect)
                        .set_zone(forge_foundation::ZoneType::None);
                }
            }
        }
    }
}

/// Phase instance — stores commands that execute at phase boundaries.
///
/// Mirrors Java's `Phase` class. Each `Phase` in Java holds lists of
/// `GameCommand` callbacks for "at <phase>", "until <phase>", and
/// per-player "until <player's> next <phase>" effects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Phase {
    #[allow(dead_code)]
    phase_type: Option<PhaseType>,
    /// Commands to execute "at" this phase.
    at: Vec<PhaseCommand>,
    /// Commands to execute "until" this phase (remove effects).
    until: Vec<PhaseCommand>,
    /// Per-player commands to execute "until <player's> next <phase>".
    until_map: HashMap<PlayerId, Vec<PhaseCommand>>,
    /// Per-player commands registered for end-of-phase execution.
    until_end_map: HashMap<PlayerId, Vec<PhaseCommand>>,
    /// Per-player commands staged to be moved to until_end_map.
    register_map: HashMap<PlayerId, Vec<PhaseCommand>>,
}

impl Phase {
    pub fn new(phase_type: PhaseType) -> Self {
        Phase {
            phase_type: Some(phase_type),
            ..Default::default()
        }
    }

    /// Clear all commands from this phase.
    /// Mirrors Java's `Phase.clearCommands()`.
    pub fn clear_commands(&mut self) {
        self.at.clear();
        self.until.clear();
        self.until_map.clear();
        self.until_end_map.clear();
        self.register_map.clear();
    }

    /// Add a command to execute "at" this phase.
    /// Mirrors Java's `Phase.addAt()`.
    pub fn add_at(&mut self, cmd: PhaseCommand) {
        self.at.insert(0, cmd);
    }

    /// Execute all "at" commands, draining the list.
    /// Mirrors Java's `Phase.executeAt()`.
    pub fn execute_at(&mut self) -> Vec<PhaseCommand> {
        std::mem::take(&mut self.at)
    }

    /// Add a command to execute "until" this phase (global or per-player).
    /// When called without a player, adds to the global until list.
    /// When called with a player, adds to the per-player until map.
    /// Mirrors Java's `Phase.addUntil()` (both overloads).
    pub fn add_until(&mut self, player: Option<PlayerId>, cmd: PhaseCommand) {
        if let Some(p) = player {
            self.until_map.entry(p).or_default().insert(0, cmd);
        } else {
            self.until.insert(0, cmd);
        }
    }

    /// Execute "until" commands, draining the list.
    /// When called without a player, executes global until commands.
    /// When called with a player, executes per-player until commands.
    /// Mirrors Java's `Phase.executeUntil()` (both overloads).
    pub fn execute_until(&mut self, player: Option<PlayerId>) -> Vec<PhaseCommand> {
        if let Some(p) = player {
            self.until_map.remove(&p).unwrap_or_default()
        } else {
            std::mem::take(&mut self.until)
        }
    }

    /// Register a command for end-of-phase execution for a player.
    /// Mirrors Java's `Phase.registerUntilEnd()`.
    pub fn register_until_end(&mut self, player: PlayerId, cmd: PhaseCommand) {
        self.register_map.entry(player).or_default().insert(0, cmd);
    }

    /// Add a command to the end-of-phase map for a player.
    /// Mirrors Java's `Phase.addUntilEnd()`.
    pub fn add_until_end(&mut self, player: PlayerId, cmd: PhaseCommand) {
        self.until_end_map.entry(player).or_default().insert(0, cmd);
    }

    /// Move registered commands to the until-end map.
    /// Mirrors Java's `Phase.registerUntilEndCommand()`.
    pub fn register_until_end_command(&mut self, player: PlayerId) {
        if let Some(cmds) = self.register_map.remove(&player) {
            self.until_end_map.entry(player).or_default().extend(cmds);
        }
    }

    /// Execute end-of-phase commands for a player.
    /// Mirrors Java's `Phase.executeUntilEndOfPhase()`.
    pub fn execute_until_end_of_phase(&mut self, player: PlayerId) -> Vec<PhaseCommand> {
        self.until_end_map.remove(&player).unwrap_or_default()
    }
}

/// Tracks the current turn state: whose turn, which phase, turn number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnState {
    pub turn_number: u32,
    pub active_player: PlayerId,
    pub phase: PhaseType,
    pub priority_player: PlayerId,
    pub num_players: u32,

    // Combat tracking
    pub combat_attackers_declared: bool,
    pub combat_blockers_declared: bool,
    /// Authoritative blocker -> attacker assignments for the current combat.
    pub combat_block_assignments: Vec<(CardId, CardId)>,

    // Per-turn flags
    pub drawn_for_turn: bool,
    pub n_upkeeps_this_turn: i32,
    #[serde(default)]
    pub n_combats_this_turn: i32,
    pub n_end_of_turns_this_turn: i32,
}

impl TurnState {
    pub fn new(active_player: PlayerId, num_players: u32) -> Self {
        TurnState {
            turn_number: 1,
            active_player,
            phase: PhaseType::Untap,
            priority_player: active_player,
            num_players,
            combat_attackers_declared: false,
            combat_blockers_declared: false,
            combat_block_assignments: vec![],
            drawn_for_turn: false,
            n_upkeeps_this_turn: 0,
            n_combats_this_turn: 0,
            n_end_of_turns_this_turn: 0,
        }
    }

    pub fn is_first_combat(&self) -> bool {
        self.n_combats_this_turn == 1
    }

    /// Advance to the next phase. Returns true if the turn ended (wrapped to Untap).
    pub fn advance_phase(&mut self) -> bool {
        let next = self.phase.next();
        let turn_ended = next == PhaseType::Untap && self.phase == PhaseType::Cleanup;
        self.phase = next;

        if turn_ended {
            self.turn_number += 1;
            self.combat_attackers_declared = false;
            self.combat_blockers_declared = false;
            self.combat_block_assignments.clear();
            self.drawn_for_turn = false;
        }

        // Reset combat flags when entering combat
        if self.phase == PhaseType::CombatBegin {
            self.combat_attackers_declared = false;
            self.combat_blockers_declared = false;
            self.combat_block_assignments.clear();
        }

        turn_ended
    }

    /// Advance to the next player's turn (for multiplayer).
    pub fn next_player_turn(&mut self, player_order: &[PlayerId]) {
        if let Some(pos) = player_order.iter().position(|&p| p == self.active_player) {
            let next = (pos + 1) % player_order.len();
            self.active_player = player_order[next];
            self.priority_player = self.active_player;
            self.turn_number += 1;
            self.combat_attackers_declared = false;
            self.combat_blockers_declared = false;
            self.combat_block_assignments.clear();
            self.drawn_for_turn = false;
        }
    }

    /// Advance to the next turn, consuming an extra turn if available.
    /// Returns `Some((player, skip_untap))` if the advancing player needs
    /// their skip_next_untap flag set on PlayerState; `None` otherwise.
    /// Mirrors Java's `PhaseHandler.handleNextTurn()`.
    pub fn next_turn_player(
        &self,
        extra_turns: &std::collections::VecDeque<ExtraTurn>,
        player_order: &[PlayerId],
    ) -> PlayerId {
        if let Some(extra_turn) = extra_turns.front() {
            return extra_turn.player;
        }
        let pos = player_order
            .iter()
            .position(|&p| p == self.active_player)
            .unwrap_or(0);
        player_order[(pos + 1) % player_order.len()]
    }

    pub fn advance_turn(
        &mut self,
        extra_turns: &mut std::collections::VecDeque<ExtraTurn>,
        player_order: &[PlayerId],
    ) -> Option<(PlayerId, bool)> {
        if let Some(extra_turn) = extra_turns.pop_front() {
            let player = extra_turn.player;
            self.active_player = player;
            self.priority_player = player;
            self.turn_number += 1;
            self.combat_attackers_declared = false;
            self.combat_blockers_declared = false;
            self.combat_block_assignments.clear();
            self.drawn_for_turn = false;
            if extra_turn.skip_untap {
                Some((player, true))
            } else {
                None
            }
        } else {
            self.next_player_turn(player_order);
            None
        }
    }

    pub fn is_main_phase(&self) -> bool {
        self.phase.is_main()
    }

    pub fn is_combat(&self) -> bool {
        self.phase.is_combat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_phases() {
        let mut ts = TurnState::new(PlayerId(0), 2);
        assert_eq!(ts.phase, PhaseType::Untap);

        ts.advance_phase();
        assert_eq!(ts.phase, PhaseType::Upkeep);

        ts.advance_phase();
        assert_eq!(ts.phase, PhaseType::Draw);
    }

    #[test]
    fn turn_wraps() {
        let mut ts = TurnState::new(PlayerId(0), 2);
        assert_eq!(ts.turn_number, 1);

        // Advance through all phases
        loop {
            let ended = ts.advance_phase();
            if ended {
                break;
            }
        }
        assert_eq!(ts.turn_number, 2);
        assert_eq!(ts.phase, PhaseType::Untap);
    }

    #[test]
    fn phase_commands() {
        let mut phase = Phase::new(PhaseType::Upkeep);
        let command = PhaseCommand::RemoveController {
            player: PlayerId(0),
            controller: PlayerId(1),
        };
        phase.add_at(command.clone());
        phase.add_until(None, command);

        let at_cmds = phase.execute_at();
        assert_eq!(at_cmds.len(), 1);

        let until_cmds = phase.execute_until(None);
        assert_eq!(until_cmds.len(), 1);

        // After execution, lists should be empty
        assert!(phase.execute_at().is_empty());
        assert!(phase.execute_until(None).is_empty());
    }

    #[test]
    fn phase_per_player_commands() {
        let mut phase = Phase::new(PhaseType::Cleanup);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        let command = PhaseCommand::RemoveController {
            player: p0,
            controller: p1,
        };
        phase.add_until(Some(p0), command.clone());
        phase.add_until(Some(p1), command);

        let p0_cmds = phase.execute_until(Some(p0));
        assert_eq!(p0_cmds.len(), 1);

        // p1 commands should still be there
        let p1_cmds = phase.execute_until(Some(p1));
        assert_eq!(p1_cmds.len(), 1);
    }
}
