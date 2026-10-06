//! CR 613 layer system — continuous effect application.
//!
//! Mirrors Java Forge's `GameAction.checkStaticAbilities()` and
//! `StaticAbilityContinuous.applyContinuousAbility()`.
//!
//! # How to use
//!
//! Call [`apply_continuous_effects`] after any event that could change which
//! static abilities are active (card entering/leaving the battlefield, spell
//! resolution, etc.):
//!
//! ```ignore
//! apply_continuous_effects(&mut game);
//! ```
//!
//! The function resets all derived fields (`static_power_modifier`,
//! `static_toughness_modifier`, `static_set_power`, `static_set_toughness`,
//! `granted_keywords`, `cant_attack_static`, `cant_block_static`) and
//! recomputes them from scratch.
//!
//! # Layer ordering (CR 613)
//!
//! 1. Copy effects (not yet implemented)
//! 2. Control-changing
//! 3. Text-changing (not yet implemented)
//! 4. Type-changing  → [`Layer::Type`]
//! 5. Color-changing → [`Layer::Color`]
//! 6. Ability-adding/removing → [`Layer::Ability`]
//! 7a. CDA P/T → [`Layer::Characteristic`]
//! 7b. Set P/T → [`Layer::SetPT`]
//! 7c. Modify P/T → [`Layer::ModifyPT`]
//! 7d. Counters (handled intrinsically by `Card::power()`)
//! 8. Forge rules-modifying layer → [`Layer::Rules`]

use std::collections::BTreeMap;
use std::sync::Arc;

use forge_foundation::{CardTypeLine, CoreType, Supertype, ZoneType};

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::replacement::replacement_effect::ReplacementType;
use crate::staticability::{Layer, StaticAbility, StaticMode};

// ── Effect collection ────────────────────────────────────────────────────────

/// An effect ready to be applied to a specific target card.
struct PendingEffect {
    /// CR 613 layer (used for sort ordering).
    layer: Layer,
    /// Target card index.
    target: CardId,
    /// Payload.
    kind: EffectKind,
}

enum EffectKind {
    SetController {
        controller: PlayerId,
    },
    AddPT {
        power: i32,
        toughness: i32,
    },
    SetPT {
        power: Option<i32>,
        toughness: Option<i32>,
    },
    RemoveAllCardTraits {
        timestamp: i64,
        static_id: i64,
    },
    GrantKeyword {
        keyword: String,
        idx: i64,
        static_id: i64,
        timestamp: Option<u64>,
    },
    SetName(String),
    /// Grant an activated ability (from AddAbility$). The string is the ability text.
    GrantAbility {
        text: String,
        svars: BTreeMap<String, String>,
        original_host: Option<CardId>,
        original_ability: Option<(CardId, usize)>,
    },
    /// Add a type/subtype to the card (`AddType$`). Mirrors Java layer 4.
    AddType(String),
    RemoveCardTypes,
    RemoveCreatureTypes,
    RemoveLandTypes,
    RemoveArtifactTypes,
    ReapplyChangedCardTypes(u64),
    /// Grant a triggered ability (from AddTrigger$). The string is the raw trigger text.
    GrantTrigger {
        text: String,
        svars: BTreeMap<String, String>,
        original_host: Option<CardId>,
    },
    GrantReplacement {
        text: String,
        svars: BTreeMap<String, String>,
    },
    MayLookAt {
        static_id: i64,
        players: Vec<PlayerId>,
    },
}

// ── Public API ───────────────────────────────────────────────────────────────

/// CR 613 layers a `Continuous` static contributes to.
///
/// Mirrors Java `StaticAbility.generateLayer()`. The classification is derived
/// at runtime from the authored params; `StaticAbilityIr` stores the parsed DSL
/// facts only.
const CONTINUOUS_LAYERS_WITH_DEPENDENCY: [Layer; 6] = [
    Layer::Control,
    Layer::Text,
    Layer::Type,
    Layer::Ability,
    Layer::Characteristic,
    Layer::SetPT,
];

pub fn classify_static_layers(sa: &StaticAbility) -> Vec<Layer> {
    if !sa.check_mode(&StaticMode::Continuous) {
        return Vec::new();
    }
    let mut layers = Vec::new();
    for_each_static_layer(sa, |layer| push_unique_layer(&mut layers, layer));
    if layers.is_empty() {
        layers.push(Layer::Rules);
    }
    layers
}

fn first_static_layer(sa: &StaticAbility) -> Layer {
    let mut first: Option<Layer> = None;
    if sa.check_mode(&StaticMode::Continuous) {
        for_each_static_layer(sa, |layer| {
            first = Some(first.map_or(layer, |first| first.min(layer)));
        });
    }
    first.unwrap_or(Layer::Rules)
}

fn for_each_static_layer(sa: &StaticAbility, mut layer: impl FnMut(Layer)) {
    let ir = &sa.ir;
    if ir.gain_control_param {
        layer(Layer::Control);
    }
    if ir.has_text_layer_key {
        layer(Layer::Text);
    }
    if ir.has_type_layer_key {
        layer(Layer::Type);
    }
    if ir.has_color_layer_key {
        layer(Layer::Color);
    }
    if ir.has_ability_layer_key {
        layer(Layer::Ability);
    }
    if ir.set_power || ir.set_toughness {
        layer(if ir.characteristic_defining {
            Layer::Characteristic
        } else {
            Layer::SetPT
        });
    }
    if ir.add_power || ir.add_toughness {
        layer(Layer::ModifyPT);
    }
    if ir.has_rules_layer_key {
        layer(Layer::Rules);
    }
}

fn static_layer_trait_id(source_id: CardId, sa_idx: usize) -> i64 {
    -(((source_id.index() as i64) + 1) * 10_000 + sa_idx as i64 + 1)
}

fn push_unique_layer(layers: &mut Vec<Layer>, layer: Layer) {
    if !layers.contains(&layer) {
        layers.push(layer);
    }
}

fn type_line_has_token(type_line: &CardTypeLine, token: &str) -> bool {
    if let Some(st) = Supertype::from_name(token) {
        return type_line.supertypes.contains(&st);
    }
    if let Some(ct) = CoreType::from_name(token) {
        return type_line.core_types.contains(&ct);
    }
    type_line
        .subtypes
        .iter()
        .any(|subtype| subtype.eq_ignore_ascii_case(token))
}

