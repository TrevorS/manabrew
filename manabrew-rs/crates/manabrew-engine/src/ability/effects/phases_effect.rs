use forge_foundation::ZoneType;

use super::EffectContext;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::staticability::static_ability_cant_phase::{cant_phase_in, cant_phase_out};
use crate::trigger::TriggerType;

/// Resolve `SP$ Phases` — phase permanents in or out.
///
/// Mirrors Java `PhasesEffect.java`.
/// Toggles or sets `card.phased_out` on target cards. Phased-out permanents
/// are treated as not on the battlefield for game purposes.
///
/// # Card script examples
/// ```text
/// A:SP$ Phases | ValidTgts$ Creature | TgtPrompt$ Select target creature
/// A:SP$ Phases | Defined$ Self | PhaseInOrOut$ True
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `PhasesEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(PhasesEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let activator = sa.activating_player;
    let raw = sa.ability_text.as_str();
    let phase_in_or_out = sa.ir.phase_in_or_out_text.is_some();

    let mut tgt_cards: Vec<CardId> = if let Some(all_valid) = sa.ir.all_valid_selector.as_ref() {
        ctx.game
            .player_order
            .iter()
            .flat_map(|&pid| {
                ctx.game
                    .cards_in_zone(ZoneType::Battlefield, pid)
                    .iter()
                    .copied()
            })
            .filter(|&cid| {
                (phase_in_or_out || !ctx.game.card(cid).phased_out)
                    && crate::ability::ability_utils::matches_valid_cards_for_sa(
                        ctx.game,
                        sa,
                        ctx.game.card(cid),
                        Some(all_valid),
                        "",
                    )
            })
            .collect()
    } else {
        crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
    };
    if crate::parsing::raw_has_key(raw, "AnyNumber") {
        let max = tgt_cards.len();
        tgt_cards =
            ctx.agents[activator.index()].choose_cards_for_effect(activator, &tgt_cards, 0, max);
    }

    let mut phased_out = Vec::new();
    if phase_in_or_out {
        let to_phase: Vec<CardId> = tgt_cards
            .iter()
            .copied()
            .filter(|&cid| {
                let card = ctx.game.card(cid);
                !(card.phased_out && cant_phase_in(&ctx.game.cards, card))
                    && !(!card.phased_out && cant_phase_out(&ctx.game.cards, card))
            })
            .collect();
        for cid in to_phase {
            if ctx.game.card(cid).zone != ZoneType::Battlefield {
                continue;
            }
            if ctx.game.card(cid).phased_out {
                apply_phase(ctx, cid, "In");
                if crate::parsing::raw_has_key(raw, "Tapped") {
                    ctx.game.card_mut(cid).tapped = true;
                } else if crate::parsing::raw_has_key(raw, "Untapped") {
                    ctx.game.card_mut(cid).tapped = false;
                }
            } else {
                apply_phase(ctx, cid, "Out");
                phased_out.push(cid);
            }
        }
    } else {
        for &cid in &tgt_cards {
            if ctx.game.card(cid).zone != ZoneType::Battlefield
                || ctx.game.card(cid).phased_out
                || cant_phase_out(&ctx.game.cards, ctx.game.card(cid))
            {
                continue;
            }
            apply_phase(ctx, cid, "Out");
            if crate::parsing::raw_has_key(raw, "RememberAffected") {
                if let Some(source) = sa.source {
                    ctx.game.card_mut(source).add_remembered_card(cid);
                }
            }
            phased_out.push(cid);
        }
    }
    if crate::parsing::raw_has_key(raw, "RememberValids") {
        if let Some(source) = sa.source {
            ctx.game
                .card_mut(source)
                .add_remembered_cards(tgt_cards.iter().copied());
        }
    }
    if !phased_out.is_empty() {
        ctx.trigger_handler.run_trigger(
            TriggerType::PhaseOutAll,
            RunParams {
                cards: Some(phased_out),
                ..Default::default()
            },
            false,
        );
    }
}

fn apply_phase(ctx: &mut EffectContext, card_id: crate::ids::CardId, mode: &str) {
    match mode {
        "In" => {
            if ctx.game.card(card_id).phased_out {
                ctx.game.card_mut(card_id).set_phased_out(false);
                ctx.trigger_handler.run_trigger(
                    TriggerType::PhasedIn,
                    RunParams {
                        card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );
            }
        }
        _ => {
            if !ctx.game.card(card_id).phased_out {
                ctx.game.card_mut(card_id).set_phased_out(true);
                ctx.trigger_handler.run_trigger(
                    TriggerType::PhasedOut,
                    RunParams {
                        card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use crate::HashMap;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    use crate::ability::effects::EffectContext;
    use crate::agent::PassAgent;
    use crate::card::Card;
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::handler::TriggerHandler;

    fn make_creature(game: &mut GameState, owner: PlayerId) -> CardId {
        let c = Card::new(
            CardId(0),
            "Bear".into(),
            owner,
            CardTypeLine::parse("Creature - Bear"),
            ManaCost::parse("1 G"),
            ColorSet::GREEN,
            Some(2),
            Some(2),
            vec![],
            vec![],
        );
        game.create_card(c)
    }

    #[test]
    fn phase_out_target() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let c1 = make_creature(&mut game, p0);
        game.move_card(c1, ZoneType::Battlefield, p0);
        assert!(!game.card(c1).phased_out);

        let mut sa = SpellAbility::new_simple(
            None,
            p0,
            "SP$ Phases | ValidTgts$ Creature | PhaseInOrOut$ True",
        );
        sa.target_chosen.target_card = Some(c1);

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
        super::PhasesEffect::resolve(&mut ctx, &sa);

        assert!(ctx.game.card(c1).phased_out);
    }

    #[test]
    fn phase_in_target() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let c1 = make_creature(&mut game, p0);
        game.move_card(c1, ZoneType::Battlefield, p0);
        game.card_mut(c1).set_phased_out(true);

        let mut sa = SpellAbility::new_simple(
            None,
            p0,
            "SP$ Phases | ValidTgts$ Creature | PhaseInOrOut$ True",
        );
        sa.target_chosen.target_card = Some(c1);

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
        super::PhasesEffect::resolve(&mut ctx, &sa);

        assert!(!ctx.game.card(c1).phased_out);
    }
}
