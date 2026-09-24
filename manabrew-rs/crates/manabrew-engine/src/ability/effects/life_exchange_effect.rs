use super::EffectContext;
use crate::event::RunParams;
use crate::trigger::TriggerType;

/// Resolve `SP$ LifeExchange` — exchange life totals between two players.
///
/// Mirrors Java `LifeExchangeEffect.java`.
///
/// # Card script examples
/// ```text
/// A:SP$ LifeExchange | ValidTgts$ Player
/// A:SP$ LifeExchange | Defined$ Opponent
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `LifeExchangeEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(LifeExchangeEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let tgt_players = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa);
    let (mut p1, mut p2) = match tgt_players.as_slice() {
        [] => return,
        [p] => (sa.activating_player, *p),
        [a, b, ..] => (*a, *b),
    };

    let life1 = ctx.game.player(p1).life;
    let life2 = ctx.game.player(p2).life;
    let diff = (life1 - life2).abs();

    if life2 > life1 {
        std::mem::swap(&mut p1, &mut p2);
    }
    if diff > 0
        && ctx.game.player(p1).is_alive()
        && !crate::staticability::static_ability_cant_gain_lose_pay_life::cant_lose_life(
            ctx.game, p1,
        )
        && ctx.game.player(p2).is_alive()
        && !crate::staticability::static_ability_cant_gain_lose_pay_life::cant_gain_life(
            ctx.game, p2,
        )
    {
        let lost = super::life_lose_effect::lose_life(ctx, sa, p1, diff);
        super::life_gain_effect::gain_life(ctx, sa, p2, diff);
        if lost > 0 {
            ctx.trigger_handler.run_trigger(
                TriggerType::LifeLostAll,
                RunParams {
                    player: Some(p1),
                    life_amount: Some(lost),
                    source_card: sa.source,
                    source_sa: Some(sa.clone()),
                    ..Default::default()
                },
                false,
            );
            if crate::parsing::raw_has_key(&sa.ability_text, "RememberOwnLoss")
                && p1 == sa.activating_player
            {
                if let Some(source) = sa.source {
                    ctx.game.card_mut(source).add_remembered_cmc(lost);
                }
            }
        }
    }
    if crate::parsing::raw_has_key(&sa.ability_text, "RememberDifference") {
        if let Some(source) = sa.source {
            let difference = ctx.game.player(p1).life - ctx.game.player(p2).life;
            ctx.game.card_mut(source).add_remembered_cmc(difference);
        }
    }
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
    fn life_exchange_swaps_totals() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        // Set different life totals
        game.player_mut(p0).life = 5;
        game.player_mut(p1).life = 30;

        let sa = SpellAbility::new_simple(None, p0, "SP$ LifeExchange | Defined$ Opponent");

        let mut th = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mp = vec![ManaPool::default(), ManaPool::default()];
        let templates = HashMap::default();
        let templates_variants = HashMap::default();
        let token_fallback = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter::default();
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
        super::LifeExchangeEffect::resolve(&mut ctx, &sa);

        assert_eq!(ctx.game.player(p0).life, 30);
        assert_eq!(ctx.game.player(p1).life, 5);
    }
}
