use super::*;

impl GameLoop {
    pub(crate) fn process_triggers(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
    ) {
        let _perf_scope = crate::perf::ParamsLookupScopeGuard::enter(
            crate::perf::ParamsLookupScope::PriorityTrigger,
        );
        let pending = self.trigger_handler.run_waiting_triggers(game);
        let mut pushed = Vec::new();
        for pt in pending {
            let is_static = pt
                .entry
                .spell_ability
                .trigger_source
                .zip(pt.entry.spell_ability.trigger_index)
                .and_then(|(source, index)| {
                    game.cards
                        .get(source.index())
                        .and_then(|card| card.triggers.get(index))
                })
                .is_some_and(|trigger| trigger.is_static());
            let depth = game.stack.len();
            pushed.extend(self.trigger_handler.process_pending_triggers(
                &self.mana_pools,
                game,
                agents,
                vec![pt],
            ));
            if is_static && game.stack.len() > depth {
                self.resolve_stack(game, agents);
            }
        }
        if !pushed.is_empty() {
            self.invalidate_all_mana_undo();
        }
        for log in pushed {
            self.log_stack_push(&log.source_name, &log.player_name);
            if std::env::var("FORGE_TRIGGER_TRACE").is_ok() {
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
    }
}
