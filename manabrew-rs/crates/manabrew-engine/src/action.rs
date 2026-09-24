use std::sync::Arc;

use forge_foundation::{CoreType, ZoneType};

use crate::agent::{GameEntity, PlayerAgent};
use crate::card::card_damage_map::{CardDamageMap, DamageTarget};
use crate::card::{Card, CounterType};
use crate::event::RunParams;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::replacement::replacement_handler::{
    apply_replacements, apply_replacements_with_agents, apply_replacements_with_agents_and_runtime,
    damage_run_params, has_replace_damage, run_replace_damage, ReplacementEvent,
    ReplacementRuntime,
};
use crate::replacement::GameLossReason;
use crate::replacement::ReplacementResult;
use crate::staticability::layer::{apply_continuous_effects, apply_etb_tapped_with_agents};
use crate::trigger::handler::TriggerHandler;
use crate::trigger::TriggerType;

/// Game state mutation methods — moving cards, dealing damage, state-based actions.
impl GameState {
    pub fn record_player_damage_assignment(
        &mut self,
        source: Option<CardId>,
        target_player: Option<PlayerId>,
        amount: i32,
        is_combat: bool,
    ) {
        self.player_record_damage_assignment(source, target_player, amount, is_combat);
    }

    /// Move a card from its current zone to a new zone.
    /// Move a card to a new zone. For Graveyard destinations, checks for zone-redirect
    /// replacement effects (Rest in Peace, Leyline of the Void) and redirects to the
    /// correct zone. Use `move_card_final` to skip the replacement check.
    pub fn move_card(&mut self, card_id: CardId, dest_zone: ZoneType, dest_owner: PlayerId) {
        self.move_card_internal(
            card_id, dest_zone, dest_owner, None, None, None, true, false,
        );
    }

    /// Mirrors Java `Card.setPrepared`: clearing it makes the prepared copy cease to exist if it
    /// is still in exile and exiles the effect that lets it be cast.
    pub(crate) fn set_prepared(&mut self, card_id: CardId, effect: Option<CardId>) {
        if effect.is_none() {
            if let Some(old) = self.card(card_id).prepared_effect {
                if let Some(&prepared) = self.card(old).remembered_cards.first() {
                    if self.card(prepared).zone == ZoneType::Exile {
                        let owner = self.card(prepared).owner;
                        self.remove_card_from_zone(ZoneType::Exile, owner, prepared);
                        self.card_mut(prepared).zone = ZoneType::None;
                    }
                }
                if self.card(old).zone == ZoneType::Command {
                    let owner = self.card(old).owner;
                    self.remove_card_from_zone(ZoneType::Command, owner, old);
                    self.card_mut(old).zone = ZoneType::None;
                }
            }
        }
        self.card_mut(card_id).prepared_effect = effect;
    }

    pub fn move_card_with_agents(
        &mut self,
        card_id: CardId,
        dest_zone: ZoneType,
        dest_owner: PlayerId,
        agents: &mut [Box<dyn PlayerAgent>],
    ) {
        self.move_card_internal(
            card_id,
            dest_zone,
            dest_owner,
            Some(agents),
            None,
            None,
            true,
            false,
        );
    }

    pub fn move_card_with_agents_and_replacement_runtime(
        &mut self,
        card_id: CardId,
        dest_zone: ZoneType,
        dest_owner: PlayerId,
        agents: &mut [Box<dyn PlayerAgent>],
        runtime: &mut ReplacementRuntime<'_>,
    ) {
        self.move_card_internal(
            card_id,
            dest_zone,
            dest_owner,
            Some(agents),
            None,
            Some(runtime),
            true,
            false,
        );
    }

    pub(crate) fn sacrifice_destroy(
        &mut self,
        card_id: CardId,
        agents: &mut [Box<dyn PlayerAgent>],
        runtime: &mut ReplacementRuntime<'_>,
        lki_p1p1: i32,
        lki_power: i32,
        lki_toughness: i32,
    ) {
        let owner = self.card(card_id).owner;
        let mut moved_event = ReplacementEvent::Moved {
            card: card_id,
            origin: ZoneType::Battlefield,
            destination: ZoneType::Graveyard,
            is_discard: false,
            counter_map: None,
            counter_cause: None,
            counter_is_effect: false,
            after_replacement_static_abilities: Vec::new(),
            stack_sa: None,
            fizzle: None,
        };
        let result =
            apply_replacements_with_agents_and_runtime(self, agents, runtime, &mut moved_event);
        if !matches!(
            result,
            ReplacementResult::NotReplaced | ReplacementResult::Updated
        ) {
            return;
        }
        let final_dest = if let ReplacementEvent::Moved { destination, .. } = moved_event {
            destination
        } else {
            ZoneType::Graveyard
        };
        crate::ability::effects::emit_zone_trigger_with_lki_counters(
            runtime.trigger_handler,
            card_id,
            ZoneType::Battlefield,
            final_dest,
            lki_p1p1,
            lki_power,
            lki_toughness,
        );
        runtime.trigger_handler.flush_waiting_triggers(self);
        self.move_card_internal(
            card_id,
            final_dest,
            owner,
            Some(agents),
            None,
            Some(runtime),
            false,
            false,
        );
    }

    pub(crate) fn move_card_without_replacement(
        &mut self,
        card_id: CardId,
        dest_zone: ZoneType,
        dest_owner: PlayerId,
    ) {
        self.move_card_internal(
            card_id, dest_zone, dest_owner, None, None, None, false, false,
        );
    }

    /// Discard a card. Mirrors Java's `Player.discard()`.
    ///
    /// Records the discard, marks the card, and moves it to graveyard through
    /// the normal zone-change machinery (which runs replacement effects like
    /// Madness automatically). Fires Discarded triggers afterwards.
    pub fn discard_card(
        &mut self,
        card_id: CardId,
        discard_player: PlayerId,
        sa: Option<&crate::spellability::SpellAbility>,
        agents: Option<&mut [Box<dyn PlayerAgent>]>,
        runtime: &mut ReplacementRuntime<'_>,
    ) {
        let owner = self.card(card_id).owner;
        self.player_record_discard(discard_player, 1);

        // Move to graveyard through normal zone-change with is_discard=true.
        // Replacement effects (e.g. Madness → Exile) are handled generically.
        self.move_card_internal(
            card_id,
            ZoneType::Graveyard,
            owner,
            agents,
            None,
            Some(&mut *runtime),
            true,
            true, // is_discard
        );
        let trigger_handler = &mut *runtime.trigger_handler;
        self.card_mut(card_id).set_discarded(true);

        // RememberDiscarded
        if let Some(sa) = sa {
            if sa.ir.remember_discarded {
                if let Some(source_id) = sa.source {
                    self.card_mut(source_id).add_remembered_card(card_id);
                }
            }
        }

        // Register active triggers on the card in its new zone.
        trigger_handler.register_active_trigger(self, card_id);

        // Emit zone-change trigger for Hand → actual destination.
        let dest_zone = self.card(card_id).zone;
        crate::ability::effects::zone_triggers::emit_zone_trigger(
            trigger_handler,
            card_id,
            ZoneType::Hand,
            dest_zone,
        );

        // Fire Discarded trigger.
        trigger_handler.run_trigger(
            TriggerType::Discarded,
            RunParams {
                card: Some(card_id),
                player: Some(discard_player),
                ..Default::default()
            },
            false,
        );
        if let Some(batch) = self.pending_discard_batch.as_mut() {
            batch.entry(discard_player).or_default().push(card_id);
            return;
        }
        trigger_handler.run_trigger(
            TriggerType::DiscardedAll,
            RunParams {
                card: Some(card_id),
                cards: Some(vec![card_id]),
                player: Some(discard_player),
                ..Default::default()
            },
            false,
        );
    }

    /// Idempotent: a nested call joins the open batch rather than starting a second one.
    pub fn begin_discard_batch(&mut self) {
        if self.pending_discard_batch.is_none() {
            self.pending_discard_batch = Some(crate::HashMap::default());
        }
    }

    /// Player-turn-order so the firings are deterministic. No-ops if no batch is open.
    pub fn end_discard_batch(&mut self, trigger_handler: &mut TriggerHandler) {
        let Some(mut batch) = self.pending_discard_batch.take() else {
            return;
        };
        for &player in &self.player_order.clone() {
            let Some(cards) = batch.remove(&player) else {
                continue;
            };
            if cards.is_empty() {
                continue;
            }
            trigger_handler.run_trigger(
                TriggerType::DiscardedAll,
                RunParams {
                    cards: Some(cards),
                    player: Some(player),
                    ..Default::default()
                },
                false,
            );
        }
    }

    fn run_untap_commands(&mut self, card_id: CardId) {
        let (commands, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.untap_commands)
            .into_iter()
            .partition(|(host, _)| *host == card_id);
        self.untap_commands = kept;
        for (_, command) in commands {
            command.run(self, &mut crate::game_rng::ThreadRngAdapter::default());
        }
    }

    pub(crate) fn forget_on_cast(&mut self, card_id: CardId) {
        let forget_effects: Vec<CardId> = self
            .cards
            .iter()
            .filter(|c| {
                c.zone == ZoneType::Command
                    && c.forget_on_moved_origin
                        .is_some_and(|zone| zone != ZoneType::Stack)
                    && c.remembered_cards.contains(&card_id)
            })
            .map(|c| c.id)
            .collect();
        for eff_id in forget_effects {
            let eff = self.card_mut(eff_id);
            eff.remembered_cards.retain(|&rid| rid != card_id);
            if eff.exile_when_no_remembered && eff.remembered_cards.is_empty() {
                let controller = eff.controller;
                self.remove_card_from_zone(ZoneType::Command, controller, eff_id);
                self.card_mut(eff_id).zone = ZoneType::None;
            }
        }
    }

