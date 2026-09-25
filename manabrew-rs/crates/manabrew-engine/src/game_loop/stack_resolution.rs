use super::cost_payment::CostPaymentContext;
use super::*;
use crate::replacement::replacement_handler::apply_moved_replacement;

impl GameLoop {
    /// Java `WrappedAbility.resolve`: a trigger whose intervening-if has stopped holding does
    /// nothing when it resolves, unless it is an Always trigger or asks for `NoResolvingCheck$`.
    fn trigger_requirements_still_met(game: &GameState, sa: &SpellAbility) -> bool {
        let Some(index) = sa.trigger_index else {
            return true;
        };
        let Some(host) = sa.trigger_source.or(sa.source) else {
            return true;
        };
        let Some(trigger) = game.card(host).triggers.get(index) else {
            return true;
        };
        if trigger.kind == crate::trigger::TriggerType::Always || trigger.ir.no_resolving_check {
            return true;
        }
        let triggering_objects = crate::event::RunParams {
            card: sa
                .get_triggering_cards(crate::ability::AbilityKey::Card)
                .first()
                .copied(),
            attacked_player: sa
                .get_triggering_player(crate::ability::AbilityKey::Attacked)
                .or_else(|| sa.get_triggering_player(crate::ability::AbilityKey::Defender)),
            attacking_player: sa.get_triggering_player(crate::ability::AbilityKey::AttackingPlayer),
            ..Default::default()
        };
        trigger.requirements_check(game, host)
            && trigger.meets_requirements_on_triggered_objects(
                game,
                &triggering_objects,
                sa.get_triggering_spell_ability(crate::ability::AbilityKey::SpellAbility),
                host,
            )
    }

    fn effect_kind_for_sa(sa: &SpellAbility) -> String {
        if let Some(api) = sa.api {
            return api.name().to_string();
        }
        if sa.is_trigger {
            if let Some(mode) = sa.ir.mode.as_ref() {
                return format!("Trigger({mode})");
            }
            return "Trigger".to_string();
        }
        if sa.is_activated {
            return "ActivatedAbility".to_string();
        }
        if sa.is_spell {
            return "Spell".to_string();
        }
        "Effect".to_string()
    }