/// Recompute all continuously-applied static-ability effects for the current
/// game state.
///
/// This is the Rust equivalent of Java Forge's
/// `GameAction.checkStaticAbilities()` + `StaticAbilityContinuous.applyContinuousAbility()`.
///
/// **Call this** after:
/// - Any permanent enters or leaves the battlefield.
/// - Any spell or ability resolves.
/// - Any triggered ability fires.
/// - Before querying `can_attack()` / `can_block()` for combat legality.
pub fn apply_continuous_effects(game: &mut GameState) {
    if game.hold_checking_static_abilities {
        return;
    }
    let skip = game.layer_key_after_pass.0.as_ref() == Some(&game.layer_key());
    #[cfg(feature = "layer-skip-stats")]
    super::layer_skip_stats::record(game, skip);
    if skip {
        if verify_layer_skip_mode().checks(game) {
            verify_layer_skip(game);
        }
        return;
    }
    let _perf_timer = crate::perf::ScopeTimer::start(
        crate::perf::Metric::ContinuousEffectsCalls,
        crate::perf::Metric::ContinuousEffectsNs,
    );
    let _params_lookup_scope =
        crate::perf::ParamsLookupScopeGuard::enter(crate::perf::ParamsLookupScope::Continuous);
    // ── 1. Reset all derived fields ──────────────────────────────────────
    let card_names_unchanged = game.card_names_unchanged;
    for card in game.cards.iter_mut() {
        if static_layer_reset_is_noop(card, card_names_unchanged) {
            continue;
        }
        let card = Arc::make_mut(card);
        card.clear_static_layer_changed_card_traits();
        // Remove abilities granted by continuous effects (AddAbility$).
        // The base_ability_count tracks how many abilities the card originally had.
        if card.activated_abilities.len() > card.base_ability_count {
            card.activated_abilities.truncate(card.base_ability_count);
        }
        for (ability_idx, ability) in card.activated_abilities.iter_mut().enumerate() {
            ability.ability_index = ability_idx;
        }
        let intrinsic_trigger_count = card.base_trigger_count + card.pump_trigger_count;
        if card.triggers.len() > intrinsic_trigger_count {
            card.triggers.truncate(intrinsic_trigger_count);
        }
        card.static_power_modifier = 0;
        card.static_toughness_modifier = 0;
        card.static_set_power = None;
        card.static_set_toughness = None;
        card.granted_keywords.clear();
        for inst in std::mem::take(&mut card.pump_keywords_removed_by_statics) {
            card.pump_keywords.insert(inst);
        }
        card.remove_changed_name();
        card.granted_svars.clear();
        // Restore the pre-layer type line before applying AddType$ statics.
        if let Some(type_line) = card.static_type_line_base.take() {
            card.set_type_line(type_line);
        }
        card.static_added_subtypes.clear();
        card.cant_block_static = false;
        card.may_look.retain(|&(timestamp, _)| timestamp >= 0);
    }
    for player in game.players.iter_mut() {
        player.max_land_plays_per_turn = 1;
        player.unlimited_land_plays = false;
        player.static_keywords.clear();
    }

    // ── 1b. Keyword-derived restrictions ────────────────────────────────
    // Unleash: creatures with Unleash keyword and a +1/+1 counter can't block.
    for card in game.cards.iter_mut() {
        if card.zone == ZoneType::Battlefield
            && card.counter_count(&crate::card::CounterType::P1P1) > 0
            && card.has_keyword("Unleash")
        {
            Arc::make_mut(card).cant_block_static = true;
        }
    }

    // Decayed: Java `CardFactoryUtil:3870` hangs a `Mode$ CantBlock | ValidCard$ Creature.Self`
    // static off the keyword instance, so a keyword gained from a decayed counter carries it.
    // `keyword_gen` only builds that static for a printed keyword.
    for card in game.cards.iter_mut() {
        if card.zone == ZoneType::Battlefield
            && card.is_creature()
            && (card.has_keyword("Decayed")
                || card.counters.iter().any(|(counter, &amount)| {
                    amount > 0
                        && crate::card::counter_keyword_type::CounterKeywordType::keyword(counter)
                            == Some("Decayed")
                }))
        {
            Arc::make_mut(card).cant_block_static = true;
        }
    }

    // Impending: Java `CardFactoryUtil:3937` gives the card a
    // `RemoveType$ Creature` static affecting `Card.Self+impended+counters_GE1_TIME`,
    // so a permanent cast for its impending cost is not a creature until the last
    // time counter comes off.
    for card in game.cards.iter_mut() {
        if card.zone == ZoneType::Battlefield
            && card.counter_count(&crate::card::CounterType::Time) > 0
            && card.get_impending_cost().is_some()
            && card.cast_sa.as_ref().is_some_and(|sa| {
                sa.alt_cost == Some(crate::spellability::AlternativeCost::Impending)
            })
        {
            let card = Arc::make_mut(card);
            if card.static_type_line_base.is_none() {
                card.static_type_line_base = Some(card.type_line.clone());
            }
            card.remove_type("Creature");
            let mut sanitized = card.type_line.clone();
            if sanitize_subtypes(&mut sanitized) {
                card.type_line = sanitized;
                card.update_types();
            }
        }
    }

    for player_idx in 0..game.player_order.len() {
        let pid = game.player_order[player_idx];
        let player = game.player_mut(pid);
        player.max_hand_size = 7;
        player.unlimited_hand_size = false;
    }
    let player_ids: Vec<PlayerId> = game.player_order.clone();
    for &pid in &player_ids {
        let source_cards: Vec<CardId> = game
            .cards_in_zone(ZoneType::Battlefield, pid)
            .iter()
            .chain(game.cards_in_zone(ZoneType::Command, pid))
            .copied()
            .collect();
        for source_id in source_cards {
            let static_ability_count = game.card(source_id).static_abilities.len();
            for sa_idx in 0..static_ability_count {
                let card = game.card(source_id);
                let sa = &card.static_abilities[sa_idx];
                if sa.ir.set_max_hand_size.is_none() && sa.ir.raise_max_hand_size.is_none() {
                    continue;
                }
                if !sa.check_mode(&StaticMode::Continuous) {
                    continue;
                }
                let affected = sa.ir.affected_text.as_deref().unwrap_or("");
                if !affected.eq_ignore_ascii_case("You") {
                    continue;
                }
                if !sa.check_conditions(card, game) {
                    continue;
                }
                let controller = card.controller;
                let set_value = sa.ir.set_max_hand_size.clone();
                let raise_value = sa.ir.raise_max_hand_size.clone();
                if let Some(value) = set_value {
                    let player = game.player_mut(controller);
                    if value.eq_ignore_ascii_case("Unlimited") {
                        player.unlimited_hand_size = true;
                    } else if let Ok(n) = value.parse::<i32>() {
                        player.max_hand_size = n;
                    }
                }
                if let Some(value) = raise_value {
                    if let Ok(n) = value.parse::<i32>() {
                        let player = game.player_mut(controller);
                        player.max_hand_size = player.max_hand_size.saturating_add(n);
                    }
                }
            }
        }
    }

    let controllers_before: Vec<(CardId, PlayerId)> = game
        .cards
        .iter()
        .map(|card| (card.id, card.controller))
        .collect();
    // ── 2. Build list of effects-to-apply (deferred to allow sorting) ────
    let mut pending: Vec<PendingEffect> = Vec::new();
    let mut staged: Vec<(usize, PendingEffect)> = Vec::new();
    let mut type_changed: Vec<CardId> = Vec::new();
    let mut control_changed: Vec<CardId> = Vec::new();
    let mut granted_keyword_replacements: indexmap::IndexMap<
        CardId,
        Vec<crate::replacement::replacement_effect::ReplacementEffect>,
    > = indexmap::IndexMap::new();
    let mut cant_block_targets: Vec<CardId> = Vec::new();
    let mut granted_player_rules: Vec<(CardId, StaticAbility)> = Vec::new();
    let mut granted_statics: Vec<(CardId, StaticAbility)> = Vec::new();

    for card in &game.cards {
        for (counter, &amount) in &card.counters {
            if amount <= 0 {
                continue;
            }
            if let Some(keyword) =
                crate::card::counter_keyword_type::CounterKeywordType::keyword(counter)
            {
                staged.push((
                    usize::MAX,
                    PendingEffect {
                        layer: Layer::Ability,
                        target: card.id,
                        kind: EffectKind::GrantKeyword {
                            keyword: keyword.to_string(),
                            idx: 1,
                            static_id: 0,
                            timestamp: None,
                        },
                    },
                ));
            } else if matches!(counter, crate::card::CounterType::Named(name) if name == "HONE")
                && card.zone == ZoneType::Battlefield
                && card.type_line.has_subtype("Equipment")
            {
                if let Some(equipped) = card
                    .attached_to
                    .filter(|&equipped| game.card(equipped).is_creature())
                {
                    staged.push((
                        usize::MAX,
                        PendingEffect {
                            layer: Layer::ModifyPT,
                            target: equipped,
                            kind: EffectKind::AddPT {
                                power: amount,
                                toughness: 0,
                            },
                        },
                    ));
                }
            }
        }
    }

    let mut effect_order: Vec<(CardId, usize, Layer, bool, u64)> = game
        .cards
        .iter()
        .flat_map(|card| {
            card.static_abilities
                .iter()
                .enumerate()
                .filter(move |(_, sa)| !card.face_down && sa.zones_check(card.zone))
                .map(move |(sa_idx, sa)| {
                    (
                        card.id,
                        sa_idx,
                        first_static_layer(sa),
                        sa.ir.characteristic_defining,
                        card.layer_timestamp,
                    )
                })
        })
        .collect();
    effect_order.sort_by_key(|&(_, _, _, cda, timestamp)| (!cda, timestamp));
    let mut initial: Vec<(
        CardId,
        usize,
        Option<Box<StaticAbility>>,
        bool,
        usize,
        Layer,
    )> = effect_order
        .into_iter()
        .enumerate()
        .map(|(seq, (card_id, sa_idx, layer, ..))| (card_id, sa_idx, None, false, seq, layer))
        .collect();
    initial.sort_by_key(|entry| entry.5);
    let mut statics: std::collections::VecDeque<(
        CardId,
        usize,
        Option<Box<StaticAbility>>,
        bool,
        usize,
        Layer,
    )> = initial.into();
    let mut chosen: crate::HashSet<(CardId, usize)> = crate::HashSet::default();
    let mut queued: Vec<usize> = Vec::new();
    let mut affected: Vec<CardId> = Vec::new();
    let present_counts = crate::card::valid_filter::present_memo::Scope::enter();
    while let Some((source_id, sa_idx, mut owned, is_granted, seq, first_layer)) =
        statics.pop_front()
    {
        let before = Some((first_layer, seq));
        if staged
            .iter()
            .any(|(effect_seq, effect)| is_staged_before(*effect_seq, effect, before))
        {
            let losing_traits: Vec<CardId> = staged
                .iter()
                .filter(|(effect_seq, effect)| {
                    is_staged_before(*effect_seq, effect, before)
                        && matches!(effect.kind, EffectKind::RemoveAllCardTraits { .. })
                })
                .map(|(_, effect)| effect.target)
                .collect();
            if !losing_traits.is_empty() {
                let queued = statics
                    .iter_mut()
                    .map(|(card_id, idx, owned, ..)| (*card_id, *idx, owned));
                for (card_id, idx, owned) in
                    std::iter::once((source_id, sa_idx, &mut owned)).chain(queued)
                {
                    if owned.is_none() && losing_traits.contains(&card_id) {
                        *owned = Some(Box::new(game.card(card_id).static_abilities[idx].clone()));
                    }
                }
            }
            flush_pending_effects(
                game,
                &mut staged,
                before,
                &mut type_changed,
                &mut control_changed,
                &mut granted_keyword_replacements,
            );
        }
        if CONTINUOUS_LAYERS_WITH_DEPENDENCY.contains(&first_layer)
            && !chosen.remove(&(source_id, sa_idx))
        {
            let sa = static_ability_at(game, source_id, sa_idx, &owned);
            let source = game.card(source_id);
            let applies_now = !sa.ir.characteristic_defining
                && sa.zones_check(source.zone)
                && sa.check_conditions(source, game);
            queued.clear();
            queued.extend(
                statics
                    .iter()
                    .enumerate()
                    .filter(|(_, queued)| queued.5 == first_layer)
                    .map(|(position, _)| position),
            );
            if applies_now && !queued.is_empty() {
                let statics_for_layer: Vec<(CardId, usize, StaticAbility)> =
                    std::iter::once((source_id, sa_idx, sa.clone()))
                        .chain(queued.iter().map(|&position| {
                            let (other_id, other_idx, other_owned, ..) = &statics[position];
                            (
                                *other_id,
                                *other_idx,
                                static_ability_at(game, *other_id, *other_idx, other_owned).clone(),
                            )
                        }))
                        .collect();
                let picked = find_static_ability_to_apply(game, first_layer, &statics_for_layer);
                if picked > 0 {
                    let position = queued[picked - 1];
                    let mut first = statics.remove(position).expect("position is in the queue");
                    let later_seq = first.4;
                    for queued in statics.iter_mut() {
                        if queued.4 >= seq && queued.4 < later_seq {
                            queued.4 += 1;
                        }
                    }
                    first.4 = seq;
                    chosen.insert((first.0, first.1));
                    statics.push_front((
                        source_id,
                        sa_idx,
                        owned,
                        is_granted,
                        seq + 1,
                        first_layer,
                    ));
                    statics.push_front(first);
                    continue;
                }
            }
        }
        {
            let pending_before = pending.len();
            let sa = static_ability_at(game, source_id, sa_idx, &owned);
            if !sa.zones_check(game.card(source_id).zone) {
                continue;
            }
            // Full static-ability condition gate (IsPresent$, CheckSVar$, Condition$, etc.).
            // Mirrors Java static ability checks before applying continuous effects.
            if !sa.check_conditions(game.card(source_id), game) {
                continue;
            }
            if !is_granted && owned.is_some() && !static_exists(game, source_id, sa_idx, sa) {
                continue;
            }

            if sa.check_mode(&StaticMode::Continuous)
                && (sa.ir.add_keyword_text.is_some() || sa.ir.adjust_land_plays_text.is_some())
            {
                let affected_players = affected_players_for_static(game, source_id, sa);
                let add_keywords = sa.ir.add_keyword_text.clone();
                let adjust_land_plays = sa.ir.adjust_land_plays_text.clone();
                apply_player_keyword_effects(game, &affected_players, add_keywords.as_deref());
                apply_player_rules_effects(
                    game,
                    source_id,
                    &affected_players,
                    adjust_land_plays.as_deref(),
                );
            }
            let sa = static_ability_at(game, source_id, sa_idx, &owned);
            static_affected_cards_into(game, source_id, sa, &mut affected);
            for &target in &affected {
                apply_continuous_ability(
                    game,
                    source_id,
                    sa_idx,
                    sa,
                    target,
                    &mut pending,
                    &mut granted_statics,
                    &mut granted_player_rules,
                );
                if sa.check_mode(&StaticMode::CantBlock) {
                    cant_block_targets.push(target);
                }
            }

            // Keep in sync with GameAction.checkStaticAbilities: a static granted in the
            // ability layer starts applying there, so its earlier layers never apply.
            if is_granted {
                let kept: Vec<PendingEffect> = pending
                    .drain(pending_before..)
                    .filter(|effect| effect.layer >= Layer::Ability)
                    .collect();
                pending.extend(kept);
            }
            for (offset, (target, granted)) in granted_statics.drain(..).enumerate().rev() {
                let granted_idx = game.card(target).static_abilities.len() + offset;
                statics.push_front((
                    target,
                    granted_idx,
                    Some(Box::new(granted)),
                    true,
                    seq,
                    first_layer,
                ));
            }
        }
        staged.extend(pending.drain(..).map(|effect| (seq, effect)));
    }
    drop(present_counts);

    let mut granted_by_target: indexmap::IndexMap<CardId, Vec<StaticAbility>> =
        indexmap::IndexMap::new();
    for (target, mut granted) in granted_player_rules {
        let affected_players = affected_players_for_static(game, target, &granted);
        apply_player_rules_effects(
            game,
            target,
            &affected_players,
            granted.ir.adjust_land_plays_text.as_deref(),
        );
        granted.base.set_host_card_id(target);
        granted_by_target.entry(target).or_default().push(granted);
    }
    for (target, static_abilities) in granted_by_target {
        game.card_mut(target).add_changed_card_traits(
            crate::card::card_trait_changes::CardTraitChanges {
                static_abilities,
                ..Default::default()
            },
            0,
            -1,
        );
    }

    for target in cant_block_targets {
        game.card_mut(target).cant_block_static = true;
    }

    flush_pending_effects(
        game,
        &mut staged,
        None,
        &mut type_changed,
        &mut control_changed,
        &mut granted_keyword_replacements,
    );
    type_changed.sort_unstable_by_key(|id| id.0);
    type_changed.dedup();
    let lost_static_control: Vec<(CardId, PlayerId)> = game
        .cards
        .iter()
        .filter(|card| !control_changed.contains(&card.id))
        .filter_map(|card| card.static_control_base.map(|base| (card.id, base)))
        .collect();
    for (card, base) in lost_static_control {
        game.card_mut(card).static_control_base = None;
        game.set_temp_controller(card, base);
    }
    for (card, original) in controllers_before {
        game.controller_change_zone_correction(card, original);
    }

    for (target, replacements) in granted_keyword_replacements {
        game.card_mut(target).add_changed_card_traits(
            crate::card::card_trait_changes::CardTraitChanges {
                replacements,
                ..Default::default()
            },
            0,
            -2,
        );
    }
    for card in game.cards.iter_mut() {
        let perpetual = card.perpetual_keywords();
        if perpetual.is_empty() && card.pump_keywords.is_empty() && card.granted_keywords.is_empty()
        {
            continue;
        }
        let card = Arc::make_mut(card);
        let mut keywords = perpetual;
        keywords.extend(card.pump_keywords.as_string_list());
        card.generate_keyword_triggers_for(&keywords);
        keywords.extend(card.granted_keywords.as_string_list());
        card.generate_keyword_activated_abilities(&keywords);
    }

    for target in type_changed {
        let card = game.card_mut(target);
        let mut sanitized = card.type_line.clone();
        if sanitize_subtypes(&mut sanitized) {
            if card.static_type_line_base.is_none() {
                card.static_type_line_base = Some(card.type_line.clone());
            }
            card.type_line = sanitized;
            card.update_types();
        }
    }

    // Rebuild intrinsic basic-land mana abilities after type-changing continuous
    // effects have been applied (e.g. Urborg making lands into Swamps).
    for card in game.cards.iter_mut() {
        if card.zone == ZoneType::Battlefield && card.lacks_basic_land_mana_abilities() {
            Arc::make_mut(card).apply_land_trait_changes();
        }
    }
    game.layer_key_after_pass.0 = Some(game.layer_key());
    #[cfg(feature = "layer-skip-stats")]
    {
        game.layer_key_after_pass.1 = super::layer_skip_stats::capture(game);
    }
}

