use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::card::card_util;
use crate::card::perpetual::perpetual_interface::PerpetualInterface;
use crate::card::perpetual::{perpetual_keywords, perpetual_pt_boost};

/// Parsed `NumAtt$`/`NumDef$` bonus spec: either a fixed literal or a
/// target-relative scale (Java L469–L481).
#[derive(Clone, Copy)]
enum PtBonus {
    Fixed(i32),
    Double,
    Triple,
}

impl PtBonus {
    fn parse(raw: Option<&str>, fallback: impl FnOnce() -> i32) -> Self {
        match raw {
            Some("Double") => PtBonus::Double,
            Some("Triple") => PtBonus::Triple,
            _ => PtBonus::Fixed(fallback()),
        }
    }

    /// Resolve the bonus against a concrete target's current P or T.
    fn resolve(self, current: i32) -> i32 {
        match self {
            PtBonus::Fixed(n) => n,
            PtBonus::Double => current,
            PtBonus::Triple => current * 2,
        }
    }
}

/// End-of-turn revert for Pump. Mirrors the `GameCommand.run()` in Java
/// `PumpEffect` that reverses the P/T bonus and removes granted keywords
/// when the effect duration expires.
pub fn run(
    game: &mut crate::game::GameState,
    card_id: crate::ids::CardId,
    att_bonus: i32,
    def_bonus: i32,
    keywords: &[String],
) {
    if game.card(card_id).zone != ZoneType::Battlefield {
        return;
    }
    game.card_mut(card_id).power_modifier -= att_bonus;
    game.card_mut(card_id).toughness_modifier -= def_bonus;
    for kw in keywords {
        game.card_mut(card_id).pump_keywords.remove(kw);
    }
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `PumpEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(PumpEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    if !crate::ability::spell_ability_effect::check_valid_duration(
        ctx.game,
        sa,
        sa.ir.duration.as_ref(),
    ) {
        return;
    }
    let mut pumped_targets: Vec<crate::ids::CardId> = Vec::new();

    // `Optional$` — activator confirms before any pump applies (Java L283–L292).
    if sa.ir.optional_present {
        let _card_name = sa.source.map(|cid| ctx.game.card(cid).card_name.clone());
        let prompt = sa
            .ir
            .option_question
            .as_deref()
            .unwrap_or("Apply pump to target?");
        let activator = sa.activating_player;
        if !ctx.agents[activator.index()].confirm_action(
            activator,
            Some("OptionalPump"),
            prompt,
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    let att_bonus = PtBonus::parse(sa.ir.num_att.as_deref(), || {
        match sa.ir.num_att.as_deref() {
            Some(raw) => super::resolve_numeric_value(ctx.game, sa, raw, 0),
            None => 0,
        }
    });
    let def_bonus = PtBonus::parse(sa.ir.num_def.as_deref(), || {
        match sa.ir.num_def.as_deref() {
            Some(raw) => super::resolve_numeric_value(ctx.game, sa, raw, 0),
            None => 0,
        }
    });

    // Parse KW$ parameter for keyword grants (e.g. "KW$ Haste" or "KW$ Flying & Trample")
    let mut keywords: Vec<String> = sa
        .ir
        .kw
        .as_deref()
        .map(|kw_str| {
            kw_str
                .split('&')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // `KWChoice$` — activator picks one keyword from a comma-separated list
    // (Java L297–L302, `chooseKeywordForPump`).
    if let Some(kw_choice) = sa.ir.kw_choice.as_deref() {
        let options: Vec<String> = kw_choice
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !options.is_empty() {
            let activator = sa.activating_player;
            if let Some(idx) = ctx.agents[activator.index()]
                .choose_keyword_for_pump(activator, &options, sa.source)
            {
                if let Some(kw) = options.get(idx) {
                    keywords.push(kw.clone());
                }
            }
        }
    }

    if let (Some(defined), Some(host)) = (
        crate::parsing::raw_get(&sa.ability_text, "DefinedKW"),
        sa.source,
    ) {
        if defined == "ChosenType" {
            let Some(chosen) = ctx.game.card(host).chosen_type.clone() else {
                return;
            };
            for kw in &mut keywords {
                *kw = kw.replace(defined, &chosen);
            }
        } else if defined == "ChosenColor" {
            let Some(chosen) = ctx.game.card(host).chosen_colors.first().cloned() else {
                return;
            };
            let lower = chosen.to_lowercase();
            let mut capitalized = lower.clone();
            if let Some(first) = capitalized.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            for kw in &mut keywords {
                *kw = kw
                    .replace("ChosenColor", &capitalized)
                    .replace("chosenColor", &lower);
            }
        }
    }

    // `CanBlockAny$` — synthetic keyword grant (Java L79–L85 / L240–L253).
    // Rust has no dedicated `addCanBlockAny` / `addCanBlockAdditional`, so we
    // encode the permission as pump keywords that block-restriction code can
    // match on ("CanBlockAny" / "CanBlock:N"). Full block-amount support lands
    // once the combat module reads these markers.
    if sa.ir.can_block_any {
        keywords.push("CanBlockAny".to_string());
    }
    if let Some(amt) = sa.ir.can_block_amount.as_deref() {
        keywords.push(format!("CanBlock:{amt}"));
    }

    if let (Some(remember), Some(host)) = (
        crate::parsing::raw_get(&sa.ability_text, crate::parsing::keys::REMEMBER_OBJECTS),
        sa.source,
    ) {
        let (players, cards) =
            crate::ability::ability_utils::get_defined_entities(remember, sa, ctx.game);
        let host = ctx.game.card_mut(host);
        host.add_remembered_players(players);
        host.add_remembered_cards(cards);
    }

    if let (Some(imprint), Some(host)) = (
        crate::parsing::raw_get(&sa.ability_text, "ImprintCards"),
        sa.source,
    ) {
        let cards = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, imprint,
        );
        ctx.game.card_mut(host).add_imprinted_cards(cards);
    }

    if let (Some(forget), Some(host)) = (
        crate::parsing::raw_get(&sa.ability_text, "ForgetImprinted"),
        sa.source,
    ) {
        let cards = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, forget,
        );
        ctx.game.card_mut(host).remove_imprinted_cards(cards);
    }

    let is_perpetual = sa.ir.perpetual_duration;
    let is_permanent = matches!(
        sa.ir.duration,
        Some(crate::spellability::AbilityDuration::Permanent)
    );
    let resolve_ts = if is_perpetual {
        Some(ctx.game.next_effect_timestamp())
    } else {
        None
    };

    // Overload: apply pump to ALL valid creatures instead of the chosen target.
    if sa.overloaded {
        let valid_tgts = sa.ir.valid_tgts_text.clone().unwrap_or_default();
        let valid_tgts_selector = sa.ir.valid_tgts_selector.as_ref();
        let all_bf: Vec<crate::ids::CardId> = ctx
            .game
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| ctx.game.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();
        for cid in all_bf {
            if ctx.game.card(cid).zone != ZoneType::Battlefield {
                continue;
            }
            if !super::matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                valid_tgts_selector,
                &valid_tgts,
            ) {
                continue;
            }
            let target = ctx.game.card(cid);
            let att = att_bonus.resolve(target.power());
            let def = def_bonus.resolve(target.toughness());
            apply_pump_to_card(
                ctx,
                cid,
                att,
                def,
                &keywords,
                is_perpetual,
                is_permanent,
                resolve_ts,
                sa,
            );
        }
        return;
    }

    let mut targets = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    if targets.is_empty() && matches!(sa.defined_ref(), Some(DefinedRef::ParentTarget)) {
        targets.extend(ctx.parent_target_card);
    }
    targets.extend(card_util::get_radiance(ctx.game, sa).iter().copied());
    targets.sort_unstable_by_key(|cid| cid.0);
    targets.dedup();

    let pump_zones = sa
        .ir
        .pump_zone
        .as_deref()
        .map(crate::zone::zone_type::list_value_of)
        .unwrap_or_else(|| vec![ZoneType::Battlefield]);
    for target_card in targets {
        if !pump_zones.contains(&ctx.game.card(target_card).zone) {
            continue;
        }
        let target = ctx.game.card(target_card);
        let att = att_bonus.resolve(target.power());
        let def = def_bonus.resolve(target.toughness());
        apply_pump_to_card(
            ctx,
            target_card,
            att,
            def,
            &keywords,
            is_perpetual,
            is_permanent,
            resolve_ts,
            sa,
        );
        pumped_targets.push(target_card);
    }

    if pumped_targets.is_empty() && !keywords.is_empty() {
        let pumped_players: Vec<crate::ids::PlayerId> =
            if let Some(tp) = sa.target_chosen.target_player {
                vec![tp]
            } else if let Some(d) = sa.defined() {
                crate::ability::ability_utils::resolve_defined_players_with_sa(
                    d,
                    sa,
                    sa.activating_player,
                    ctx.game,
                )
            } else {
                Vec::new()
            };
        for player in pumped_players {
            for kw in &keywords {
                crate::player::add_pump_keyword_with_duration(
                    ctx.game,
                    player,
                    kw.clone(),
                    sa.ir.duration.as_ref(),
                );
            }
        }
    }

    // `AtEOT$ <action>` — register an end-of-turn delayed trigger that performs
    // `action` on the pumped targets (Java PumpEffect L486).
    if let Some(action) = sa
        .ir
        .at_eot
        .as_deref()
        .filter(|_| !pumped_targets.is_empty())
    {
        crate::ability::spell_ability_effect::register_at_eot(
            ctx.trigger_handler,
            ctx.game,
            sa,
            action,
            pumped_targets,
        );
    }

    // NoteCardsFor$ / ClearNotedCardsFor$ (PumpEffect.java:404-417). The note is written on
    // the pump's target players, keyed by the label, and read back by the NotedFor player
    // property.
    let note_key = crate::parsing::raw_get(&sa.ability_text, "NoteCardsFor");
    let clear_keys = crate::parsing::raw_get(&sa.ability_text, "ClearNotedCardsFor");
    if note_key.is_some() || clear_keys.is_some() {
        let note_players: Vec<crate::ids::PlayerId> =
            if let Some(tp) = sa.target_chosen.target_player {
                vec![tp]
            } else if let Some(d) = sa.defined() {
                crate::ability::ability_utils::resolve_defined_players_with_sa(
                    d,
                    sa,
                    sa.activating_player,
                    ctx.game,
                )
            } else {
                Vec::new()
            };
        if let Some(key) = note_key {
            let noted = crate::parsing::raw_get(&sa.ability_text, "NoteCards").unwrap_or("Self");
            let cards = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, noted,
            );
            for card_id in cards {
                let note = format!("Id:{}", card_id.0);
                for &player in &note_players {
                    crate::player::add_note_for_name(ctx.game, player, key, note.clone());
                }
            }
        }
        if let Some(keys) = clear_keys {
            for key in keys.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                for &player in &note_players {
                    crate::player::clear_notes_for_name(ctx.game, player, key);
                }
            }
        }
    }

    let _ = crate::ability::spell_ability_effect::replace_dying(ctx.game, sa);
}

pub(super) fn apply_pump_to_card(
    ctx: &mut EffectContext,
    card_id: crate::ids::CardId,
    att: i32,
    def: i32,
    keywords: &[String],
    is_perpetual: bool,
    is_permanent: bool,
    resolve_ts: Option<i64>,
    sa: &crate::spellability::SpellAbility,
) {
    let keywords: Vec<String> = {
        let card = ctx.game.card(card_id);
        keywords
            .iter()
            .map(|kw| {
                if kw.contains("CardManaCost") {
                    kw.replace("CardManaCost", &card.mana_cost.short_string())
                } else if kw.contains("ConvertedManaCost") {
                    kw.replace("ConvertedManaCost", &card.mana_value().to_string())
                } else {
                    kw.clone()
                }
            })
            .collect()
    };
    let keywords = keywords.as_slice();
    let until_next_turn = matches!(
        sa.ir.duration,
        Some(
            crate::spellability::AbilityDuration::UntilYourNextTurn
                | crate::spellability::AbilityDuration::UntilTheEndOfYourNextTurn
                | crate::spellability::AbilityDuration::UntilHostLeavesPlay
                | crate::spellability::AbilityDuration::UntilLoseControlOfHost
                | crate::spellability::AbilityDuration::AsLongAsControl
                | crate::spellability::AbilityDuration::AsLongAsInPlay
        )
    );
    if is_perpetual {
        let ts = resolve_ts.expect("perpetual resolve timestamp must exist");
        let card = ctx.game.card_mut(card_id);
        perpetual_pt_boost::PerpetualPtBoost {
            timestamp: ts,
            power: att,
            toughness: def,
        }
        .apply_effect(card);
        for kw in keywords {
            perpetual_keywords::PerpetualKeywords {
                timestamp: ts,
                add_keywords: vec![kw.clone()],
                remove_keywords: Vec::new(),
                remove_all: false,
            }
            .apply_effect(card);
        }
    } else if is_permanent {
        if att != 0 || def != 0 {
            let timestamp = ctx.game.next_effect_timestamp();
            ctx.game
                .card_mut(card_id)
                .add_pt_boost_at(att, def, timestamp);
        }
        let card = ctx.game.card_mut(card_id);
        if !keywords.is_empty() {
            card.capture_changed_characteristics_baseline_if_needed();
        }
        for kw in keywords {
            if card.add_changed_card_keywords(kw) {
                card.add_lasting_keyword_triggers(kw);
            }
        }
    } else if until_next_turn {
        if att != 0 || def != 0 {
            let timestamp = ctx.game.next_effect_timestamp();
            ctx.game
                .card_mut(card_id)
                .add_pt_boost_at(att, def, timestamp);
            crate::ability::spell_ability_effect::add_until_command(
                ctx.game,
                sa.ir.duration.as_ref(),
                sa.activating_player,
                sa.source,
                crate::phase::PhaseCommand::RemovePtBoost {
                    card: card_id,
                    timestamp,
                },
            );
        }
        if !keywords.is_empty() {
            ctx.game
                .card_mut(card_id)
                .capture_changed_characteristics_baseline_if_needed();
        }
        for kw in keywords {
            ctx.game.card_mut(card_id).add_changed_card_keywords(kw);
            crate::ability::spell_ability_effect::add_until_command(
                ctx.game,
                sa.ir.duration.as_ref(),
                sa.activating_player,
                sa.source,
                crate::phase::PhaseCommand::RemoveKeyword {
                    card: card_id,
                    keyword: kw.clone(),
                },
            );
        }
    } else {
        ctx.game.card_mut(card_id).add_pt_boost(att, def);
        for kw in keywords {
            ctx.game.card_mut(card_id).add_pump_keyword(kw);
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

    fn make_creature(game: &mut GameState, owner: PlayerId, name: &str) -> CardId {
        let c = Card::new(
            CardId(0),
            name.into(),
            owner,
            CardTypeLine::parse("Creature - Human Soldier"),
            ManaCost::parse("1 W"),
            ColorSet::WHITE,
            Some(2),
            Some(2),
            vec![],
            vec![],
        );
        game.create_card(c)
    }

    fn make_ctx<'a>(
        game: &'a mut GameState,
        agents: &'a mut Vec<Box<dyn crate::agent::PlayerAgent>>,
        th: &'a mut TriggerHandler,
        mp: &'a mut Vec<ManaPool>,
        templates: &'a HashMap<String, Card>,
        templates_variants: &'a HashMap<(String, String), usize>,
        token_fallback: &'a HashMap<String, String>,
        edition_dates: &'a HashMap<String, String>,
        rng: &'a mut dyn crate::game_rng::GameRng,
    ) -> EffectContext<'a> {
        EffectContext {
            game,
            combat: None,
            agents,
            trigger_handler: th,
            token_templates: templates,
            token_art_variants: templates_variants,
            token_fallback,
            edition_dates,
            mana_pools: mp,
            parent_target_card: None,
            rng,
        }
    }

    #[test]
    fn non_targeted_pump_defaults_to_self_like_java() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let guardian = make_creature(&mut game, p0, "Guardian of New Benalia");
        game.move_card(guardian, ZoneType::Battlefield, p0);

        let sa = SpellAbility::new_simple(
            Some(guardian),
            p0,
            "AB$ Pump | KW$ Indestructible | SpellDescription$ CARDNAME gains indestructible until end of turn.",
        );

        let mut th = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mp = vec![ManaPool::default(), ManaPool::default()];
        let templates = HashMap::default();
        let templates_variants: HashMap<(String, String), usize> = HashMap::default();
        let token_fallback: HashMap<String, String> = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter;
        let mut ctx = make_ctx(
            &mut game,
            &mut agents,
            &mut th,
            &mut mp,
            &templates,
            &templates_variants,
            &token_fallback,
            &edition_dates,
            &mut rng_adapter,
        );

        super::PumpEffect::resolve(&mut ctx, &sa);

        assert!(ctx.game.card(guardian).has_indestructible());
    }

    #[test]
    fn targeted_pump_does_not_fall_back_to_source() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let source = make_creature(&mut game, p0, "Source");
        game.move_card(source, ZoneType::Battlefield, p0);

        let sa = SpellAbility::new_simple(
            Some(source),
            p0,
            "SP$ Pump | ValidTgts$ Creature | KW$ Indestructible",
        );

        let mut th = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mp = vec![ManaPool::default(), ManaPool::default()];
        let templates = HashMap::default();
        let templates_variants: HashMap<(String, String), usize> = HashMap::default();
        let token_fallback: HashMap<String, String> = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter;
        let mut ctx = make_ctx(
            &mut game,
            &mut agents,
            &mut th,
            &mut mp,
            &templates,
            &templates_variants,
            &token_fallback,
            &edition_dates,
            &mut rng_adapter,
        );

        super::PumpEffect::resolve(&mut ctx, &sa);

        assert!(!ctx.game.card(source).has_indestructible());
    }
}
