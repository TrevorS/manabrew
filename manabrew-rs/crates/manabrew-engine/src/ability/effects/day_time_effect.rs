//! DayTime effect — switch between Day and Night.
//!
//! Ported 1:1 from Java's `DayTimeEffect.java`.
//! Day/Night cycle: set the game to Day, Night, or Switch.

use super::EffectContext;
use crate::ability::ability_ir::DayTimeValue;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DayTimeEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DayTimeEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let value = match sa.ir.day_time_value {
        Some(DayTimeValue::Day) => false,
        Some(DayTimeValue::Night) => true,
        Some(DayTimeValue::Switch) => !ctx.game.get_day_time().unwrap_or(false),
        None => return,
    };
    ctx.game.set_day_time(Some(value), ctx.trigger_handler);
}