fn verify_layer_skip_mode() -> crate::perf::CacheVerify {
    static MODE: std::sync::OnceLock<crate::perf::CacheVerify> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| crate::perf::cache_verify_mode("manabrew_engine_VERIFY_LAYER_SKIP"))
}

fn verify_layer_skip(game: &GameState) {
    let mut check = game.clone();
    check.layer_key_after_pass.0 = None;
    apply_continuous_effects(&mut check);
    for (after, before) in check.cards.iter().zip(game.cards.iter()) {
        if !Arc::ptr_eq(after, before) {
            assert_eq!(
                format!("{after:?}"),
                format!("{before:?}"),
                "apply_continuous_effects skipped a pass that changes card {}",
                before.id.0
            );
        }
    }
    let mut rest = game.clone();
    rest.cards.clear();
    check.cards.clear();
    assert_eq!(
        format!("{check:?}"),
        format!("{rest:?}"),
        "apply_continuous_effects skipped a pass that changes the game"
    );
}

#[allow(clippy::too_many_arguments)]
fn apply_continuous_ability(
    game: &GameState,
    source_id: CardId,
    sa_idx: usize,
    sa: &StaticAbility,
    target: CardId,
    pending: &mut Vec<PendingEffect>,
    granted_statics: &mut Vec<(CardId, StaticAbility)>,
    granted_player_rules: &mut Vec<(CardId, StaticAbility)>,
) {
    let source_card = game.card(source_id);
    // CharacteristicDefining statics always affect only the host card.
    // Mirrors Java StaticAbilityContinuous.getAffectedCards() line 1036.
    let is_cda = sa.ir.characteristic_defining;
    if sa.check_mode(&StaticMode::Continuous) {
        if let Some(gain_control) = sa.ir.gain_control_text.as_deref() {
            let new_controller = match gain_control {
                "You" | "YouCtrl" => Some(source_card.controller),
                "Opponent" => Some(game.opponent_of(source_card.controller)),
                _ => None,
            };
            if let Some(controller) = new_controller {
                pending.push(PendingEffect {
                    layer: Layer::Control,
                    target,
                    kind: EffectKind::SetController { controller },
                });
            }
        }

        let add_power = sa.ir.add_power_text.as_deref();
        let add_toughness = sa.ir.add_toughness_text.as_deref();
        if add_power.is_some() || add_toughness.is_some() {
            let p = resolve_add_pt_value(game, source_id, target, add_power);
            let t = resolve_add_pt_value(game, source_id, target, add_toughness);
            pending.push(PendingEffect {
                layer: Layer::ModifyPT,
                target,
                kind: EffectKind::AddPT {
                    power: p,
                    toughness: t,
                },
            });
        }

        pending.extend(
            type_effects(game, source_card, sa, target)
                .into_iter()
                .map(|kind| PendingEffect {
                    layer: Layer::Type,
                    target,
                    kind,
                }),
        );

        let set_power = sa.ir.set_power_text.as_deref();
        let set_toughness = sa.ir.set_toughness_text.as_deref();
        let target_card = game.card(target);
        let newer_animate_pt = target_card
            .animate_state
            .as_ref()
            .and_then(|state| state.new_pt_timestamp)
            .max(target_card.permanent_new_pt_timestamp)
            .is_some_and(|timestamp| is_cda || timestamp > source_card.layer_timestamp);
        if (set_power.is_some() || set_toughness.is_some()) && !newer_animate_pt {
            let sp = resolve_set_pt_value(game, source_id, target, set_power);
            let st = resolve_set_pt_value(game, source_id, target, set_toughness);
            // Java parity: CharacteristicDefining$ True routes
            // SetP/T through layer 7a, otherwise 7b.
            let layer = if is_cda {
                Layer::Characteristic
            } else {
                Layer::SetPT
            };
            pending.push(PendingEffect {
                layer,
                target,
                kind: EffectKind::SetPT {
                    power: sp,
                    toughness: st,
                },
            });
        }

        if sa.ir.remove_all_abilities {
            pending.push(PendingEffect {
                layer: Layer::Ability,
                target,
                kind: EffectKind::RemoveAllCardTraits {
                    timestamp: source_card.layer_timestamp as i64,
                    static_id: static_layer_trait_id(source_id, sa_idx),
                },
            });
        }

        let removed_by_newer_effect =
            game.card(target)
                .changed_card_traits
                .iter()
                .any(|(&(timestamp, _), change)| {
                    change.remove_all && timestamp > source_card.layer_timestamp as i64
                });
        if let Some(kws) = sa
            .ir
            .add_keyword_text
            .as_deref()
            .filter(|_| !removed_by_newer_effect)
        {
            // AddKeyword$ supports multiple keywords separated by " & ".
            let static_id = static_layer_trait_id(source_id, sa_idx);
            let mut idx = 1;
            for kw in kws.split('&').map(str::trim).filter(|s| !s.is_empty()) {
                // Java `StaticAbilityContinuous:714` replaces a CardColors keyword
                // with one copy per colour of the affected card, so a colourless
                // one is granted nothing at all.
                if kw.contains("CardColors") || kw.contains("cardColors") {
                    for color in game.card(target).color.iter() {
                        let name = color.long_name();
                        let expanded = kw
                            .replace("CardColors", &capitalize(name))
                            .replace("cardColors", name);
                        pending.push(PendingEffect {
                            layer: Layer::Ability,
                            target,
                            kind: EffectKind::GrantKeyword {
                                keyword: expanded,
                                idx,
                                static_id,
                                timestamp: Some(source_card.layer_timestamp),
                            },
                        });
                        idx += 1;
                    }
                    continue;
                }
                pending.push(PendingEffect {
                    layer: Layer::Ability,
                    target,
                    kind: EffectKind::GrantKeyword {
                        keyword: kw.to_string(),
                        idx,
                        static_id,
                        timestamp: Some(source_card.layer_timestamp),
                    },
                });
                idx += 1;
            }
        }

        if let Some(name) = sa.ir.set_name_text.as_deref() {
            let resolved = if name == "ChosenName" {
                source_card.get_named_card().map(str::to_string)
            } else if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            if let Some(resolved) = resolved {
                pending.push(PendingEffect {
                    layer: Layer::Text,
                    target,
                    kind: EffectKind::SetName(resolved),
                });
            }
        }

        // AddAbility$ — grant an activated ability to the affected card.
        // The value is an SVar name on the source card containing the ability text.
        // E.g. Abundant Growth: AddAbility$ AbundantGrowthTap
        //   SVar:AbundantGrowthTap:AB$ Mana | Cost$ T | Produced$ Any
        if let Some(add_ability) = sa.ir.add_ability_text.as_deref() {
            for svar_name in add_ability.split(" & ") {
                if let Some(ab_text) = source_card.svars.get(svar_name.trim()).cloned() {
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantAbility {
                            text: ab_text,
                            svars: source_card.svars.clone(),
                            original_host: Some(source_id),
                            original_ability: None,
                        },
                    });
                }
            }
        }

        if let Some(filter) = sa.ir.gains_abilities_of.as_deref() {
            let zones = if sa.ir.gains_abilities_of_zones.is_empty() {
                vec![ZoneType::Battlefield]
            } else {
                sa.ir.gains_abilities_of_zones.clone()
            };
            let selector = crate::parsing::cached_compiled_selector(filter);
            for gained in game.cards.iter().filter(|card| zones.contains(&card.zone)) {
                if !crate::card::valid_filter::matches_valid_card_selector_in_game(
                    &selector,
                    gained,
                    source_card,
                    game,
                ) {
                    continue;
                }
                for ab in &gained.activated_abilities {
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantAbility {
                            text: ab.ability_text.clone(),
                            svars: gained.svars.clone(),
                            original_host: Some(gained.id),
                            original_ability: Some((gained.id, ab.ability_index)),
                        },
                    });
                }
            }
        }

        if let Some(filter) = sa.ir.gains_trigger_abs_of.as_deref() {
            let zones = if sa.ir.gains_abilities_of_zones.is_empty() {
                vec![ZoneType::Battlefield]
            } else {
                sa.ir.gains_abilities_of_zones.clone()
            };
            let selector = crate::parsing::cached_compiled_selector(filter);
            for gained in game.cards.iter().filter(|card| zones.contains(&card.zone)) {
                if !crate::card::valid_filter::matches_valid_card_selector_in_game(
                    &selector,
                    gained,
                    source_card,
                    game,
                ) {
                    continue;
                }
                for trig in &gained.triggers {
                    let mut params: Vec<_> =
                        trig.base.card_trait_base.get_map_params().iter().collect();
                    params.sort();
                    let text = params
                        .into_iter()
                        .map(|(key, value)| format!("{key}$ {value}"))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantTrigger {
                            text,
                            svars: gained.svars.clone(),
                            original_host: Some(gained.id),
                        },
                    });
                }
            }
        }

        if let Some(defined) = sa.ir.gains_abilities_of_defined.as_deref() {
            for gained in crate::ability::ability_utils::get_defined_cards(
                game,
                Some(source_id),
                defined,
                Some(source_card.controller),
            ) {
                let gained = game.card(gained);
                for ab in &gained.activated_abilities {
                    let text = match sa.ir.gains_abilities_limit_per_turn.as_deref() {
                        Some(limit) => {
                            format!("{} | ActivationLimit$ {limit}", ab.ability_text)
                        }
                        None => ab.ability_text.clone(),
                    };
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantAbility {
                            text,
                            svars: gained.svars.clone(),
                            original_host: Some(gained.id),
                            original_ability: Some((gained.id, ab.ability_index)),
                        },
                    });
                }
            }
        }

        if let Some(add_trigger) = sa.ir.add_trigger_text.as_deref() {
            for svar_name in add_trigger
                .split(" & ")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(trig_text) = source_card.svars.get(svar_name).cloned() {
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantTrigger {
                            text: trig_text,
                            svars: source_card.svars.clone(),
                            original_host: Some(source_id),
                        },
                    });
                }
            }
        }

        if let Some(add_replacement) = sa.ir.add_replacement_effect_text.as_deref() {
            for svar_name in add_replacement
                .split(" & ")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(re_text) = source_card.svars.get(svar_name).cloned() {
                    pending.push(PendingEffect {
                        layer: Layer::Ability,
                        target,
                        kind: EffectKind::GrantReplacement {
                            text: re_text,
                            svars: source_card.svars.clone(),
                        },
                    });
                }
            }
        }

        if let Some(add_static) = sa.ir.add_static_ability_text.as_deref() {
            for svar_name in add_static
                .split(" & ")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(static_text) = source_card.svars.get(svar_name).cloned() {
                    if let Some(granted) = crate::staticability::parse_static_ability(&static_text)
                    {
                        if granted.check_mode(&StaticMode::Continuous) {
                            granted_statics.push((target, granted));
                        } else {
                            granted_player_rules.push((target, granted));
                        }
                    }
                }
            }
        }

        for subtype in resolve_added_basic_land_types(source_card, sa.ir.add_type_text.as_deref()) {
            if let Some(ab_text) = basic_land_mana_ability_text(&subtype) {
                pending.push(PendingEffect {
                    layer: Layer::Ability,
                    target,
                    kind: EffectKind::GrantAbility {
                        text: ab_text.to_string(),
                        svars: BTreeMap::new(),
                        original_host: None,
                        original_ability: None,
                    },
                });
            }
        }

        if let Some(look) = sa.ir.may_look_at.as_deref() {
            let players = if look == "True" {
                let may_play = sa.ir.may_play
                    && sa.ir.may_play_limit.is_none_or(|limit| {
                        crate::staticability::static_ability_continuous::may_play_turn(
                            sa,
                            source_card,
                            game,
                        ) < limit
                    });
                if may_play {
                    vec![
                        crate::staticability::static_ability_continuous::may_play_player(
                            sa,
                            source_card,
                            game.card(target),
                            game,
                        ),
                    ]
                } else {
                    Vec::new()
                }
            } else {
                crate::ability::ability_utils::get_defined_players(
                    game,
                    Some(source_id),
                    look,
                    Some(source_card.controller),
                )
            };
            pending.push(PendingEffect {
                layer: Layer::Rules,
                target,
                kind: EffectKind::MayLookAt {
                    static_id: static_layer_trait_id(source_id, sa_idx),
                    players,
                },
            });
        }
    }
}