    fn move_card_internal(
        &mut self,
        card_id: CardId,
        dest_zone: ZoneType,
        dest_owner: PlayerId,
        mut agents: Option<&mut [Box<dyn PlayerAgent>]>,
        trigger_handler: Option<&mut TriggerHandler>,
        mut runtime: Option<&mut ReplacementRuntime<'_>>,
        apply_move_replacement: bool,
        is_discard: bool,
    ) {
        let (src_zone, src_owner, was_permanent, was_land, is_token) = {
            let card = &self.cards[card_id.index()];
            (
                card.zone,
                card.controller,
                card.type_line.is_permanent(),
                card.is_land(),
                card.is_token,
            )
        };
        if dest_zone == ZoneType::Exile && self.cards[card_id.index()].effect_source.is_some() {
            self.remove_card_from_zone(src_zone, src_owner, card_id);
            self.card_mut(card_id).set_zone(ZoneType::None);
            return;
        }
        if crate::game_loop::GameLoop::card_trace_matches(&self.cards[card_id.index()].card_name) {
            eprintln!(
                "[card-trace] move {} {:?} {:?} -> {:?} (owner={:?} sick={} cast_from={:?})",
                self.cards[card_id.index()].card_name,
                card_id,
                src_zone,
                dest_zone,
                dest_owner,
                self.cards[card_id.index()].summoning_sick,
                self.cards[card_id.index()].cast_from,
            );
        }
        let indirect_aura = dest_zone == ZoneType::Battlefield && src_zone != ZoneType::Stack && {
            let card = self.card(card_id);
            card.type_line.has_subtype("Aura")
                && card.attached_to.is_none()
                && card.attached_to_player.is_none()
        };
        if indirect_aura && self.aura_attach_candidates(card_id).is_empty() {
            return;
        }
        if dest_zone == ZoneType::Battlefield
            && !matches!(src_zone, ZoneType::Stack | ZoneType::Battlefield)
        {
            let card = self.card_mut(card_id);
            card.cast_sa = None;
            card.svars.remove("XPaid");
        }
        let mut etb_counters = std::collections::BTreeMap::new();
        if dest_zone == ZoneType::Battlefield {
            for keyword in self.cards[card_id.index()].keywords.as_string_list() {
                let mut parts = keyword.split(':');
                if !parts
                    .next()
                    .is_some_and(|head| head.eq_ignore_ascii_case("etbCounter"))
                {
                    continue;
                }
                let counter_type =
                    crate::ability::effects::parse_counter_type(parts.next().unwrap_or_default());
                let amount_text = parts.next().unwrap_or_default();
                if let Some(extra_params) = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "no Condition")
                {
                    let params = crate::parsing::Params::from_raw(extra_params);
                    // Java's `makeEtbCounter` appends `extraparams` onto a replacement whose base
                    // `ValidCard$` is `Card.Self`; a `ValidCard$` here overrides that base clause
                    // (last key wins in Java's own param map) rather than adding an IsPresent-style
                    // condition, so it is checked against the card itself, not the requirements IR.
                    if let Some(valid_card) =
                        params.selector_untracked(crate::parsing::keys::VALID_CARD)
                    {
                        let card = &self.cards[card_id.index()];
                        if !crate::card::valid_filter::matches_valid_card_selector_opt_in_game(
                            Some(valid_card),
                            card,
                            card,
                            self,
                        ) {
                            continue;
                        }
                    }
                    let requirements =
                        crate::card::valid_filter::CardTraitRequirementsIr::from_key_values(
                            params.iter(),
                            params
                                .selector_untracked(crate::parsing::keys::IS_PRESENT)
                                .cloned(),
                            params.selector_untracked("IsPresent2").cloned(),
                        );
                    let card = self.card(card_id);
                    if !requirements.meets(self, card, card) {
                        continue;
                    }
                }
                let amount = amount_text.parse::<i32>().unwrap_or_else(|_| {
                    let card = &self.cards[card_id.index()];
                    card.svars
                        .get(amount_text)
                        .map(|expression| {
                            if matches!(expression.as_str(), "Count$xPaid" | "Count$XPaid") {
                                card.svars
                                    .get("XPaid")
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(0)
                            } else {
                                expression.parse().unwrap_or_else(|_| {
                                    crate::svar::resolve_count_svar(
                                        expression, self, card_id, dest_owner,
                                    )
                                })
                            }
                        })
                        .unwrap_or(0)
                });
                *etb_counters.entry(counter_type).or_default() += amount.max(0);
            }
            let card = &self.cards[card_id.index()];
            if card.type_line.has_subtype("Saga") && card.has_chapter() {
                let amount = if card.has_keyword("Read ahead") {
                    agents
                        .as_deref_mut()
                        .and_then(|agents| {
                            agents[dest_owner.index()].choose_number(
                                dest_owner,
                                Some(card_id),
                                "How many lore counters?",
                                Some("Choose a chapter and start with that many lore counters."),
                                1,
                                card.get_final_chapter_nr(),
                            )
                        })
                        .unwrap_or(1)
                        .clamp(1, card.get_final_chapter_nr())
                } else {
                    1
                };
                *etb_counters.entry(CounterType::Lore).or_default() += amount;
            }
            if card.type_line.is_planeswalker() {
                let loyalty = card
                    .initial_loyalty
                    .as_deref()
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(0);
                *etb_counters
                    .entry(crate::card::CounterType::Loyalty)
                    .or_default() += loyalty.max(0);
            }
            // Java `CardFactoryUtil:2316` gives Impending an ETB replacement gated on
            // `Card.Self+impended`, so the time counters are there as it enters and it
            // is never briefly a creature.
            if let Some((_, amount)) = card.get_impending_cost() {
                let impended = card.cast_sa.as_ref().is_some_and(|sa| {
                    sa.alt_cost == Some(crate::spellability::AlternativeCost::Impending)
                });
                if impended {
                    *etb_counters
                        .entry(crate::card::CounterType::Time)
                        .or_default() += amount.max(0);
                }
            }
            let sunburst = card.sunburst_count();
            if sunburst > 0 && card.has_keyword("Sunburst") {
                let counter_type = if card.is_creature() {
                    crate::card::CounterType::P1P1
                } else {
                    crate::card::CounterType::Charge
                };
                *etb_counters.entry(counter_type).or_default() += sunburst;
            }
            etb_counters.retain(|_, amount| *amount > 0);
        }
        let counter_cause = self.cards[card_id.index()].cast_sa.clone();
        let counter_map = (!etb_counters.is_empty()).then(|| {
            vec![crate::replacement::replacement_handler::CounterMapValue {
                source: Some(dest_owner),
                counters: etb_counters,
            }]
        });
        let mut moved_event = ReplacementEvent::Moved {
            card: card_id,
            origin: src_zone,
            destination: dest_zone,
            is_discard,
            counter_map,
            counter_cause,
            counter_is_effect: dest_zone == ZoneType::Battlefield,
            after_replacement_static_abilities: Vec::new(),
            stack_sa: None,
            fizzle: None,
        };
        let tapped_before_replacement = self.card(card_id).tapped;
        if apply_move_replacement {
            let result = match (agents.as_deref_mut(), runtime.as_deref_mut()) {
                (Some(agents), Some(runtime)) => apply_replacements_with_agents_and_runtime(
                    self,
                    agents,
                    runtime,
                    &mut moved_event,
                ),
                (Some(agents), None) => {
                    apply_replacements_with_agents(self, agents, &mut moved_event)
                }
                (None, _) => apply_replacements(self, &mut moved_event),
            };
            if !matches!(
                result,
                ReplacementResult::NotReplaced | ReplacementResult::Updated
            ) {
                return;
            }
        }
        let (mut trigger_handler, mut rng) = match runtime {
            Some(runtime) => (Some(&mut *runtime.trigger_handler), Some(&mut *runtime.rng)),
            None => (trigger_handler, None),
        };
        let (dest_zone, etb_counter_map, counter_cause, after_replacement_static_abilities) =
            match moved_event {
                ReplacementEvent::Moved {
                    destination,
                    counter_map,
                    counter_cause,
                    after_replacement_static_abilities,
                    ..
                } => (
                    destination,
                    counter_map,
                    counter_cause,
                    after_replacement_static_abilities,
                ),
                _ => (dest_zone, None, None, Vec::new()),
            };
        if indirect_aura && dest_zone == ZoneType::Battlefield {
            if let Some(agents) = agents.as_deref_mut() {
                self.attach_aura_on_indirect_etb(agents, card_id, dest_owner);
            }
        }
        let etb_counter_map = if dest_zone != ZoneType::Battlefield {
            etb_counter_map
        } else {
            let staged_etb_counters = std::mem::take(&mut self.card_mut(card_id).etb_counters);
            if staged_etb_counters.is_empty() {
                etb_counter_map
            } else {
                let mut counter_map = etb_counter_map.unwrap_or_default();
                for (placer, counters) in staged_etb_counters {
                    counter_map.push(crate::replacement::replacement_handler::CounterMapValue {
                        source: placer.or(Some(dest_owner)),
                        counters,
                    });
                }
                Some(counter_map)
            }
        };
        let replacement_marked_etb_tapped = dest_zone == ZoneType::Battlefield
            && self.card(card_id).tapped
            && !tapped_before_replacement;
        let dest_owner = if dest_zone == ZoneType::Command {
            self.card(card_id).owner
        } else {
            dest_owner
        };
        let host_left_battlefield =
            src_zone == ZoneType::Battlefield && dest_zone != ZoneType::Battlefield;
        if host_left_battlefield && was_permanent {
            self.player_record_permanent_left_battlefield(src_owner);
        }
        // Java `Card.clearCastSA` — the cast-SA link dies once the instance
        // leaves the battlefield (a new cast produces a fresh instance).
        if host_left_battlefield {
            self.add_change_zone_lki_info(self.cards[card_id.index()].clone());
            let lki_controller = self.card(card_id).controller;
            self.card_mut(card_id).lki_controller = Some(lki_controller);
            self.card_mut(card_id).cast_sa = None;
            // `gameCard.addLeavesPlayCommand(() -> gameCard.setPrepared(null))`.
            if self.card(card_id).is_prepared() {
                self.set_prepared(card_id, None);
            }
            self.card_mut(card_id).clear_temp_controllers();
            self.card_mut(card_id).set_original_controller_eot(None);
            let (commands, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.leaves_play_commands)
                .into_iter()
                .partition(|(host, _)| *host == card_id);
            self.leaves_play_commands = kept;
            for (_, command) in commands {
                match rng.as_deref_mut() {
                    Some(rng) => command.run(self, rng),
                    None => command.run(self, &mut crate::game_rng::ThreadRngAdapter::default()),
                }
            }
            self.run_untap_commands(card_id);
        }
        if dest_zone == ZoneType::Graveyard && was_permanent && !is_token {
            self.player_record_permanent_put_into_graveyard(self.card(card_id).owner);
        }
        let forget_effects: Vec<CardId> = self
            .cards
            .iter()
            .filter(|c| {
                c.zone == ZoneType::Command
                    && c.forget_on_moved_origin == Some(src_zone)
                    && dest_zone != ZoneType::Stack
                    && c.remembered_cards.contains(&card_id)
            })
            .map(|c| c.id)
            .collect();
        let exile_on_moved_effects: Vec<CardId> = self
            .cards
            .iter()
            .filter(|c| {
                c.zone == ZoneType::Command
                    && c.exile_on_moved_origins.contains(&src_zone)
                    && c.remembered_cards.contains(&card_id)
            })
            .map(|c| c.id)
            .collect();
        for eff_id in exile_on_moved_effects {
            let controller = self.card(eff_id).controller;
            self.remove_card_from_zone(ZoneType::Command, controller, eff_id);
            self.card_mut(eff_id).zone = ZoneType::None;
        }

        if src_zone == ZoneType::Battlefield && dest_zone != ZoneType::Battlefield {
            self.add_left_battlefield_this_turn(card_id);
        }
        if src_zone == ZoneType::Graveyard && dest_zone != ZoneType::Graveyard {
            self.add_left_graveyard_this_turn(card_id);
        }

        // Tokens and copy-tokens cease to exist when leaving the battlefield (CR 110.5g).
        // Set zone to None (limbo) and remove from source zone without adding to destination.
        if is_token && dest_zone != ZoneType::Battlefield && dest_zone != ZoneType::Stack {
            if let Some(table) = self.pending_change_zone_table.as_mut() {
                table.put(Some(src_zone), Some(dest_zone), card_id);
            }
            self.save_zone_lki(dest_zone, dest_owner, card_id, src_zone);
            let mut exile_effects = Vec::new();
            for eff_id in forget_effects.iter().copied() {
                let eff = self.card_mut(eff_id);
                eff.remembered_cards.retain(|&rid| rid != card_id);
                if eff.exile_when_no_remembered && eff.remembered_cards.is_empty() {
                    exile_effects.push(eff_id);
                }
            }
            let attachments: Vec<CardId> = self.cards[card_id.index()].attachments.clone();
            for aura_id in attachments {
                self.card_mut(aura_id).attached_to = None;
                self.card_mut(aura_id).is_bestowed = false;
            }
            self.card_mut(card_id).attachments.clear();
            self.detach(card_id);

            self.card_mut(card_id).zone = ZoneType::None;
            if src_zone != ZoneType::None {
                self.remove_card_from_zone(src_zone, src_owner, card_id);
            }
            // Effect cards with ForgetOnMoved should be removed from the game
            // entirely (zone = None), not moved to Exile. Moving them to Exile
            // creates phantom cards that diverge from Java parity.
            for eff_id in exile_effects {
                let controller = self.card(eff_id).controller;
                self.remove_card_from_zone(ZoneType::Command, controller, eff_id);
                self.card_mut(eff_id).zone = ZoneType::None;
            }
            apply_continuous_effects(self);
            debug_assert!(self.card_zone_location_matches_card(card_id));
            return;
        }