    pub fn resolve_stack(&mut self, game: &mut GameState, agents: &mut [Box<dyn PlayerAgent>]) {
        let _perf_scope = crate::perf::ParamsLookupScopeGuard::enter(
            crate::perf::ParamsLookupScope::StackResolution,
        );
        if game.stack.is_empty() {
            return;
        }

        if Self::stack_trace_enabled() {
            let names: Vec<String> = game
                .stack
                .iter()
                .map(|entry| {
                    entry
                        .spell_ability
                        .source
                        .map(|cid| game.card(cid).card_name.clone())
                        .unwrap_or_else(|| "<effect>".to_string())
                })
                .collect();
            eprintln!(
                "[stack-trace] RESOLVE start phase={:?} active={:?} priority={:?} depth={} {:?}",
                game.turn.phase,
                game.active_player(),
                game.turn.priority_player,
                game.stack.len(),
                names
            );
        }

        // LKI: Snapshot battlefield state before resolution.
        // Mirrors Java MagicStack line 623: game.copyLastState() before resolving.
        game.copy_last_state();

        let mut entry = game.stack.resolve_stack().unwrap();
        let stack_item_name = entry
            .spell_ability
            .source
            .and_then(|cid| game.cards.get(cid.index()).map(|c| c.card_name.clone()))
            .unwrap_or_else(|| "Ability".to_string());
        self.log_stack_resolved_item(&stack_item_name);
        if Self::stack_trace_enabled() {
            eprintln!(
                "[stack-trace] POP resolving={} remaining_depth={}",
                stack_item_name,
                game.stack.len()
            );
        }

        let copied_spell_host =
            crate::card::card_factory::is_copied_spell_host(game, &entry.spell_ability);

        // Fizzle check — mirrors Java's MagicStack.hasFizzled() (CR 608.2b).
        // A spell or ability is countered by game rules if ALL of its targets
        // are illegal on resolution. Walk the SA chain; if every targeting node
        // has only invalid targets, the whole thing fizzles.
        if Self::trigger_trace_enabled() && entry.optional_trigger_decider.is_some() {
            eprintln!(
                "[trigger-trace] RESOLVING optional trigger from stack: {} api={:?}",
                stack_item_name, entry.spell_ability.api
            );
        }
        if Self::has_fizzled(&mut entry.spell_ability, game) {
            crate::agent::notify_all_agents(
                agents,
                crate::agent::GameLogEvent::warning(format!(
                    "{stack_item_name} fizzles (all targets invalid)"
                ))
                .with_player(entry.spell_ability.activating_player),
            );
            // CR 608.2b: A countered spell is still put into its owner's
            // graveyard (or exile for flashback/escape). Only triggers and
            // activated abilities have no physical card to move.
            if !entry.spell_ability.is_trigger
                && !entry.spell_ability.is_activated
                && !entry.spell_ability.is_copy
            {
                if let Some(card_id) = entry.spell_ability.source {
                    let owner = game.card(card_id).owner;
                    let dest = if entry.spell_ability.alt_cost
                        == Some(crate::spellability::AlternativeCost::Harmonize)
                        || entry.spell_ability.alt_cost
                            == Some(crate::spellability::AlternativeCost::Flashback)
                    {
                        apply_moved_replacement(
                            game,
                            card_id,
                            ZoneType::Graveyard,
                            Some(&entry.spell_ability),
                            Some(true),
                            Some(agents),
                            Some(&mut self.replacement_runtime()),
                        )
                    } else if entry.spell_ability.alt_cost
                        == Some(crate::spellability::AlternativeCost::Escape)
                    {
                        ZoneType::Exile
                    } else {
                        // Apply Moved replacement WITH agents for proper RNG consumption
                        // (e.g. Rest in Peace + Leyline of the Void both redirecting).
                        apply_moved_replacement(
                            game,
                            card_id,
                            ZoneType::Graveyard,
                            Some(&entry.spell_ability),
                            Some(true),
                            Some(agents),
                            Some(&mut self.replacement_runtime()),
                        )
                    };
                    if game.card(card_id).zone == ZoneType::Stack {
                        self.move_card_with_runtime(game, card_id, dest, owner, agents);
                    }
                }
            }
            if copied_spell_host {
                Self::cease_to_exist_copied_spell(game, entry.spell_ability.source);
            }
            apply_continuous_effects(game);
            game.copy_last_state_combat_lki(&self.combat);
            game.stack.finish_resolving();
            return;
        }
        game.copy_last_state_combat_lki(&self.combat);

        // Copy spells: resolve effect only. CR 707.10 / 111.11 — a copy of a
        // permanent spell becomes a token (`GameAction.changeZone` line 94).
        if entry.spell_ability.is_copy
            && !(copied_spell_host && (entry.is_creature_spell || entry.is_permanent_spell))
        {
            let should_create_token_copy = (entry.is_creature_spell || entry.is_permanent_spell)
                && entry.spell_ability.source.is_some();
            if should_create_token_copy {
                self.resolve_copied_permanent_as_token(game, agents, &entry);
            } else {
                self.resolve_spell_effect(game, agents, &entry);
            }
            if copied_spell_host {
                Self::cease_to_exist_copied_spell(game, entry.spell_ability.source);
            }
            crate::perf::increment(crate::perf::Metric::SpellAbilityClones, 3);
            self.trigger_handler.run_trigger(
                TriggerType::AbilityResolves,
                RunParams {
                    card: entry.spell_ability.source,
                    spell_card: entry.spell_ability.source,
                    spell_controller: Some(entry.spell_ability.activating_player),
                    spell_ability: Some(entry.spell_ability.clone()),
                    source_sa: Some(entry.spell_ability.clone()),
                    cause: Some(entry.spell_ability.clone()),
                    cause_card: entry.spell_ability.source,
                    ..Default::default()
                },
                false,
            );
            apply_continuous_effects(game);
            game.stack.finish_resolving();
            return;
        }

        if entry.spell_ability.is_trigger
            && !Self::trigger_requirements_still_met(game, &entry.spell_ability)
        {
            apply_continuous_effects(game);
            game.stack.finish_resolving();
            return;
        }

        if entry.spell_ability.is_trigger || entry.spell_ability.is_activated {
            // Optional trigger confirmation — mirrors Java's WrappedAbility.resolve()
            // calling confirmTrigger() FIRST, before cost payment or effect resolution.
            // This happens at resolution time, AFTER the trigger has been on the stack
            // and priority has passed.
            if let Some(decider) = entry.optional_trigger_decider {
                let mut description = entry
                    .optional_trigger_description
                    .clone()
                    .unwrap_or_default();
                if let Some(triggered_card_id) = entry
                    .spell_ability
                    .get_triggering_card(crate::ability::AbilityKey::Card)
                {
                    let triggered_name = game.card(triggered_card_id).card_name.clone();
                    if !triggered_name.is_empty() && !description.contains(&triggered_name) {
                        if !description.is_empty() {
                            description.push(' ');
                        }
                        description.push_str(&format!("Triggered by {triggered_name}."));
                    }
                }
                let api = entry.spell_ability.api;
                let accepted = agents[decider.index()].choose_optional_trigger(
                    decider,
                    &description,
                    entry.spell_ability.source,
                    api,
                );
                if !accepted {
                    apply_continuous_effects(game);
                    game.stack.finish_resolving();
                    return;
                }
            }

            if entry.spell_ability.is_trigger || entry.spell_ability.trigger_source.is_some() {
                if let Some(cost) = entry.spell_ability.pay_costs.clone() {
                    let player = entry.spell_ability.activating_player;
                    let source = entry.spell_ability.source.unwrap_or(CardId(0));
                    let api = entry.spell_ability.api;
                    let x_paid_before = game.card(source).svars.get("XPaid").cloned();
                    if crate::cost::has_x_in_any_cost_part(&cost) {
                        game.card_mut(source).svars.remove("XPaid");
                    }
                    let available = crate::mana::calculate_available_mana_excluding(
                        &self.mana_pools[player.index()],
                        game,
                        player,
                        Some(source),
                    );
                    if !crate::cost::can_pay_with_ability(
                        &cost,
                        game,
                        &available,
                        source,
                        player,
                        Some(&entry.spell_ability),
                    ) {
                        Self::reset_x_mana_cost_paid(game, source, x_paid_before);
                        apply_continuous_effects(game);
                        game.stack.finish_resolving();
                        return;
                    }
                    let mut need_x = true;
                    if !self.announce_values_like_x(
                        game,
                        agents,
                        player,
                        &mut entry.spell_ability,
                        Some(&cost),
                        &mut need_x,
                    ) {
                        Self::reset_x_mana_cost_paid(game, source, x_paid_before);
                        apply_continuous_effects(game);
                        game.stack.finish_resolving();
                        return;
                    }
                    if !self.pay_ability_cost(
                        game,
                        agents,
                        player,
                        source,
                        &cost,
                        api,
                        cost.mandatory,
                        CostPaymentContext::TriggerResolve,
                        Some(&mut entry.spell_ability),
                    ) {
                        Self::reset_x_mana_cost_paid(game, source, x_paid_before);
                        apply_continuous_effects(game);
                        game.stack.finish_resolving();
                        return;
                    }
                }
            }

            // Triggered/activated ability: resolve the effect
            if let Some(source_id) = entry
                .spell_ability
                .trigger_source
                .or(entry.spell_ability.source)
            {
                game.card_mut(source_id)
                    .add_ability_resolved_for(Some(&entry.spell_ability));
            }
            self.resolve_spell_effect(game, agents, &entry);
            crate::perf::increment(crate::perf::Metric::SpellAbilityClones, 3);
            self.trigger_handler.run_trigger(
                TriggerType::AbilityResolves,
                RunParams {
                    card: entry.spell_ability.source,
                    spell_card: entry.spell_ability.source,
                    spell_controller: Some(entry.spell_ability.activating_player),
                    spell_ability: Some(entry.spell_ability.clone()),
                    source_sa: Some(entry.spell_ability.clone()),
                    cause: Some(entry.spell_ability.clone()),
                    cause_card: entry.spell_ability.source,
                    ..Default::default()
                },
                false,
            );
        } else if let Some(card_id) = entry.spell_ability.source {
            let alt_cost = entry.spell_ability.alt_cost;
            let player = entry.spell_ability.activating_player;

            if entry.is_creature_spell || entry.is_permanent_spell {
                // Permanent spell: move to battlefield
                let origin = game.card(card_id).zone;

                // Propagate kicked flag to the card so triggers with
                // ValidCard$ Card.Self+kicked can check it after resolution.
                if entry.spell_ability.kicked {
                    game.card_mut(card_id).set_kicked(true);
                }

                // Mirrors Java `GameAction.changeZone()` setting `castFrom` on
                // the resolving spell's host card so `wasCast`/`wasCastByYou`
                // valid filters (e.g. Sunderflock's ETB) see the card as cast.
                // Set BEFORE resolving the ETB so the trigger pickup matches.
                game.card_mut(card_id).cast_from = entry.cast_from_zone;

                // Resolve any ETB effects defined on the card
                self.resolve_spell_effect(game, agents, &entry);
                crate::perf::increment(crate::perf::Metric::SpellAbilityClones, 3);
                self.trigger_handler.run_trigger(
                    TriggerType::AbilityResolves,
                    RunParams {
                        card: Some(card_id),
                        spell_card: Some(card_id),
                        spell_controller: Some(player),
                        spell_ability: Some(entry.spell_ability.clone()),
                        source_sa: Some(entry.spell_ability.clone()),
                        cause: Some(entry.spell_ability.clone()),
                        cause_card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );

                // Morph/Megamorph: enter face-down as a 2/2 creature
                if alt_cost.is_some_and(|ac| ac.is_morph()) {
                    let c = game.card_mut(card_id);
                    let face_up_keyword_cost = crate::card::card_factory_util::face_up_keyword_cost;
                    let disguise_cost = face_up_keyword_cost(c, "Disguise");
                    let megamorph_cost = face_up_keyword_cost(c, "Megamorph");
                    let is_mega = disguise_cost.is_none() && megamorph_cost.is_some();
                    let morph_details = disguise_cost
                        .clone()
                        .or(megamorph_cost)
                        .or_else(|| face_up_keyword_cost(c, "Morph"))
                        .unwrap_or_else(|| "3".to_string());
                    c.set_face_down(true);
                    c.set_original_state_as_face_down();
                    // Java's MayPlay copy rebuilds its params from `originalMapParams`
                    // (`CardTraitBase.copyHelper`), dropping the `FaceDownKeyword$ Ward:2`
                    // that `putParam` set on the Disguise cast.
                    if disguise_cost.is_some()
                        && !entry.spell_ability.cast_with_may_play
                        && entry.spell_ability.may_play_source.is_none()
                    {
                        c.add_intrinsic_keyword_with_triggers("Ward:2");
                    }
                    c.static_set_power = Some(crate::spellability::MORPH_PT);
                    c.static_set_toughness = Some(crate::spellability::MORPH_PT);

                    // Add "turn face up" activated ability (morph cost → SetState TurnFaceUp).
                    // This is a game rule, not a card ability — face-down morph creatures
                    // can always be turned face up by paying the morph cost.
                    crate::card::card_factory_util::ability_morph_up(
                        c,
                        &morph_details,
                        is_mega,
                        disguise_cost.is_some(),
                    );
                }

                if entry.spell_ability.target_chosen.target_card.is_some()
                    || entry.spell_ability.target_chosen.target_player.is_some()
                {
                    Self::attach_aura_on_resolution(game, agents, &entry, card_id);
                }
                game.ensure_pending_change_zone_table();
                if origin != ZoneType::Battlefield {
                    self.move_card_with_runtime(
                        game,
                        card_id,
                        ZoneType::Battlefield,
                        player,
                        agents,
                    );
                }

                // Attach aura to its chosen target.
                // Mirrors Java's GameAction.changeZone (line 373-389) which detects
                // an Aura entering from the stack while not-yet-attached and routes
                // through `attachAuraOnIndirectETB`. For *cast* Auras the spell
                // already has a chosen target — narrow the chooser candidates to
                // that single target so the decision-log emits a `pick_one[1]`
                // (matching Java's deterministic behaviour, which only sees the
                // chosen target after the resolution-time filter). Without this
                // narrowing, the candidate enumeration over the full battlefield
                // can include a *second* legendary creature (e.g. a mirror-match
                // Ashling controlled by the opponent), and the agent ends up
                // attaching the Aura to the wrong card.
                Self::attach_aura_on_resolution(game, agents, &entry, card_id);

                // Evoke: register a one-shot ETB trigger that sacrifices this creature.
                // This mirrors Forge Java semantics where Evoke uses a ChangesZone trigger
                // and allows normal ETB abilities to trigger before the sacrifice resolves.
                // Java parity: `CardFactoryUtil` attaches the "sacrifice when it enters"
                // trigger once per Evoke keyword on the card, not once per cast. A card
                // with both intrinsic `Evoke:1 U` and a granted `Evoke:4` (e.g. an
                // Elemental in hand under Ashling, the Limitless) therefore carries
                // two Evoke sac triggers. All of them fire on ETB; the first sacrifice
                // succeeds and the rest are no-ops because the creature has already
                // left the battlefield, but each one still consumes a stack entry.
                if alt_cost == Some(crate::spellability::AlternativeCost::Evoke) {
                    let evoke_keyword_count =
                        (entry.spell_ability.evoke_keyword_count as usize).max(1);
                    self.register_evoke_sacrifice_triggers(
                        game,
                        card_id,
                        player,
                        evoke_keyword_count,
                    );
                }

                let room_door = (game.card(card_id).type_line.has_subtype("Room")
                    && game.card(card_id).has_s_var("RoomRightSplitCost"))
                .then(|| {
                    entry
                        .spell_ability
                        .ir
                        .card_state_name
                        .clone()
                        .unwrap_or_else(|| "LeftSplit".to_string())
                });
                if let Some(door) = room_door.as_deref() {
                    if let Some(state) = forge_foundation::CardStateName::from_str_compat(door) {
                        game.card_mut(card_id).unlock_room_door(state);
                    }
                }

                // Register triggers for the new permanent
                self.trigger_handler.register_active_trigger(game, card_id);

                if let Some(door) = room_door {
                    let card_state_name = Some(door);
                    self.trigger_handler.run_trigger(
                        TriggerType::UnlockDoor,
                        RunParams {
                            card: Some(card_id),
                            player: Some(player),
                            card_state_name,
                            ..Default::default()
                        },
                        true,
                    );
                }

                // Emit ChangesZone trigger (ETB)
                crate::ability::effects::emit_zone_trigger(
                    &mut self.trigger_handler,
                    card_id,
                    origin,
                    ZoneType::Battlefield,
                );
                if let Some(table) = game.pending_change_zone_table.take() {
                    table.trigger_changes_zone_all(
                        &mut self.trigger_handler,
                        game,
                        Some(&entry.spell_ability),
                    );
                }

                // -- Post-ETB effects for alternative costs --

                if alt_cost == Some(crate::spellability::AlternativeCost::Sneak) {
                    game.card_mut(card_id).set_tapped(true);
                }
                if alt_cost == Some(crate::spellability::AlternativeCost::Sneak)
                    && game.card(card_id).is_creature()
                {
                    let returned = entry
                        .spell_ability
                        .paid_hash
                        .get(crate::cost::cost_return::HASH_LKI)
                        .and_then(|ids| ids.first())
                        .and_then(|id| id.parse::<u32>().ok())
                        .map(CardId);
                    let defender = returned.and_then(|returned| {
                        self.combat.get_defender_by_attacker(returned).or_else(|| {
                            self.combat
                                .get_combat_lki(returned)
                                .and_then(|lki| lki.defender)
                        })
                    });
                    if let Some(defender) = defender {
                        self.combat.add_attacker(
                            card_id,
                            defender,
                            game.card(card_id).zone_timestamp,
                        );
                        let defending_player = defender.controlling_player(game);
                        let card = game.card_mut(card_id);
                        card.set_attacking_player(defending_player);
                        card.mark_attacked_this_turn();
                    }
                }

                // Dash: grant haste, register delayed trigger to return to hand at EOT
                if alt_cost == Some(crate::spellability::AlternativeCost::Dash) {
                    let timestamp = game.next_timestamp();
                    game.card_mut(card_id).add_pump_keyword("Haste", timestamp);
                    self.trigger_handler.register_delayed_trigger(
                        crate::trigger::handler::DelayedTrigger {
                            mode: TriggerType::Phase,
                            trigger_mode: Box::new(crate::trigger::trigger_phase::TriggerPhase {
                                phases: vec![forge_foundation::PhaseType::EndOfTurn],
                                valid_player: None,
                            }) as Box<dyn crate::trigger::TriggerBehavior>,
                            params: crate::parsing::Params::default(),
                            execute_svar: format!(
                                "DB$ ChangeZone | Origin$ Battlefield | Destination$ Hand | Defined$ CardUID_{}", card_id.0
                            ),
                            controller: player,
                            source_card: card_id,
                            created_turn: game.turn.turn_number,
                            created_phase: game.turn.phase,
                            target_card: Some(card_id),
                            remembered_amount: 0,
                            remembered_cards: Vec::new(),
                            remembered_players: Vec::new(),
                            remembered_lki_cards: Vec::new(),
                            target_card_zone_timestamp: None,
                            sort_after_active: false,
                trigger_order: None,
                source_timestamp: None,
                spawning_ability: None,
                        },
                    );
                }

                // Warp: mark card so it can be cast from exile on a later turn,
                // and register delayed trigger to exile at EOT.
                // Mirrors Java PermanentEffect + StaticAbilityCastWithFlash for Warp.
                if alt_cost == Some(crate::spellability::AlternativeCost::Warp) {
                    game.card_mut(card_id)
                        .keywords
                        .add(crate::card::KEYWORD_WARP_EXILED);
                    self.trigger_handler.register_delayed_trigger(
                        crate::trigger::handler::DelayedTrigger {
                            mode: TriggerType::Phase,
                            trigger_mode: Box::new(crate::trigger::trigger_phase::TriggerPhase {
                                phases: vec![forge_foundation::PhaseType::EndOfTurn],
                                valid_player: None,
                            }) as Box<dyn crate::trigger::TriggerBehavior>,
                            params: crate::parsing::Params::default(),
                            execute_svar: format!(
                                "DB$ ChangeZone | Origin$ Battlefield | Destination$ Exile | Defined$ CardUID_{}", card_id.0
                            ),
                            controller: player,
                            source_card: card_id,
                            created_turn: game.turn.turn_number,
                            created_phase: game.turn.phase,
                            target_card: Some(card_id),
                            remembered_amount: 0,
                            remembered_cards: Vec::new(),
                            remembered_players: Vec::new(),
                            remembered_lki_cards: Vec::new(),
                            target_card_zone_timestamp: None,
                            sort_after_active: false,
                trigger_order: None,
                source_timestamp: None,
                spawning_ability: None,
                        },
                    );
                }

                // Bestow: attach to a creature as an Aura
                if alt_cost == Some(crate::spellability::AlternativeCost::Bestow) {
                    if let Some(target) = entry.spell_ability.target_chosen.target_card {
                        game.card_mut(card_id).unanimate_bestow();
                        game.attach_to(card_id, target);
                    }
                }

                // Blitz: grant haste + "dies: draw a card" + sacrifice at EOT
                if alt_cost == Some(crate::spellability::AlternativeCost::Blitz) {
                    let timestamp = game.next_timestamp();
                    game.card_mut(card_id).add_pump_keyword("Haste", timestamp);
                    let trig_id = game.card(card_id).triggers.len() as u32;
                    let params = crate::parsing::Params::from_raw(
                        "Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self"
                    );
                    let dies_trigger = crate::trigger::Trigger {
                        id: trig_id,
                        base: {
                            let mut base =
                                crate::game_loop::trigger_replacement_base::TriggerReplacementBase::default();
                            base.card_trait_base.set_id(trig_id as i32);
                            base.card_trait_base.set_intrinsic(false);
                            base.valid_host_zones = Some(vec![ZoneType::Battlefield]);
                            base
                        },
                        kind: crate::trigger::TriggerType::ChangesZone,
                        mode: Box::new(crate::trigger::trigger_changes_zone::TriggerChangesZone),
                        ir: crate::trigger::TriggerIr::from_params(&params),
                        execute: "BlitzDiesDraw".to_string(),
                        optional: false,
                        description: "When this creature dies, draw a card.".to_string(),
                        static_trigger: false,
                        trigger_remembered: Vec::new(),
                        spawning_ability: None,
                        original_host: None,
                    };
                    game.card_mut(card_id).add_lasting_trigger(dies_trigger);
                    game.card_mut(card_id)
                        .set_s_var("BlitzDiesDraw", "DB$ Draw | NumCards$ 1 | Defined$ You");
                    self.trigger_handler.unregister_active_triggers(card_id);
                    self.trigger_handler.register_active_trigger(game, card_id);

                    self.trigger_handler.register_delayed_trigger(
                        crate::trigger::handler::DelayedTrigger {
                            mode: TriggerType::Phase,
                            trigger_mode: Box::new(crate::trigger::trigger_phase::TriggerPhase {
                                phases: vec![forge_foundation::PhaseType::EndOfTurn],
                                valid_player: None,
                            })
                                as Box<dyn crate::trigger::TriggerBehavior>,
                            params: crate::parsing::Params::default(),
                            execute_svar: format!("DB$ Sacrifice | Defined$ CardUID_{}", card_id.0),
                            controller: player,
                            source_card: card_id,
                            created_turn: game.turn.turn_number,
                            created_phase: game.turn.phase,
                            target_card: Some(card_id),
                            remembered_amount: 0,
                            remembered_cards: Vec::new(),
                            remembered_players: Vec::new(),
                            remembered_lki_cards: Vec::new(),
                            target_card_zone_timestamp: None,
                            sort_after_active: false,
                            trigger_order: None,
                            source_timestamp: None,
                            spawning_ability: None,
                        },
                    );
                }
            } else {
                // Non-permanent spell: resolve effect, then route to destination zone
                game.card_mut(card_id).cast_from = entry.cast_from_zone;
                self.resolve_spell_effect(game, agents, &entry);
                game.card_mut(card_id).cast_from = None;
                crate::perf::increment(crate::perf::Metric::SpellAbilityClones, 3);
                self.trigger_handler.run_trigger(
                    TriggerType::AbilityResolves,
                    RunParams {
                        card: Some(card_id),
                        spell_card: Some(card_id),
                        spell_controller: Some(player),
                        spell_ability: Some(entry.spell_ability.clone()),
                        source_sa: Some(entry.spell_ability.clone()),
                        cause: Some(entry.spell_ability.clone()),
                        cause_card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );
                let owner = game.card(card_id).owner;
                // Only move if still in stack zone (some effects move the card themselves)
                if game.card(card_id).zone != ZoneType::Exile
                    && game.card(card_id).zone != ZoneType::Library
                    && game.card(card_id).zone != ZoneType::Hand
                {
                    // Determine destination based on alternative cost / keywords
                    let dest = if alt_cost == Some(crate::spellability::AlternativeCost::Harmonize)
                        || alt_cost == Some(crate::spellability::AlternativeCost::Flashback)
                    {
                        apply_moved_replacement(
                            game,
                            card_id,
                            ZoneType::Graveyard,
                            Some(&entry.spell_ability),
                            Some(false),
                            Some(agents),
                            Some(&mut self.replacement_runtime()),
                        )
                    } else if alt_cost == Some(crate::spellability::AlternativeCost::Escape) {
                        ZoneType::Exile
                    } else if entry.spell_ability.buyback_paid {
                        // Buyback: return to hand instead of graveyard
                        ZoneType::Hand
                    } else if game.card(card_id).has_rebound()
                        && entry.cast_from_zone == Some(ZoneType::Hand)
                    {
                        // Rebound: exile instead of graveyard (will be cast next upkeep)
                        self.trigger_handler.register_delayed_trigger(
                            crate::trigger::handler::DelayedTrigger {
                                mode: TriggerType::Phase,
                                trigger_mode: Box::new(
                                    crate::trigger::trigger_phase::TriggerPhase {
                                        phases: vec![forge_foundation::PhaseType::Upkeep],
                                        valid_player: Some(
                                            crate::parsing::cached_compiled_selector("You"),
                                        ),
                                    },
                                )
                                    as Box<dyn crate::trigger::TriggerBehavior>,
                                params: crate::parsing::Params::default(),
                                execute_svar: format!(
                                    "DB$ Play | Defined$ CardUID_{} | WithoutManaCost$ True",
                                    card_id.0
                                ),
                                controller: player,
                                source_card: card_id,
                                created_turn: game.turn.turn_number,
                                created_phase: game.turn.phase,
                                target_card: Some(card_id),
                                remembered_amount: 0,
                                remembered_cards: Vec::new(),
                                remembered_players: Vec::new(),
                                remembered_lki_cards: Vec::new(),
                                target_card_zone_timestamp: None,
                                sort_after_active: false,
                                trigger_order: None,
                                source_timestamp: None,
                                spawning_ability: None,
                            },
                        );
                        ZoneType::Exile
                    } else {
                        apply_moved_replacement(
                            game,
                            card_id,
                            ZoneType::Graveyard,
                            Some(&entry.spell_ability),
                            Some(false),
                            Some(agents),
                            Some(&mut self.replacement_runtime()),
                        )
                    };
                    if game.card(card_id).zone == ZoneType::Stack {
                        self.move_card_with_runtime(game, card_id, dest, owner, agents);
                    }
                }
            }
        }

        // Mark resolution complete on the stack.
        game.stack.finish_resolving();

        // Continuous effects might change after resolution
        apply_continuous_effects(game);
        self.run_static_state_triggers(game, agents);

        // LKI: Second snapshot after resolution and SBAs, before processing triggers.
        // Mirrors Java MagicStack line 676: game.copyLastState() in finishResolving().
        // Java does not run SBA here; it defers that to the next priority loop.
        // Keep the snapshot pre-SBA so deep parity aligns with Java's
        // GameEventPlayerPriority boundary.
        game.copy_last_state();
        game.copy_last_state_combat_lki(&self.combat);

        // Java parity: triggers fired during resolution are queued now and only
        self.trigger_handler.flush_waiting_triggers(game);
        self.trigger_handler.reset_active_triggers(game);
        if game.stack.is_empty() && self.trigger_handler.pre_matched_trigger_count() == 0 {
            game.clear_change_zone_lki_info();
        }
    }

    pub(crate) fn resolve_spell_effect(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        entry: &StackEntry,
    ) {
        // Reset shared parity tables for this stack resolution. The
        // change-zone table must exist (not None) so that moves performed
        // during this resolution are recorded for the post-pass
        // `ChangesZoneAll` trigger fire below.
        game.clear_pending_damage_maps();
        game.clear_pending_change_zone_table();
        game.ensure_pending_change_zone_table();

        // Mirrors Java `MagicStack.resolveStack` (MagicStack.java:651-653):
        // call `handleRemembering` on the root SA before resolving so
        // `RememberTargets$` populates the host's remembered lists in time
        // for sub-abilities like `RememberObjects$ RememberedController`.
        crate::ability::ability_utils::handle_remembering(game, &entry.spell_ability);

        // Walk the SpellAbility chain: resolve each node's effect, propagating
        // the parent SA's chosen target card so sub-abilities can resolve
        // `Defined$ ParentTarget`. Mirrors Java's resolveApiAbility() + resolveSubAbilities().
        let mut parent_target_card: Option<CardId> = None;
        let mut parent_additional_target_cards: indexmap::IndexMap<CardId, i32> =
            indexmap::IndexMap::new();
        let mut parent_target_player = None;
        let mut parent_additional_target_players: Vec<crate::ids::PlayerId> = Vec::new();
        let mut parent_target_stack_entry: Option<u32> = None;
        let mut inherited_trigger_index = entry.spell_ability.trigger_index;
        let root_kicked = entry.spell_ability.kicked;
        let root_x_paid = entry.spell_ability.x_mana_cost_paid;
        let root_trigger_objects = entry.spell_ability.trigger_objects.clone();
        let root_trigger_spell_abilities = &entry.spell_ability.trigger_spell_abilities;
        let root_trigger_source = entry.spell_ability.trigger_source;
        let root_trigger_source_zone_timestamp = entry.spell_ability.trigger_source_zone_timestamp;
        let root_trigger_spawning_ability = entry.spell_ability.trigger_spawning_ability.clone();
        let root_discarded_cost_cards = &entry.spell_ability.discarded_cost_cards;
        let root_paid_hash = &entry.spell_ability.paid_hash;
        let chain_target_cards = entry.spell_ability.chain_target_cards_from_root();
        let mut current = Some(&entry.spell_ability);
        let mut is_first = true;
        while let Some(sa) = current {
            // Refresh agent snapshots between sub-abilities so that prompts
            // (e.g. "choose discard") reflect state changes from earlier
            // sub-abilities (e.g. "draw 2" before "discard 2").
            if !is_first {
                for agent in agents.iter_mut() {
                    agent.snapshot_state(game, &self.mana_pools);
                }
            }
            is_first = false;

            // Propagate kicked flag from root SA to sub-abilities for condition checks
            let mut sa_with_ctx;
            let needs_ctx_clone = (root_kicked && !sa.kicked)
                || sa.x_mana_cost_paid != root_x_paid
                || (parent_target_card.is_some() && sa.target_chosen.target_card.is_none())
                || (parent_target_player.is_some() && sa.target_chosen.target_player.is_none())
                || (parent_target_stack_entry.is_some()
                    && sa.target_chosen.target_stack_entry.is_none())
                || (inherited_trigger_index.is_some() && sa.trigger_index.is_none())
                || sa.parent_targeting_card != parent_target_card
                || sa.parent_targeting_player != parent_target_player
                || (sa.trigger_objects.is_empty() && !root_trigger_objects.is_empty())
                || (sa.trigger_spell_abilities.is_empty()
                    && !root_trigger_spell_abilities.is_empty())
                || (sa.trigger_source.is_none() && root_trigger_source.is_some())
                || (sa.trigger_source_zone_timestamp.is_none()
                    && root_trigger_source_zone_timestamp.is_some())
                || (sa.trigger_spawning_ability.is_none()
                    && root_trigger_spawning_ability.is_some())
                || (sa.discarded_cost_cards.is_empty() && !root_discarded_cost_cards.is_empty())
                || (sa.paid_hash.is_empty() && !root_paid_hash.is_empty())
                || sa.chain_target_cards != chain_target_cards;
            let sa_ref = if needs_ctx_clone {
                sa_with_ctx = sa.clone();
                if root_kicked && !sa_with_ctx.kicked {
                    sa_with_ctx.kicked = true;
                }
                sa_with_ctx.x_mana_cost_paid = root_x_paid;
                if !sa_with_ctx.uses_targeting() {
                    if sa_with_ctx.target_chosen.target_card.is_none() {
                        sa_with_ctx.target_chosen.target_card = parent_target_card;
                        sa_with_ctx
                            .target_chosen
                            .divided_map
                            .clone_from(&parent_additional_target_cards);
                    }
                    if sa_with_ctx.target_chosen.target_player.is_none() {
                        sa_with_ctx.target_chosen.target_player = parent_target_player;
                        sa_with_ctx
                            .target_chosen
                            .additional_target_players
                            .clone_from(&parent_additional_target_players);
                    }
                }
                if sa_with_ctx.target_chosen.target_stack_entry.is_none() {
                    sa_with_ctx.target_chosen.target_stack_entry = parent_target_stack_entry;
                }
                if sa_with_ctx.trigger_index.is_none() {
                    sa_with_ctx.trigger_index = inherited_trigger_index;
                }
                sa_with_ctx.parent_targeting_card = parent_target_card;
                sa_with_ctx.parent_targeting_player = parent_target_player;
                if sa_with_ctx.trigger_objects.is_empty() {
                    sa_with_ctx.trigger_objects = root_trigger_objects.clone();
                }
                if sa_with_ctx.trigger_spell_abilities.is_empty() {
                    sa_with_ctx
                        .trigger_spell_abilities
                        .clone_from(root_trigger_spell_abilities);
                }
                if sa_with_ctx.trigger_source.is_none() {
                    sa_with_ctx.trigger_source = root_trigger_source;
                }
                if sa_with_ctx.trigger_source_zone_timestamp.is_none() {
                    sa_with_ctx.trigger_source_zone_timestamp = root_trigger_source_zone_timestamp;
                }
                if sa_with_ctx.trigger_spawning_ability.is_none() {
                    sa_with_ctx.trigger_spawning_ability = root_trigger_spawning_ability.clone();
                }
                if sa_with_ctx.discarded_cost_cards.is_empty() {
                    sa_with_ctx
                        .discarded_cost_cards
                        .clone_from(root_discarded_cost_cards);
                }
                if sa_with_ctx.paid_hash.is_empty() {
                    sa_with_ctx.paid_hash.clone_from(root_paid_hash);
                }
                sa_with_ctx
                    .chain_target_cards
                    .clone_from(&chain_target_cards);
                &sa_with_ctx
            } else {
                sa
            };
            self.resolve_single_effect(game, agents, sa_ref, parent_target_card);
            if sa_ref.target_chosen.target_card.is_some() {
                parent_target_card = sa_ref.target_chosen.target_card;
                parent_additional_target_cards.clone_from(&sa_ref.target_chosen.divided_map);
            }
            if sa_ref.target_chosen.target_player.is_some() {
                parent_target_player = sa_ref.target_chosen.target_player;
                parent_additional_target_players
                    .clone_from(&sa_ref.target_chosen.additional_target_players);
            }
            parent_target_stack_entry = sa_ref
                .target_chosen
                .target_stack_entry
                .or(parent_target_stack_entry);
            inherited_trigger_index = sa_ref.trigger_index;
            // An `UnlessCost$` node resolves its own sub-chain, gated on
            // `UnlessResolveSubs$`, so walking into it here would run it twice.
            current = if crate::ability::effects::sub_ability_handled_internally(sa_ref) {
                None
            } else {
                sa.get_sub_ability()
            };
        }

        // Mirror Java's `SpellAbility.resolve` post-pass: fire `ChangesZoneAll`
        // for every move accumulated during this spell/ability so triggers like
        // Teval's "whenever a card leaves your graveyard, create a token" see
        // moves performed by the resolving SA chain (e.g. its DBReturn pulling
        // a land from the graveyard back onto the battlefield).
        if let Some(table) = game.pending_change_zone_table.take() {
            table.trigger_changes_zone_all(&mut self.trigger_handler, game, None);
        }

        // Avoid leaking shared tables into subsequent stack entries.
        game.clear_pending_damage_maps();
        game.clear_pending_change_zone_table();
        game.stack.clear_recently_removed();
    }

    /// Check whether a spell/ability should fizzle (CR 608.2b).
    /// Mirrors Java's `MagicStack.hasFizzled()`.
    ///
    /// Walks the SpellAbility chain. If every targeting node has only invalid
    /// targets, the whole spell/ability is countered by game rules.
    /// Returns `false` if no node uses targeting at all.
    fn attach_aura_on_resolution(
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        entry: &StackEntry,
        card_id: CardId,
    ) {
        if game.card(card_id).type_line.has_subtype("Aura")
            && game.card(card_id).attached_to.is_none()
            && game.card(card_id).attached_to_player.is_none()
        {
            let enchant_type = game
                .card(card_id)
                .keywords
                .iter_strings()
                .find_map(|kw| crate::keyword::extract_keyword_cost_str(kw, "Enchant"))
                .unwrap_or_default()
                .to_string();
            let normalized = enchant_type
                .split_once(':')
                .map(|(k, _)| k)
                .unwrap_or(&enchant_type);
            let can_target_player =
                normalized.starts_with("Player") || normalized.starts_with("Opponent");

            let mut candidates: Vec<crate::agent::types::GameEntity> = Vec::new();
            if let Some(target_card) = entry.spell_ability.target_chosen.target_card {
                // Cast Aura: only the chosen target is a valid attach
                // target at resolution time. Skip the full enumeration
                // and offer just that card.
                if crate::parsing::enchant_type_matches_card(
                    &enchant_type,
                    game.card(target_card),
                    Some(game.card(card_id)),
                ) && !crate::staticability::static_ability_cant_attach::cant_attach(
                    &game.cards,
                    game.card(card_id),
                    game.card(target_card),
                    false,
                ) {
                    candidates.push(crate::agent::types::GameEntity::Card(target_card));
                }
            } else if let Some(target_player) = entry.spell_ability.target_chosen.target_player {
                if Self::is_player_target_valid(target_player, game) {
                    candidates.push(crate::agent::types::GameEntity::Player(target_player));
                }
            } else if can_target_player {
                for i in 0..game.players.len() {
                    let p = crate::ids::PlayerId(i as u32);
                    if Self::is_player_target_valid(p, game) {
                        candidates.push(crate::agent::types::GameEntity::Player(p));
                    }
                }
            } else {
                let battlefield: Vec<CardId> =
                    game.cards_in_all_zones(ZoneType::Battlefield).collect();
                for cid in battlefield {
                    if !crate::parsing::enchant_type_matches_card(
                        &enchant_type,
                        game.card(cid),
                        Some(game.card(card_id)),
                    ) {
                        continue;
                    }
                    if crate::staticability::static_ability_cant_attach::cant_attach(
                        &game.cards,
                        game.card(card_id),
                        game.card(cid),
                        false,
                    ) {
                        continue;
                    }
                    candidates.push(crate::agent::types::GameEntity::Card(cid));
                }
            }

            if !candidates.is_empty() {
                let chooser = entry.spell_ability.activating_player;
                let chosen = agents[chooser.index()].choose_single_entity_for_effect(
                    chooser,
                    &candidates,
                    false,
                );
                let attached = matches!(
                    chosen,
                    Some(crate::agent::types::GameEntity::Card(_))
                        | Some(crate::agent::types::GameEntity::Player(_))
                );
                match chosen {
                    Some(crate::agent::types::GameEntity::Card(c)) => {
                        game.attach_to(card_id, c);
                    }
                    Some(crate::agent::types::GameEntity::Player(p)) => {
                        game.attach_to_player(card_id, p);
                    }
                    None => {}
                }
                // Refresh continuous effects so any abilities the Aura
                // grants its newly enchanted host (e.g. Leyline
                // Immersion's `AddAbility$ AddMana` granting `{T}: Add
                // five mana of any combination of colors`) become
                // visible immediately. Without this, downstream
                // playability checks in the same priority loop see
                // the host without the granted ability and may filter
                // out spells the player should be able to cast.
                if attached {
                    crate::staticability::layer::apply_continuous_effects(game);
                }
            }
        }
    }

    fn has_fizzled(sa: &mut SpellAbility, game: &GameState) -> bool {
        let result = Self::has_fizzled_inner(sa, game, None);
        // Java: `return fizzle != null && fizzle;`
        result.unwrap_or(false)
    }

    /// Recursive helper mirroring Java's `hasFizzled(sa, source, fizzle)`.
    /// Returns `Option<bool>`:
    ///   `None`        = no targeting node seen in chain yet
    ///   `Some(true)`  = all targeting nodes have only invalid targets
    ///   `Some(false)` = at least one valid target found somewhere
    fn has_fizzled_inner(
        sa: &mut SpellAbility,
        game: &GameState,
        mut fizzle: Option<bool>,
    ) -> Option<bool> {
        if sa.uses_targeting() {
            // Check if we actually have any chosen targets (mirrors Java's
            // `!sa.isZeroTargets()` — Rust stores at most one target per slot
            // so having any slot filled means non-zero targets)
            let has_any_chosen = !sa.target_chosen.all_target_cards().is_empty()
                || sa.target_chosen.target_player.is_some()
                || sa.target_chosen.target_stack_entry.is_some();

            if has_any_chosen {
                // This node uses targeting and has chosen targets — fizzling
                // is now possible.
                if fizzle.is_none() {
                    fizzle = Some(true);
                }

                // Check each chosen target. If ANY is still valid, fizzle = false.
                // Mirrors Java's for loop over `sa.getTargets()`.

                for target_card_id in sa.target_chosen.all_target_cards() {
                    if Self::is_card_target_valid(
                        sa,
                        target_card_id,
                        if sa.target_chosen.target_card == Some(target_card_id) {
                            sa.target_chosen.target_card_zone_timestamp
                        } else {
                            None
                        },
                        game,
                    ) {
                        fizzle = Some(false);
                    } else {
                        if sa.target_chosen.target_card == Some(target_card_id) {
                            sa.target_chosen.target_card = None;
                            sa.target_chosen.target_card_zone_timestamp = None;
                        }
                        sa.target_chosen.divided_map.shift_remove(&target_card_id);
                    }
                }

                if let Some(target_player_id) = sa.target_chosen.target_player {
                    if Self::is_player_target_valid(target_player_id, game) {
                        fizzle = Some(false);
                    } else {
                        sa.target_chosen.target_player = None;
                    }
                }

                if let Some(target_stack_id) = sa.target_chosen.target_stack_entry {
                    if game.stack.find_by_id(target_stack_id).is_some() {
                        fizzle = Some(false);
                    } else {
                        sa.target_chosen.target_stack_entry = None;
                    }
                }

                // CantFizzle param (e.g. Gilded Drake) overrides fizzle
                if sa.ir.cant_fizzle {
                    fizzle = Some(false);
                }
            }
        }

        // Recurse into sub-abilities — mirrors Java's:
        //   if (sa.getSubAbility() != null)
        //       fizzle = hasFizzled(sa.getSubAbility(), source, fizzle);
        if let Some(sub) = sa.get_sub_ability_mut() {
            fizzle = Self::has_fizzled_inner(sub, game, fizzle);
        }

        fizzle
    }

    /// Check if a card target is still valid at resolution time.
    /// The card must still be in a zone that makes it a legal target:
    /// - For Battlefield targets (Creature/Permanent/Any): must be on battlefield
    /// - For CardInZone targets: must be in the specified zone
    /// - The card must also still be targetable (hexproof etc.)
    fn is_card_target_valid(
        sa: &SpellAbility,
        target_card_id: CardId,
        target_zone_timestamp: Option<u64>,
        game: &GameState,
    ) -> bool {
        // Check if card index is valid
        if target_card_id.index() >= game.cards.len() {
            return false;
        }

        let card = game.card(target_card_id);

        // Java parity: target object identity uses both card id and game timestamp.
        if let Some(chosen_ts) = target_zone_timestamp {
            if card.zone_timestamp != chosen_ts {
                return false;
            }
        }

        let in_target_zone = match sa.target_restrictions.as_ref() {
            Some(tr) if !tr.tgt_zone.is_empty() => tr.tgt_zone.contains(&card.zone),
            _ => card.zone == ZoneType::Battlefield,
        };
        if !in_target_zone {
            return false;
        }

        if let Some(ref tr) = sa.target_restrictions {
            if !tr.valid_tgts.is_empty()
                && !crate::ability::ability_utils::matches_valid_cards_for_sa(
                    game,
                    sa,
                    card,
                    Some(&tr.compiled_valid_tgts()),
                    "Card",
                )
            {
                return false;
            }
        }

        // Card must still be targetable (hexproof, shroud, protection, etc.)
        // Use the activating player as the source controller
        crate::spellability::target_restrictions::can_be_targeted_by_sa(
            game,
            target_card_id,
            sa.activating_player,
            sa,
        )
    }

    /// Check if a player target is still valid (player must still be alive).
    fn is_player_target_valid(target_player_id: PlayerId, game: &GameState) -> bool {
        if target_player_id.index() >= game.players.len() {
            return false;
        }
        !game.player(target_player_id).has_lost
    }

    /// Resolve a single effect line by delegating to the effects module.
    pub(crate) fn resolve_single_effect(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        sa: &SpellAbility,
        parent_target_card: Option<CardId>,
    ) {
        let source_name = sa
            .source
            .and_then(|cid| {
                game.cards
                    .get(cid.index())
                    .map(|c| c.log_name().to_string())
            })
            .unwrap_or_else(|| "Unknown source".to_string());
        let effect_kind = Self::effect_kind_for_sa(sa);
        let mut event = crate::agent::GameLogEvent::stack(format!(
            "Effect resolved: {effect_kind} | source={source_name}"
        ))
        .with_player(sa.activating_player);
        if let Some(source_id) = sa.source {
            event = event.with_source_card(source_id);
        }
        if let Some(target_id) = sa.target_chosen.target_card {
            event = event.with_target_card(target_id);
        }
        crate::agent::notify_all_agents(agents, event);

        let mut ctx = EffectContext {
            game,
            combat: Some(&mut self.combat),
            agents,
            trigger_handler: &mut self.trigger_handler,
            token_templates: &self.token_templates,
            token_art_variants: &self.token_art_variants,
            token_fallback: &self.token_fallback,
            edition_dates: &self.edition_dates,
            mana_pools: &mut self.mana_pools,
            parent_target_card,
            rng: &mut *self.game_rng,
        };
        effects::resolve_effect(&mut ctx, sa);
    }

    /// CR 707.10 / 111.11 — copy of a permanent spell becomes a token.
    fn register_evoke_sacrifice_triggers(
        &mut self,
        game: &GameState,
        card_id: CardId,
        player: PlayerId,
        evoke_keyword_count: usize,
    ) {
        // `CardCopyService.copyCard` rebuilds a card with a paper card from its script, which
        // makes the `T:` triggers before the keyword triggers; a token or a copied permanent
        // goes through `copyStats`, which copies the keywords first.
        let keyword_triggers_first = {
            let card = game.card(card_id);
            card.is_token || card.copied_permanent.is_some()
        };
        for i in 0..evoke_keyword_count {
            self.trigger_handler.register_delayed_trigger(
                crate::trigger::handler::DelayedTrigger {
                    mode: TriggerType::ChangesZone,
                    trigger_mode: Box::new(crate::trigger::trigger_changes_zone::TriggerChangesZone)
                        as Box<dyn crate::trigger::TriggerBehavior>,
                    params: crate::parsing::Params::from_raw(
                        "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self",
                    ),
                    execute_svar: "DB$ Sacrifice".to_string(),
                    controller: player,
                    source_card: card_id,
                    created_turn: game.turn.turn_number,
                    created_phase: game.turn.phase,
                    target_card: Some(card_id),
                    remembered_amount: 0,
                    remembered_cards: Vec::new(),
                    remembered_players: Vec::new(),
                    remembered_lki_cards: Vec::new(),
                    target_card_zone_timestamp: None,
                    sort_after_active: i > 0 || !keyword_triggers_first,
                    trigger_order: None,
                    source_timestamp: None,
                    spawning_ability: None,
                },
            );
        }
    }

    fn cease_to_exist_copied_spell(game: &mut GameState, host: Option<CardId>) {
        if let Some(host) = host.filter(|&host| game.card(host).zone == ZoneType::Stack) {
            let controller = game.card(host).controller;
            game.remove_card_from_zone(ZoneType::Stack, controller, host);
            game.card_mut(host).zone = ZoneType::None;
        }
    }

    fn resolve_copied_permanent_as_token(
        &mut self,
        game: &mut GameState,
        agents: &mut [Box<dyn PlayerAgent>],
        entry: &StackEntry,
    ) {
        use crate::ability::effects::token_effect_base::{TokenCreateTable, TokenEffectBase};
        let Some(original_id) = entry.spell_ability.source else {
            return;
        };
        let player = entry.spell_ability.activating_player;
        let original = game.card(original_id).clone();
        let mut proto = crate::ability::effects::copy_permanent_effect::get_proto_type(
            &entry.spell_ability,
            &original,
            player,
        );
        proto.copied_permanent = Some(original_id);
        let mut token_table = TokenCreateTable::default();
        token_table.put(player, proto, 1);
        let mut trigger_list = crate::card::card_zone_table::CardZoneTable::default();
        let mut ctx = EffectContext {
            game,
            combat: Some(&mut self.combat),
            agents,
            trigger_handler: &mut self.trigger_handler,
            token_templates: &self.token_templates,
            token_art_variants: &self.token_art_variants,
            token_fallback: &self.token_fallback,
            edition_dates: &self.edition_dates,
            mana_pools: &mut self.mana_pools,
            parent_target_card: None,
            rng: &mut *self.game_rng,
        };
        let created = crate::ability::effects::token_effect_base::TOKEN_EFFECT_BASE
            .make_token_table(
                &mut ctx,
                token_table,
                true,
                &mut trigger_list,
                &entry.spell_ability,
            );
        if entry.spell_ability.alt_cost == Some(crate::spellability::AlternativeCost::Evoke) {
            let evoke_keyword_count = (entry.spell_ability.evoke_keyword_count as usize).max(1);
            for &token_id in &created.created {
                self.register_evoke_sacrifice_triggers(game, token_id, player, evoke_keyword_count);
            }
        }
        trigger_list.trigger_changes_zone_all(
            &mut self.trigger_handler,
            game,
            Some(&entry.spell_ability),
        );
    }
}
