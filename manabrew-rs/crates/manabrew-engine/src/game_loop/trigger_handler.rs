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
        let mut pending = self.trigger_handler.run_waiting_triggers(game);
        let mut ran = !pending.is_empty();
        let mut pushed = Vec::new();
        let mut player = active;
        loop {
            let next = game.next_player(player);
            let (mut group, rest): (Vec<_>, Vec<_>) =
                std::mem::take(&mut pending).into_iter().partition(|pt| {
                    next == active || pt.entry.spell_ability.activating_player == player
                });
            pending = rest;
            if player != active {
                let matched = self.trigger_handler.take_matched_triggers_of(game, player);
                ran |= !matched.is_empty();
                group.extend(matched);
            }
            for pt in group {
                let is_static = pt.static_trigger;
                let one_off_effect = pt
                    .entry
                    .spell_ability
                    .trigger_source
                    .zip(pt.entry.spell_ability.trigger_index)
                    .filter(|&(source, index)| {
                        game.cards.get(source.index()).is_some_and(|card| {
                            card.effect_source.is_some()
                                && card.triggers.get(index).is_some_and(|trigger| {
                                    trigger.base.card_trait_base.has_param("OneOff")
                                })
                        })
                    })
                    .map(|(source, _)| source);
                let depth = game.stack.len();
                pushed.extend(self.trigger_handler.process_pending_triggers(
                    &self.mana_pools,
                    game,
                    agents,
                    vec![pt],
                ));
                if let Some(effect) = one_off_effect {
                    let controller = game.card(effect).controller;
                    game.remove_card_from_zone(ZoneType::Command, controller, effect);
                    game.card_mut(effect).zone = ZoneType::None;
                }
                if is_static && game.stack.len() > depth {
                    self.resolve_stack(game, agents);
                }
            }
            if next == active {
                break;
            }
            player = next;
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