        let leaves_as_new_object = (src_zone == ZoneType::Stack
            && !matches!(dest_zone, ZoneType::Stack | ZoneType::Battlefield))
            || (src_zone == ZoneType::Battlefield && dest_zone != ZoneType::Battlefield);
        if leaves_as_new_object && src_zone == ZoneType::Battlefield {
            let card = &self.cards[card_id.index()];
            let lki_transformed = card.is_transformed && !card.type_line.has_subtype("Room");
            self.card_mut(card_id).lki_transformed = lki_transformed;
        }
        if leaves_as_new_object && self.cards[card_id.index()].is_transformed {
            self.card_mut(card_id).transform();
        }
        if leaves_as_new_object {
            self.card_mut(card_id).cast_from = None;
            self.card_mut(card_id).chosen_charm_modes.clear();
        }
        if dest_zone != ZoneType::Battlefield && !(src_zone == dest_zone && dest_zone.is_hidden()) {
            let card = self.card_mut(card_id);
            card.reset_activations_per_turn();
            card.reset_ability_resolved_this_turn();
            card.number_game_activations.clear();
            card.activations_this_game.clear();
        }
        if src_zone == ZoneType::Exile && dest_zone != ZoneType::Exile {
            self.card_mut(card_id)
                .keywords
                .remove(crate::card::KEYWORD_WARP_EXILED);
        }
        if dest_zone != ZoneType::Stack {
            if let Some(exiled_with) = self.card_mut(card_id).exiled_with.take() {
                self.card_mut(exiled_with).remove_exiled_card(card_id);
            }
        }

        // Remove from source zone
        if src_zone != ZoneType::None {
            self.remove_card_from_zone(src_zone, src_owner, card_id);
        }

        if src_zone == ZoneType::Exile && dest_zone != ZoneType::Exile {
            self.card_mut(card_id)
                .keywords
                .retain(|kw| !kw.starts_with(crate::card::KEYWORD_PLOTTED_PREFIX));
        }

        // Update card's zone
        self.card_mut(card_id).zone = dest_zone;
        if src_zone != dest_zone {
            self.card_mut(card_id).turn_in_zone = self.turn.turn_number;
            self.card_mut(card_id).set_discarded(false);
        }

        if let Some(table) = self.pending_change_zone_table.as_mut() {
            table.put(Some(src_zone), Some(dest_zone), card_id);
        }

        if src_zone == ZoneType::Battlefield {
            let left_at = self.cards[card_id.index()].zone_timestamp;
            self.card_mut(card_id).lki_zone_timestamp = Some(left_at);
        }

        // Assign a zone timestamp so same-player triggers are ordered by
        // zone entry order (matching Java's Zone.cardList insertion order).
        if dest_zone != ZoneType::Stack {
            self.assign_zone_timestamp(card_id);
        }

        // Track LKI: record which zone this card came from on the destination zone.
        self.save_zone_lki(dest_zone, dest_owner, card_id, src_zone);

        if !matches!(src_zone, ZoneType::Battlefield) && dest_zone != ZoneType::Battlefield {
            self.card_mut(card_id)
                .restore_changed_characteristics_baseline();
        }

        // Reset state on zone change
        match dest_zone {
            ZoneType::Battlefield => {
                if !matches!(src_zone, ZoneType::Battlefield | ZoneType::None)
                    && !crate::staticability::static_ability_counters_remain::counters_remain(
                        &self.cards,
                        &self.cards[card_id.index()],
                        dest_zone,
                    )
                {
                    self.card_mut(card_id).counters.clear();
                }
                // A permanent enters under the destination player's control.
                // This must be updated before ETB-trigger registration so
                // triggered abilities inherit the correct controller.
                self.card_mut(card_id).controller = dest_owner;
                self.card_mut(card_id).enter_battlefield();
                if replacement_marked_etb_tapped {
                    self.card_mut(card_id).set_tapped(true);
                }
                // Add to destination zone first so the card is "on the
                // battlefield" when ETB-tapped checks run against it.
                self.add_card_to_zone(dest_zone, dest_owner, card_id);
                if was_land {
                    self.player_record_landfall(dest_owner);
                }
                // Apply ETB-tapped effects (intrinsic + extrinsic). When the
                // replacement chain already tapped this card it also already
                // prompted the affected player to choose the applied effect,
                // so neither the prompt nor the apply pass should fire again
                // here — Java's flow runs the choose-and-apply step exactly
                // once via the replacement chain.
                if !replacement_marked_etb_tapped {
                    apply_etb_tapped_with_agents(self, card_id, agents);
                }
                if let Some(handler) = trigger_handler.as_deref_mut() {
                    handler.register_active_trigger(self, card_id);
                }
                if let Some(counter_map) = etb_counter_map {
                    let table =
                        crate::game_entity_counter_table::GameEntityCounterTable::from_counter_map(
                            crate::agent::GameEntity::Card(card_id),
                            counter_map,
                        );
                    table.apply_replaced_counter_effect(
                        self,
                        trigger_handler.as_deref_mut(),
                        counter_cause.as_deref(),
                        RunParams::default(),
                    );
                    for (source, static_abilities) in after_replacement_static_abilities {
                        crate::replacement::replace_add_counter::apply_after_replacement_static_abilities(
                            self,
                            source,
                            static_abilities,
                        );
                    }
                }
                // Update LKI snapshot: card just entered the battlefield.
                // Ensures it's available for later TriggeredCard$CardPower lookups
                // even if it dies within the same resolution chain.
                self.update_lki_snapshot(card_id);
                apply_continuous_effects(self);
                debug_assert!(self.card_zone_location_matches_card(card_id));
                return;
            }
            ZoneType::Graveyard | ZoneType::Hand | ZoneType::Exile | ZoneType::Library => {
                // Save last-known information before resetting.
                // Mirrors Java's LKI system for trigger SVars like TriggeredCard$CardPower.
                if src_zone == ZoneType::Battlefield {
                    let card = &self.cards[card_id.index()];
                    let lki_p = card.power();
                    let lki_t = card.toughness();
                    let lki_tapped = card.tapped;
                    let lki_attached_to = card.attached_to;
                    let card = self.card_mut(card_id);
                    card.lki_power = Some(lki_p);
                    card.lki_toughness = Some(lki_t);
                    card.lki_tapped = Some(lki_tapped);
                    card.lki_attached_to = lki_attached_to;
                }

                // Detach any attachments before resetting state.
                let attachments: Vec<CardId> = self.cards[card_id.index()].attachments.clone();
                for aura_id in attachments {
                    self.card_mut(aura_id).attached_to = None;
                    // Bestow: when host leaves, revert aura to creature
                    self.card_mut(aura_id).is_bestowed = false;
                }
                self.card_mut(card_id).attachments.clear();
                // Also detach this card from its host if it was an Aura/Equipment.
                self.detach(card_id);

                // Reset battlefield state when leaving (including static modifiers).
                let keep_counters =
                    crate::staticability::static_ability_counters_remain::counters_remain(
                        &self.cards,
                        &self.cards[card_id.index()],
                        dest_zone,
                    );
                let card = self.card_mut(card_id);
                card.tapped = false;
                card.damage = 0;
                card.power_modifier = 0;
                card.toughness_modifier = 0;
                card.pt_boosts.clear();
                card.static_power_modifier = 0;
                card.static_toughness_modifier = 0;
                card.static_set_power = None;
                card.static_set_toughness = None;
                card.granted_keywords.clear();
                if let Some(type_line) = card.static_type_line_base.take() {
                    card.set_type_line(type_line);
                }
                card.changed_card_types.clear();
                card.static_added_subtypes.clear();
                card.cant_block_static = false;
                card.summoning_sick = true;
                card.monstrous = false;
                card.controller = card.owner;
                card.turn_face_up();
                card.is_bestowed = false;
                // CR 400.7: a permanent that changes zones becomes a new
                // object with no cast history. Mirrors Java's
                // changeZone-creates-new-Card behaviour.
                card.cast_from = None;
                // `Card.ExiledWithSource` compares the host's game timestamp
                // (`equalsWithGameTimestamp`), so the new object has exiled nothing.
                card.imprinted_cards.clear();
                card.exiled_cards.clear();
                card.reset_crewed();
                card.reset_saddled();
                card.activations_this_game.clear();
                if !keep_counters {
                    card.counters.clear();
                }
                // Clear temporary triggers added by Animate effects (e.g.
                // Supernatural Stamina's "when this creature dies, return it").
                // Per CR 400.7 a permanent that changes zones becomes a new
                // object; it must not retain one-shot death-return triggers.
                // Without this, a creature that dies-and-returns would still
                // carry the trigger, making it "immortal" for the rest of the
                // turn.
                card.clear_pump_triggers();
                card.clear_pump_keywords();
                card.cant_have_keywords.clear();
                // Restore intrinsic keywords from the animate snapshot so
                // Animate-granted keywords (e.g. Sneak Attack's `Keywords$
                // Haste`) do not persist into the new object the card
                // becomes when it changes zones (CR 400.7).
                if let Some(state) = card.animate_state.take() {
                    card.restore_animate_snapshot(
                        state.original_type_line,
                        state.original_base_power,
                        state.original_base_toughness,
                        state.original_color,
                    );
                    if let Some(orig_kws) = state.original_keywords {
                        card.keywords = orig_kws;
                        card.update_keywords();
                    }
                    for ts in state.trait_change_timestamps {
                        card.remove_changed_card_traits(ts, 0);
                    }
                }
                // After the until-end-of-turn snapshot: that snapshot can hold what a lasting
                // change (Earthbend) made, and the baseline predates both.
                card.restore_changed_characteristics_baseline();
                card.clear_changed_card_traits_keeping_perpetual();
                if let Some(state) = card.clone_state.take() {
                    card.restore_clone_snapshot_keeping_svars(*state);
                } else {
                    card.remove_clone_states();
                }
            }
            ZoneType::Command => {
                // Detach any attachments before resetting state.
                let attachments: Vec<CardId> = self.cards[card_id.index()].attachments.clone();
                for aura_id in attachments {
                    self.card_mut(aura_id).attached_to = None;
                }
                self.card_mut(card_id).attachments.clear();
                self.detach(card_id);

                // Commander returning to command zone: reset battlefield state.
                let keep_counters =
                    crate::staticability::static_ability_counters_remain::counters_remain(
                        &self.cards,
                        &self.cards[card_id.index()],
                        dest_zone,
                    );
                let card = self.card_mut(card_id);
                card.tapped = false;
                card.damage = 0;
                card.power_modifier = 0;
                card.toughness_modifier = 0;
                card.pt_boosts.clear();
                card.static_power_modifier = 0;
                card.static_toughness_modifier = 0;
                card.static_set_power = None;
                card.static_set_toughness = None;
                card.granted_keywords.clear();
                if let Some(type_line) = card.static_type_line_base.take() {
                    card.set_type_line(type_line);
                }
                card.changed_card_types.clear();
                card.static_added_subtypes.clear();
                card.restore_changed_characteristics_baseline();
                card.cant_block_static = false;
                card.summoning_sick = true;
                card.monstrous = false;
                card.controller = card.owner;
                card.cast_from = None;
                if !keep_counters {
                    card.counters.clear();
                }
                if let Some(state) = card.clone_state.take() {
                    card.restore_clone_snapshot(*state);
                } else {
                    card.remove_clone_states();
                }
            }
            ZoneType::Stack => {
                self.card_mut(card_id).controller = dest_owner;
            }
            _ => {}
        }

        // Add to destination zone
        self.add_card_to_zone(dest_zone, dest_owner, card_id);

        // Commander 903.9a tracking: once a commander enters graveyard or exile,
        // SBA may offer moving it to the command zone exactly once.
        let commander_entered_gy_or_exile = self.card(card_id).is_commander
            && matches!(dest_zone, ZoneType::Graveyard | ZoneType::Exile);
        self.card_mut(card_id).move_to_command_zone = commander_entered_gy_or_exile;

