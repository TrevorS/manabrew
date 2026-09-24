//! Replacement logic for `Event$ Moved`.
//!
//! Mirrors Java `ReplaceMoved.java` in `forge/game/replacement/`.

use crate::HashMap;

use forge_foundation::ZoneType;

use crate::ability::effects::{self, EffectContext};
use crate::agent::{PassAgent, PlayerAgent};
use crate::card::Card;
use crate::game::GameState;
use crate::game_rng::ThreadRngAdapter;
use crate::ids::CardId;
use crate::mana::ManaPool;
use crate::spellability::build_spell_ability;
use crate::trigger::TriggerHandler;

use super::replacement_effect::{zone_matches, ReplacementEffect};
use super::replacement_handler::{ReplacementEvent, ReplacementRuntime};
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceMoved.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::Moved {
        return false;
    }
    let (moving_id, origin, destination, is_discard, stack_sa, fizzle) = match event {
        ReplacementEvent::Moved {
            card,
            origin,
            destination,
            is_discard,
            stack_sa,
            fizzle,
            ..
        } => (
            *card,
            *origin,
            *destination,
            *is_discard,
            stack_sa.as_deref(),
            *fizzle,
        ),
        _ => return false,
    };
    // Discard$ True — only match when the move is from a discard action.
    // Mirrors Java ReplaceMoved.canReplace() Discard$ check.
    if let Some(requires_discard) = effect.ir.discard {
        if requires_discard != is_discard {
            return false;
        }
    }
    if let Some(dest) = effect.ir.destination_text.as_deref() {
        if !zone_matches(dest, destination) {
            return false;
        }
    }
    if let Some(exclude) = effect.ir.exclude_destination_text.as_deref() {
        if zone_matches(exclude, destination) {
            return false;
        }
    }
    if let Some(orig) = effect.ir.origin_text.as_deref() {
        if !zone_matches(orig, origin) {
            return false;
        }
    }
    let moving_card = &game.cards[moving_id.index()];
    if let Some(valid) = effect.ir.valid_card_selector.as_ref() {
        if !effect.matches_compiled_valid_card(valid, moving_card, source_card) {
            return false;
        }
    }
    // FlashbackCast$ True — only match when the card was cast via Flashback.
    if effect.ir.flashback_cast == Some(true) && !moving_card.cast_with_flashback {
        return false;
    }
    // HarmonizeCast$ True — only match when the card was cast via Harmonize.
    if effect.ir.harmonize_cast == Some(true) && !moving_card.cast_with_harmonize {
        return false;
    }
    if let Some(valid_lki) = effect.ir.valid_lki_text.as_deref() {
        if !effect.matches_valid_card(valid_lki, moving_card, source_card) {
            return false;
        }
    }
    if let Some(valid_stack_sa) = effect.ir.valid_stack_sa_text.as_deref() {
        let Some(stack_sa) = stack_sa else {
            return false;
        };
        let stack_sa_host = stack_sa.source.map(|id| game.card(id));
        if !crate::spellability::valid_sa::matches_valid_sa(
            valid_stack_sa,
            stack_sa,
            source_card,
            stack_sa_host,
        ) {
            return false;
        }
    }
    if effect.ir.fizzle.is_some() && effect.ir.fizzle != fizzle {
        return false;
    }
    // Mirrors Java `ReplaceMoved.canReplace()` L103: only gate ETB chains.
    if destination == ZoneType::Battlefield && !effect.can_replace_etb(source_card, moving_card) {
        return false;
    }
    true
}