/// Keep in sync with the reset loop at the top of `apply_continuous_effects`: a card this
/// answers true for is skipped there, so it must be exactly the state that loop would leave.
fn static_layer_reset_is_noop(card: &crate::card::Card, card_names_unchanged: bool) -> bool {
    card.changed_card_traits
        .keys()
        .all(|(_, static_id)| *static_id >= 0)
        && card.activated_abilities.len() <= card.base_ability_count
        && card
            .activated_abilities
            .iter()
            .enumerate()
            .all(|(ability_idx, ability)| ability.ability_index == ability_idx)
        && card.triggers.len() <= card.base_trigger_count + card.pump_trigger_count
        && card.static_power_modifier == 0
        && card.static_toughness_modifier == 0
        && (card.face_down
            || (card.static_set_power.is_none() && card.static_set_toughness.is_none()))
        && card.granted_keywords.has_no_entries()
        && card.pump_keywords_removed_by_statics.is_empty()
        && (card_names_unchanged
            || card
                .svars
                .get("OriginalName")
                .is_none_or(|name| *name == card.card_name))
        && card.granted_svars.is_empty()
        && card.static_type_line_base.is_none()
        && card.static_added_subtypes.is_empty()
        && !card.cant_block_static
        && card.may_look.iter().all(|&(timestamp, _)| timestamp >= 0)
}

fn is_staged_before(seq: usize, effect: &PendingEffect, before: Option<(Layer, usize)>) -> bool {
    before.is_none_or(|bound| (effect.layer, seq) < bound)
}

fn flush_pending_effects(
    game: &mut GameState,
    staged: &mut Vec<(usize, PendingEffect)>,
    before: Option<(Layer, usize)>,
    type_changed: &mut Vec<CardId>,
    control_changed: &mut Vec<CardId>,
    granted_keyword_replacements: &mut indexmap::IndexMap<
        CardId,
        Vec<crate::replacement::replacement_effect::ReplacementEffect>,
    >,
) {
    if !staged
        .iter()
        .any(|(seq, effect)| is_staged_before(*seq, effect, before))
    {
        return;
    }
    staged.sort_by_key(|(seq, effect)| (effect.layer, *seq));
    let ready = staged.partition_point(|(seq, effect)| is_staged_before(*seq, effect, before));
    let effects: Vec<PendingEffect> = staged.drain(..ready).map(|(_, effect)| effect).collect();
    type_changed.extend(
        effects
            .iter()
            .filter(|effect| effect.layer == Layer::Type)
            .map(|effect| effect.target),
    );
    control_changed.extend(
        effects
            .iter()
            .filter(|effect| matches!(effect.kind, EffectKind::SetController { .. }))
            .map(|effect| effect.target),
    );
    apply_pending_effects(game, effects, granted_keyword_replacements);
}

