use super::*;

impl GameLoop {
    pub(crate) fn process_triggers(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
    ) -> bool {
        let _perf_scope = crate::perf::ParamsLookupScopeGuard::enter(
            crate::perf::ParamsLookupScope::PriorityTrigger,
        );
        let active = game.active_player();
        let mut pending = self.trigger_handler.run_waiting_matched_triggers(game);
        let mut ran = !pending.is_empty();
        let mut pushed = Vec::new();
        let start = game
            .player_order
            .iter()
            .position(|&p| p == active)
            .unwrap_or(0);
        let players: Vec<PlayerId> = (0..game.player_order.len())
            .map(|i| game.player_order[(start + i) % game.player_order.len()])
            .filter(|&p| p == active || game.player(p).is_alive())
            .collect();
        for (i, &player) in players.iter().enumerate() {
            if self.is_aborted() {
                game.game_over = true;
                break;
            }
            let last = i + 1 == players.len();
            let (mut group, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut pending)
                .into_iter()
                .partition(|(pt, ..)| last || pt.entry.spell_ability.activating_player == player);
            pending = rest;
            if player != active {
                let matched = self.trigger_handler.take_matched_triggers_of(game, player);
                ran |= !matched.is_empty();
                group.extend(matched);
                TriggerHandler::order_simultaneous_triggers(&mut group);
            }
            for (pt, ..) in group {
                if self.is_aborted() {
                    game.game_over = true;
                    break;
                }
                let is_static = pt.static_trigger;
                let trigger_host = pt
                    .entry
                    .spell_ability
                    .trigger_source
                    .zip(pt.entry.spell_ability.trigger_index);
                let depth = game.stack.len();
                pushed.extend(self.trigger_handler.process_pending_triggers(
                    &self.mana_pools,
                    game,
                    agents,
                    vec![pt],
                ));
                if let Some(effect) = trigger_host
                    .filter(|&(source, index)| {
                        Self::removes_host_after_trigger(game, source, index)
                    })
                    .map(|(source, _)| source)
                {
                    let controller = game.card(effect).controller;
                    game.remove_card_from_zone(ZoneType::Command, controller, effect);
                    game.card_mut(effect).zone = ZoneType::None;
                }
                if is_static && game.stack.len() > depth {
                    self.resolve_stack(game, agents);
                }
            }
        }
        if !pushed.is_empty() {
            self.invalidate_all_mana_undo();
        }
        for log in pushed {
            self.log_stack_push(&log.source_name, &log.player_name);
            if Self::trigger_trace_enabled() {
                eprintln!(
                    "[trigger-trace] T{} {:?} PUSHED trigger to stack: {} optional={} api={}",
                    game.turn.turn_number,
                    game.turn.phase,
                    log.source_name,
                    log.optional,
                    log.trigger_api
                );
            }
        }
        ran
    }

    fn removes_host_after_trigger(game: &GameState, host: CardId, index: usize) -> bool {
        let Some(card) = game.cards.get(host.index()) else {
            return false;
        };
        let Some(trigger) = card.triggers.get(index) else {
            return false;
        };
        let params = &trigger.base.card_trait_base;
        let mut remove_boon = card.boon;
        if let Some(amount) = params.get_param("BoonAmount") {
            let ability = trigger.ensure_ability(game, host, card.controller);
            let x = crate::svar::resolve_numeric_value(
                game,
                ability
                    .as_ref()
                    .unwrap_or(&SpellAbility::new_empty(Some(host), card.controller)),
                amount,
                0,
            );
            let y = card.get_ability_activated_this_game(ability.as_ref()) as i32;
            if y < x {
                remove_boon = false;
            }
        }
        (params.has_param("OneOff") && card.effect_source.is_some()) || remove_boon
    }

    pub(crate) fn run_static_state_triggers(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
    ) {
        for pt in self.trigger_handler.run_static_state_triggers(game) {
            if !self.trigger_handler.can_run_state_trigger(game, &pt) {
                continue;
            }
            let depth = game.stack.len();
            self.trigger_handler
                .process_pending_triggers(&self.mana_pools, game, agents, vec![pt]);
            if game.stack.len() > depth {
                self.resolve_stack(game, agents);
            }
        }
    }
}