/// Mirrors Java `ReplacementHandler.executeReplacement()` for Moved.
pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &mut GameState,
    source_card_id: CardId,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    runtime: Option<&mut ReplacementRuntime<'_>>,
) -> ReplacementResult {
    let (moving_id, destination) = match event {
        ReplacementEvent::Moved {
            card, destination, ..
        } => (*card, *destination),
        _ => return ReplacementResult::NotReplaced,
    };
    if let Some(new_dest) = effect.ir.new_destination_text.as_deref() {
        let new_zone = match new_dest.trim() {
            "Exile" => Some(ZoneType::Exile),
            "Graveyard" => Some(ZoneType::Graveyard),
            "Hand" => Some(ZoneType::Hand),
            "Library" => Some(ZoneType::Library),
            "Battlefield" => Some(ZoneType::Battlefield),
            "Command" => Some(ZoneType::Command),
            _ => None,
        };
        if let Some(z) = new_zone {
            if let ReplacementEvent::Moved { destination, .. } = event {
                *destination = z;
            }
            if z == ZoneType::Exile && effect.ir.exiled_with_effect_source {
                let exile_source = game
                    .card(source_card_id)
                    .effect_source
                    .unwrap_or(source_card_id);
                game.card_mut(moving_id).set_exiled_by(Some(exile_source));
                game.card_mut(exile_source).add_remembered_card(moving_id);
            }
            return ReplacementResult::Updated;
        }
    }
    if let Some(replace_with_key) = effect.replace_with() {
        let succeeded = execute_replace_with(
            effect,
            replace_with_key,
            game,
            source_card_id,
            event,
            agents,
            runtime,
        );
        if !succeeded {
            return ReplacementResult::NotReplaced;
        }
    } else if let Some(ability) = effect.base.get_overriding_ability() {
        if destination != ZoneType::Battlefield
            && !execute_replacement_ability(effect, ability.clone(), game, event, agents, runtime)
        {
            return ReplacementResult::NotReplaced;
        }
    }
    if let Some(result) = effect.ir.replacement_result.as_deref() {
        return match result {
            "Updated" => ReplacementResult::Updated,
            "Replaced" => ReplacementResult::Replaced,
            "Skipped" => ReplacementResult::Skipped,
            "Prevented" => ReplacementResult::Prevented,
            _ => ReplacementResult::Replaced,
        };
    }
    ReplacementResult::Replaced
}

pub(super) fn execute_replace_with(
    effect: &ReplacementEffect,
    replace_with: &str,
    game: &mut GameState,
    source_card_id: CardId,
    event: &ReplacementEvent,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    runtime: Option<&mut ReplacementRuntime<'_>>,
) -> bool {
    let Some(raw) = crate::core::HasSVars::get_svar(&effect.base.card_trait_base, replace_with)
        .map(str::to_string)
        .or_else(|| game.card(source_card_id).svars.get(replace_with).cloned())
    else {
        return false;
    };
    let controller = game.card(source_card_id).controller;
    let sa = build_spell_ability(game, source_card_id, &raw, controller);
    execute_replacement_ability(effect, sa, game, event, agents, runtime)
}

