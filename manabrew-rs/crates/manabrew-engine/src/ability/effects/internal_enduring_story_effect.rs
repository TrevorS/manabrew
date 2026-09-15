use super::EffectContext;

#[manabrew_engine_macros::spell_effect(InternalEnduringStoryEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let p = sa.activating_player;
    if !crate::player::has_lost(ctx.game, p) {
        let set_code = sa
            .source
            .and_then(|host| ctx.game.card(host).set_code.clone());
        ctx.game.player_set_enduring_story(p, true, set_code);
    }
}