        // Forget remembered objects for command effects with ForgetOnMoved.
        let mut exile_effects = Vec::new();
        for eff_id in forget_effects {
            let eff = self.card_mut(eff_id);
            eff.remembered_cards.retain(|&rid| rid != card_id);
            if eff.exile_when_no_remembered && eff.remembered_cards.is_empty() {
                exile_effects.push(eff_id);
            }
        }
        // Effect cards with ForgetOnMoved should be removed from the game
        // entirely (zone = None), not moved to Exile.
        for eff_id in exile_effects {
            let controller = self.card(eff_id).controller;
            self.remove_card_from_zone(ZoneType::Command, controller, eff_id);
            self.card_mut(eff_id).zone = ZoneType::None;
        }

        // Expire temporary effect cards linked to this host leaving play
        // (Duration$ UntilHostLeavesPlay / UntilHostLeavesPlayOrEOT).
        if host_left_battlefield {
            let linked_effects: Vec<CardId> = self
                .cards
                .iter()
                .filter(|c| c.zone == ZoneType::Command && c.temp_effect_host == Some(card_id))
                .map(|c| c.id)
                .collect();
            for eff_id in linked_effects {
                let controller = self.card(eff_id).controller;
                self.remove_card_from_zone(ZoneType::Command, controller, eff_id);
                self.card_mut(eff_id).zone = ZoneType::None;
            }

            // Return cards exiled by this host via ChangeZoneAll Duration$ UntilHostLeavesPlay
            // (e.g. Deputy of Detention: exiled permanents return when it leaves).
            let exiled_by_host: Vec<(CardId, PlayerId, ZoneType)> = self
                .cards
                .iter()
                .filter(|c| c.zone == ZoneType::Exile && c.exiled_by == Some(card_id))
                .map(|c| {
                    (
                        c.id,
                        c.owner,
                        c.until_host_leaves_origin.unwrap_or(ZoneType::Battlefield),
                    )
                })
                .collect();
            for (exiled_id, owner, origin) in exiled_by_host {
                self.card_mut(exiled_id).cleanup_exiled_with();
                self.move_card(exiled_id, origin, owner);
                if let Some(handler) = trigger_handler.as_deref_mut() {
                    let returned_zone = self.card(exiled_id).zone;
                    handler.register_active_trigger(self, exiled_id);
                    crate::ability::effects::zone_triggers::emit_zone_trigger(
                        handler,
                        exiled_id,
                        ZoneType::Exile,
                        returned_zone,
                    );
                }
            }
        }