pub(super) fn execute_replacement_ability(
    effect: &ReplacementEffect,
    mut sa: crate::spellability::SpellAbility,
    game: &mut GameState,
    event: &ReplacementEvent,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    mut runtime: Option<&mut ReplacementRuntime<'_>>,
) -> bool {
    effect.set_replacing_objects(event, &mut sa);

    // `local_agents_storage` keeps the fallback Vec alive when the caller
    // didn't provide agents; we hand back a borrow into it.
    #[allow(unused_assignments)]
    let mut local_agents_storage: Option<Vec<Box<dyn PlayerAgent>>> = None;
    let agents: &mut [Box<dyn PlayerAgent>] = if let Some(agents) = agents {
        agents
    } else {
        local_agents_storage = Some(
            (0..game.players.len())
                .map(|_| Box::new(PassAgent) as Box<dyn PlayerAgent>)
                .collect(),
        );
        local_agents_storage.as_mut().unwrap().as_mut_slice()
    };

    let mut local_mana_pools: Vec<ManaPool> =
        (0..game.players.len()).map(|_| ManaPool::new()).collect();
    let mana_pools_for_targets: &[ManaPool] = if let Some(rt) = runtime.as_ref() {
        rt.mana_pools.as_slice()
    } else {
        local_mana_pools.as_slice()
    };
    if sa.uses_targeting() && !sa.setup_targets(game, agents, mana_pools_for_targets) {
        return false;
    }

    let mut local_trigger_handler = TriggerHandler::new();
    let local_token_templates: HashMap<String, Card> = HashMap::default();
    let local_token_art_variants: HashMap<(String, String), usize> = HashMap::default();
    let local_token_fallback: HashMap<String, String> = HashMap::default();
    let local_edition_dates: HashMap<String, String> = HashMap::default();
    let mut local_rng = ThreadRngAdapter::default();

    let mut parent_target_card: Option<CardId> = None;
    let mut parent_target_player = None;
    let mut parent_additional_target_players: Vec<crate::ids::PlayerId> = Vec::new();
    let chain_target_cards = sa.chain_target_cards_from_root();
    let mut current_sa: Option<&crate::spellability::SpellAbility> = Some(&sa);
    while let Some(cur) = current_sa {
        let mut sa_with_ctx;
        let sa_ref = if (parent_target_player.is_some()
            && cur.target_chosen.target_player.is_none()
            && !cur.uses_targeting())
            || cur.parent_targeting_player != parent_target_player
            || cur.chain_target_cards != chain_target_cards
        {
            sa_with_ctx = cur.clone();
            sa_with_ctx
                .chain_target_cards
                .clone_from(&chain_target_cards);
            if !cur.uses_targeting() && sa_with_ctx.target_chosen.target_player.is_none() {
                sa_with_ctx.target_chosen.target_player = parent_target_player;
                sa_with_ctx
                    .target_chosen
                    .additional_target_players
                    .clone_from(&parent_additional_target_players);
            }
            sa_with_ctx.parent_targeting_player = parent_target_player;
            &sa_with_ctx
        } else {
            cur
        };

        let (
            trigger_handler_ref,
            token_templates_ref,
            token_art_ref,
            token_fb_ref,
            edition_dates_ref,
            mana_pools_ref,
            rng_ref,
        ): (
            &mut TriggerHandler,
            &HashMap<String, Card>,
            &HashMap<(String, String), usize>,
            &HashMap<String, String>,
            &HashMap<String, String>,
            &mut Vec<ManaPool>,
            &mut dyn crate::game_rng::GameRng,
        ) = if let Some(rt) = runtime.as_deref_mut() {
            (
                rt.trigger_handler,
                rt.token_templates,
                rt.token_art_variants,
                rt.token_fallback,
                rt.edition_dates,
                rt.mana_pools,
                rt.rng,
            )
        } else {
            (
                &mut local_trigger_handler,
                &local_token_templates,
                &local_token_art_variants,
                &local_token_fallback,
                &local_edition_dates,
                &mut local_mana_pools,
                &mut local_rng,
            )
        };

        let mut ctx = EffectContext {
            game,
            combat: None,
            agents,
            trigger_handler: trigger_handler_ref,
            token_templates: token_templates_ref,
            token_art_variants: token_art_ref,
            token_fallback: token_fb_ref,
            edition_dates: edition_dates_ref,
            mana_pools: mana_pools_ref,
            parent_target_card,
            rng: rng_ref,
        };
        effects::resolve_effect(&mut ctx, sa_ref);
        parent_target_card = sa_ref.target_chosen.target_card.or(parent_target_card);
        if sa_ref.target_chosen.target_player.is_some() {
            parent_target_player = sa_ref.target_chosen.target_player;
            parent_additional_target_players
                .clone_from(&sa_ref.target_chosen.additional_target_players);
        }
        // An `UnlessCost$` node resolves its own sub-chain, gated on
        // `UnlessResolveSubs$`, so walking into it here would run it twice.
        current_sa = if crate::ability::effects::sub_ability_handled_internally(sa_ref) {
            None
        } else {
            cur.get_sub_ability()
        };
    }
    true
}

// `set_replacing_objects_for_moved` was inlined into the cross-event
// `ReplacementEffect::set_replacing_objects` dispatcher in `replacement_effect.rs`
// to mirror Java's polymorphic `setReplacingObjects` hook.