fn apply_pending_effects(
    game: &mut GameState,
    effects: Vec<PendingEffect>,
    granted_keyword_replacements: &mut indexmap::IndexMap<
        CardId,
        Vec<crate::replacement::replacement_effect::ReplacementEffect>,
    >,
) {
    for effect in effects {
        let target = effect.target;
        if matches!(effect.kind, EffectKind::SetName(_)) {
            crate::card::valid_filter::present_memo::invalidate();
        }
        let present_attributes_before = matches!(
            effect.kind,
            EffectKind::SetController { .. }
                | EffectKind::RemoveCardTypes
                | EffectKind::AddType(_)
                | EffectKind::ReapplyChangedCardTypes(_)
        )
        .then(|| crate::card::valid_filter::present_memo::attributes(game.card(target)));
        match effect.kind {
            EffectKind::SetController { controller } => {
                if game.card(effect.target).static_control_base.is_none() {
                    let card = game.card_mut(effect.target);
                    card.static_control_base = Some(card.controller);
                }
                game.set_temp_controller(effect.target, controller);
            }
            EffectKind::AddPT { power, toughness } => {
                let card = game.card_mut(effect.target);
                card.static_power_modifier += power;
                card.static_toughness_modifier += toughness;
            }
            EffectKind::SetPT { power, toughness } => {
                let card = game.card_mut(effect.target);
                // Layer 7b: override the base P/T for this calculation cycle.
                // We use `static_set_power` rather than mutating `base_power`
                // so the original base value is preserved for the next reset.
                if let Some(p) = power {
                    card.static_set_power = Some(p);
                }
                if let Some(t) = toughness {
                    card.static_set_toughness = Some(t);
                }
            }
            EffectKind::RemoveAllCardTraits {
                timestamp,
                static_id,
            } => {
                let card = game.card_mut(effect.target);
                card.granted_keywords.clear();
                let removed = card.pump_keywords.take_older_than(timestamp as u64);
                card.pump_keywords_removed_by_statics.extend(removed);
                card.add_changed_card_traits(
                    crate::card::card_trait_changes::CardTraitChanges::remove_all_layer(
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ),
                    timestamp,
                    static_id,
                );
            }
            EffectKind::SetName(name) => {
                game.card_names_unchanged = false;
                game.card_mut(effect.target).add_changed_name(&name);
            }
            EffectKind::GrantKeyword {
                keyword: kw,
                idx,
                static_id,
                timestamp,
            } => {
                let card = game.card_mut(effect.target);
                let kw: String = if kw.contains("CardManaCost") {
                    kw.replace("CardManaCost", &card.mana_cost.short_string())
                } else if kw.contains("ConvertedManaCost") {
                    kw.replace("ConvertedManaCost", &card.mana_value().to_string())
                } else {
                    kw.to_string()
                };
                let keyword = crate::keyword::keyword_collection::parse_keyword_string(&kw).0;
                let redundant =
                    keyword.is_multiple_redundant() && card.keywords.contains_string(&kw);
                let mut inst =
                    crate::keyword::keyword_instance::KeywordInstanceData::new(keyword, kw.clone());
                inst.idx = idx;
                inst.static_id = static_id;
                inst.timestamp = timestamp;
                if card.granted_keywords.insert(inst.clone()) && !redundant {
                    let own_svars = std::mem::take(&mut card.svars);
                    card.generate_keyword_triggers_for_instance(&inst);
                    let keyword_svars = std::mem::replace(&mut card.svars, own_svars);
                    for (name, value) in keyword_svars {
                        if !card.svars.contains_key(&name) {
                            card.granted_svars.insert(name, value);
                        }
                    }
                }
                if kw == "Riot" {
                    if let Some(re) = crate::card::card_factory_util::riot_replacement(false) {
                        granted_keyword_replacements
                            .entry(effect.target)
                            .or_default()
                            .push(re);
                    }
                }
            }
            kind @ (EffectKind::RemoveCardTypes
            | EffectKind::RemoveLandTypes
            | EffectKind::RemoveArtifactTypes
            | EffectKind::RemoveCreatureTypes
            | EffectKind::AddType(_)
            | EffectKind::ReapplyChangedCardTypes(_)) => {
                apply_type_effect(game.card_mut(effect.target), kind);
            }
            EffectKind::GrantAbility {
                text,
                svars,
                original_host,
                original_ability,
            } => {
                // Parse the ability text and add it to the target's activated abilities.
                // This grants abilities like "{T}: Add one mana of any color."
                game.card_mut(effect.target).granted_svars.extend(svars);
                let target_idx = effect.target.index();
                let next_idx = game.cards[target_idx].activated_abilities.len();
                if let Some(mut ab) =
                    crate::ability::activated::parse_activated_ability(&text, next_idx)
                {
                    ab.original_host = original_host;
                    ab.original_ability = original_ability;
                    game.card_mut(effect.target).activated_abilities.push(ab);
                }
            }
            EffectKind::GrantTrigger {
                text,
                svars,
                original_host,
            } => {
                game.card_mut(effect.target).granted_svars.extend(svars);
                let target = &game.cards[effect.target.index()];
                let intrinsic_count = (target.base_trigger_count + target.pump_trigger_count)
                    .min(target.triggers.len());
                let base_id = target.triggers[..intrinsic_count]
                    .iter()
                    .map(|t| t.id)
                    .max()
                    .unwrap_or(0);
                let key = (
                    effect.target,
                    target.zone_timestamp,
                    original_host,
                    original_host.map_or(0, |host| game.card(host).zone_timestamp),
                    text.clone(),
                );
                let next_seq = game.granted_trigger_ids.len() as u32;
                let seq = *game.granted_trigger_ids.entry(key).or_insert(next_seq);
                let mut next_id_mut = base_id.saturating_add(1).saturating_add(seq);
                if let Some(mut trig) = crate::trigger::parse_trigger(&text, &mut next_id_mut) {
                    trig.original_host = original_host;
                    game.card_mut(effect.target).add_trigger(trig);
                }
            }
            EffectKind::GrantReplacement { text, svars } => {
                if let Some(mut re) =
                    crate::replacement::replacement_effect::parse_replacement_effect(&text)
                {
                    for (name, value) in svars {
                        crate::core::HasSVars::set_svar(&mut re.base.card_trait_base, name, value);
                    }
                    granted_keyword_replacements
                        .entry(effect.target)
                        .or_default()
                        .push(re);
                }
            }
            EffectKind::MayLookAt { static_id, players } => {
                game.card_mut(effect.target)
                    .add_may_look_at(static_id, players);
            }
        }
        if present_attributes_before.is_some_and(|before| {
            before != crate::card::valid_filter::present_memo::attributes(game.card(target))
        }) {
            crate::card::valid_filter::present_memo::invalidate();
        }
    }
}

fn apply_player_keyword_effects(
    game: &mut GameState,
    affected_players: &[PlayerId],
    add_keywords: Option<&str>,
) {
    let Some(add_keywords) = add_keywords else {
        return;
    };
    for &player in affected_players {
        for keyword in add_keywords
            .split(" & ")
            .map(str::trim)
            .filter(|kw| !kw.is_empty())
        {
            let keywords = &mut game.player_mut(player).static_keywords;
            if !keywords.iter().any(|existing| existing == keyword) {
                keywords.push(keyword.to_string());
            }
            if keyword.starts_with("Protection")
                && game.player(player).keyword_effect_card.is_none()
            {
                let effect = crate::player::player_factory_util::new_player_effect_card(
                    player,
                    "Keyword Effects",
                    None,
                );
                let effect_id = game.add_player_effect_card(player, effect);
                game.player_mut(player).keyword_effect_card = Some(effect_id);
            }
        }
    }
}

fn apply_player_rules_effects(
    game: &mut GameState,
    source_id: CardId,
    affected_players: &[PlayerId],
    adjust_land_plays: Option<&str>,
) {
    let Some(adjust_land_plays) = adjust_land_plays else {
        return;
    };
    if affected_players.is_empty() {
        return;
    }
    if adjust_land_plays.eq_ignore_ascii_case("Unlimited") {
        for &player in affected_players {
            game.player_mut(player).unlimited_land_plays = true;
        }
        return;
    }
    let amount = resolve_rules_amount(game, source_id, adjust_land_plays);
    for &player in affected_players {
        game.player_mut(player).max_land_plays_per_turn += amount;
    }
}

fn static_ability_at<'a>(
    game: &'a GameState,
    source_id: CardId,
    sa_idx: usize,
    owned: &'a Option<Box<StaticAbility>>,
) -> &'a StaticAbility {
    match owned {
        Some(sa) => sa,
        None => &game.card(source_id).static_abilities[sa_idx],
    }
}

fn affected_players_for_static(
    game: &GameState,
    source_id: CardId,
    sa: &StaticAbility,
) -> Vec<PlayerId> {
    let Some(affected) = sa.ir.affected_text.as_deref() else {
        return Vec::new();
    };
    let source = game.card(source_id);
    game.player_order
        .iter()
        .copied()
        .filter(|&player| {
            !sa.ignore_effect_players.contains(&player)
                && crate::card::valid_filter::matches_valid(
                    affected,
                    None,
                    Some(player),
                    source,
                    source.controller,
                    game,
                )
        })
        .collect()
}

fn resolve_rules_amount(game: &GameState, source_id: CardId, value: &str) -> i32 {
    if let Ok(n) = value.trim().parse::<i32>() {
        return n;
    }
    let source = game.card(source_id);
    if let Some(svar_expr) = source.svars.get(value.trim()) {
        if svar_expr.starts_with("Count$") {
            return crate::ability::effects::resolve_count_svar(
                svar_expr,
                game,
                source_id,
                source.controller,
            );
        }
        return crate::ability::effects::evaluate_svar(
            svar_expr,
            &crate::spellability::SpellAbility::new_empty(Some(source_id), source.controller),
        );
    }
    0
}