        apply_continuous_effects(self);
        if let Some(handler) = trigger_handler {
            handler.register_active_trigger(self, card_id);
        }
        debug_assert!(self.card_zone_location_matches_card(card_id));
    }

    pub(crate) fn setup_static_effect(
        &mut self,
        copied: CardId,
        cause: &crate::spellability::SpellAbility,
    ) {
        let Some(static_effect) = cause.ir.static_effect.as_deref() else {
            return;
        };
        if !self.card(copied).type_line.is_permanent() {
            return;
        }
        let Some(source) = cause.source else {
            return;
        };
        if let Some(check_svar) = cause.ir.static_effect_check_svar.as_deref() {
            let cmp = cause
                .ir
                .static_effect_svar_compare
                .as_deref()
                .unwrap_or("GE1");
            let calculate_amount = |amount: &str| {
                amount.parse::<i32>().unwrap_or_else(|_| {
                    self.card(source).get_s_var(amount).map_or(0, |expr| {
                        crate::svar::resolve_svar_expression(
                            expr,
                            self,
                            source,
                            cause.activating_player,
                            cause,
                        )
                    })
                })
            };
            let lhs = calculate_amount(check_svar);
            let rhs = calculate_amount(cmp.get(2..).unwrap_or_default());
            if !crate::parsing::compare::compare_expr(
                lhs,
                &format!("{}{rhs}", cmp.get(..2).unwrap_or_default()),
            ) {
                return;
            }
        }

        let name = format!("Static Effect #{}", cause.id);
        let opt = self
            .cards_in_zone(ZoneType::Command, cause.activating_player)
            .iter()
            .copied()
            .find(|&cid| self.card(cid).card_name == name);

        let eff = match opt {
            Some(eff) => eff,
            None => {
                let Some(mut st_ab) = self.card(source).get_s_var(static_effect).and_then(|raw| {
                    crate::staticability::parse_static_ability(&format!("S$ {raw}"))
                }) else {
                    return;
                };
                let eff =
                    crate::ability::spell_ability_effect::create_effect(self, cause, &name, "");
                st_ab.ir.active_zones = vec![ZoneType::Command];
                st_ab.ir.has_zone_keys = true;
                st_ab.ir.affected_zones = ZoneType::ALL.to_vec();
                st_ab.base.set_intrinsic(true);
                let effect = self.card_mut(eff);
                effect.add_static_ability(st_ab);
                effect.set_forget_on_moved_origin(Some(ZoneType::Battlefield));
                effect.set_exile_when_no_remembered(true);
                eff
            }
        };

        self.card_mut(eff).add_remembered_card(copied);
        apply_continuous_effects(self);
    }

    pub fn has_static_ability_affecting_zone(
        &self,
        zone: ZoneType,
        layer: crate::staticability::Layer,
    ) -> bool {
        self.cards.iter().any(|card| {
            card.zone.is_static_ability_source()
                && card.static_abilities.iter().any(|st_ab| {
                    let affects_zone = if st_ab.ir.affected_zones.is_empty() {
                        zone == ZoneType::Battlefield
                    } else {
                        st_ab.ir.affected_zones.contains(&zone)
                    };
                    affects_zone
                        && st_ab.check_conditions_full(
                            &crate::staticability::StaticMode::Continuous,
                            card,
                            self,
                        )
                        && crate::staticability::classify_static_layers(st_ab).contains(&layer)
                })
        })
    }

    pub fn deal_damage_to_card(&mut self, target: CardId, amount: i32) {
        self.add_damage_after_prevention(DamageTarget::Card(target), amount, None, false);
    }

    pub fn deal_damage(
        &mut self,
        source: CardId,
        target: DamageTarget,
        amount: i32,
        agents: &mut [Box<dyn PlayerAgent>],
        runtime: &mut ReplacementRuntime<'_>,
    ) -> (GameEntity, i32) {
        let entity = match target {
            DamageTarget::Card(card) => GameEntity::Card(card),
            DamageTarget::Player(player) => GameEntity::Player(player),
        };
        if amount <= 0 {
            return (entity, 0);
        }
        let can_be_dealt_damage = match target {
            DamageTarget::Card(card) => self.card(card).can_be_dealt_damage(),
            DamageTarget::Player(player) => {
                !crate::staticability::static_ability_cant_gain_lose_pay_life::cant_lose_life(
                    self, player,
                ) && !crate::player::has_keyword(self, player, "Protection from everything")
                    && !crate::player::player_predicates::is_protected_from(self, player, source)
            }
        };
        if !can_be_dealt_damage {
            return (entity, 0);
        }
        let event = damage_run_params(source, target, amount, false);
        if !has_replace_damage(self, &event) {
            return self.deal_replaced_damage(event);
        }
        let mut damage_map = CardDamageMap::default();
        damage_map.put(source, target, amount);
        let mut prevent_map = CardDamageMap::default();
        run_replace_damage(
            self,
            Some(agents),
            runtime,
            false,
            &mut damage_map,
            &mut prevent_map,
        );
        prevent_map.trigger_prevent_damage(runtime.trigger_handler, false);
        match damage_map.entries().first() {
            Some(&(source, target, amount)) => {
                self.deal_replaced_damage(damage_run_params(source, target, amount, false))
            }
            None => (entity, 0),
        }
    }

    fn deal_replaced_damage(&mut self, event: ReplacementEvent) -> (GameEntity, i32) {
        match event {
            ReplacementEvent::DamageToCard {
                target,
                amount: mut final_amount,
                source,
                ..
            } => {
                // Consume PreventDamage shields. Each shield prevents 1 damage and
                // is removed. Mirrors Java's per-shield ReplaceDamage effect cards
                // in the Command zone, but using the legacy `damage_prevention`
                // counter pending the proper Command-zone effect-card port.
                let shields = self.cards[target.index()].damage_prevention;
                if shields > 0 && final_amount > 0 {
                    let consumed = shields.min(final_amount);
                    self.card_mut(target).damage_prevention -= consumed;
                    final_amount -= consumed;
                }
                let mut dealt = 0;
                if final_amount > 0 {
                    dealt = self
                        .card_mut(target)
                        .add_damage_after_prevention(final_amount);
                    // Fire DealtDamage replacement event after damage is applied.
                    let mut dealt_event = ReplacementEvent::DealtDamage {
                        target,
                        amount: dealt,
                        source,
                    };
                    if dealt > 0 {
                        self.card_mut(target).add_assigned_damage(dealt);
                        if self.card(target).is_creature()
                            && source.is_some_and(|source| {
                                self.get_change_zone_lki_info(source).has_deathtouch()
                            })
                        {
                            self.card_mut(target).mark_deathtouch_damage();
                        }
                        apply_replacements(self, &mut dealt_event);
                    }
                }
                (GameEntity::Card(target), dealt)
            }
            ReplacementEvent::DamageToPlayer { target, amount, .. } => {
                let dealt = if amount > 0 {
                    self.player_deal_damage(target, amount)
                } else {
                    0
                };
                (GameEntity::Player(target), dealt)
            }
            _ => unreachable!(),
        }
    }

    pub fn add_damage_after_prevention(
        &mut self,
        target: DamageTarget,
        amount: i32,
        source: Option<CardId>,
        is_combat: bool,
    ) -> i32 {
        if amount <= 0 {
            return 0;
        }
        let event = match target {
            DamageTarget::Card(target) => {
                if !self.card(target).can_be_dealt_damage() {
                    return 0;
                }
                ReplacementEvent::DamageToCard {
                    target,
                    amount,
                    source,
                    is_combat,
                }
            }
            DamageTarget::Player(target) => {
                if crate::staticability::static_ability_cant_gain_lose_pay_life::cant_lose_life(
                    self, target,
                ) || crate::player::has_keyword(self, target, "Protection from everything")
                    || source.is_some_and(|source| {
                        crate::player::player_predicates::is_protected_from(self, target, source)
                    })
                {
                    return 0;
                }
                ReplacementEvent::DamageToPlayer {
                    target,
                    amount,
                    source,
                    is_combat,
                }
            }
        };
        self.deal_replaced_damage(event).1
    }

    pub fn deal_damage_to_player(&mut self, target: PlayerId, amount: i32) -> i32 {
        self.add_damage_after_prevention(DamageTarget::Player(target), amount, None, false)
    }

    pub fn process_damage(
        &mut self,
        trigger_handler: &mut TriggerHandler,
        mut agents: Option<&mut [Box<dyn PlayerAgent>]>,
    ) -> Vec<(PlayerId, i32)> {
        let mut life_lost_all_damage_map = Vec::new();
        for player in self.player_order.clone() {
            let lost =
                crate::player::process_damage(self, trigger_handler, agents.as_deref_mut(), player);
            if lost > 0 {
                life_lost_all_damage_map.push((player, lost));
            }
        }
        life_lost_all_damage_map
    }

    pub fn lose_life_simultaneously(
        &mut self,
        trigger_handler: &mut TriggerHandler,
        agents: Option<&mut [Box<dyn PlayerAgent>]>,
    ) {
        let life_lost_all_damage_map = self.process_damage(trigger_handler, agents);
        run_life_lost_all(trigger_handler, &life_lost_all_damage_map);
    }

    /// Check and apply state-based actions. Returns true if any were applied.
    pub fn check_state_based_actions(&mut self) -> bool {
        self.check_state_based_actions_with_triggers(None, None)
    }

    /// Check and apply state-based actions. Returns true if any were applied.
    /// If provided, emits ChangesZone triggers for SBA zone moves.
    /// `legend_keep_fn` — optional callback for legend rule: given (player, duplicates),
    /// returns the CardId to keep.  Mirrors Java's `chooseSingleEntityForEffect`.
    pub fn check_state_based_actions_with_triggers(
        &mut self,
        trigger_handler: Option<&mut TriggerHandler>,
        legend_keep_fn: Option<&mut dyn FnMut(PlayerId, &[CardId]) -> CardId>,
    ) -> bool {
        self.check_state_based_actions_impl(trigger_handler, legend_keep_fn, None)
    }

    pub fn check_state_based_actions_with_trigger_agents(
        &mut self,
        trigger_handler: Option<&mut TriggerHandler>,
        agents: &mut [Box<dyn PlayerAgent>],
    ) -> bool {
        self.check_state_based_actions_impl(trigger_handler, None, Some(agents))
    }

    fn state_based_action_role(&mut self, card_id: CardId) -> bool {
        let roles: Vec<CardId> = self
            .card(card_id)
            .attachments
            .iter()
            .copied()
            .filter(|&attached| self.card(attached).type_line.has_subtype("Role"))
            .collect();
        if roles.is_empty() {
            return false;
        }
        let mut check_again = false;
        for pid in self.player_order.clone() {
            let mut roles_by_player: Vec<CardId> = roles
                .iter()
                .copied()
                .filter(|&role| self.card(role).controller == pid)
                .collect();
            if roles_by_player.len() <= 1 {
                continue;
            }
            roles_by_player.sort_by(|&a, &b| {
                crate::card::card_predicates::compare_by_game_timestamp(self, a, b)
            });
            roles_by_player.pop();
            for role in roles_by_player {
                self.detach(role);
            }
            check_again = true;
        }
        check_again
    }

    fn state_based_action_saga(
        &self,
        cid: CardId,
        trigger_handler: Option<&TriggerHandler>,
        sacrifice_list: &mut Vec<CardId>,
    ) -> bool {
        let card = self.card(cid);
        if !card.type_line.has_subtype("Saga") || !card.has_chapter() {
            return false;
        }
        if crate::staticability::static_ability_cant_sacrifice::cant_sacrifice(
            &self.cards,
            card,
            None,
            true,
        ) {
            return false;
        }
        if card.counter_count(&CounterType::Lore) < card.get_final_chapter_nr() {
            return false;
        }
        if self.stack.has_source_chapter_on_stack(self, cid)
            || trigger_handler.is_some_and(|handler| handler.has_source_chapter_pending(self, cid))
        {
            return false;
        }
        sacrifice_list.push(cid);
        true
    }

    fn on_player_lost(
        &mut self,
        player: PlayerId,
        trigger_handler: &mut Option<&mut TriggerHandler>,
    ) {
        self.player_mut(player).left_game = true;
        let is_multiplayer = self.player_order.len() > 2;
        let all_cards: Vec<CardId> = (0..self.cards.len()).map(|i| CardId(i as u32)).collect();

        if !is_multiplayer {
            // CR 707.9: at the end of the game every face-down card is revealed.
            for &cid in &all_cards {
                self.card_mut(cid).force_turn_face_up();
            }
            return;
        }

        // CR 724.4 / CR 725.4. Reassigned before the sweep so the old effect
        if self.monarch == Some(player) {
            let heir = if self.turn.active_player == player {
                self.next_player(player)
            } else {
                self.turn.active_player
            };
            self.player_set_monarch(heir, trigger_handler.as_deref_mut());
        }
        if self.initiative_holder == Some(player) {
            let heir = if self.turn.active_player == player {
                self.next_player(player)
            } else {
                self.turn.active_player
            };
            self.player_take_initiative(heir, trigger_handler.as_deref_mut());
        }

        let next = self.next_player(player);
        for &cid in &all_cards {
            let (zone, owner, controller) = {
                let card = &self.cards[cid.index()];
                (card.zone, card.owner, card.controller)
            };
            if zone == ZoneType::None {
                continue;
            }
            if owner != player {
                // CR 800.4c: nothing stays enchanting the leaving player.
                if self.cards[cid.index()].attached_to_player == Some(player) {
                    self.card_mut(cid).attached_to_player = None;
                }
                continue;
            }
            if self.cards[cid.index()].effect_source.is_some() && zone == ZoneType::Command {
                // Mirrors Java: lingering effects move to the next player so
                // they continue to work.
                self.remove_card_from_zone(ZoneType::Command, controller, cid);
                self.card_mut(cid).controller = next;
                self.add_card_to_zone(ZoneType::Command, next, cid);
                continue;
            }
            // CR 800.4a: objects owned by the leaving player leave the game.
            for &other in &all_cards {
                if other == cid {
                    continue;
                }
                let other_card = self.card_mut(other);
                other_card.imprinted_cards.retain(|&r| r != cid);
                other_card.exiled_cards.retain(|&r| r != cid);
                other_card.remembered_cards.retain(|&r| r != cid);
                other_card.attachments.retain(|&r| r != cid);
                other_card.gain_control_targets.retain(|&r| r != cid);
                if other_card.attached_to == Some(cid) {
                    other_card.attached_to = None;
                }
            }
            if let Some(handler) = trigger_handler.as_deref_mut() {
                crate::ability::effects::emit_zone_trigger(handler, cid, zone, ZoneType::None);
            }
            self.remove_card_from_zone(zone, controller, cid);
            self.card_mut(cid).zone = ZoneType::None;
        }

        apply_continuous_effects(self);

        // CR 800.4d as Java implements it: permanents the leaving player
        for &cid in &all_cards {
            let (zone, owner, controller) = {
                let card = &self.cards[cid.index()];
                (card.zone, card.owner, card.controller)
            };
            if zone == ZoneType::Battlefield && controller == player && owner != player {
                if let Some(handler) = trigger_handler.as_deref_mut() {
                    crate::ability::effects::emit_zone_trigger(
                        handler,
                        cid,
                        ZoneType::Battlefield,
                        ZoneType::Exile,
                    );
                }
                self.move_card_without_replacement(cid, ZoneType::Exile, owner);
            }
        }
    }

    fn move_battlefield_card_to_graveyard_for_sba(
        &mut self,
        cid: CardId,
        trigger_handler: &mut Option<&mut TriggerHandler>,
        agents: &mut Option<&mut [Box<dyn PlayerAgent>]>,
        parts: &mut Option<&mut SbaReplacementParts<'_>>,
    ) {
        let owner = self.card(cid).owner;
        let mut moved_event = ReplacementEvent::Moved {
            card: cid,
            origin: ZoneType::Battlefield,
            destination: ZoneType::Graveyard,
            is_discard: false,
            counter_map: None,
            counter_cause: None,
            counter_is_effect: false,
            after_replacement_static_abilities: Vec::new(),
            stack_sa: None,
            fizzle: None,
        };
        let result = match (
            agents.as_deref_mut(),
            trigger_handler.as_deref_mut(),
            parts.as_deref_mut(),
        ) {
            (Some(agents), Some(handler), Some(parts)) => {
                let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
                    trigger_handler: handler,
                    token_templates: parts.token_templates,
                    token_art_variants: parts.token_art_variants,
                    token_fallback: parts.token_fallback,
                    edition_dates: parts.edition_dates,
                    mana_pools: &mut *parts.mana_pools,
                    rng: &mut *parts.rng,
                };
                crate::replacement::replacement_handler::apply_replacements_with_agents_and_runtime(
                    self,
                    agents,
                    &mut runtime,
                    &mut moved_event,
                )
            }
            (Some(agents), _, _) => apply_replacements_with_agents(self, agents, &mut moved_event),
            _ => apply_replacements(self, &mut moved_event),
        };
        if !matches!(
            result,
            ReplacementResult::NotReplaced | ReplacementResult::Updated
        ) {
            return;
        }
        let final_dest = if let ReplacementEvent::Moved { destination, .. } = moved_event {
            destination
        } else {
            ZoneType::Graveyard
        };
        let old_zone = self.card(cid).zone;
        // Emit trigger BEFORE move_card so LKI state is still available for
        // trigger matching. Persist/Undying and Modular inspect the dying card.
        if let Some(handler) = trigger_handler.as_deref_mut() {
            let (lki_power, lki_toughness, lki_counters) = match self.get_lki_snapshot(cid) {
                Some(lki) => (lki.power, lki.toughness, lki.counters.clone()),
                None => {
                    let card = self.card(cid);
                    (card.power(), card.toughness(), card.counters.clone())
                }
            };
            let lki_p1p1 = *lki_counters.get(&CounterType::P1P1).unwrap_or(&0);
            self.card_mut(cid).lki_counters = Some(lki_counters);
            self.card_mut(cid)
                .set_lki_power_toughness(Some(lki_power), Some(lki_toughness));
            crate::ability::effects::emit_zone_trigger_with_lki_counters(
                handler,
                cid,
                old_zone,
                final_dest,
                lki_p1p1,
                lki_power,
                lki_toughness,
            );
            handler.flush_waiting_triggers(self);
        }
        self.move_card_without_replacement(cid, final_dest, owner);
    }

    pub(crate) fn order_cards_by_their_owners(
        &self,
        list: Vec<CardId>,
        dest: ZoneType,
        agents: &mut Option<&mut [Box<dyn PlayerAgent>]>,
    ) -> Vec<CardId> {
        self.order_cards_by_their_owners_for_sa(list, dest, None, agents)
    }

    pub(crate) fn order_cards_by_their_owners_for_sa(
        &self,
        list: Vec<CardId>,
        dest: ZoneType,
        sa: Option<&crate::spellability::SpellAbility>,
        agents: &mut Option<&mut [Box<dyn PlayerAgent>]>,
    ) -> Vec<CardId> {
        if list.len() <= 1 {
            return list;
        }
        let gain_control_decider = sa.filter(|sa| sa.ir.gain_control).and_then(|sa| {
            match sa.ir.gain_control_text.as_deref() {
                // Java `getDefinedPlayers` answers `True` with every player in turn order.
                None | Some("True") => self.alive_players().first().copied(),
                Some(defined) => crate::ability::ability_utils::resolve_defined_players_with_sa(
                    defined,
                    sa,
                    sa.activating_player,
                    self,
                )
                .first()
                .copied(),
            }
        });
        let active_player = self.active_player();
        let start = self
            .player_order
            .iter()
            .position(|&pid| pid == active_player)
            .unwrap_or(0);
        let mut complete_list = Vec::with_capacity(list.len());
        for offset in 0..self.player_order.len() {
            let pid = self.player_order[(start + offset) % self.player_order.len()];
            let sub_list: Vec<CardId> = list
                .iter()
                .copied()
                .filter(|&cid| {
                    let card = self.card(cid);
                    let decider = if dest == ZoneType::Battlefield {
                        card.controller
                    } else {
                        card.owner
                    };
                    gain_control_decider.unwrap_or(decider) == pid
                })
                .collect();
            match agents.as_deref_mut() {
                Some(agents) if sub_list.len() > 1 => complete_list.extend(
                    agents[pid.index()].order_move_to_zone_list(self, pid, &sub_list, dest),
                ),
                _ => complete_list.extend(sub_list),
            }
        }
        complete_list
    }

    pub(crate) fn check_state_based_actions_with_runtime(
        &mut self,
        trigger_handler: &mut TriggerHandler,
        parts: &mut SbaReplacementParts<'_>,
        agents: &mut [Box<dyn PlayerAgent>],
    ) -> bool {
        for (player, pool) in self.players.iter_mut().zip(parts.mana_pools.iter()) {
            player.mana_pool_colors = pool.mana_colors();
        }
        self.check_state_based_actions_with_parts(
            Some(trigger_handler),
            None,
            Some(agents),
            Some(parts),
        )
    }

    fn check_state_based_actions_impl(
        &mut self,
        trigger_handler: Option<&mut TriggerHandler>,
        legend_keep_fn: Option<&mut dyn FnMut(PlayerId, &[CardId]) -> CardId>,
        agents: Option<&mut [Box<dyn PlayerAgent>]>,
    ) -> bool {
        self.check_state_based_actions_with_parts(trigger_handler, legend_keep_fn, agents, None)
    }

    fn check_state_based_actions_with_parts(
        &mut self,
        mut trigger_handler: Option<&mut TriggerHandler>,
        mut legend_keep_fn: Option<&mut dyn FnMut(PlayerId, &[CardId]) -> CardId>,
        mut agents: Option<&mut [Box<dyn PlayerAgent>]>,
        mut parts: Option<&mut SbaReplacementParts<'_>>,
    ) -> bool {
        self.statics_current_after_sba = false;
        // Capture battlefield state before SBA processing. Used by DisableTriggers
        // (Hushbringer) to check LKI — if a creature with DisableTriggers dies in
        // the same SBA batch as another creature, it still suppresses death triggers.
        // Mirrors Java's LastStateBattlefield passed through RunParams.
        self.pre_sba_battlefield = self
            .cards
            .iter()
            .filter(|c| c.zone == ZoneType::Battlefield)
            .map(|c| c.id)
            .collect();
        self.copy_last_state();

        let mut any_changes = false;
        let mut newly_lost_players: Vec<PlayerId> = Vec::new();

        // Check players with 0 or less life
        for pid in self.player_order.clone() {
            if self.player(pid).tried_to_draw_from_empty_library && self.player(pid).is_alive() {
                self.player_mut(pid).tried_to_draw_from_empty_library = false;
                let mut event = ReplacementEvent::GameLoss {
                    player: pid,
                    reason: GameLossReason::Milled,
                };
                let result = apply_replacements(self, &mut event);
                if result != ReplacementResult::Replaced && !self.player(pid).has_lost {
                    self.player_mark_lost(pid, GameLossReason::Milled);
                    newly_lost_players.push(pid);
                    any_changes = true;
                }
            }
            if self.player(pid).life <= 0 && self.player(pid).is_alive() {
                let mut event = ReplacementEvent::GameLoss {
                    player: pid,
                    reason: GameLossReason::LifeReachedZero,
                };
                let result = apply_replacements(self, &mut event);
                if result != ReplacementResult::Replaced && !self.player(pid).has_lost {
                    self.player_mark_lost(pid, GameLossReason::LifeReachedZero);
                    newly_lost_players.push(pid);
                    any_changes = true;
                }
            }
            // Check poison counters (10+ = lose)
            if self.player(pid).poison_counters >= 10 && self.player(pid).is_alive() {
                let mut event = ReplacementEvent::GameLoss {
                    player: pid,
                    reason: GameLossReason::Poisoned,
                };
                let result = apply_replacements(self, &mut event);
                if result != ReplacementResult::Replaced {
                    if !self.player(pid).has_lost {
                        self.player_mark_lost(pid, GameLossReason::Poisoned);
                        newly_lost_players.push(pid);
                    }
                    any_changes = true;
                }
            }
            // Check commander damage (21+ from a single commander source = lose)
            if self.player(pid).commander_damage_enabled {
                let commander_dmg_entries: Vec<(u32, i32)> = self
                    .player(pid)
                    .commander_damage_received
                    .iter()
                    .map(|(&k, &v)| (k, v))
                    .collect();
                for (_card_raw_id, dmg) in commander_dmg_entries {
                    if dmg >= 21 && self.player(pid).is_alive() && !self.player(pid).has_lost {
                        self.player_mark_lost(pid, GameLossReason::CommanderDamage);
                        newly_lost_players.push(pid);
                        any_changes = true;
                    }
                }
            }

            // CR 704.5z: If a player controls a permanent with Start your
            // engines! and that player has no speed, their speed becomes 1.
            if self.player(pid).speed == 0
                && self
                    .cards_in_zone(ZoneType::Battlefield, pid)
                    .iter()
                    .any(|&cid| self.card(cid).has_keyword("Start your engines"))
            {
                self.increase_player_speed(pid, trigger_handler.as_deref_mut());
                any_changes = true;
            }
        }

        for pid in self.player_order.clone() {
            if !self.player(pid).is_alive()
                && !self.player(pid).left_game
                && !newly_lost_players.contains(&pid)
            {
                newly_lost_players.push(pid);
                any_changes = true;
            }
        }

        if !newly_lost_players.is_empty() {
            for pid in &newly_lost_players {
                self.on_player_lost(*pid, &mut trigger_handler);
                self.stack.remove_instances_controlled_by(*pid);
            }
            if let Some(handler) = trigger_handler.as_deref_mut() {
                for pid in &newly_lost_players {
                    handler.run_trigger(
                        TriggerType::LosesGame,
                        RunParams {
                            player: Some(*pid),
                            ..Default::default()
                        },
                        false,
                    );
                    handler.on_player_lost(*pid);
                }
            }
        }

        // `GameAction.checkStateEffects` runs `checkGameOverCondition` first and returns
        // once the game is over, before any permanent is looked at.
        let alive = self.alive_players();
        if alive.len() <= 1 {
            self.game_over = true;
            if alive.len() == 1 {
                self.winner = Some(alive[0]);
            }
            return !newly_lost_players.is_empty();
        }

        for pass in 0..9 {
            let statics_applied = !self.hold_checking_static_abilities;
            apply_continuous_effects(self);
            self.statics_current_after_sba = statics_applied;
            if pass > 0 {
                self.pre_sba_battlefield = self
                    .cards
                    .iter()
                    .filter(|c| c.zone == ZoneType::Battlefield)
                    .map(|c| c.id)
                    .collect();
            }
            let outer_table = self
                .pending_change_zone_table
                .replace(crate::card::card_zone_table::CardZoneTable::default());
            let outer_last_state = self
                .replacement_last_state_battlefield
                .replace(self.pre_sba_battlefield.clone());
            let changed = self.state_based_actions_pass(
                &mut trigger_handler,
                &mut legend_keep_fn,
                &mut agents,
                &mut parts,
            );
            self.replacement_last_state_battlefield = outer_last_state;
            let cards = &self.cards;
            let pre_sba_battlefield = &self.pre_sba_battlefield;
            let combat_lki_len = self.last_state_battlefield_combat_lki.len();
            self.last_state_battlefield_combat_lki.retain(|(id, _)| {
                !pre_sba_battlefield.contains(id) || cards[id.index()].zone != ZoneType::Graveyard
            });
            if changed || self.last_state_battlefield_combat_lki.len() != combat_lki_len {
                self.statics_current_after_sba = false;
            }
            let table = std::mem::replace(&mut self.pending_change_zone_table, outer_table);
            if let (Some(handler), Some(table)) = (trigger_handler.as_deref_mut(), table) {
                table.trigger_changes_zone_all(handler, self, None);
                handler.flush_waiting_triggers(self);
            }
            if !changed {
                break;
            }
            any_changes = true;
        }

        // Check game over
        let alive = self.alive_players();
        if alive.len() <= 1 {
            self.game_over = true;
            if alive.len() == 1 {
                self.winner = Some(alive[0]);
            }
        }

        any_changes
    }

    fn handle_legend_rule(
        &self,
        pid: PlayerId,
        no_reg_creats: &mut Vec<CardId>,
        legend_keep_fn: &mut Option<&mut dyn FnMut(PlayerId, &[CardId]) -> CardId>,
        agents: &mut Option<&mut [Box<dyn PlayerAgent>]>,
    ) -> bool {
        let battlefield = self.cards_in_zone(ZoneType::Battlefield, pid).to_vec();
        let mut by_name: std::collections::BTreeMap<String, Vec<CardId>> =
            std::collections::BTreeMap::new();
        for cid in battlefield {
            let c = self.card(cid);
            if !c.type_line.is_legendary() {
                continue;
            }
            if crate::staticability::static_ability_ignore_legend_rule::ignore_legend_rule(
                &self.cards,
                c,
            ) {
                continue;
            }
            by_name.entry(c.card_name.clone()).or_default().push(cid);
        }
        let mut recheck = false;
        for (_name, ids) in by_name {
            if ids.len() <= 1 {
                continue;
            }
            recheck = true;
            let keep = if let Some(chooser) = legend_keep_fn.as_deref_mut() {
                chooser(pid, &ids)
            } else if let Some(agents) = agents.as_deref_mut() {
                agents[pid.index()].snapshot_state(self, &[]);
                agents[pid.index()].choose_legend_keep(pid, &ids)
            } else {
                ids[0]
            };
            for cid in ids {
                if cid != keep && !no_reg_creats.contains(&cid) {
                    no_reg_creats.push(cid);
                }
            }
        }
        recheck
    }

    fn state_based_action_704_5d(&self, card_id: CardId, zone: ZoneType) -> bool {
        let card = &self.cards[card_id.index()];
        card.is_token
            && !matches!(zone, ZoneType::Battlefield | ZoneType::Stack)
            && !(zone == ZoneType::Exile && card.is_in_prepared_spell_state())
    }

    fn state_based_actions_pass(
        &mut self,
        trigger_handler: &mut Option<&mut TriggerHandler>,
        legend_keep_fn: &mut Option<&mut dyn FnMut(PlayerId, &[CardId]) -> CardId>,
        agents: &mut Option<&mut [Box<dyn PlayerAgent>]>,
        parts: &mut Option<&mut SbaReplacementParts<'_>>,
    ) -> bool {
        let mut any_changes = false;
        let mut sacrifice_list: Vec<CardId> = Vec::new();

        let tokens_outside_battlefield: Vec<(CardId, crate::zone::ZoneKey)> = self
            .iter_zones()
            .flat_map(|(key, zone)| zone.cards.iter().map(move |&cid| (cid, key)))
            .filter(|&(cid, key)| self.state_based_action_704_5d(cid, key.zone_type))
            .collect();
        for (cid, key) in tokens_outside_battlefield {
            self.remove_card_from_zone(key.zone_type, key.owner, cid);
            self.card_mut(cid).zone = ZoneType::None;
            any_changes = true;
        }

        // Check creatures with lethal damage or 0 toughness
        let battlefield_cards: Vec<CardId> = self
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| self.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();

        let mut no_reg_creats: Vec<CardId> = Vec::new();
        let mut des_creats: Vec<CardId> = Vec::new();
        for cid in battlefield_cards {
            let card = &self.cards[cid.index()];
            if !card.is_creature() {
                continue;
            }
            if card.toughness() <= 0 {
                no_reg_creats.push(cid);
            } else if card.lethal_damage() || card.has_deathtouch_damage {
                des_creats.push(cid);
            } else {
                continue;
            }
            self.card_mut(cid).has_deathtouch_damage = false;
            self.statics_current_after_sba = false;
        }

        for &pid in &self.player_order.clone() {
            any_changes |= self.handle_legend_rule(pid, &mut no_reg_creats, legend_keep_fn, agents);
        }
        des_creats.retain(|cid| !no_reg_creats.contains(cid));

        self.hold_checking_static_abilities = true;
        if no_reg_creats.len() > 1 {
            no_reg_creats =
                self.order_cards_by_their_owners(no_reg_creats, ZoneType::Graveyard, agents);
        }
        for cid in no_reg_creats {
            self.move_battlefield_card_to_graveyard_for_sba(cid, trigger_handler, agents, parts);
            any_changes = true;
        }

        if des_creats.len() > 1 {
            des_creats.retain(|&cid| !self.cards[cid.index()].has_keyword("Indestructible"));
            des_creats = self.order_cards_by_their_owners(des_creats, ZoneType::Graveyard, agents);
        }
        for cid in des_creats {
            if self.cards[cid.index()].has_keyword("Indestructible") {
                continue;
            }
            // CR 702.89: Umbra armor (Totem Armor) — if enchanted creature
            // would be destroyed, instead remove all damage and destroy the aura.
            let has_umbra = self.cards[cid.index()].attachments.iter().any(|&aid| {
                aid.index() < self.cards.len()
                    && self.cards[aid.index()].zone == ZoneType::Battlefield
                    && (self.cards[aid.index()].has_keyword("Umbra armor")
                        || self.cards[aid.index()].has_keyword("Totem armor"))
            });
            if has_umbra {
                // Find the first umbra armor aura and destroy it instead
                let umbra_id = self.cards[cid.index()]
                    .attachments
                    .iter()
                    .copied()
                    .find(|&aid| {
                        aid.index() < self.cards.len()
                            && self.cards[aid.index()].zone == ZoneType::Battlefield
                            && (self.cards[aid.index()].has_keyword("Umbra armor")
                                || self.cards[aid.index()].has_keyword("Totem armor"))
                    });
                if let Some(umbra_id) = umbra_id {
                    // Remove all damage from the creature
                    self.card_mut(cid).damage = 0;
                    self.card_mut(cid).has_deathtouch_damage = false;
                    // Destroy the aura instead
                    let umbra_owner = self.cards[umbra_id.index()].owner;
                    let old_zone = self.cards[umbra_id.index()].zone;
                    self.move_card(umbra_id, ZoneType::Graveyard, umbra_owner);
                    if let Some(handler) = trigger_handler.as_deref_mut() {
                        crate::ability::effects::emit_zone_trigger(
                            handler,
                            umbra_id,
                            old_zone,
                            ZoneType::Graveyard,
                        );
                    }
                    any_changes = true;
                    continue; // Creature survives
                }
            }

            // Run Destroy replacement effects (R$-based indestructible, etc.).
            // Mirrors Java GameAction.destroy() → ReplacementHandler.run(Destroy, …).
            let mut destroy_event = ReplacementEvent::Destroy { target: cid };
            let result = apply_replacements(self, &mut destroy_event);
            if result != ReplacementResult::Replaced {
                self.move_battlefield_card_to_graveyard_for_sba(
                    cid,
                    trigger_handler,
                    agents,
                    parts,
                );
                // Same-SBA-batch LTB lookback is derived per-event from
                // `pre_sba_battlefield` in `TriggerHandler::ltb_trigger_refs_for_event`.
                // No global registration needed.
                any_changes = true;
            } else {
                // Indestructible — destruction was replaced; creature stays.
                // Damage is still marked but the creature does not die.
            }
        }

        let battlefield_cards: Vec<CardId> = self
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| self.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();

        for cid in battlefield_cards {
            let should_put_in_graveyard = {
                let card = self.card(cid);
                card.type_line.is_planeswalker() && card.counter_count(&CounterType::Loyalty) <= 0
            };
            if !should_put_in_graveyard {
                continue;
            }

            self.move_battlefield_card_to_graveyard_for_sba(cid, trigger_handler, agents, parts);
            any_changes = true;
        }

        let saga_cards: Vec<CardId> = self
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| self.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();
        for cid in saga_cards {
            any_changes |=
                self.state_based_action_saga(cid, trigger_handler.as_deref(), &mut sacrifice_list);
        }

        if sacrifice_list.len() > 1 {
            sacrifice_list =
                self.order_cards_by_their_owners(sacrifice_list, ZoneType::Graveyard, agents);
        }
        if !sacrifice_list.is_empty() {
            if let (Some(handler), Some(agents), Some(parts)) = (
                trigger_handler.as_deref_mut(),
                agents.as_deref_mut(),
                parts.as_deref_mut(),
            ) {
                let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
                    trigger_handler: handler,
                    token_templates: parts.token_templates,
                    token_art_variants: parts.token_art_variants,
                    token_fallback: parts.token_fallback,
                    edition_dates: parts.edition_dates,
                    mana_pools: &mut *parts.mana_pools,
                    rng: &mut *parts.rng,
                };
                if !crate::game_loop::perform_sacrifice(self, &mut runtime, agents, &sacrifice_list)
                    .is_empty()
                {
                    any_changes = true;
                }
            } else {
                for cid in sacrifice_list.drain(..) {
                    let owner = self.card(cid).owner;
                    self.move_card_without_replacement(cid, ZoneType::Graveyard, owner);
                }
                any_changes = true;
            }
        }
        self.hold_checking_static_abilities = false;

        // CR 704.5q: +1/+1 and -1/-1 counter cancellation
        for &pid in &self.player_order.clone() {
            let battlefield = self.cards_in_zone(ZoneType::Battlefield, pid).to_vec();
            for cid in battlefield {
                let p1 = self.card(cid).counter_count(&CounterType::P1P1);
                let m1 = self.card(cid).counter_count(&CounterType::M1M1);
                if p1 > 0 && m1 > 0 {
                    let cancel = p1.min(m1);
                    self.card_mut(cid)
                        .remove_counter(&CounterType::P1P1, cancel);
                    self.card_mut(cid)
                        .remove_counter(&CounterType::M1M1, cancel);
                    any_changes = true;
                }
            }
        }

        // CR 903.9a: a commander in graveyard or exile may move to command zone.
        for &pid in &self.player_order.clone() {
            let mut commander_candidates = self.cards_in_zone(ZoneType::Graveyard, pid).to_vec();
            commander_candidates.extend(self.cards_in_zone(ZoneType::Exile, pid).iter().copied());
            for cid in commander_candidates {
                if !self.card(cid).can_move_to_command_zone() {
                    continue;
                }
                self.card_mut(cid).move_to_command_zone = false;
                self.statics_current_after_sba = false;
                let accepted = if let Some(agents) = agents.as_deref_mut() {
                    let name = self.card(cid).card_name.clone();
                    let message = format!(
                        "{name}: If a commander is in a graveyard or in exile and that card was put into that zone since the last time state-based actions were checked, its owner may put it into the command zone."
                    );
                    agents[pid.index()].confirm_action(
                        pid,
                        Some("ChangeZoneToAltDestination"),
                        &message,
                        &[],
                        Some(cid),
                        None,
                    )
                } else {
                    false
                };
                if accepted {
                    self.move_card_without_replacement(cid, ZoneType::Command, pid);
                    any_changes = true;
                }
            }
        }

        let role_hosts: Vec<CardId> = self
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| self.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();
        for cid in role_hosts {
            any_changes |= self.state_based_action_role(cid);
        }

        // CR 704.5n: Aura SBA — an Aura on the battlefield that is not attached
        // to a legal permanent (or whose host left the battlefield) is put into
        // its owner's graveyard.
        {
            let aura_ids: Vec<CardId> = self
                .player_order
                .iter()
                .flat_map(|&pid| self.cards_in_zone(ZoneType::Battlefield, pid))
                .map(|&cid| &self.cards[cid.index()])
                .filter(|c| {
                    c.zone == ZoneType::Battlefield
                        && c.type_line.has_subtype("Aura")
                        && !c.type_line.is_creature() // Bestowed auras that became creatures stay
                })
                .filter(|c| {
                    match (c.attached_to, c.attached_to_player) {
                        (None, None) => true, // Not attached to anything — orphaned
                        (None, Some(player_id)) => {
                            if player_id.index() >= self.players.len() {
                                return true;
                            }
                            let player = &self.players[player_id.index()];
                            let enchant_type = c
                                .keywords
                                .iter_strings()
                                .find_map(|kw| {
                                    crate::keyword::extract_keyword_cost_str(kw, "Enchant")
                                })
                                .unwrap_or_default();
                            player.has_lost || !enchant_type.eq_ignore_ascii_case("Player")
                        }
                        (Some(host_id), _) => {
                            if host_id.index() >= self.cards.len() {
                                return true; // Invalid host ID
                            }
                            let host = &self.cards[host_id.index()];
                            // CR 704.5n: check if the enchant restriction is still met.
                            // E.g. "Enchant creature" requires a battlefield creature, while
                            // Animate Dead's "Enchant creature card in a graveyard" remains legal
                            // while attached to a creature card in a graveyard.
                            let enchant_type = c
                                .keywords
                                .iter_strings()
                                .find_map(|kw| {
                                    crate::keyword::extract_keyword_cost_str(kw, "Enchant")
                                })
                                .unwrap_or_default();
                            !crate::parsing::enchant_type_matches_card(enchant_type, host, Some(c))
                                || !can_attachment_remain_attached(&self.cards, c, host, true)
                        }
                    }
                })
                .map(|c| c.id)
                .collect();
            let aura_ids = self.order_cards_by_their_owners(aura_ids, ZoneType::Graveyard, agents);

            for aura_id in aura_ids {
                let owner = self.card(aura_id).owner;
                let old_zone = self.card(aura_id).zone;
                self.move_card(aura_id, ZoneType::Graveyard, owner);
                if let Some(handler) = trigger_handler.as_deref_mut() {
                    crate::ability::effects::emit_zone_trigger(
                        handler,
                        aura_id,
                        old_zone,
                        ZoneType::Graveyard,
                    );
                }
                any_changes = true;
            }
        }

        {
            let unattach_ids: Vec<CardId> = self
                .cards
                .iter()
                .filter(|c| c.zone == ZoneType::Battlefield && !c.type_line.has_subtype("Aura"))
                .filter(|c| match c.attached_to {
                    Some(host_id) if host_id.index() < self.cards.len() => {
                        let host = &self.cards[host_id.index()];
                        host.zone == ZoneType::Battlefield
                            && (c.is_creature()
                                || c.type_line.core_types.contains(&CoreType::Battle)
                                || !can_attachment_remain_attached(&self.cards, c, host, true))
                    }
                    _ => false,
                })
                .map(|c| c.id)
                .collect();

            for attachment_id in unattach_ids {
                self.detach(attachment_id);
                any_changes = true;
            }
        }

        any_changes
    }

    /// Untap all permanents controlled by a player.
    /// Runs Untap replacement effects for each permanent.
    pub fn untap_all(&mut self, player: PlayerId) {
        let cards: Vec<CardId> = self.cards_in_zone(ZoneType::Battlefield, player).to_vec();
        for cid in cards {
            // Use untap() which runs replacement effects
            self.untap_during_untap_step(cid, player);
        }
    }

    /// Draw a card for a player. Returns the drawn card ID, or None if the draw
    /// was skipped or the library is empty.
    ///
    /// Runs Draw replacement effects before drawing.  If the draw is replaced
    /// (e.g. "skip your draw step"), returns `None`.
    ///
    /// Mirrors Java `GameAction.draw()` calling `ReplacementHandler.run(Draw, …)`.
    pub fn draw_card(&mut self, player: PlayerId) -> Option<CardId> {
        self.player_draw_one(player)
    }

    /// Draw a card with agent access for Optional replacement effects (Dredge).
    pub fn draw_card_with_agents(
        &mut self,
        player: PlayerId,
        agents: &mut [Box<dyn crate::agent::PlayerAgent>],
    ) -> Option<CardId> {
        self.player_draw_one_internal(player, false, Some(agents))
    }

    /// Draw N cards for a player. Returns drawn card IDs.
    pub fn draw_cards(&mut self, player: PlayerId, n: usize) -> Vec<CardId> {
        self.player_draw_cards(player, n)
    }

    /// Shuffle a player's library using the provided RNG.
    pub fn shuffle_library(&mut self, player: PlayerId, rng: &mut impl rand::Rng) {
        self.shuffle_zone_cards_with_rand(ZoneType::Library, player, rng);
    }

    /// Reset per-turn state for all cards and players of a given player.
    pub fn new_turn_for_player(&mut self, player: PlayerId) {
        for pid in self.player_order.clone() {
            let hand_size = self.zone(ZoneType::Hand, pid).len() as i32;
            self.player_mut(pid).on_cleanup_phase(hand_size);
        }
        self.player_new_turn(player);

        let all_card_ids: Vec<CardId> = (0..self.cards.len()).map(|i| CardId(i as u32)).collect();
        for cid in all_card_ids {
            if self.cards[cid.index()].zone == ZoneType::Battlefield {
                self.card_mut(cid).started_turn_tapped = self.cards[cid.index()].tapped;
            }
            if self.cards[cid.index()].controller == player {
                self.card_mut(cid).new_turn();
            } else {
                self.card_mut(cid).clear_global_turn_state();
            }
        }
    }

    /// Tap a card. Returns true if it was untapped.
    /// Runs Tap replacement effects before tapping.
    pub fn tap(&mut self, card_id: CardId) -> bool {
        let card = &self.cards[card_id.index()];
        if card.tapped {
            return false;
        }
        // Run Tap replacement effects.
        let mut event = ReplacementEvent::Tap { card: card_id };
        let result = apply_replacements(self, &mut event);
        if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
            return false; // Tap was prevented
        }
        self.card_mut(card_id).tapped = true;
        self.card_mut(card_id).tapped_this_turn += 1;
        true
    }

    /// Untap a card. Returns true if it was tapped.
    /// Runs Untap replacement effects before untapping.
    pub fn untap(&mut self, card_id: CardId) -> bool {
        self.untap_internal(card_id, None)
    }

    pub fn untap_during_untap_step(&mut self, card_id: CardId, player: PlayerId) -> bool {
        self.untap_internal(card_id, Some(player))
    }

    fn untap_internal(&mut self, card_id: CardId, player: Option<PlayerId>) -> bool {
        let card = &self.cards[card_id.index()];
        if !card.tapped {
            return false;
        }
        let mut event = ReplacementEvent::Untap {
            card: card_id,
            player,
        };
        if crate::replacement::replacement_handler::cant_happen_check(self, &event) {
            return false;
        }
        let card = &self.cards[card_id.index()];
        let stun = CounterType::Named("STUN".to_string());
        if card.counter_count(&stun) > 0 && card.can_remove_counters(&stun) {
            // Stun counters replace the untap event: remove one counter and keep the
            // permanent tapped. This mirrors Java's built-in stun untap replacement.
            self.card_mut(card_id).remove_counter(&stun, 1);
            return false;
        }
        // Run Untap replacement effects.
        let result = apply_replacements(self, &mut event);
        if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
            return false; // Untap was prevented
        }
        self.run_untap_commands(card_id);
        self.card_mut(card_id).tapped = false;
        true
    }

    /// Change the controller of a permanent to `new_controller`.
    /// Mirrors Java's `GameAction.controllerChangeZoneCorrection()` — moves the
    /// card between per-player zone lists and updates the controller field.
    pub fn change_controller(&mut self, card_id: CardId, new_controller: PlayerId) {
        let card = &self.cards[card_id.index()];
        if card.controller == new_controller {
            return;
        }
        let old_controller = card.controller;
        let zone = card.zone;

        // Move between zone lists
        if zone != ZoneType::None {
            self.remove_card_from_zone(zone, old_controller, card_id);
            self.add_card_to_zone(zone, new_controller, card_id);
        }
        let (commands, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.change_controller_commands)
                .into_iter()
                .partition(|(host, _)| *host == card_id);
        self.change_controller_commands = kept;
        for (_, command) in commands {
            command.run(self, &mut crate::game_rng::ThreadRngAdapter::default());
        }
        self.card_mut(card_id).controller = new_controller;
        if zone == ZoneType::Battlefield {
            self.card_mut(card_id).summoning_sick = true;
        }
    }

    fn aura_attach_candidates(&self, aura_id: CardId) -> Vec<GameEntity> {
        let aura = self.card(aura_id);
        let Some(enchant_type) = aura
            .keywords
            .iter_strings()
            .find_map(|kw| crate::keyword::extract_keyword_cost_str(kw, "Enchant"))
        else {
            return Vec::new();
        };
        let normalized = enchant_type
            .split_once(':')
            .map_or(enchant_type, |(kind, _)| kind);
        if normalized.starts_with("Player") || normalized.starts_with("Opponent") {
            return (0..self.players.len())
                .map(|index| PlayerId(index as u32))
                .filter(|&player| crate::player::player_predicates::can_be_attached(self, player))
                .map(GameEntity::Player)
                .collect();
        }
        self.cards_in_all_zones(ZoneType::Battlefield)
            .chain(self.cards_in_all_zones(ZoneType::Graveyard))
            .filter(|&target| {
                target != aura_id
                    && crate::parsing::enchant_type_matches_card(
                        enchant_type,
                        self.card(target),
                        Some(aura),
                    )
                    && crate::card::card_predicates::can_be_attached(self, target, aura_id)
            })
            .map(GameEntity::Card)
            .collect()
    }

    fn attach_aura_on_indirect_etb(
        &mut self,
        agents: &mut [Box<dyn PlayerAgent>],
        aura_id: CardId,
        controller: PlayerId,
    ) {
        let candidates = self.aura_attach_candidates(aura_id);
        if candidates.is_empty() {
            return;
        }
        match agents[controller.index()].choose_single_entity_for_effect(
            controller,
            &candidates,
            false,
        ) {
            Some(GameEntity::Card(target)) => self.attach_to(aura_id, target),
            Some(GameEntity::Player(player)) => self.attach_to_player(aura_id, player),
            None => {}
        }
    }

    /// Attach `aura_id` to `target_id`.
    /// If `aura_id` was already attached elsewhere, detach it first.
    /// Mirrors Java's `Card.enchantEntity()` / `Card.equip()`.
    pub fn attach_to(&mut self, aura_id: CardId, target_id: CardId) {
        // Detach from previous host if any
        self.detach(aura_id);
        self.card_mut(aura_id).attached_to = Some(target_id);
        self.card_mut(aura_id).attached_to_player = None;
        self.card_mut(aura_id).attached_this_turn = true;
        self.card_mut(aura_id).layer_timestamp = self.next_timestamp();
        self.card_mut(target_id).attachments.push(aura_id);
    }

    pub fn attach_to_player(&mut self, aura_id: CardId, player_id: PlayerId) {
        self.detach(aura_id);
        self.card_mut(aura_id).attached_to = None;
        self.card_mut(aura_id).attached_to_player = Some(player_id);
        self.card_mut(aura_id).attached_this_turn = true;
        self.card_mut(aura_id).layer_timestamp = self.next_timestamp();
    }

    /// Detach `aura_id` from whatever it is currently attached to.
    /// Mirrors Java's `Card.unattachFromEntity()`.
    pub fn detach(&mut self, aura_id: CardId) {
        if let Some(host_id) = self.card_mut(aura_id).attached_to.take() {
            self.card_mut(host_id).attachments.retain(|&a| a != aura_id);
            // Bestow: when unattached, revert to a creature
            self.card_mut(aura_id).is_bestowed = false;
        }
        self.card_mut(aura_id).attached_to_player = None;
    }

    /// Move a card from its current zone to the bottom of a player's library.
    /// Unlike `move_card`, this places the card at the bottom rather than the top.
    pub fn put_on_bottom_of_library(&mut self, card_id: CardId, owner: PlayerId) {
        let card = &self.cards[card_id.index()];
        let src_zone = card.zone;
        let src_owner = card.controller;

        if src_zone != ZoneType::None {
            self.remove_card_from_zone(src_zone, src_owner, card_id);
        }

        self.card_mut(card_id).zone = ZoneType::Library;
        self.assign_zone_timestamp(card_id);
        self.add_card_to_zone_bottom(ZoneType::Library, owner, card_id);
    }

    /// Remove a spell from the stack by its entry ID (used by Counter).
    /// Mirrors Java's `Game.getStack().remove(sa)`.
    pub fn remove_from_stack(&mut self, entry_id: u32) -> bool {
        self.stack.remove_by_id(entry_id).is_some()
    }
}

