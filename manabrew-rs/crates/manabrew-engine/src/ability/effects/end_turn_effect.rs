use super::EffectContext;

/// Resolve `SP$ EndTurn` — end the current turn.
///
/// Mirrors Java `EndTurnEffect.java`.
/// Sets the `end_turn_requested` flag on `GameState`. The game loop checks
/// this flag in the turn state machine to skip remaining phases and jump
/// directly to the Cleanup step.
///
/// # Card script examples
/// ```text
/// A:SP$ EndTurn
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `EndTurnEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(EndTurnEffect)]
fn resolve(ctx: &mut EffectContext, _sa: &crate::spellability::SpellAbility) {
    // CR 721.1a
    ctx.trigger_handler.clear_waiting_triggers();
    // Every spell and ability on the stack is EXILED, this one included — it still finishes
    // resolving. Popping the entries instead sent the card to its owner's graveyard.
    let stack_cards: Vec<crate::ids::CardId> = ctx
        .game
        .player_order
        .clone()
        .iter()
        .flat_map(|&pid| {
            ctx.game
                .cards_in_zone(forge_foundation::ZoneType::Stack, pid)
                .to_vec()
        })
        .collect();
    for card_id in stack_cards {
        let owner = ctx.game.card(card_id).owner;
        ctx.move_card(card_id, forge_foundation::ZoneType::Exile, owner);
    }
    while ctx.game.stack.pop().is_some() {}
    ctx.game.end_turn_requested = true;
}

#[cfg(test)]
mod tests {
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use crate::HashMap;

    use crate::ability::effects::EffectContext;
    use crate::agent::PassAgent;
    use crate::game::GameState;
    use crate::ids::PlayerId;
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::handler::TriggerHandler;

    #[test]
    fn end_turn_sets_flag() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);

        let sa = SpellAbility::new_simple(None, p0, "SP$ EndTurn");

        let mut th = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mp = vec![ManaPool::default(), ManaPool::default()];
        let templates = HashMap::default();
        let templates_variants = HashMap::default();
        let token_fallback = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter;
        let mut ctx = EffectContext {
            game: &mut game,
            combat: None,
            agents: &mut agents,
            trigger_handler: &mut th,
            token_templates: &templates,
            token_art_variants: &templates_variants,
            token_fallback: &token_fallback,
            edition_dates: &edition_dates,
            mana_pools: &mut mp,
            parent_target_card: None,
            rng: &mut rng_adapter,
        };
        super::EndTurnEffect::resolve(&mut ctx, &sa);

        assert!(ctx.game.end_turn_requested);
        // Stack should be empty after EndTurn
        assert!(ctx.game.stack.is_empty());
    }
}
