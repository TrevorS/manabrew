use forge_foundation::ZoneType;

use super::EffectContext;

/// Resolve `SP$ Regenerate` — add a regeneration shield to a creature.
///
/// Mirrors Java `RegenerateEffect.java`.
/// Increments `card.regeneration_shields` on the target creature.
/// When a creature with shields would be destroyed, a shield is consumed
/// instead: the creature is tapped, removed from combat, and damage is removed.
/// Shields reset at end of turn.
///
/// # Card script examples
/// ```text
/// A:SP$ Regenerate | Defined$ Self
/// A:SP$ Regenerate | ValidTgts$ Creature.YouCtrl | TgtPrompt$ Select target creature you control
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `RegenerateEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(RegenerateEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let list: Vec<crate::ids::CardId> =
        crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
            .into_iter()
            .filter(|&card| ctx.game.card(card).zone == ZoneType::Battlefield)
            .collect();
    create_regeneration_effect(ctx, sa, &list);
}

fn create_regeneration_effect(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    list: &[crate::ids::CardId],
) {
    if list.is_empty() {
        return;
    }
    let host_name = sa
        .source
        .map(|host| ctx.game.card(host).card_name.clone())
        .unwrap_or_default();
    let eff = crate::ability::spell_ability_effect::create_effect(
        ctx.game,
        sa,
        &format!("{host_name}'s Regeneration"),
        "",
    );
    let controller = sa.activating_player;
    let effect = ctx.game.card_mut(eff);
    for &card in list {
        effect.add_remembered_card(card);
    }
    effect.set_forget_on_moved_origin(Some(ZoneType::Battlefield));
    effect.set_exile_when_no_remembered(true);
    if let Some(mut replacement) = crate::replacement::parse_replacement_effect(
        "R$ Event$ Destroy | ActiveZones$ Command | ValidCard$ Card.IsRemembered | Regeneration$ True | Description$ Regeneration (if creature would be destroyed, regenerate it instead)",
    ) {
        replacement.set_host_card(effect);
        let mut regeneration = crate::ability::ability_factory::build_spell_ability_from_host_card(
            effect,
            "DB$ Regeneration | Defined$ ReplacedCard",
            controller,
        );
        regeneration.sub_ability = Some(Box::new(
            crate::ability::ability_factory::build_spell_ability_from_host_card(
                effect,
                "DB$ ChangeZone | Defined$ Self | Origin$ Command | Destination$ Exile | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ EQ0",
                controller,
            ),
        ));
        replacement.base.set_overriding_ability(regeneration);
        effect.add_replacement_effect(replacement);
    }
    for &card in list {
        ctx.game.card_mut(card).regeneration_shields += 1;
    }
    ctx.game.end_of_turn.add_until(
        None,
        crate::phase::PhaseCommand::ExileEffect { effect: eff },
    );
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
    fn regenerate_adds_shield() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let c1 = make_creature(&mut game, p0);
        game.move_card(c1, ZoneType::Battlefield, p0);
        assert_eq!(game.card(c1).regeneration_shields, 0);

        let sa = SpellAbility::new_simple(Some(c1), p0, "SP$ Regenerate | Defined$ Self");

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
        super::RegenerateEffect::resolve(&mut ctx, &sa);

        assert_eq!(ctx.game.card(c1).regeneration_shields, 1);
    }

    #[test]
    fn regenerate_stacks_shields() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let c1 = make_creature(&mut game, p0);
        game.move_card(c1, ZoneType::Battlefield, p0);

        let sa = SpellAbility::new_simple(Some(c1), p0, "SP$ Regenerate | Defined$ Self");

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
        super::RegenerateEffect::resolve(&mut ctx, &sa);
        super::RegenerateEffect::resolve(&mut ctx, &sa);

        assert_eq!(ctx.game.card(c1).regeneration_shields, 2);
    }
}
