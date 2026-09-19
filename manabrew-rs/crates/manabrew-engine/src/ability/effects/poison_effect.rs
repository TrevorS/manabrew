use super::{resolve_numeric_svar, EffectContext};
use crate::ability::ability_ir::EffectIr;
use crate::spellability::SpellAbility;

/// Resolve `DB$ Poison` / `SP$ Poison` — add poison counters to players.
///
/// Mirrors Java `PoisonEffect.java` (~45 lines).
///
/// Real card patterns:
/// - `DB$ Poison | Defined$ Player | Num$ 1`    (Ichor Rats — all players)
/// - `DB$ Poison | Defined$ Opponent | Num$ 1`  (Prologue to Phyresis)
/// - `DB$ Poison | Defined$ You | Num$ 1`       (Phyrexian Vatmother)
/// - `DB$ Poison | ValidTgts$ Player | Num$ 1`  (Hand of the Praetors — targeted)
/// - `DB$ Poison | Defined$ TriggeredTarget | Num$ 1` (trigger context)
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `PoisonEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(PoisonEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = resolve_poison_amount(ctx, sa);
    if amount == 0 {
        return;
    }

    for target_player in crate::ability::spell_ability_effect::get_target_players(ctx.game, sa) {
        if !ctx.game.player(target_player).is_alive()
            || crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_player(
                &ctx.game.cards,
                target_player,
                &crate::card::CounterType::Poison,
            )
        {
            continue;
        }
        if amount > 0 {
            ctx.game.player_add_poison(target_player, amount);
        } else {
            ctx.game.player_remove_poison(target_player, -amount);
        }
    }
}

fn resolve_poison_amount(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    if let Some(EffectIr::Poison(ir)) = &sa.ir.effect {
        if let Some(amount) = &ir.amount {
            let resolved = amount.resolve_for_spell_ability(ctx.game, sa, 1);
            #[cfg(debug_assertions)]
            debug_assert_eq!(
                resolved,
                resolve_numeric_svar(ctx.game, sa, "Num", 1),
                "compiled Poison amount diverged from string params"
            );
            return resolved;
        }
    }

    resolve_numeric_svar(ctx.game, sa, "Num", 1)
}