/// Check if a card has a shock-land-style "enters tapped unless you pay life" effect.
///
/// Looks for `R:Event$ Moved | Destination$ Battlefield | ReplaceWith$ <SVar>`
/// where the SVar is `DB$ Tap | ETB$ True | UnlessCost$ PayLife<N>`.
///
/// Returns `Some(life_cost)` if found (e.g. `Some(2)` for shock lands), `None` otherwise.
/// Called from `play_card` / `resolve_stack` where agents are available for prompting.
pub fn get_etb_unless_life_cost(card: &crate::card::Card) -> Option<i32> {
    for re in &card.replacement_effects {
        if re.event != ReplacementType::Moved {
            continue;
        }
        if re.ir.destination_zone != Some(ZoneType::Battlefield) {
            continue;
        }
        if let Some(svar_name) = re.replace_with() {
            if svar_name == "ETBTapped" {
                continue;
            }
            if let Some(svar_val) = card.svars.get(svar_name) {
                if svar_val.contains("DB$ Tap") && svar_val.contains("ETB$ True") {
                    // Parse life cost from "UnlessCost$ PayLife<N>"
                    if let Some(pos) = svar_val.find("PayLife<") {
                        let after = &svar_val[pos + 8..]; // skip "PayLife<"
                        if let Some(end) = after.find('>') {
                            if let Ok(n) = after[..end].parse::<i32>() {
                                return Some(n);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Check if a card has a "enters tapped unless you reveal a <type> from hand" effect.
///
/// Looks for `R:Event$ Moved | Destination$ Battlefield | ReplaceWith$ <SVar>`
/// where the SVar is `DB$ Tap | ETB$ True | UnlessCost$ Reveal<N/Filter>`.
///
/// Returns `Some((n, filter))` if found (e.g. `Some((1, "Merfolk"))` for Wanderwine Hub).
pub fn get_etb_unless_reveal_cost(card: &crate::card::Card) -> Option<(i32, String)> {
    for re in &card.replacement_effects {
        if re.event != ReplacementType::Moved {
            continue;
        }
        if re.ir.destination_zone != Some(ZoneType::Battlefield) {
            continue;
        }
        if let Some(svar_name) = re.replace_with() {
            if svar_name == "ETBTapped" {
                continue;
            }
            if let Some(svar_val) = card.svars.get(svar_name) {
                if svar_val.contains("DB$ Tap") && svar_val.contains("ETB$ True") {
                    // Parse reveal cost from "UnlessCost$ Reveal<N/Filter>"
                    if let Some(pos) = svar_val.find("Reveal<") {
                        let after = &svar_val[pos + 7..]; // skip "Reveal<"
                        if let Some(end) = after.find('>') {
                            let inner = &after[..end]; // "1/Merfolk" or "1/Filter"
                            let mut parts = inner.splitn(2, '/');
                            let n = parts
                                .next()
                                .and_then(|s| s.trim().parse::<i32>().ok())
                                .unwrap_or(1);
                            let filter = parts.next().unwrap_or("").trim().to_string();
                            return Some((n, filter));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Resolve an AddPower$/AddToughness$ parameter that may be a literal integer
/// or an SVar reference (e.g. "X" → Count$Valid Enchantment.YouCtrl).
fn resolve_add_pt_value(
    game: &GameState,
    source_id: CardId,
    affected: CardId,
    val_str: Option<&str>,
) -> i32 {
    let val_str = match val_str {
        Some(val_str) => val_str.trim(),
        None => return 0,
    };
    let evaluated_on = if val_str.contains("Affected") {
        affected
    } else {
        source_id
    };

    // Try direct integer parse first
    if let Ok(n) = val_str.parse::<i32>() {
        return n;
    }

    // Java `AbilityUtils.calculateAmount` strips a leading sign and multiplies the
    // resolved amount by it, so `AddPower$ -X` negates the SVar rather than naming one.
    let (sign, val_str) = match val_str.strip_prefix('-') {
        Some(rest) => (-1, rest.trim()),
        None => (1, val_str.strip_prefix('+').unwrap_or(val_str).trim()),
    };

    let source = game.card(source_id);
    if val_str.starts_with("Count$") {
        return sign
            * crate::ability::effects::resolve_count_svar(
                val_str,
                game,
                evaluated_on,
                source.controller,
            );
    }

    // It's an SVar reference — look it up on the source card
    if let Some(svar_expr) = source.svars.get(val_str) {
        if svar_expr.starts_with("Count$") {
            return sign
                * crate::ability::effects::resolve_count_svar(
                    svar_expr,
                    game,
                    evaluated_on,
                    source.controller,
                );
        }
        return sign
            * crate::svar::resolve_svar_expression(
                svar_expr,
                game,
                evaluated_on,
                source.controller,
                &crate::spellability::SpellAbility::new_empty(
                    Some(evaluated_on),
                    source.controller,
                ),
            );
    }

    0
}

/// Resolve a SetPower$/SetToughness$ parameter that may be a literal integer or
/// an SVar reference (e.g. "X" → SVar:X:Count$Valid Creature.ChosenType).
/// Mirrors Java `AbilityUtils.calculateAmount(hostCard, param, stAb)`.
fn resolve_set_pt_value(
    game: &GameState,
    source_id: CardId,
    affected: CardId,
    val_str: Option<&str>,
) -> Option<i32> {
    let val_str = val_str?.trim();
    // Try direct integer parse first
    if let Ok(n) = val_str.parse::<i32>() {
        return Some(n);
    }
    let evaluated_on = if val_str.contains("Affected") {
        affected
    } else {
        source_id
    };

    let source = game.card(source_id);
    if val_str.starts_with("Count$") {
        return Some(crate::ability::effects::resolve_count_svar(
            val_str,
            game,
            evaluated_on,
            source.controller,
        ));
    }

    // It's an SVar reference — look it up on the source card
    if let Some(svar_expr) = source.svars.get(val_str) {
        if svar_expr.starts_with("Count$") {
            return Some(crate::ability::effects::resolve_count_svar(
                svar_expr,
                game,
                evaluated_on,
                source.controller,
            ));
        }
        // Simple SVar evaluation (e.g. Number$2)
        return Some(crate::ability::effects::evaluate_svar(
            svar_expr,
            &crate::spellability::SpellAbility::new_empty(Some(evaluated_on), source.controller),
        ));
    }

    None
}

fn basic_land_mana_ability_text(subtype: &str) -> Option<&'static str> {
    match subtype {
        "Plains" => Some("AB$ Mana | Cost$ T | Produced$ W | SpellDescription$ Add {W}."),
        "Island" => Some("AB$ Mana | Cost$ T | Produced$ U | SpellDescription$ Add {U}."),
        "Swamp" => Some("AB$ Mana | Cost$ T | Produced$ B | SpellDescription$ Add {B}."),
        "Mountain" => Some("AB$ Mana | Cost$ T | Produced$ R | SpellDescription$ Add {R}."),
        "Forest" => Some("AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}."),
        _ => None,
    }
}

fn resolve_added_basic_land_types(
    source: &crate::card::Card,
    add_type: Option<&str>,
) -> Vec<String> {
    resolve_added_types(source, add_type)
        .into_iter()
        .filter(|added| basic_land_mana_ability_text(added).is_some())
        .collect()
}

fn affected_text(sa: &StaticAbility) -> &str {
    sa.ir
        .affected_text
        .as_deref()
        .or(sa.ir.valid_cards_text.as_deref())
        .or(sa.ir.valid_card_text.as_deref())
        .unwrap_or("Creature.YouControl")
}

fn static_affected_cards(game: &GameState, source_id: CardId, sa: &StaticAbility) -> Vec<CardId> {
    let mut affected = Vec::new();
    static_affected_cards_into(game, source_id, sa, &mut affected);
    affected
}

fn static_affected_cards_into(
    game: &GameState,
    source_id: CardId,
    sa: &StaticAbility,
    affected: &mut Vec<CardId>,
) {
    let source_card = game.card(source_id);
    let affected_str = affected_text(sa);
    affected.clear();
    if sa.ir.characteristic_defining {
        affected.push(source_id);
    } else if let Some(defined) = sa.ir.affected_defined.as_deref() {
        let selector = sa
            .ir
            .affected_text
            .as_deref()
            .map(crate::parsing::cached_compiled_selector);
        for cid in crate::ability::ability_utils::get_defined_cards(
            game,
            Some(source_id),
            defined,
            Some(source_card.controller),
        ) {
            let card = game.card(cid);
            if card.phased_out {
                continue;
            }
            if selector.as_ref().is_none_or(|selector| {
                crate::card::valid_filter::matches_valid_card_selector_in_game(
                    selector,
                    card,
                    source_card,
                    game,
                )
            }) {
                affected.push(cid);
            }
        }
    } else if affected_str.eq_ignore_ascii_case("Card.Self")
        || affected_str.starts_with("Card.Self+")
    {
        // Self-referencing static: only affects the source card itself,
        // but qualifiers after "+" must still be checked (e.g.
        // "Card.Self+counters_GE2_CHARGE" only matches when the card
        // has >=2 charge counters). Mirrors Java's
        // StaticAbilityContinuous.getAffectedCards() which validates
        // all qualifiers even for self-referencing statics.
        let in_affected_zone = if sa.ir.affected_zones.is_empty() {
            source_card.zone == ZoneType::Battlefield
        } else {
            sa.ir.affected_zones.contains(&source_card.zone)
        };
        if in_affected_zone
            && crate::card::valid_filter::matches_valid_card_selector_in_game(
                &crate::parsing::cached_compiled_selector(affected_str),
                source_card,
                source_card,
                game,
            )
        {
            affected.push(source_id);
        }
    } else if affected_str.eq_ignore_ascii_case("Card.EnchantedBy")
        || affected_str.contains(".EquippedBy")
        || affected_str.contains(".EnchantedBy")
    {
        // Aura / Equipment static effects: affect what this source is
        // attached to. Java treats EquippedBy and EnchantedBy
        // identically: both resolve to the entity the source is
        // attached to. (e.g. Short Sword: "Creature.EquippedBy",
        // Control Magic: "Card.EnchantedBy")
        if let Some(cid) = source_card.attached_to {
            if game.card(cid).zone == ZoneType::Battlefield
                && crate::card::valid_filter::matches_valid_card_selector_in_game(
                    &crate::parsing::cached_compiled_selector(affected_str),
                    game.card(cid),
                    source_card,
                    game,
                )
            {
                affected.push(cid);
            }
        }
    } else {
        let selector = crate::parsing::cached_compiled_selector(affected_str);
        // AffectedZone$ overrides the default Battlefield filter (e.g.
        // Ashling, the Limitless grants Evoke:4 to Elementals in Hand).
        let affected_zones = if sa.ir.affected_zones.is_empty() {
            None
        } else {
            Some(sa.ir.affected_zones.as_slice())
        };
        for card in &game.cards {
            let zone_matches = match &affected_zones {
                Some(zones) => zones.contains(&card.zone),
                None => card.zone == ZoneType::Battlefield,
            };
            if zone_matches
                && crate::card::valid_filter::matches_valid_card_selector_in_game(
                    &selector,
                    card,
                    source_card,
                    game,
                )
            {
                affected.push(card.id);
            }
        }
    }
}

struct StaticEffectUndo {
    cards: Vec<(CardId, Arc<crate::card::Card>)>,
    zones: Option<crate::zone::ZoneStore>,
    granted_trigger_ids: Option<crate::HashMap<(CardId, u64, Option<CardId>, u64, String), u32>>,
}

fn find_static_ability_to_apply(
    game: &mut GameState,
    layer: Layer,
    statics_for_layer: &[(CardId, usize, StaticAbility)],
) -> usize {
    // CR 611.2c continuous effects from resolved abilities always affect the same objects the same way
    let is_resolved =
        |game: &GameState, source_id: CardId| game.card(source_id).type_line.has_subtype("Effect");
    if statics_for_layer.len() == 1 || is_resolved(game, statics_for_layer[0].0) {
        return 0;
    }
    let mut dependencies: Vec<(usize, usize)> = Vec::new();
    for (index, (source_id, sa_idx, st_ab)) in statics_for_layer.iter().enumerate() {
        if is_resolved(game, *source_id) {
            continue;
        }
        let exists = static_exists(game, *source_id, *sa_idx, st_ab);
        let affected_here = static_affected_cards(game, *source_id, st_ab);
        let effect_results = generate_continuous_effect_changes(game, layer, *source_id, st_ab);
        for (other_index, (other_id, other_idx, other_st_ab)) in
            statics_for_layer.iter().enumerate()
        {
            if index == other_index {
                continue;
            }
            #[cfg(debug_assertions)]
            let before = (
                game.cards.clone(),
                game.zone_store_snapshot(),
                game.granted_trigger_ids.clone(),
            );
            let Some(undo) =
                apply_continuous_ability_before(game, layer, *other_id, *other_idx, other_st_ab)
            else {
                continue;
            };
            // CR 613.8a applying the other would change the existence of the first effect,
            // what it applies to, or what it does to any of the things it applies to
            let dependency = exists != static_exists(game, *source_id, *sa_idx, st_ab)
                || affected_here != static_affected_cards(game, *source_id, st_ab)
                || effect_results
                    != generate_continuous_effect_changes(game, layer, *source_id, st_ab);
            remove_static_effect(game, undo);
            #[cfg(debug_assertions)]
            debug_assert!(
                before
                    .0
                    .iter()
                    .zip(&game.cards)
                    .all(|(card, restored)| Arc::ptr_eq(card, restored))
                    && before.1 == game.zone_store_snapshot()
                    && before.2 == game.granted_trigger_ids
            );
            if dependency {
                dependencies.push((index, other_index));
            }
        }
        if dependencies.is_empty() && index == 0 {
            return 0;
        }
    }
    // CR 613.8b If several dependent effects form a dependency loop, then this rule is ignored
    let reaches = |from: usize, to: usize| {
        let mut seen = vec![false; statics_for_layer.len()];
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            if node == to {
                return true;
            }
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            stack.extend(
                dependencies
                    .iter()
                    .filter(|&&(dependent, _)| dependent == node)
                    .map(|&(_, depended)| depended),
            );
        }
        false
    };
    let acyclic: Vec<(usize, usize)> = dependencies
        .iter()
        .copied()
        .filter(|&(dependent, depended)| !reaches(depended, dependent))
        .collect();
    (0..statics_for_layer.len())
        .find(|&index| !acyclic.iter().any(|&(dependent, _)| dependent == index))
        .unwrap_or(0)
}

fn static_exists(
    game: &GameState,
    source_id: CardId,
    sa_idx: usize,
    st_ab: &StaticAbility,
) -> bool {
    game.card(source_id)
        .static_abilities
        .get(sa_idx)
        .is_some_and(|current| current.base.get_map_params() == st_ab.base.get_map_params())
}

fn generate_continuous_effect_changes(
    game: &GameState,
    layer: Layer,
    source_id: CardId,
    st_ab: &StaticAbility,
) -> Vec<PlayerId> {
    if layer != Layer::Control {
        return Vec::new();
    }
    let controller = game.card(source_id).controller;
    match st_ab.ir.gain_control_text.as_deref() {
        Some("You" | "YouCtrl") => vec![controller],
        Some("Opponent") => vec![game.opponent_of(controller)],
        _ => Vec::new(),
    }
}

fn apply_continuous_ability_before(
    game: &mut GameState,
    layer: Layer,
    source_id: CardId,
    sa_idx: usize,
    st_ab: &StaticAbility,
) -> Option<StaticEffectUndo> {
    let source = game.card(source_id);
    if !st_ab.zones_check(source.zone) || !st_ab.check_conditions(source, game) {
        return None;
    }
    let mut effects = Vec::new();
    for target in static_affected_cards(game, source_id, st_ab) {
        apply_continuous_ability(
            game,
            source_id,
            sa_idx,
            st_ab,
            target,
            &mut effects,
            &mut Vec::new(),
            &mut Vec::new(),
        );
    }
    effects.retain(|effect| effect.layer == layer);
    if effects.is_empty() {
        return None;
    }
    let mut targets: Vec<CardId> = effects.iter().map(|effect| effect.target).collect();
    targets.sort_unstable_by_key(|id| id.0);
    targets.dedup();
    let undo = StaticEffectUndo {
        cards: targets
            .into_iter()
            .map(|id| (id, game.cards[id.index()].clone()))
            .collect(),
        zones: effects
            .iter()
            .any(|effect| matches!(effect.kind, EffectKind::SetController { .. }))
            .then(|| game.zone_store_snapshot()),
        granted_trigger_ids: effects
            .iter()
            .any(|effect| matches!(effect.kind, EffectKind::GrantTrigger { .. }))
            .then(|| game.granted_trigger_ids.clone()),
    };
    apply_pending_effects(game, effects, &mut indexmap::IndexMap::new());
    Some(undo)
}

fn remove_static_effect(game: &mut GameState, undo: StaticEffectUndo) {
    for (id, card) in undo.cards {
        game.cards[id.index()] = card;
    }
    if let Some(zones) = undo.zones {
        game.replace_zone_store(zones);
    }
    if let Some(granted_trigger_ids) = undo.granted_trigger_ids {
        game.granted_trigger_ids = granted_trigger_ids;
    }
}

fn apply_type_effect(card: &mut crate::card::Card, kind: EffectKind) {
    match kind {
        EffectKind::RemoveCardTypes => {
            if card.static_type_line_base.is_none() {
                card.static_type_line_base = Some(card.type_line.clone());
            }
            card.type_line
                .core_types
                .retain(|t| matches!(t, CoreType::Instant | CoreType::Sorcery));
            card.update_types();
        }
        EffectKind::RemoveLandTypes | EffectKind::RemoveArtifactTypes => {
            let is_removed: fn(&str) -> bool = match kind {
                EffectKind::RemoveLandTypes => crate::game::TypeRegistry::is_land_type,
                _ => |s| crate::game::TypeRegistry::is_subtype_in("ArtifactTypes", s),
            };
            if card.type_line.subtypes.iter().any(|s| is_removed(s)) {
                if card.static_type_line_base.is_none() {
                    card.static_type_line_base = Some(card.type_line.clone());
                }
                card.type_line.subtypes.retain(|s| !is_removed(s));
                card.update_types();
            }
        }
        EffectKind::RemoveCreatureTypes => {
            if card.type_line.subtypes.iter().any(|s| {
                crate::game::TypeRegistry::creature_types()
                    .iter()
                    .any(|ct| ct.eq_ignore_ascii_case(s))
            }) {
                if card.static_type_line_base.is_none() {
                    card.static_type_line_base = Some(card.type_line.clone());
                }
                card.type_line.subtypes.retain(|s| {
                    !crate::game::TypeRegistry::creature_types()
                        .iter()
                        .any(|ct| ct.eq_ignore_ascii_case(s))
                });
                card.update_types();
            }
        }
        EffectKind::AddType(t) => {
            if !type_line_has_token(&card.type_line, &t) {
                if card.static_type_line_base.is_none() {
                    card.static_type_line_base = Some(card.type_line.clone());
                }
                card.add_type(&t);
                card.static_added_subtypes.push(t);
            }
        }
        EffectKind::ReapplyChangedCardTypes(after) => {
            if card.static_type_line_base.is_some() {
                let changes: Vec<crate::card::card_changed_type::CardChangedType> = card
                    .changed_card_types
                    .iter()
                    .filter(|(timestamp, _)| *timestamp > after)
                    .map(|(_, change)| change.clone())
                    .collect();
                for change in &changes {
                    card.apply_changed_card_type(change);
                }
            }
        }
        _ => {}
    }
}

fn type_effects(
    game: &GameState,
    source_card: &crate::card::Card,
    sa: &StaticAbility,
    target: CardId,
) -> Vec<EffectKind> {
    let mut effects = Vec::new();
    if sa.ir.remove_card_types {
        effects.push(EffectKind::RemoveCardTypes);
    }
    if sa.ir.remove_land_types {
        effects.push(EffectKind::RemoveLandTypes);
    }
    if sa.ir.remove_creature_types {
        effects.push(EffectKind::RemoveCreatureTypes);
    }
    if sa.ir.remove_artifact_types {
        effects.push(EffectKind::RemoveArtifactTypes);
    }
    let added_types = resolve_added_types(source_card, sa.ir.add_type_text.as_deref());
    let changes_type = !added_types.is_empty() || !effects.is_empty();
    effects.extend(added_types.into_iter().map(EffectKind::AddType));
    if changes_type
        && game
            .card(target)
            .changed_card_types
            .iter()
            .any(|(timestamp, _)| *timestamp > source_card.layer_timestamp)
    {
        effects.push(EffectKind::ReapplyChangedCardTypes(
            source_card.layer_timestamp,
        ));
    }
    effects
}

fn resolve_added_types(source: &crate::card::Card, add_type: Option<&str>) -> Vec<String> {
    let Some(add_type) = add_type else {
        return Vec::new();
    };
    let mut resolved = Vec::new();
    for raw in add_type.split('&').map(str::trim).filter(|s| !s.is_empty()) {
        match raw {
            "ChosenType" => {
                if let Some(chosen) = source.chosen_type.as_ref() {
                    resolved.push(chosen.clone());
                }
            }
            "ChosenType2" => {
                if let Some(chosen) = source.chosen_type2.as_ref() {
                    resolved.push(chosen.clone());
                }
            }
            "AllBasicLandType" => {
                resolved.extend(
                    ["Plains", "Island", "Swamp", "Mountain", "Forest"]
                        .into_iter()
                        .map(str::to_string),
                );
            }
            other => resolved.push(other.to_string()),
        }
    }
    resolved
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Keep in sync with `CardType.sanisfySubtypes`: after type changes, a subtype stays only
/// while the card has a card type it belongs to. Returns whether anything was removed.
pub(crate) fn sanitize_subtypes(type_line: &mut CardTypeLine) -> bool {
    use crate::game::TypeRegistry;
    let creature = type_line.core_types.contains(&CoreType::Creature)
        || type_line.core_types.contains(&CoreType::Kindred);
    let cleared_all_creature_types = !creature && std::mem::take(&mut type_line.all_creature_types);
    if type_line.subtypes.is_empty() || !TypeRegistry::subtype_sections_loaded() {
        return cleared_all_creature_types;
    }
    let has = |t: CoreType| type_line.core_types.contains(&t);
    let land = has(CoreType::Land);
    let artifact = has(CoreType::Artifact);
    let enchantment = has(CoreType::Enchantment);
    let spell = has(CoreType::Instant) || has(CoreType::Sorcery);
    let walker = has(CoreType::Planeswalker);
    let dungeon = has(CoreType::Dungeon);
    let battle = has(CoreType::Battle);
    let plane = has(CoreType::Plane);
    let before = type_line.subtypes.len();
    type_line.subtypes.retain(|s| {
        (creature && TypeRegistry::is_creature_type(s))
            || (land && TypeRegistry::is_land_type(s))
            || (artifact && TypeRegistry::is_subtype_in("ArtifactTypes", s))
            || (enchantment && TypeRegistry::is_subtype_in("EnchantmentTypes", s))
            || (spell && TypeRegistry::is_subtype_in("SpellTypes", s))
            || (walker && TypeRegistry::is_subtype_in("WalkerTypes", s))
            || (dungeon && TypeRegistry::is_subtype_in("DungeonTypes", s))
            || (battle && TypeRegistry::is_subtype_in("BattleTypes", s))
            || (plane && TypeRegistry::is_subtype_in("PlanarTypes", s))
    });
    type_line.subtypes.len() != before || cleared_all_creature_types
}

/// `StringUtils.capitalize`, for the colour name Java splices into a CardColors keyword.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    use crate::card::Card;
    use crate::ids::{CardId, PlayerId};

    // Build a minimal two-player game with empty zones.
    fn new_game() -> GameState {
        GameState::new(&["Alice", "Bob"], 20)
    }

    fn add_creature(
        game: &mut GameState,
        owner: PlayerId,
        power: i32,
        toughness: i32,
        keywords: Vec<String>,
        abilities: Vec<String>,
    ) -> CardId {
        let card = Card::new(
            CardId(0), // reassigned by create_card
            "Creature".to_string(),
            owner,
            CardTypeLine::parse("Creature"),
            ManaCost::parse("1 G"),
            ColorSet::GREEN,
            Some(power),
            Some(toughness),
            keywords,
            abilities,
        );
        let id = game.create_card(card);
        game.move_card(id, ZoneType::Battlefield, owner);
        id
    }

    fn add_enchantment(game: &mut GameState, owner: PlayerId, abilities: Vec<String>) -> CardId {
        let card = Card::new(
            CardId(0),
            "Enchantment".to_string(),
            owner,
            CardTypeLine::parse("Enchantment"),
            ManaCost::parse("2 W"),
            ColorSet::WHITE,
            None,
            None,
            vec![],
            abilities,
        );
        let id = game.create_card(card);
        game.move_card(id, ZoneType::Battlefield, owner);
        id
    }

    fn cant_attack_opponent(game: &GameState, attacker: CardId) -> bool {
        let card = game.card(attacker);
        crate::staticability::static_ability_cant_attack_block::cant_attack(
            game,
            &game.cards,
            card,
            crate::combat::DefenderId::Player(game.opponent_of(card.controller)),
        )
    }

    fn add_land(
        game: &mut GameState,
        owner: PlayerId,
        name: &str,
        type_line: &str,
        abilities: Vec<String>,
    ) -> CardId {
        let card = Card::new(
            CardId(0),
            name.to_string(),
            owner,
            CardTypeLine::parse(type_line),
            ManaCost::no_cost(),
            ColorSet::COLORLESS,
            None,
            None,
            vec![],
            abilities,
        );
        let id = game.create_card(card);
        game.move_card(id, ZoneType::Battlefield, owner);
        id
    }

    fn add_effect(game: &mut GameState, owner: PlayerId, abilities: Vec<String>) -> CardId {
        let card = Card::new(
            CardId(0),
            "Effect".to_string(),
            owner,
            CardTypeLine::parse("Effect"),
            ManaCost::parse("0"),
            ColorSet::COLORLESS,
            None,
            None,
            vec![],
            abilities,
        );
        let id = game.create_card(card);
        game.move_card(id, ZoneType::Command, owner);
        id
    }

    // ── Anthem (+1/+1) ────────────────────────────────────────────────────

    #[test]
    fn anthem_boosts_your_creatures() {
        let mut game = new_game();
        let alice = PlayerId(0);
        let bob = PlayerId(1);

        // Add two creatures for Alice and one for Bob.
        let a1 = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        let a2 = add_creature(&mut game, alice, 1, 1, vec![], vec![]);
        let b1 = add_creature(&mut game, bob, 2, 2, vec![], vec![]);

        // Add Glorious Anthem-style enchantment controlled by Alice.
        let _anthem = add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1 | Description$ Creatures you control get +1/+1.".to_string()],
        );

        apply_continuous_effects(&mut game);

        // Alice's creatures get +1/+1.
        assert_eq!(game.card(a1).power(), 3, "Alice's 2/2 should be 3/3");
        assert_eq!(game.card(a1).toughness(), 3);
        assert_eq!(game.card(a2).power(), 2, "Alice's 1/1 should be 2/2");
        assert_eq!(game.card(a2).toughness(), 2);

        // Bob's creature is unaffected.
        assert_eq!(
            game.card(b1).power(),
            2,
            "Bob's creature should be unchanged"
        );
        assert_eq!(game.card(b1).toughness(), 2);
    }

    #[test]
    fn command_effect_adjusts_land_plays_for_affected_player() {
        let mut game = new_game();
        let alice = PlayerId(0);
        let bob = PlayerId(1);

        let effect = add_effect(
            &mut game,
            alice,
            vec![
                "S$ Mode$ Continuous | EffectZone$ Command | Affected$ You | AdjustLandPlays$ 1"
                    .to_string(),
            ],
        );

        apply_continuous_effects(&mut game);

        assert_eq!(game.player(alice).max_land_plays_per_turn, 2);
        assert_eq!(game.player(bob).max_land_plays_per_turn, 1);

        game.move_card(effect, ZoneType::Exile, alice);
        apply_continuous_effects(&mut game);

        assert_eq!(game.player(alice).max_land_plays_per_turn, 1);
    }

    #[test]
    fn anthem_resets_when_removed() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        let anthem = add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1".to_string()],
        );

        apply_continuous_effects(&mut game);
        assert_eq!(game.card(creature).power(), 3);

        // Remove the anthem from the battlefield.
        game.move_card(anthem, ZoneType::Graveyard, alice);
        apply_continuous_effects(&mut game);

        assert_eq!(
            game.card(creature).power(),
            2,
            "Bonus should be gone after anthem leaves"
        );
    }

    #[test]
    fn stacking_anthems() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 1, 1, vec![], vec![]);
        // Two separate +1/+1 anthems.
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1".to_string()],
        );
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1".to_string()],
        );

        apply_continuous_effects(&mut game);
        assert_eq!(game.card(creature).power(), 3, "Two anthems should give +2");
        assert_eq!(game.card(creature).toughness(), 3);
    }

    // ── Keyword granting ──────────────────────────────────────────────────

    #[test]
    fn grant_flying_to_your_creatures() {
        let mut game = new_game();
        let alice = PlayerId(0);
        let bob = PlayerId(1);

        let a1 = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        let b1 = add_creature(&mut game, bob, 2, 2, vec![], vec![]);

        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddKeyword$ Flying | Description$ Creatures you control have flying.".to_string()],
        );

        apply_continuous_effects(&mut game);

        assert!(
            game.card(a1).has_flying(),
            "Alice's creature should have flying"
        );
        assert!(
            !game.card(b1).has_flying(),
            "Bob's creature should not have flying"
        );
    }

    #[test]
    fn grant_multiple_keywords() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddKeyword$ Flying & First Strike".to_string()],
        );

        apply_continuous_effects(&mut game);

        assert!(game.card(creature).has_flying());
        assert!(game.card(creature).has_first_strike());
    }

    // ── SetPT (Layer 7b) ──────────────────────────────────────────────────

    #[test]
    fn set_pt_overrides_base() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 5, 5, vec![], vec![]);
        // Effect: set all your creatures to 0/1 (e.g. Humility).
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | SetPower$ 0 | SetToughness$ 1".to_string()],
        );

        apply_continuous_effects(&mut game);
        assert_eq!(game.card(creature).power(), 0);
        assert_eq!(game.card(creature).toughness(), 1);
    }

    #[test]
    fn modify_pt_adds_on_top_of_set_pt() {
        // CR 613.7c: ModifyPT applies after SetPT within the same turn.
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 5, 5, vec![], vec![]);
        // Layer 7b: set to 0/1.
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | SetPower$ 0 | SetToughness$ 1".to_string()],
        );
        // Layer 7c: +1/+1 anthem on top.
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1".to_string()],
        );

        apply_continuous_effects(&mut game);
        // 0 + 1 = 1 power, 1 + 1 = 2 toughness.
        assert_eq!(game.card(creature).power(), 1);
        assert_eq!(game.card(creature).toughness(), 2);
    }

    // ── CantAttack / CantBlock ────────────────────────────────────────────

    #[test]
    fn cant_attack_static_applies() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        // Pacifism-like effect.
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ CantAttack | ValidCard$ Creature.YouCtrl | Description$ Creatures you control can't attack.".to_string()],
        );

        assert!(cant_attack_opponent(&game, creature));
    }

    #[test]
    fn cant_block_flag_set() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ CantBlock | Affected$ Creature.YouControl".to_string()],
        );

        apply_continuous_effects(&mut game);
        assert!(game.card(creature).cant_block_static);
    }

    #[test]
    fn cant_attack_static_ends_when_its_source_leaves() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let creature = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        let restrictor = add_enchantment(
            &mut game,
            alice,
            vec!["S$ Mode$ CantAttack | ValidCard$ Creature.YouCtrl".to_string()],
        );

        assert!(cant_attack_opponent(&game, creature));

        game.move_card(restrictor, ZoneType::Graveyard, alice);
        assert!(!cant_attack_opponent(&game, creature));
    }

    #[test]
    fn lands_gain_swamp_mana_ability_from_urborg_style_effect() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let urborg = add_land(
            &mut game,
            alice,
            "Urborg, Tomb of Yawgmoth",
            "Legendary Land",
            vec!["S$ Mode$ Continuous | Affected$ Land | AddType$ Swamp | Description$ Each land is a Swamp in addition to its other land types.".to_string()],
        );
        let black_gate = add_land(
            &mut game,
            alice,
            "The Black Gate",
            "Legendary Land Gate",
            vec![],
        );

        apply_continuous_effects(&mut game);

        for land_id in [urborg, black_gate] {
            let land = game.card(land_id);
            assert!(
                land.type_line.has_subtype("Swamp"),
                "{} should gain the Swamp subtype",
                land.card_name
            );
            assert!(
                land.activated_abilities.iter().any(|ab| {
                    ab.is_mana_ability
                        && ab
                            .produced_ir
                            .as_ref()
                            .is_some_and(|ir| ir.as_script_text() == "B")
                }),
                "{} should gain an intrinsic black mana ability from Swamp",
                land.card_name
            );
        }
    }

    // ── ETB Tapped ────────────────────────────────────────────────────────

    #[test]
    fn no_etb_tapped_without_ability() {
        let mut game = new_game();
        let alice = PlayerId(0);

        let id = add_creature(&mut game, alice, 2, 2, vec![], vec![]);
        // Fresh ETB, no static — should not be tapped.
        assert!(
            !game.card(id).tapped,
            "Normal creature should not enter tapped"
        );
    }
}
