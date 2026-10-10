//! AnimateEffectBase — abstract base for animate effects.
//!
//! Mirrors Java's `AnimateEffectBase.java`.
//! Provides shared logic for `AnimateEffect` and `AnimateAllEffect`
//! that handles setting power/toughness, types, colors, and keywords.

use crate::spellability::SpellAbility;

/// Parsed animate parameters from a spell ability.
/// Captures the fields that animate effects need to apply.
#[derive(Debug, Clone, Default)]
pub struct AnimateParams {
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    pub add_types: Vec<String>,
    pub add_keywords: Vec<String>,
    pub colors: Option<Vec<String>>,
}

/// Parse shared animate parameters from a spell ability.
/// Used by both `animate_effect` and `animate_all_effect`.
pub fn parse_animate_params(sa: &SpellAbility) -> AnimateParams {
    AnimateParams {
        power: sa.ir.animate_power,
        toughness: sa.ir.animate_toughness,
        add_types: sa
            .ir
            .animate_types_text
            .as_deref()
            .map(|types| types.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default(),
        add_keywords: sa
            .ir
            .animate_keywords_text
            .as_deref()
            // Java splits Animate's Keywords$ on " & " (AnimateEffect.java:80), not on commas;
            // a keyword like "Protection from black" has no comma but several words.
            .map(|kws| {
                kws.split('&')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        colors: sa
            .ir
            .animate_colors_text
            .as_deref()
            .map(|colors| colors.split(',').map(|s| s.trim().to_string()).collect()),
    }
}