pub fn run_life_lost_all(
    trigger_handler: &mut TriggerHandler,
    life_lost_all_damage_map: &[(PlayerId, i32)],
) {
    for &(player, lost) in life_lost_all_damage_map {
        trigger_handler.run_trigger(
            TriggerType::LifeLostAll,
            RunParams {
                player: Some(player),
                life_amount: Some(lost),
                ..Default::default()
            },
            false,
        );
    }
}

fn can_attachment_remain_attached(
    cards: &[Arc<Card>],
    attachment: &Card,
    target: &Card,
    check_sba: bool,
) -> bool {
    if target.zone != ZoneType::Battlefield {
        return true;
    }
    if attachment.type_line.has_subtype("Equipment") && !target.is_creature() {
        return false;
    }
    if attachment.type_line.has_subtype("Fortification")
        && (!target.is_land() || attachment.is_land())
    {
        return false;
    }
    if crate::staticability::static_ability_cant_attach::cant_attach(
        cards, attachment, target, check_sba,
    ) {
        return false;
    }
    !crate::staticability::static_ability_colorless_damage_source::target_is_protected_from_source(
        cards, target, attachment,
    )
}

pub(crate) struct SbaReplacementParts<'a> {
    pub token_templates: &'a crate::HashMap<String, crate::card::Card>,
    pub token_art_variants: &'a crate::HashMap<(String, String), usize>,
    pub token_fallback: &'a crate::HashMap<String, String>,
    pub edition_dates: &'a crate::HashMap<String, String>,
    pub mana_pools: &'a mut Vec<crate::mana::ManaPool>,
    pub rng: &'a mut dyn crate::game_rng::GameRng,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Card;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost};

    fn make_creature(game: &mut GameState, name: &str, owner: PlayerId, p: i32, t: i32) -> CardId {
        let card = Card::new(
            CardId(0),
            name.to_string(),
            owner,
            CardTypeLine::parse("Creature Bear"),
            ManaCost::parse("1 G"),
            ColorSet::GREEN,
            Some(p),
            Some(t),
            vec![],
            vec![],
        );
        game.create_card(card)
    }

    #[test]
    fn move_card_to_battlefield() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let cid = make_creature(&mut game, "Bear", PlayerId(0), 2, 2);
        game.move_card(cid, ZoneType::Hand, PlayerId(0));
        assert_eq!(game.zone(ZoneType::Hand, PlayerId(0)).len(), 1);

        game.move_card(cid, ZoneType::Battlefield, PlayerId(0));
        assert_eq!(game.zone(ZoneType::Hand, PlayerId(0)).len(), 0);
        assert_eq!(game.zone(ZoneType::Battlefield, PlayerId(0)).len(), 1);
        assert_eq!(game.card(cid).zone, ZoneType::Battlefield);
    }

    #[test]
    fn state_based_actions_lethal_damage() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let cid = make_creature(&mut game, "Bear", PlayerId(0), 2, 2);
        game.move_card(cid, ZoneType::Battlefield, PlayerId(0));

        game.deal_damage_to_card(cid, 2);
        assert!(game.check_state_based_actions());
        assert_eq!(game.zone(ZoneType::Graveyard, PlayerId(0)).len(), 1);
    }

    #[test]
    fn state_based_actions_zero_life() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        game.deal_damage_to_player(PlayerId(0), 20);
        game.lose_life_simultaneously(&mut TriggerHandler::new(), None);
        game.check_state_based_actions();
        assert!(game.player(PlayerId(0)).has_lost);
        assert!(game.game_over);
        assert_eq!(game.winner, Some(PlayerId(1)));
    }

    #[test]
    fn draw_card() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let cid = make_creature(&mut game, "Bear", PlayerId(0), 2, 2);
        game.move_card(cid, ZoneType::Library, PlayerId(0));

        let drawn = game.draw_card(PlayerId(0));
        assert_eq!(drawn, Some(cid));
        assert_eq!(game.card(cid).zone, ZoneType::Hand);
    }

    #[test]
    fn tap_untap() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let cid = make_creature(&mut game, "Bear", PlayerId(0), 2, 2);
        game.move_card(cid, ZoneType::Battlefield, PlayerId(0));

        assert!(game.tap(cid));
        assert!(game.card(cid).tapped);
        assert!(!game.tap(cid)); // already tapped
        assert!(game.untap(cid));
        assert!(!game.card(cid).tapped);
    }

    #[test]
    fn stun_counter_replaces_untap() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let cid = make_creature(&mut game, "Bear", PlayerId(0), 2, 2);
        game.move_card(cid, ZoneType::Battlefield, PlayerId(0));
        game.tap(cid);
        game.card_mut(cid)
            .add_counter(&CounterType::Named("STUN".to_string()), 1);

        assert!(!game.untap(cid));
        assert!(game.card(cid).tapped);
        assert_eq!(
            game.card(cid)
                .counter_count(&CounterType::Named("STUN".to_string())),
            0
        );
    }
}
