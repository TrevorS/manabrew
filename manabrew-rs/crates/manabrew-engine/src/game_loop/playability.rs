use super::*;

use crate::cost::cost_adjustment::apply_cost_reductions;

impl GameLoop {
    pub(super) fn mana_from_cost(cost: &crate::cost::Cost) -> forge_foundation::ManaCost {
        let mut out = forge_foundation::ManaCost::generic(0);
        for part in &cost.parts {
            if let CostPart::Mana { cost: mc, .. } = part {
                out = out.add(mc);
            }
        }
        out
    }

    /// The mana a `Mode$ RaiseCost` adds. A `Waterbend` part counts as generic mana:
    /// `cost_waterbend::can_pay` clears the tap allowance outside AI control, mirroring
    /// Java's `calculateManaCost` in test mode, so the probe has to find real mana for it.
    /// `can_pay_ignoring_mana_for_spell` answers true for it, so nothing else would.
    fn raise_mana_from_cost(
        game: &GameState,
        cost: &crate::cost::Cost,
        source: CardId,
        player: PlayerId,
    ) -> forge_foundation::ManaCost {
        let mut out = Self::mana_from_cost(cost);
        for part in &cost.parts {
            if let CostPart::Waterbend { amount } = part {
                let n = amount.resolve(game, source, player).max(0);
                out = out.add(&forge_foundation::ManaCost::generic(n));
            }
        }
        out
    }

    fn can_use_source_level_mana_fallback(
        game: &GameState,
        player: PlayerId,
        available_mana: &crate::mana::mana_pool::ManaPool,
    ) -> bool {
        if game.action_space_mana_probe == crate::mana::ActionSpaceManaProbe::ComputerUtilMana {
            return false;
        }
        let has_all_color_source = available_mana
            .source_colors
            .as_ref()
            .is_some_and(|sources| {
                sources.iter().any(|&source| {
                    (source & forge_foundation::mana::ManaAtom::COLORS_SUPERPOSITION)
                        == forge_foundation::mana::ManaAtom::COLORS_SUPERPOSITION
                })
            });
        has_all_color_source || crate::mana::has_replacement_adjusted_available_mana(game, player)
    }

    pub(crate) fn card_trace_enabled() -> bool {
        Self::card_trace_filter().is_some()
    }

    fn card_trace_filter() -> Option<&'static str> {
        static FILTER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        FILTER
            .get_or_init(|| {
                std::env::var("FORGE_CARD_TRACE")
                    .ok()
                    .filter(|f| !f.is_empty())
            })
            .as_deref()
    }

    pub(crate) fn card_trace_matches(name: &str) -> bool {
        Self::card_trace_filter().is_some_and(|filter| name.eq_ignore_ascii_case(filter))
    }

    pub(crate) fn stack_trace_enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("FORGE_STACK_TRACE").is_ok())
    }

    pub(crate) fn trigger_trace_enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("FORGE_TRIGGER_TRACE").is_ok())
    }

    pub(crate) fn payment_trace_enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("FORGE_PAYMENT_TRACE").is_ok())
    }

    pub(crate) fn zone_trace_enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("FORGE_ZONE_TRACE").is_ok())
    }

    /// Mana a spell cast of this card could draw on: `RestrictValid$` sources that the
    /// spell does not satisfy are left out, as `AbilityManaPart.meetsManaRestrictions` does.
    fn spell_payment_context(
        card: &crate::card::Card,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> mana::ManaPaymentContext {
        mana::ManaPaymentContext {
            is_spell: true,
            is_activated_ability: false,
            sa_on_stack: false,
            type_line: Some(card.type_line.clone()),
            card_name: Some(card.card_name.clone()),
            card_color: Some(card.color),
            chosen_types_by_source: chosen_types_by_source.clone(),
            ..Default::default()
        }
    }

    pub(super) fn available_mana_for_spell_card(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> crate::mana::ManaPool {
        let payment_ctx = Self::spell_payment_context(game.card(card_id), chosen_types_by_source);
        mana::calculate_available_mana_with_context(
            self.pool(player),
            game,
            player,
            Some(card_id),
            &[],
            Some(&payment_ctx),
        )
    }

    fn can_pay_graveyard_spell_mana(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        mana_cost: &forge_foundation::ManaCost,
        available_mana: &crate::mana::ManaPool,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let card = game.card(card_id);
        let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
            game,
            card,
            player,
            ZoneType::Graveyard,
        );
        let raise_mana = crate::cost::cost_adjustment::compute_raise_cost_parts(
            game,
            card,
            player,
            ZoneType::Graveyard,
        )
        .as_ref()
        .map(|rc| Self::raise_mana_from_cost(game, rc, card_id, player))
        .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));
        let base = cost_adj.apply(&mana_cost.without_x()).add(&raise_mana);
        let payable = crate::mana::apply_player_life_payment_keywords(game, player, &base);
        let reduced = apply_cost_reductions(game, player, card_id, card, &payable);
        crate::mana::can_pay_spell_mana_cost_for_action_space(
            game,
            self.pool(player),
            player,
            card_id,
            &reduced,
            &Self::spell_payment_context(card, chosen_types_by_source),
        ) || (Self::can_use_source_level_mana_fallback(game, player, available_mana)
            && available_mana.can_pay(&reduced))
    }

    fn can_pay_face_down_cast(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        alt_cost: Option<&crate::cost::Cost>,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let mut host = game.card(card_id).clone();
        host.turn_face_down_no_update();
        host.set_original_state_as_face_down();
        let zone = host.zone;
        let raise_cost =
            crate::cost::cost_adjustment::compute_raise_cost_parts(game, &host, player, zone);
        let raise_mana = raise_cost
            .as_ref()
            .map(|rc| Self::raise_mana_from_cost(game, rc, card_id, player))
            .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));
        let mana = crate::cost::cost_adjustment::compute_cost_adjustment_for_payment(
            game,
            &host,
            player,
            zone,
            &[],
            &[],
            true,
        )
        .apply(&alt_cost.map_or_else(
            || forge_foundation::ManaCost::generic(crate::spellability::MORPH_GENERIC_COST),
            Self::mana_from_cost,
        ))
        .add(&raise_mana);
        let payment_ctx = mana::ManaPaymentContext {
            is_cast_face_down: true,
            ..Self::spell_payment_context(&host, chosen_types_by_source)
        };
        let mana_ok = crate::mana::can_pay_spell_mana_cost_for_action_space(
            game,
            self.pool(player),
            player,
            card_id,
            &mana,
            &payment_ctx,
        ) || {
            let available = mana::calculate_available_mana_with_context(
                self.pool(player),
                game,
                player,
                Some(card_id),
                &[],
                Some(&payment_ctx),
            );
            Self::can_use_source_level_mana_fallback(game, player, &available)
                && available.can_pay(&mana)
        };
        mana_ok
            && alt_cost.is_none_or(|cost| {
                crate::cost::can_pay_ignoring_mana_for_spell(cost, game, card_id, player)
            })
            && raise_cost.as_ref().is_none_or(|rc| {
                crate::cost::can_pay_ignoring_mana_for_spell(rc, game, card_id, player)
            })
    }

    fn can_cast_may_play_spell(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        zone: ZoneType,
        alt_cost: Option<String>,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let card = game.card(card_id);
        let mut cast_sa =
            crate::spellability::build_spell_ability_for_card_cast(game, card_id, player);
        cast_sa.restriction.variables.set_zone(zone);
        if crate::staticability::static_ability_cant_be_cast::cant_be_cast_ability_from_zone(
            &game.cards,
            &cast_sa,
            card,
            player,
            game,
        ) || !crate::spellability::spell::can_play(&cast_sa, game)
        {
            return false;
        }
        if let Some(ref tr) = cast_sa.target_restrictions {
            if tr.get_min_targets(game, &cast_sa) > 0
                && !target_restrictions::has_candidates_in_spell_ability_chain(
                    game, player, &cast_sa,
                )
            {
                return false;
            }
        }
        let available_mana =
            self.available_mana_for_spell_card(game, player, card_id, chosen_types_by_source);
        let cost_adj =
            crate::cost::cost_adjustment::compute_cost_adjustment(game, card, player, zone);
        let alt_cost = alt_cost.as_deref().map(crate::cost::parse_cost);
        let base_cost = alt_cost
            .as_ref()
            .map(Self::mana_from_cost)
            .unwrap_or_else(|| card.mana_cost.clone());
        let raise_cost =
            crate::cost::cost_adjustment::compute_raise_cost_parts(game, card, player, zone);
        let raise_mana = raise_cost
            .as_ref()
            .map(|rc| Self::raise_mana_from_cost(game, rc, card_id, player))
            .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));
        available_mana.can_pay(&cost_adj.apply(&base_cost).add(&raise_mana))
            && alt_cost.as_ref().is_none_or(|cost| {
                crate::cost::can_pay_ignoring_mana_for_spell(cost, game, card_id, player)
            })
            && raise_cost.as_ref().is_none_or(|cost| {
                crate::cost::can_pay_ignoring_mana_for_spell(cost, game, card_id, player)
            })
    }

    fn can_play_card_state_spell(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        state_name: forge_foundation::CardStateName,
        zone: ZoneType,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let Some((host, sa)) = crate::spellability::build_spell_ability_for_card_state_cast(
            game, card_id, player, state_name,
        ) else {
            return false;
        };
        self.can_play_state_spell(
            game,
            player,
            card_id,
            host,
            sa,
            zone,
            chosen_types_by_source,
        )
    }

    pub(super) fn secondary_flashback_costs(game: &GameState, card_id: CardId) -> Vec<String> {
        let card = game.card(card_id);
        if card.is_transformed
            || card
                .other_part
                .as_ref()
                .is_none_or(|other| other.state_name != forge_foundation::CardStateName::Secondary)
            || !game.cards.iter().any(|source| {
                source.zone.is_static_ability_source()
                    && source.static_abilities.iter().any(|st| {
                        st.ir
                            .add_keyword_text
                            .as_deref()
                            .is_some_and(|keywords| keywords.contains("Flashback"))
                    })
            })
        {
            return Vec::new();
        }
        let mut host = card.clone();
        host.transform();
        crate::spellability::spell::alternate_host_with_statics(game, host)
            .get_all_flashback_costs()
    }

    pub(super) fn flashback_costs(game: &GameState, card_id: CardId) -> Vec<String> {
        let mut costs = game.card(card_id).get_all_flashback_costs();
        costs.extend(Self::secondary_flashback_costs(game, card_id));
        costs
    }

    fn can_play_secondary_flashback(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        flashback_cost: &str,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let Some((host, mut sa)) = crate::spellability::build_spell_ability_for_card_state_cast(
            game,
            card_id,
            player,
            forge_foundation::CardStateName::Secondary,
        ) else {
            return false;
        };
        let Some(mut cost) = sa
            .pay_costs
            .as_ref()
            .map(crate::cost::Cost::copy_with_no_mana)
        else {
            return false;
        };
        cost.parts
            .extend(crate::cost::parse_cost(flashback_cost).parts);
        cost.sort();
        sa.pay_costs = Some(cost);
        sa.alt_cost = Some(crate::spellability::AlternativeCost::Flashback);
        self.can_play_state_spell(
            game,
            player,
            card_id,
            host,
            sa,
            ZoneType::Graveyard,
            chosen_types_by_source,
        )
    }

    fn can_play_state_spell(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        host: crate::card::Card,
        mut sa: SpellAbility,
        zone: ZoneType,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        sa.restriction.variables.set_zone(zone);
        if crate::staticability::static_ability_cant_be_cast::cant_be_cast_ability_from_zone(
            &game.cards,
            &sa,
            &host,
            player,
            game,
        ) || !crate::spellability::spell::can_play(&sa, game)
            || !target_restrictions::has_candidates_in_spell_ability_chain(game, player, &sa)
            || sa.target_restrictions.as_ref().is_some_and(|tr| {
                !matches!(
                    tr.target_kind,
                    crate::spellability::TargetKind::Player
                        | crate::spellability::TargetKind::Any
                        | crate::spellability::TargetKind::Spell
                ) && tr.get_min_targets(game, &sa)
                    > crate::card::card_util::get_valid_cards_to_target(game, &sa).len() as i32
            })
        {
            return false;
        }
        let Some(cost) = sa.pay_costs.as_ref() else {
            return false;
        };
        if !crate::cost::can_pay_ignoring_mana_for_spell(cost, game, card_id, player) {
            return false;
        }
        // Forge's probe reads `sa.getHostCard()`, the card's current face, which in hand is
        // still the front one: `CostAdjustment.checkRequirement` tests a static's `ValidCard$`
        // against it and `AbilityManaPart.meetsManaRestrictions` a `Spell.<Type>` restriction.
        // The payment itself runs with the card on the stack as this face.
        let in_hand = game.card(card_id);
        let cost_adj =
            crate::cost::cost_adjustment::compute_cost_adjustment(game, in_hand, player, zone);
        let raise_mana =
            crate::cost::cost_adjustment::compute_raise_cost_parts(game, in_hand, player, zone)
                .as_ref()
                .map(|rc| Self::raise_mana_from_cost(game, rc, card_id, player))
                .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));
        let base = cost_adj
            .apply(&Self::mana_from_cost(cost).without_x())
            .add(&raise_mana);
        let payable = crate::mana::apply_player_life_payment_keywords(game, player, &base);
        let reduced = apply_cost_reductions(game, player, card_id, in_hand, &payable);
        let payment_ctx = mana::ManaPaymentContext {
            is_spell: true,
            is_activated_ability: false,
            sa_on_stack: false,
            type_line: Some(in_hand.type_line.clone()),
            card_name: Some(in_hand.card_name.clone()),
            card_color: Some(in_hand.color),
            chosen_types_by_source: chosen_types_by_source.clone(),
            ..Default::default()
        };
        crate::mana::can_pay_spell_mana_cost_for_action_space(
            game,
            self.pool(player),
            player,
            card_id,
            &reduced,
            &payment_ctx,
        )
    }

    fn apply_stack_statics(game: &GameState, spell_hosts: &[CardId]) -> Option<GameState> {
        if spell_hosts.is_empty()
            || !game.has_static_ability_affecting_zone(
                ZoneType::Stack,
                crate::staticability::Layer::Ability,
            )
        {
            return None;
        }
        let mut pre = game.clone();
        for &card_id in spell_hosts {
            let host = pre.card_mut(card_id);
            host.cast_from = Some(host.zone);
            host.zone = ZoneType::Stack;
        }
        crate::staticability::layer::apply_continuous_effects(&mut pre);
        Some(pre)
    }

    pub(super) fn stack_copy(game: &GameState, mut host: crate::card::Card) -> crate::card::Card {
        if !game.has_static_ability_affecting_zone(
            ZoneType::Stack,
            crate::staticability::Layer::Ability,
        ) {
            return host;
        }
        host.cast_from = Some(host.zone);
        host.zone = ZoneType::Stack;
        crate::spellability::spell::alternate_host_with_statics(game, host)
    }

    fn can_play_backside_web_slinging(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> bool {
        let Some((host, mut sa)) = crate::spellability::build_spell_ability_for_card_state_cast(
            game,
            card_id,
            player,
            forge_foundation::CardStateName::Backside,
        ) else {
            return false;
        };
        let Some(web_cost) = Self::stack_copy(game, host.clone()).get_web_slinging_cost() else {
            return false;
        };
        sa.pay_costs = Some(crate::cost::parse_cost(&format!(
            "{web_cost} Return<1/Creature.tapped/tapped creature>"
        )));
        sa.alt_cost = Some(crate::spellability::AlternativeCost::WebSlinging);
        self.can_play_state_spell(
            game,
            player,
            card_id,
            host,
            sa,
            ZoneType::Hand,
            chosen_types_by_source,
        )
    }

    fn may_play_secondary_spell_grants(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        zone: ZoneType,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> Vec<u8> {
        let card = game.card(card_id);
        if card.face_down {
            return Vec::new();
        }
        let state_name = if card.is_modal() {
            forge_foundation::CardStateName::Backside
        } else {
            forge_foundation::CardStateName::Secondary
        };
        let Some((host, sa)) = crate::spellability::build_spell_ability_for_card_state_cast(
            game, card_id, player, state_name,
        ) else {
            return Vec::new();
        };
        let mut alt_costs = 0u8;
        let mut options = Vec::new();
        for (source, st_ab) in
            crate::staticability::static_ability_continuous::may_play_grants(game, player, &host)
        {
            let alt_cost = crate::staticability::static_ability_continuous::may_play_alt_mana_cost(
                st_ab, source, &host, game,
            );
            if alt_cost.is_some() {
                alt_costs += 1;
            }
            if !crate::staticability::static_ability_continuous::grants_zone_permissions_for(
                st_ab, source, &host, game, &sa,
            ) {
                continue;
            }
            let mut option_sa = sa.clone();
            if let Some(ref cost) = alt_cost {
                option_sa.pay_costs = Some(crate::cost::parse_cost(cost));
            }
            if self.can_play_state_spell(
                game,
                player,
                card_id,
                host.clone(),
                option_sa,
                zone,
                chosen_types_by_source,
            ) {
                options.push(if alt_cost.is_some() { alt_costs } else { 0 });
            }
        }
        options
    }

    /// Get cards the active player can play.
    pub(crate) fn get_playable_cards(
        &self,
        game: &GameState,
        player: PlayerId,
        must_be_instant: bool,
    ) -> Vec<crate::agent::PlayOption> {
        let mut playable = Vec::new();
        let hand = game.cards_in_zone(ZoneType::Hand, player);
        let has_flash_permission = |card_id: CardId| {
            let card = game.card(card_id);
            card.type_line.is_instant()
                || card.has_keyword("Flash")
                || card.get_offering_type().is_some()
                || card.get_keyword_cost("MayFlashCost").is_some_and(|cost| {
                    crate::cost::can_pay_ignoring_mana_for_spell(
                        &crate::cost::parse_cost(&cost),
                        game,
                        card_id,
                        player,
                    )
                })
                || crate::staticability::static_ability_cast_with_flash::any_with_flash_for_card(
                    game, card, player,
                )
                || crate::staticability::static_ability_continuous::may_play_with_flash(
                    game, player, card,
                )
        };
        let can_may_play_from_static = |card_id: CardId| {
            let card = game.card(card_id);
            crate::staticability::static_ability_continuous::may_play_grants(game, player, card)
                .any(|(source, sa)| {
                    crate::staticability::static_ability_continuous::grants_zone_permissions(
                        sa, source, card, game,
                    )
                })
        };
        let accepting_may_play_grants = |card_id: CardId, sa: &SpellAbility| -> usize {
            let card = game.card(card_id);
            crate::staticability::static_ability_continuous::may_play_grants(game, player, card)
                .filter(|(source, st_ab)| {
                    crate::staticability::static_ability_continuous::grants_zone_permissions_for(
                        st_ab, source, card, game, sa,
                    )
                })
                .count()
        };
        let sneak_window = |card: &Card| {
            game.turn.phase == forge_foundation::PhaseType::CombatDeclareBlockers
                && card.get_sneak_cost().is_some()
                && self
                    .combat
                    .get_unblocked_attackers()
                    .iter()
                    .any(|&attacker| {
                        let attacker = game.card(attacker);
                        attacker.zone == ZoneType::Battlefield && attacker.controller == player
                    })
        };
        // First MayPlay alt-cost (e.g. Airbend's `MayPlayAltManaCost$ 2`)
        // granted to `card_id`. Returns the cost string if any.
        let may_play_alt_cost = |card_id: CardId| -> Option<String> {
            let card = game.card(card_id);
            crate::staticability::static_ability_continuous::may_play_grants(game, player, card)
                .find_map(|(source, sa)| {
                    crate::staticability::static_ability_continuous::may_play_alt_mana_cost(
                        sa, source, card, game,
                    )
                    .filter(|cost| {
                        crate::staticability::static_ability_continuous::is_mana_alt_cost(cost)
                    })
                })
        };
        // Count distinct MayPlay statics that grant permission to cast
        // `card_id`. Java's `GameActionUtil.getMayPlaySpellOptions` enumerates
        // one alternative SA per `CardPlayOption` returned by
        // `source.mayPlay(activator)`, so the same exiled card can produce
        // multiple play options when several statics grant permission (e.g.
        // multiple airbend Effects each remembering it).
        let count_may_play_grants = |card_id: CardId| -> usize {
            let card = game.card(card_id);
            crate::staticability::static_ability_continuous::may_play_grants(game, player, card)
                .filter(|(source, sa)| {
                    crate::staticability::static_ability_continuous::can_play_or_granted(
                        sa, source, card, game,
                    )
                })
                .count()
        };
        let chosen_types_by_source: crate::HashMap<CardId, String> = game
            .cards
            .iter()
            .filter_map(|c| c.chosen_type.clone().map(|chosen| (c.id, chosen)))
            .collect();
        let stack_statics = Self::apply_stack_statics(game, hand);

        for &card_id in hand {
            let card = game.card(card_id);
            let probe_host = stack_statics
                .as_ref()
                .map_or(card, |stack_statics| stack_statics.card(card_id));
            if self.can_play_card_state_spell(
                game,
                player,
                card_id,
                forge_foundation::CardStateName::Secondary,
                ZoneType::Hand,
                &chosen_types_by_source,
            ) {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Secondary,
                    alt_cost_index: 0,
                });
            }
            // Java's `Card.collectSpellAbilities`: `isModal() && hasState(Backside)` adds every
            // spell/land ability of the back face unconditionally — a Modal DFC's back face is
            // castable from hand exactly like the front, not just as a land (`BackFaceLand`
            // above covers the land case; this is the spell one, e.g. Peter Parker //
            // Amazing Spider-Man). `PlayCardMode::Secondary` doubles for this: a card only ever
            // has a `Secondary` state (Adventure/Omen) or a modal `Backside`, never both, so
            // `cast_spell.rs` reads the state to build from off the card itself.
            if card.is_modal()
                && self.can_play_card_state_spell(
                    game,
                    player,
                    card_id,
                    forge_foundation::CardStateName::Backside,
                    ZoneType::Hand,
                    &chosen_types_by_source,
                )
            {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Secondary,
                    alt_cost_index: 0,
                });
            }
            if card.is_modal()
                && self.can_play_backside_web_slinging(
                    game,
                    player,
                    card_id,
                    &chosen_types_by_source,
                )
            {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::WebSlinging,
                    ),
                    alt_cost_index: 1,
                });
            }
            // A split card that is not a Room offers its right half as a spell of its own,
            // as Java builds one Spell per CardState. A Room's right half is the same
            // permanent behind a second door, so it keeps the cost-swap path below.
            if !card.type_line.has_subtype("Room")
                && self.can_play_card_state_spell(
                    game,
                    player,
                    card_id,
                    forge_foundation::CardStateName::RightSplit,
                    ZoneType::Hand,
                    &chosen_types_by_source,
                )
            {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::RoomRightSplit,
                    alt_cost_index: 0,
                });
            }
            if card.is_land() {
                if crate::staticability::static_ability_cant_be_cast::cant_play_land_ability(
                    &game.cards,
                    card,
                    player,
                ) {
                    continue;
                }
                let land_sa = SpellAbility::new_land(Some(card_id), player);
                if !must_be_instant && crate::spellability::land_ability::can_play(&land_sa, game) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Normal,
                        alt_cost_index: 0,
                    });
                    if card
                        .other_part
                        .as_ref()
                        .is_some_and(|other| other.is_modal && other.type_line.is_land())
                    {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::BackFaceLand,
                            alt_cost_index: 0,
                        });
                    }
                }
                // Java `Card.getAllPossibleAbilities` walks the card's spell abilities
                // whatever its types are, so a land with Disguise is castable face down
                // as well as playable as a land.
                if card.has_morph
                    && !must_be_instant
                    && self.can_pay_face_down_cast(
                        game,
                        player,
                        card_id,
                        None,
                        &chosen_types_by_source,
                    )
                {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Alternative(
                            crate::spellability::AlternativeCost::Morph,
                        ),
                        alt_cost_index: 0,
                    });
                }
            } else {
                // MDFC: emit both the back-face LAND and the front-face SPELL
                // (Java `getPossibleActions`). Only *modal* backs are playable;
                // a transform back land (e.g. Search for Azcanta // Azcanta) is
                // not — mirror `Card.hasPlayableLandFace` (gated on isModal).
                if card
                    .other_part
                    .as_ref()
                    .is_some_and(|other| other.is_modal && other.type_line.is_land())
                {
                    let cant_play_land =
                        crate::staticability::static_ability_cant_be_cast::cant_play_land_ability(
                            &game.cards,
                            card,
                            player,
                        );
                    if !cant_play_land {
                        let land_sa = SpellAbility::new_land(Some(card_id), player);
                        if !must_be_instant
                            && crate::spellability::land_ability::can_play(&land_sa, game)
                        {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::BackFaceLand,
                                alt_cost_index: 0,
                            });
                        }
                    }
                }
                let sneak_window = sneak_window(card);
                let normal_timing = !must_be_instant || has_flash_permission(card_id);
                // Java `CardFactoryUtil:2961` gives foretell no sorcery-speed restriction, so a
                // sorcery can still be foretold in a step where it could not be cast.
                let foretell_window =
                    card.get_foretell_cost().is_some() && game.turn.active_player == player;
                if !normal_timing && !sneak_window && !foretell_window {
                    continue;
                }

                let cast_sa =
                    crate::spellability::build_spell_ability_for_card_cast(game, card_id, player);
                if crate::staticability::static_ability_cant_be_cast::cant_be_cast_ability_from_zone(
                    &game.cards,
                    &cast_sa,
                    card,
                    player,
                    game,
                ) {
                    continue;
                }

                // Spell-level checks: not on battlefield, no split second
                let timing_ok = if normal_timing && card.get_keyword_cost("MayFlashCost").is_none()
                {
                    crate::spellability::spell::can_play(&cast_sa, game)
                } else {
                    let mut sneak_sa = cast_sa.clone();
                    sneak_sa.restriction.variables.set_instant_speed(true);
                    crate::spellability::spell::can_play(&sneak_sa, game)
                };
                if !timing_ok {
                    continue;
                }

                // NonStackingEffect is an AI hint in Java (AiController), not a game rule.
                // Do NOT filter here — let the agent decide whether to cast duplicates.

                if let Some(ref tr) = cast_sa.target_restrictions {
                    let min_targets = tr.get_min_targets(game, &cast_sa);
                    if min_targets > 0
                        && !target_restrictions::has_candidates_in_spell_ability_chain(
                            game, player, &cast_sa,
                        )
                    {
                        continue;
                    }
                }

                // Check if we can pay the mana cost (normal or alternative).
                // Mirror Java's per-spell restriction filtering: mana sources
                // with RestrictValid$ that don't match the spell being cast
                // must not count toward availability.
                let payment_ctx = mana::ManaPaymentContext {
                    is_spell: true,
                    is_activated_ability: false,
                    sa_on_stack: false,
                    type_line: Some(card.type_line.clone()),
                    card_name: Some(card.card_name.clone()),
                    card_color: Some(card.color),
                    chosen_types_by_source: chosen_types_by_source.clone(),
                    ..Default::default()
                };
                let available_mana_cell = std::cell::OnceCell::new();
                let available_mana = || {
                    available_mana_cell.get_or_init(|| {
                        mana::calculate_available_mana_with_context(
                            self.pool(player),
                            game,
                            player,
                            Some(card_id),
                            &[],
                            Some(&payment_ctx),
                        )
                    })
                };

                // Apply cost reduction/increase from static abilities
                let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                    game,
                    card,
                    player,
                    ZoneType::Hand,
                );
                let raise_cost = crate::cost::cost_adjustment::compute_raise_cost_parts(
                    game,
                    card,
                    player,
                    ZoneType::Hand,
                );
                let raise_mana = raise_cost
                    .as_ref()
                    .map(|rc| Self::raise_mana_from_cost(game, rc, card_id, player))
                    .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));

                // Check mana conversion for playability
                let any_color =
                    crate::staticability::static_ability_mana_convert::can_spend_mana_as_any_color(
                        &game.cards,
                        player,
                        card,
                    );

                // Check normal cost OR any alternative costs
                // For X-cost spells, check only the non-X portion (X=0 is valid)
                // Delve: reduce generic cost by number of graveyard cards
                // Convoke: reduce total cost by number of untapped creatures
                let normal_ok = {
                    let base = if card.mana_cost.count_x() > 0 {
                        cost_adj.apply(&card.mana_cost.without_x())
                    } else {
                        cost_adj.apply(&card.mana_cost)
                    };
                    let base = base.add(&raise_mana);
                    let payable_base =
                        crate::mana::apply_player_life_payment_keywords(game, player, &base);
                    // Phyrexian mana: check AIPhyrexianPayment to determine
                    // if life payment is allowed for this card. Uses greedy
                    // simulation matching Java's ComputerUtilMana behavior.
                    let has_phyrexian = payable_base.shards().iter().any(|s| s.is_phyrexian());
                    if has_phyrexian {
                        let phyrexian_life_allowed = match card.ai_phyrexian_payment.as_deref() {
                            Some("Never") => false,
                            Some(s) if s.starts_with("OnFatalDamage.") => {
                                let dmg: i32 = s[14..].parse().unwrap_or(0);
                                let opp = game.opponent_of(player);
                                game.player(opp).life <= dmg
                            }
                            _ => true,
                        };
                        if phyrexian_life_allowed {
                            crate::mana::can_pay_spell_mana_cost_for_action_space(
                                game,
                                self.pool(player),
                                player,
                                card_id,
                                &payable_base,
                                &payment_ctx,
                            )
                        } else {
                            let colored = payable_base.phyrexian_to_colored();
                            let reduced =
                                apply_cost_reductions(game, player, card_id, probe_host, &colored);
                            if any_color {
                                available_mana().can_pay_any_color(&reduced)
                            } else {
                                available_mana().can_pay(&reduced)
                            }
                        }
                    } else {
                        let reduced =
                            apply_cost_reductions(game, player, card_id, probe_host, &payable_base);
                        if any_color {
                            available_mana().can_pay_any_color(&reduced)
                        } else {
                            crate::mana::can_pay_spell_mana_cost_for_action_space(
                                game,
                                self.pool(player),
                                player,
                                card_id,
                                &reduced,
                                &payment_ctx,
                            )
                            // The incremental simulator mirrors payment choice order.
                            // If it misses an availability-only source (e.g.
                            // Any-color/static land-type or replacement-expanded
                            // mana), fall back to the source-level mask used by
                            // Java's action-space feasibility check.
                            || (Self::can_use_source_level_mana_fallback(
                                game,
                                player,
                                available_mana(),
                            ) && available_mana().can_pay(&reduced))
                        }
                    }
                };
                let room_right_split_ok = card.type_line.has_subtype("Room")
                    && card.svars.get("RoomRightSplitCost").is_some_and(|cost| {
                        let cost = forge_foundation::ManaCost::parse(cost);
                        let adjusted = cost_adj.apply(&cost).add(&raise_mana);
                        if any_color {
                            available_mana().can_pay_any_color(&adjusted)
                        } else {
                            available_mana().can_pay(&adjusted)
                        }
                    });

                // Spectacle: alt cost if opponent lost life this turn
                let spectacle_ok = if let Some(spec_cost_str) = card.get_spectacle_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&spec_cost_str))
                        .add(&raise_mana);
                    game.player_opponents_lost_life_this_turn(player)
                        && available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                // Evoke: alt cost for creatures. A card may have multiple Evoke
                // costs simultaneously — e.g. Mulldrifter (intrinsic Evoke {2}{U})
                // in P0's hand while Ashling, the Limitless grants Evoke {4} via
                // its `AddKeyword$ Evoke:4` static. Each is a separate alternative
                // cost in MTG, so enumerate them as separate playable entries to
                // match Java's count.
                // Keep the ORIGINAL index (position in `get_all_evoke_costs()`)
                // alongside the cost string, so each payable Evoke can be tied
                // to the exact keyword instance cast_spell uses at payment time.
                let evoke_payable: Vec<(usize, String)> = card
                    .get_all_evoke_costs()
                    .into_iter()
                    .enumerate()
                    .filter(|(_, cost_str)| {
                        let evoke_cost = crate::cost::parse_cost(cost_str);
                        let evoke_mana = Self::mana_from_cost(&evoke_cost);
                        let adjusted = cost_adj.apply(&evoke_mana).add(&raise_mana);
                        available_mana().can_pay(&adjusted)
                            && crate::cost::can_pay_ignoring_mana_for_spell(
                                &evoke_cost,
                                game,
                                card_id,
                                player,
                            )
                    })
                    .collect();
                let evoke_ok = !evoke_payable.is_empty();

                // Dash: alt cost
                let dash_ok = if let Some(dash_cost_str) = card.get_dash_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&dash_cost_str))
                        .add(&raise_mana);
                    available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                // Blitz: alt cost
                let blitz_ok = if let Some(blitz_cost_str) = card.get_blitz_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&blitz_cost_str))
                        .add(&raise_mana);
                    available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                // Web-slinging: alt cost, and the Return part needs a tapped creature
                let web_slinging_ok =
                    probe_host
                        .get_web_slinging_cost()
                        .is_some_and(|web_cost_str| {
                            let adjusted = cost_adj
                                .apply(&forge_foundation::ManaCost::parse(&web_cost_str))
                                .add(&raise_mana);
                            available_mana().can_pay(&adjusted)
                                && game
                                    .cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
                                    .iter()
                                    .any(|id| game.card(*id).tapped && game.card(*id).is_creature())
                        });

                let sneak_ok = sneak_window
                    && card.get_sneak_cost().is_some_and(|sneak_cost_str| {
                        let adjusted = cost_adj
                            .apply(&forge_foundation::ManaCost::parse(&sneak_cost_str))
                            .add(&raise_mana);
                        available_mana().can_pay(&adjusted)
                    });
                if !normal_timing {
                    if sneak_ok {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::Alternative(
                                crate::spellability::AlternativeCost::Sneak,
                            ),
                            alt_cost_index: 0,
                        });
                    }
                    if foretell_window
                        && available_mana().can_pay(&forge_foundation::ManaCost::generic(2))
                    {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::ForetellExile,
                            alt_cost_index: 0,
                        });
                    }
                    continue;
                }

                // Overload: alt cost
                let overload_ok = if let Some(ovl_cost_str) = card.get_overload_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&ovl_cost_str))
                        .add(&raise_mana);
                    available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                // StaticAbilityAlternativeCost (Mode$ AlternativeCost)
                let static_alt_indices: Vec<usize> =
                    crate::staticability::static_ability_alternative_cost::alternative_costs(
                        game,
                        &game.cards,
                        &cast_sa,
                        card,
                        player,
                    )
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| {
                        let base = Self::mana_from_cost(&entry.cost);
                        let adjusted = cost_adj.apply(&base).add(&raise_mana);
                        available_mana().can_pay(&adjusted)
                            && crate::cost::can_pay_ignoring_mana_for_spell(
                                &entry.cost,
                                game,
                                card_id,
                                player,
                            )
                    })
                    .map(|(index, _)| index)
                    .collect();
                let static_alt_ok = !static_alt_indices.is_empty();

                // Suspend: special action, pay suspend cost to exile with time counters
                // (Suspend is not a spell cast — cost reduction doesn't apply)
                let suspend_ok = if let Some((suspend_cost_str, _counters)) =
                    card.get_suspend_cost()
                {
                    available_mana().can_pay(&forge_foundation::ManaCost::parse(&suspend_cost_str))
                } else {
                    false
                };

                // Foretell: pay {2} to exile face-down from hand
                // (This is a special action, not a cast — always costs {2})
                let foretell_exile_ok = if card.get_foretell_cost().is_some() {
                    available_mana().can_pay(&forge_foundation::ManaCost::generic(2))
                } else {
                    false
                };

                // Emerge: alt cost minus sacrificed creature's mana value
                let emerge_ok = if let Some(emerge_cost_str) = card.get_emerge_cost() {
                    // Simplified: check if emerge base cost is affordable
                    // (actual cost reduction from sac'd creature computed at cast time)
                    let adjusted =
                        cost_adj.apply(&forge_foundation::ManaCost::parse(&emerge_cost_str));
                    available_mana().can_pay(&adjusted) || {
                        // Even if base emerge cost isn't payable, if we have creatures to sac
                        // the reduction might make it payable — approximate check
                        !game
                            .cards_in_zone(ZoneType::Battlefield, player)
                            .iter()
                            .filter(|&&cid| game.card(cid).is_creature())
                            .collect::<Vec<_>>()
                            .is_empty()
                    }
                } else {
                    false
                };

                // Spree: Java's `CostAdjustment` only folds `ModeCost$` into
                // the spell's cost AFTER modes are chosen, so playability
                // gates only the base cost. Mode affordability is rechecked
                // at cast time. Mirror that here — gating on
                // `base + cheapest mode` here makes Rust drop Spree spells
                // from the action space that Java still offers.

                // Offering: sacrifice a permanent of a type to reduce cost
                let offering_ok = if let Some(offering_type) = card.get_offering_type() {
                    let offering_type_lower = offering_type.to_lowercase();
                    // Check if we have a permanent of the right type to sacrifice
                    game.cards_in_zone(ZoneType::Battlefield, player)
                        .iter()
                        .any(|&cid| {
                            cid != card_id && {
                                let c = game.card(cid);
                                match offering_type_lower.as_str() {
                                    "creature" => c.is_creature(),
                                    "artifact" => c.type_line.is_artifact(),
                                    "enchantment" => c.type_line.is_enchantment(),
                                    "land" => c.type_line.is_land(),
                                    _ => c.type_line.has_subtype(&offering_type),
                                }
                            }
                        })
                } else {
                    false
                };

                // Morph: can cast any Morph card face-down for the morph generic cost
                let morph_ok = card.has_morph
                    && self.can_pay_face_down_cast(
                        game,
                        player,
                        card_id,
                        None,
                        &chosen_types_by_source,
                    );

                // Bestow: cast as an Aura for bestow cost.
                // Requires a valid creature target on the battlefield (Aura targeting).
                let bestow_ok = if let Some(bestow_cost_str) = card.get_bestow_cost() {
                    let adjusted =
                        cost_adj.apply(&forge_foundation::ManaCost::parse(&bestow_cost_str));
                    let can_afford = available_mana().can_pay(&adjusted);
                    // Bestow turns the creature into an Aura targeting a creature.
                    // Only offer bestow if at least one creature exists to enchant.
                    let has_creature_target = can_afford
                        && game
                            .cards
                            .iter()
                            .any(|c| c.zone == ZoneType::Battlefield && c.is_creature());
                    has_creature_target
                } else {
                    false
                };

                // Warp: alt cost for creatures
                let warp_ok = if let Some(warp_cost_str) = card.get_warp_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&warp_cost_str))
                        .add(&raise_mana);
                    available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                let impending_ok = if let Some((impending_cost, _)) = card.get_impending_cost() {
                    let adjusted = cost_adj
                        .apply(&forge_foundation::ManaCost::parse(&impending_cost))
                        .add(&raise_mana);
                    available_mana().can_pay(&adjusted)
                } else {
                    false
                };

                let may_play_costs: Vec<crate::cost::Cost> =
                    crate::staticability::static_ability_continuous::may_play_alt_costs(
                        game, player, card,
                    )
                    .iter()
                    .map(|cost| crate::cost::parse_cost(cost))
                    .collect();
                let may_play_payable =
                    |cost: &crate::cost::Cost, mana: &forge_foundation::ManaCost| {
                        available_mana().can_pay(mana)
                            && crate::cost::can_pay_ignoring_mana_for_spell(
                                cost, game, card_id, player,
                            )
                    };
                let may_play_ok: Vec<u8> = may_play_costs
                    .iter()
                    .enumerate()
                    .filter(|(_, cost)| {
                        let mana = cost_adj.apply(&Self::mana_from_cost(cost)).add(&raise_mana);
                        may_play_payable(cost, &mana)
                    })
                    .map(|(idx, _)| idx as u8)
                    .collect();
                let may_play_morph_ok: Vec<u8> = if card.has_morph {
                    may_play_costs
                        .iter()
                        .enumerate()
                        .filter(|(_, cost)| {
                            self.can_pay_face_down_cast(
                                game,
                                player,
                                card_id,
                                Some(cost),
                                &chosen_types_by_source,
                            )
                        })
                        .map(|(idx, _)| idx as u8)
                        .collect()
                } else {
                    Vec::new()
                };

                if !normal_ok
                    && may_play_ok.is_empty()
                    && may_play_morph_ok.is_empty()
                    && !room_right_split_ok
                    && !spectacle_ok
                    && !evoke_ok
                    && !dash_ok
                    && !blitz_ok
                    && !sneak_ok
                    && !web_slinging_ok
                    && !overload_ok
                    && !static_alt_ok
                    && !suspend_ok
                    && !foretell_exile_ok
                    && !emerge_ok
                    && !offering_ok
                    && !morph_ok
                    && !bestow_ok
                    && !warp_ok
                    && !impending_ok
                {
                    continue;
                }

                // Check additional non-mana costs from SP$ line (e.g. Sac<1/Creature>,
                // BeholdExile<...>) through shared cost payability logic.
                // Use the _for_spell variant so CantSacrifice statics (e.g. Yasharn)
                // can properly evaluate ValidCause$ Spell restrictions.
                let sp_additional_ok = if let Some(sc) = card.action_spell_cost.as_ref() {
                    crate::cost::can_pay_ignoring_mana_for_spell(sc, game, card_id, player)
                } else {
                    true
                };
                let raised_additional_ok = if let Some(ref rc) = raise_cost {
                    crate::cost::can_pay_ignoring_mana_for_spell(rc, game, card_id, player)
                } else {
                    true
                };
                let additional_costs_ok = sp_additional_ok && raised_additional_ok;

                if additional_costs_ok {
                    let all_valid = target_restrictions::has_candidates_in_spell_ability_chain(
                        game, player, &cast_sa,
                    );
                    if all_valid {
                        if normal_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Normal,
                                alt_cost_index: 0,
                            });
                        }
                        for &alt_cost_index in &may_play_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::MayPlay(None),
                                alt_cost_index,
                            });
                        }
                        if room_right_split_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::RoomRightSplit,
                                alt_cost_index: 0,
                            });
                        }
                        if spectacle_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Spectacle,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        // Push one Evoke entry per payable Evoke cost. PlayOption
                        // currently carries only an `Alternative(Evoke)` discriminant
                        // (no per-entry cost), but downstream cost selection will
                        // resolve the right cost at cast time. Java enumerates each
                        // Evoke cost separately for ActionSpace; matching that count
                        // is what keeps the deterministic agent's RNG aligned.
                        // `alt_cost_index` disambiguates multiple Evoke costs on
                        // the same card: intrinsic `Evoke {2}{U}` at index 0
                        // versus Ashling's granted `Evoke {4}` at index 1.
                        // cast_spell.rs uses the index to look up the correct
                        // cost in `get_all_evoke_costs()`.
                        //
                        // Java's ActionSpace sorts SAs by
                        // `sa.toUnsuppressedString()` which reflects the Evoke
                        // cost text, so the two Evoke variants interleave by
                        // cost-string order. Mirror that here by sorting payable
                        // entries by the cost string, but preserve the original
                        // index so cast-time lookup still hits the right one.
                        let mut ordered: Vec<(usize, String)> = evoke_payable.clone();
                        ordered.sort_by(|a, b| a.1.cmp(&b.1));
                        for (idx, _cost) in ordered {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Evoke,
                                ),
                                alt_cost_index: idx as u8,
                            });
                        }
                        if dash_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Dash,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if blitz_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Blitz,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if sneak_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Sneak,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if web_slinging_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::WebSlinging,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if overload_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Overload,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        for &index in &static_alt_indices {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::StaticAlternative,
                                alt_cost_index: index as u8,
                            });
                        }
                        if emerge_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Emerge,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if suspend_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Suspend,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if foretell_exile_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::ForetellExile,
                                alt_cost_index: 0,
                            });
                        }
                        if bestow_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Bestow,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if warp_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Warp,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                        if impending_ok {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Impending,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                    }
                }
                if morph_ok {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Alternative(
                            crate::spellability::AlternativeCost::Morph,
                        ),
                        alt_cost_index: 0,
                    });
                }
                for &alt_cost_index in &may_play_morph_ok {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::MayPlay(Some(
                            crate::spellability::AlternativeCost::Morph,
                        )),
                        alt_cost_index,
                    });
                }
            }
        }

        // Check graveyard for MayPlay$ static abilities (e.g. Walk-In Closet
        // "You may play lands from your graveyard"). Mirrors Java
        // GameActionUtil.canPlayCardMayPlay() for graveyard zone.
        {
            let gy_cards: Vec<CardId> = game.cards_in_zone(ZoneType::Graveyard, player).to_vec();
            for &card_id in &gy_cards {
                let card = game.card(card_id);
                if !card.is_land() {
                    if !crate::staticability::static_ability_continuous::may_play_grants(
                        game, player, card,
                    )
                    .any(|(source, st_ab)| {
                        crate::staticability::static_ability_continuous::grants_zone_permissions(
                            st_ab, source, card, game,
                        )
                    }) {
                        for alt_cost_index in self.may_play_secondary_spell_grants(
                            game,
                            player,
                            card_id,
                            ZoneType::Graveyard,
                            &chosen_types_by_source,
                        ) {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Secondary,
                                alt_cost_index,
                            });
                        }
                        continue;
                    }
                    let normal_sa = crate::spellability::build_spell_ability_for_card_cast(
                        game, card_id, player,
                    );
                    let normal_grants = accepting_may_play_grants(card_id, &normal_sa);
                    if normal_grants > 0
                        && (!must_be_instant || has_flash_permission(card_id))
                        && self.can_cast_may_play_spell(
                            game,
                            player,
                            card_id,
                            ZoneType::Graveyard,
                            may_play_alt_cost(card_id),
                            &chosen_types_by_source,
                        )
                    {
                        for grant in 0..normal_grants {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Normal,
                                alt_cost_index: grant as u8,
                            });
                        }
                    }
                    for alt_cost_index in self.may_play_secondary_spell_grants(
                        game,
                        player,
                        card_id,
                        ZoneType::Graveyard,
                        &chosen_types_by_source,
                    ) {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::Secondary,
                            alt_cost_index,
                        });
                    }
                    if let Some(warp_cost) = card.get_warp_cost() {
                        let mut warp_sa = normal_sa.clone();
                        warp_sa.alt_cost = Some(crate::spellability::AlternativeCost::Warp);
                        let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                            game,
                            card,
                            player,
                            ZoneType::Graveyard,
                        );
                        if (!must_be_instant || has_flash_permission(card_id))
                            && accepting_may_play_grants(card_id, &warp_sa) > 0
                            && self
                                .available_mana_for_spell_card(
                                    game,
                                    player,
                                    card_id,
                                    &chosen_types_by_source,
                                )
                                .can_pay(
                                    &cost_adj.apply(&forge_foundation::ManaCost::parse(&warp_cost)),
                                )
                        {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Warp,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                    }
                    if let Some(sneak_cost) = card.get_sneak_cost().filter(|_| sneak_window(card)) {
                        let mut sneak_sa = normal_sa;
                        sneak_sa.alt_cost = Some(crate::spellability::AlternativeCost::Sneak);
                        let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                            game,
                            card,
                            player,
                            ZoneType::Graveyard,
                        );
                        if accepting_may_play_grants(card_id, &sneak_sa) > 0
                            && self
                                .available_mana_for_spell_card(
                                    game,
                                    player,
                                    card_id,
                                    &chosen_types_by_source,
                                )
                                .can_pay(
                                    &cost_adj
                                        .apply(&forge_foundation::ManaCost::parse(&sneak_cost)),
                                )
                        {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::Alternative(
                                    crate::spellability::AlternativeCost::Sneak,
                                ),
                                alt_cost_index: 0,
                            });
                        }
                    }
                    continue;
                }
                for alt_cost_index in self.may_play_secondary_spell_grants(
                    game,
                    player,
                    card_id,
                    ZoneType::Graveyard,
                    &chosen_types_by_source,
                ) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Secondary,
                        alt_cost_index,
                    });
                }
                if must_be_instant {
                    continue;
                }
                let may_play_grants =
                    crate::staticability::static_ability_continuous::may_play_grants(
                        game, player, card,
                    )
                    .filter(|(source, sa)| {
                        crate::staticability::static_ability_continuous::can_play_or_granted(
                            sa, source, card, game,
                        )
                    })
                    .count();
                if may_play_grants > 0
                    && crate::spellability::land_ability::can_play(
                        &SpellAbility::new_land(Some(card_id), player),
                        game,
                    )
                {
                    playable.extend(Self::may_play_land_options(
                        game,
                        player,
                        card_id,
                        may_play_grants,
                    ));
                }
            }
        }

        // Check battlefield for Room enchantments with a locked door that can be
        // unlocked. Java models this as a `StaticAbilityApiBased` (`ST$ UnlockDoor`)
        // which falls through to the CastSpell branch in the harness, NOT as an
        // activated ability. We mirror that by putting it in `playable` with
        // `PlayCardMode::UnlockDoor`.
        if !must_be_instant {
            let bf_cards: Vec<CardId> = game.cards_in_zone(ZoneType::Battlefield, player).to_vec();
            for &card_id in &bf_cards {
                let card = game.card(card_id);
                if !card.type_line.has_subtype("Room") {
                    continue;
                }
                let fully_unlocked = card
                    .svars
                    .get("UnlockedRoomCount")
                    .and_then(|count| count.parse::<i32>().ok())
                    .is_some_and(|count| count >= 2)
                    || (!card.full_name.is_empty()
                        && card.card_name == card.full_name
                        && card.full_name.contains(" // "));
                if fully_unlocked {
                    continue;
                }
                // Find the synthetic UnlockDoor activated ability
                let has_unlock_ab = card.activated_abilities.iter().any(|ab| ab.is_unlock_door);
                if !has_unlock_ab {
                    continue;
                }
                // Check if the ability can actually be activated (cost, etc.)
                // We reuse the same checks from get_activatable_abilities inline.
                for ab in &card.activated_abilities {
                    if !ab.is_unlock_door {
                        continue;
                    }
                    // Java `Card.getAllPossibleAbilities:7409` offers an unlock only for a
                    // door still locked (`getLockedRooms`).
                    let door = ab
                        .params
                        .get("CardState")
                        .and_then(forge_foundation::CardStateName::from_str_compat);
                    if !door.is_some_and(|state| card.room_door_locked(state)) {
                        continue;
                    }
                    let mana_cost = Self::mana_from_cost(&ab.cost);
                    let available_mana =
                        mana::calculate_available_mana(self.pool(player), game, player);
                    if available_mana.can_pay(&mana_cost)
                        && crate::cost::can_pay_ignoring_mana(&ab.cost, game, card_id, player)
                    {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::UnlockDoor,
                            alt_cost_index: 0,
                        });
                    }
                }
            }
        }

        // Check graveyard for cast permissions such as Flashback, Escape, and Harmonize.
        let graveyard: Vec<CardId> = game.cards_in_zone(ZoneType::Graveyard, player).to_vec();
        for card_id in graveyard {
            let card = game.card(card_id);
            let flashback_costs = card.get_all_flashback_costs();
            for (index, cost) in Self::secondary_flashback_costs(game, card_id)
                .iter()
                .enumerate()
            {
                if self.can_play_secondary_flashback(
                    game,
                    player,
                    card_id,
                    cost,
                    &chosen_types_by_source,
                ) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Alternative(
                            crate::spellability::AlternativeCost::Flashback,
                        ),
                        alt_cost_index: (flashback_costs.len() + index) as u8,
                    });
                }
            }
            if flashback_costs.is_empty()
                && card.get_harmonize_cost().is_none()
                && card.get_escape_cost().is_none()
                && card.get_mayhem_cost().is_none()
            {
                continue;
            }
            if must_be_instant && !has_flash_permission(card_id) {
                continue;
            }
            let cast_sa =
                crate::spellability::build_spell_ability_for_card_cast(game, card_id, player);
            if let Some(ref tr) = cast_sa.target_restrictions {
                if tr.get_min_targets(game, &cast_sa) > 0
                    && !target_restrictions::has_candidates_in_spell_ability_chain(
                        game, player, &cast_sa,
                    )
                {
                    continue;
                }
            }
            let available_mana =
                self.available_mana_for_spell_card(game, player, card_id, &chosen_types_by_source);
            let sp_additional_ok = if let Some(sc) = card.action_spell_cost.as_ref() {
                crate::cost::can_pay_ignoring_mana_for_spell(sc, game, card_id, player)
            } else {
                true
            };
            let flashback_payable: Vec<usize> = flashback_costs
                .iter()
                .enumerate()
                .filter(|(_, fb_cost_str)| {
                    let fb_cost = crate::cost::parse_cost(fb_cost_str);
                    let fb_mana = Self::mana_from_cost(&fb_cost);
                    self.can_pay_graveyard_spell_mana(
                        game,
                        player,
                        card_id,
                        &fb_mana,
                        &available_mana,
                        &chosen_types_by_source,
                    ) && sp_additional_ok
                        && crate::cost::can_pay_ignoring_mana_for_spell(
                            &fb_cost, game, card_id, player,
                        )
                })
                .map(|(index, _)| index)
                .collect();
            let harmonize_ok = if let Some(harmonize_cost_str) = card.get_harmonize_cost() {
                let harmonize_mana = forge_foundation::ManaCost::parse(&harmonize_cost_str);
                let harmonize_base = if harmonize_mana.count_x() > 0 {
                    harmonize_mana.without_x()
                } else {
                    harmonize_mana
                };
                available_mana.can_pay(&harmonize_base) && sp_additional_ok
            } else {
                false
            };
            let escape_ok = if let Some((escape_mana_str, exile_count)) = card.get_escape_cost() {
                let escape_mc = forge_foundation::ManaCost::parse(&escape_mana_str);
                let other_gy_count = game
                    .cards_in_zone(ZoneType::Graveyard, player)
                    .iter()
                    .filter(|&&cid| cid != card_id)
                    .count() as i32;
                self.can_pay_graveyard_spell_mana(
                    game,
                    player,
                    card_id,
                    &escape_mc,
                    &available_mana,
                    &chosen_types_by_source,
                ) && other_gy_count >= exile_count
            } else {
                false
            };
            let mayhem_ok = if let Some(mayhem_cost_str) = card.get_mayhem_cost() {
                let mayhem_mana = Self::mana_from_cost(&crate::cost::parse_cost(&mayhem_cost_str));
                card.was_discarded()
                    && card.entered_current_zone_this_turn(game.turn.turn_number)
                    && self.can_pay_graveyard_spell_mana(
                        game,
                        player,
                        card_id,
                        &mayhem_mana,
                        &available_mana,
                        &chosen_types_by_source,
                    )
                    && sp_additional_ok
            } else {
                false
            };
            if mayhem_ok {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::Mayhem,
                    ),
                    alt_cost_index: 0,
                });
            }
            for index in flashback_payable {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::Flashback,
                    ),
                    alt_cost_index: index as u8,
                });
            }
            if harmonize_ok {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::Harmonize,
                    ),
                    alt_cost_index: 0,
                });
            }
            if escape_ok {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::Escape,
                    ),
                    alt_cost_index: 0,
                });
            }
        }

        // Check exile for Foretold cards (face-down in exile with foretell cost).
        let mut exile: Vec<CardId> = game.cards_in_zone(ZoneType::Exile, player).to_vec();
        for &other in &game.player_order {
            if other != player {
                exile.extend_from_slice(game.cards_in_zone(ZoneType::Exile, other));
            }
        }
        for card_id in exile {
            let card = game.card(card_id);
            let can_may_play = can_may_play_from_static(card_id);
            if !can_may_play && card.owner != player {
                continue;
            }
            if can_may_play {
                for alt_cost_index in self.may_play_secondary_spell_grants(
                    game,
                    player,
                    card_id,
                    ZoneType::Exile,
                    &chosen_types_by_source,
                ) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Secondary,
                        alt_cost_index,
                    });
                }
                if card.is_land() {
                    let land_sa = SpellAbility::new_land(Some(card_id), player);
                    if !must_be_instant
                        && crate::spellability::land_ability::can_play(&land_sa, game)
                    {
                        playable.extend(Self::may_play_land_options(
                            game,
                            player,
                            card_id,
                            count_may_play_grants(card_id).max(1),
                        ));
                    }
                    continue;
                }
                if must_be_instant && !has_flash_permission(card_id) {
                    continue;
                }
                let may_play_costs =
                    crate::staticability::static_ability_continuous::may_play_alt_costs(
                        game, player, card,
                    );
                let normal_grants = count_may_play_grants(card_id)
                    .max(1)
                    .saturating_sub(may_play_costs.len());
                if normal_grants > 0
                    && self.can_cast_may_play_spell(
                        game,
                        player,
                        card_id,
                        ZoneType::Exile,
                        None,
                        &chosen_types_by_source,
                    )
                {
                    for grant in 0..normal_grants {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::Normal,
                            alt_cost_index: grant as u8,
                        });
                    }
                }
                for (alt_cost_index, alt_cost) in may_play_costs.iter().enumerate() {
                    if self.can_cast_may_play_spell(
                        game,
                        player,
                        card_id,
                        ZoneType::Exile,
                        Some(alt_cost.clone()),
                        &chosen_types_by_source,
                    ) {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::MayPlay(None),
                            alt_cost_index: alt_cost_index as u8,
                        });
                    }
                }
                if !must_be_instant {
                    playable.extend(self.may_play_morph_options(
                        game,
                        player,
                        card_id,
                        normal_grants,
                        &chosen_types_by_source,
                    ));
                }
                let room_right_split_cost = card
                    .type_line
                    .has_subtype("Room")
                    .then(|| card.svars.get("RoomRightSplitCost").cloned())
                    .flatten();
                if let Some(cost) = room_right_split_cost {
                    if normal_grants > 0
                        && self.can_cast_may_play_spell(
                            game,
                            player,
                            card_id,
                            ZoneType::Exile,
                            Some(cost),
                            &chosen_types_by_source,
                        )
                    {
                        for _ in 0..normal_grants {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::RoomRightSplit,
                                alt_cost_index: 0,
                            });
                        }
                    }
                    for (alt_cost_index, alt_cost) in may_play_costs.into_iter().enumerate() {
                        if self.can_cast_may_play_spell(
                            game,
                            player,
                            card_id,
                            ZoneType::Exile,
                            Some(alt_cost),
                            &chosen_types_by_source,
                        ) {
                            playable.push(crate::agent::PlayOption {
                                card_id,
                                mode: crate::agent::PlayCardMode::RoomRightSplit,
                                alt_cost_index: alt_cost_index as u8 + 1,
                            });
                        }
                    }
                }
                continue;
            }
            if card.face_down {
                if let Some(foretell_cost_str) = card.get_foretell_cost() {
                    if card.entered_current_zone_this_turn(game.turn.turn_number) {
                        continue;
                    }
                    if must_be_instant && !has_flash_permission(card_id) {
                        continue;
                    }
                    let available_mana = self.available_mana_for_spell_card(
                        game,
                        player,
                        card_id,
                        &chosen_types_by_source,
                    );
                    let foretell_mc = forge_foundation::ManaCost::parse(&foretell_cost_str);
                    let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                        game,
                        card,
                        player,
                        ZoneType::Exile,
                    );
                    let adjusted = cost_adj.apply(&foretell_mc);
                    if available_mana.can_pay(&adjusted) {
                        playable.push(crate::agent::PlayOption {
                            card_id,
                            mode: crate::agent::PlayCardMode::Alternative(
                                crate::spellability::AlternativeCost::Foretell,
                            ),
                            alt_cost_index: 0,
                        });
                    }
                }
            } else if let Some(plotted_turn) = card
                .keywords
                .iter_strings()
                .chain(card.granted_keywords.iter_strings())
                .find_map(crate::card::parse_plotted_turn)
            {
                // Plot: plotted card in exile can be cast for free on a later turn,
                // and Forge also rejects cards that entered exile this turn.
                if game.turn.turn_number <= plotted_turn
                    || card.entered_current_zone_this_turn(game.turn.turn_number)
                    || !crate::player::can_cast_sorcery(game, player)
                {
                    continue;
                }
                let cast_sa =
                    crate::spellability::build_spell_ability_for_card_cast(game, card_id, player);
                if cast_sa
                    .target_restrictions
                    .as_ref()
                    .is_some_and(|tr| tr.get_min_targets(game, &cast_sa) > 0)
                    && !target_restrictions::has_candidates_in_spell_ability_chain(
                        game, player, &cast_sa,
                    )
                {
                    continue;
                }
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Alternative(
                        crate::spellability::AlternativeCost::Plot,
                    ),
                    alt_cost_index: 0,
                });
            } else if card.has_keyword(crate::card::KEYWORD_WARP_EXILED) {
                // Warp: exiled card can be cast for its normal mana cost
                if must_be_instant && !has_flash_permission(card_id) {
                    continue;
                }
                let available_mana = self.available_mana_for_spell_card(
                    game,
                    player,
                    card_id,
                    &chosen_types_by_source,
                );
                let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                    game,
                    card,
                    player,
                    ZoneType::Exile,
                );
                let adjusted = cost_adj.apply(&card.mana_cost);
                if available_mana.can_pay(&adjusted) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Normal,
                        alt_cost_index: 0,
                    });
                }
            }
        }

        // Check Command zone for commanders (with commander tax)
        let command_zone: Vec<CardId> = game.cards_in_zone(ZoneType::Command, player).to_vec();
        for card_id in command_zone {
            let card = game.card(card_id);
            if game.player_is_commander(player, card_id) {
                if must_be_instant && !has_flash_permission(card_id) {
                    continue;
                }
                let tax = game.player_commander_tax(player, card_id);
                let cost_adj = crate::cost::cost_adjustment::compute_cost_adjustment(
                    game,
                    card,
                    player,
                    ZoneType::Command,
                );
                let adjusted_cost = cost_adj.apply(&card.mana_cost);
                // Use a context-aware availability check so mana abilities
                // with `RestrictValid$` (e.g. Secluded Courtyard's
                // "Spell.Creature+ChosenType") are filtered out when the
                // commander isn't a matching creature type. Without this,
                // command-zone casts could incorrectly pull colored mana
                // from chosen-type-gated sources.
                let payment_ctx = mana::ManaPaymentContext {
                    is_spell: true,
                    is_activated_ability: false,
                    sa_on_stack: false,
                    type_line: Some(card.type_line.clone()),
                    card_name: Some(card.card_name.clone()),
                    card_color: Some(card.color),
                    chosen_types_by_source: game
                        .cards
                        .iter()
                        .filter_map(|c| c.chosen_type.clone().map(|chosen| (c.id, chosen)))
                        .collect(),
                    ..Default::default()
                };
                let available_mana = mana::calculate_available_mana_with_context(
                    self.pool(player),
                    game,
                    player,
                    Some(card_id),
                    &[],
                    Some(&payment_ctx),
                );
                if available_mana.can_pay_with_extra_generic(&adjusted_cost, tax) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Normal,
                        alt_cost_index: 0,
                    });
                }
            }
        }

        // `ActionSpace.getPossibleActions` adds `lib.get(0)` for every player, not just `player`.
        let library_tops: Vec<CardId> = game
            .player_order
            .iter()
            .filter_map(|&owner| game.zone(ZoneType::Library, owner).peek_top())
            .collect();
        for card_id in library_tops {
            if !can_may_play_from_static(card_id) {
                continue;
            }
            for alt_cost_index in self.may_play_secondary_spell_grants(
                game,
                player,
                card_id,
                ZoneType::Library,
                &chosen_types_by_source,
            ) {
                playable.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::Secondary,
                    alt_cost_index,
                });
            }
            if game.card(card_id).is_land() {
                let land_sa = SpellAbility::new_land(Some(card_id), player);
                if !must_be_instant && crate::spellability::land_ability::can_play(&land_sa, game) {
                    playable.extend(Self::may_play_land_options(
                        game,
                        player,
                        card_id,
                        count_may_play_grants(card_id).max(1),
                    ));
                }
                continue;
            }
            if must_be_instant && !has_flash_permission(card_id) {
                continue;
            }
            let may_play_costs =
                crate::staticability::static_ability_continuous::may_play_alt_costs(
                    game,
                    player,
                    game.card(card_id),
                );
            let normal_grants = count_may_play_grants(card_id)
                .max(1)
                .saturating_sub(may_play_costs.len());
            if normal_grants > 0
                && self.can_cast_may_play_spell(
                    game,
                    player,
                    card_id,
                    ZoneType::Library,
                    None,
                    &chosen_types_by_source,
                )
            {
                for grant in 0..normal_grants {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::Normal,
                        alt_cost_index: grant as u8,
                    });
                }
            }
            for (alt_cost_index, alt_cost) in may_play_costs.into_iter().enumerate() {
                if self.can_cast_may_play_spell(
                    game,
                    player,
                    card_id,
                    ZoneType::Library,
                    Some(alt_cost),
                    &chosen_types_by_source,
                ) {
                    playable.push(crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::MayPlay(None),
                        alt_cost_index: alt_cost_index as u8,
                    });
                }
            }
            if !must_be_instant {
                playable.extend(self.may_play_morph_options(
                    game,
                    player,
                    card_id,
                    normal_grants,
                    &chosen_types_by_source,
                ));
            }
        }

        self.trace_playability(game, player, must_be_instant, &playable);
        playable
    }

    /// The checks are recomputed for the normal cost; `offered` is what the action space holds.
    fn trace_playability(
        &self,
        game: &GameState,
        player: PlayerId,
        must_be_instant: bool,
        playable: &[crate::agent::PlayOption],
    ) {
        for zone in [
            ZoneType::Hand,
            ZoneType::Graveyard,
            ZoneType::Exile,
            ZoneType::Command,
        ] {
            for &card_id in game.cards_in_zone(zone, player) {
                let card = game.card(card_id);
                if !Self::card_trace_matches(&card.card_name) {
                    continue;
                }
                let offered: Vec<crate::agent::PlayCardMode> = playable
                    .iter()
                    .filter(|option| option.card_id == card_id)
                    .map(|option| option.mode)
                    .collect();
                let mut sa =
                    crate::spellability::build_spell_ability_for_card_cast(game, card_id, player);
                sa.restriction.variables.set_zone(zone);
                let cant_be_cast =
                    crate::staticability::static_ability_cant_be_cast::cant_be_cast_ability_from_zone(
                        &game.cards,
                        &sa,
                        card,
                        player,
                        game,
                    );
                let flash = card.type_line.is_instant()
                    || card.has_keyword("Flash")
                    || crate::staticability::static_ability_cast_with_flash::any_with_flash_for_card(
                        game, card, player,
                    );
                let targets = sa.target_restrictions.as_ref().map(|tr| {
                    (
                        tr.get_min_targets(game, &sa),
                        target_restrictions::has_candidates_in_spell_ability_chain(
                            game, player, &sa,
                        ),
                    )
                });
                let raise = crate::cost::cost_adjustment::compute_raise_cost_parts(
                    game, card, player, zone,
                )
                .as_ref()
                .map(Self::mana_from_cost)
                .unwrap_or_else(|| forge_foundation::ManaCost::generic(0));
                let cost =
                    crate::cost::cost_adjustment::compute_cost_adjustment(game, card, player, zone)
                        .apply(&card.mana_cost.without_x())
                        .add(&raise);
                let reduced = apply_cost_reductions(
                    game,
                    player,
                    card_id,
                    card,
                    &crate::mana::apply_player_life_payment_keywords(game, player, &cost),
                );
                let chosen: crate::HashMap<CardId, String> = game
                    .cards
                    .iter()
                    .filter_map(|c| c.chosen_type.clone().map(|chosen| (c.id, chosen)))
                    .collect();
                let mana = self.available_mana_for_spell_card(game, player, card_id, &chosen);
                let simulated = crate::mana::can_pay_spell_mana_cost_for_action_space(
                    game,
                    self.pool(player),
                    player,
                    card_id,
                    &reduced,
                    &Self::spell_payment_context(card, &chosen),
                );
                let may_play_from: Vec<&str> =
                    crate::staticability::static_ability_continuous::may_play_grants(
                        game, player, card,
                    )
                    .filter(|(source, st)| {
                        crate::staticability::static_ability_continuous::can_play_or_granted(
                            st, source, card, game,
                        )
                    })
                    .map(|(source, _)| source.card_name.as_str())
                    .collect();
                eprintln!(
                    "[card-trace] T{} P{} {:?} {}#{} {zone:?}: offered {offered:?} | cant_be_cast={cant_be_cast} \
                     can_play={} instant_only={must_be_instant} flash={flash} | (min targets, candidates)={targets:?} \
                     | cost {} -> {reduced} | mana {} from {:?} sources (W{} U{} B{} R{} G{} C{}) pool_pays={} \
                     simulated_pays={simulated} | may_play from {may_play_from:?}",
                    game.turn.turn_number,
                    player.0,
                    game.turn.phase,
                    card.card_name,
                    card_id.index(),
                    crate::spellability::spell::can_play(&sa, game),
                    card.mana_cost,
                    mana.total_mana(),
                    mana.total_sources,
                    mana.white(),
                    mana.blue(),
                    mana.black(),
                    mana.red(),
                    mana.green(),
                    mana.colorless(),
                    mana.can_pay(&reduced),
                );
            }
        }
    }

    fn may_play_morph_options(
        &self,
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        normal_grants: usize,
        chosen_types_by_source: &crate::HashMap<CardId, String>,
    ) -> Vec<crate::agent::PlayOption> {
        let card = game.card(card_id);
        if !card.has_morph {
            return Vec::new();
        }
        let morph = crate::agent::PlayOption {
            card_id,
            mode: crate::agent::PlayCardMode::Alternative(
                crate::spellability::AlternativeCost::Morph,
            ),
            alt_cost_index: 0,
        };
        let mut options =
            if self.can_pay_face_down_cast(game, player, card_id, None, chosen_types_by_source) {
                vec![morph; normal_grants]
            } else {
                Vec::new()
            };
        let alt_cost_grants =
            crate::staticability::static_ability_continuous::may_play_grants(game, player, card)
                .filter_map(|(source, st_ab)| {
                    crate::staticability::static_ability_continuous::may_play_alt_mana_cost(
                        st_ab, source, card, game,
                    )
                    .map(|cost| (st_ab.ir.may_play_without_mana_cost, cost))
                });
        for (alt_cost_index, (without_mana_cost, cost)) in alt_cost_grants.enumerate() {
            let cost = crate::cost::parse_cost(&cost);
            if !without_mana_cost
                && self.can_pay_face_down_cast(
                    game,
                    player,
                    card_id,
                    Some(&cost),
                    chosen_types_by_source,
                )
            {
                options.push(crate::agent::PlayOption {
                    card_id,
                    mode: crate::agent::PlayCardMode::MayPlay(Some(
                        crate::spellability::AlternativeCost::Morph,
                    )),
                    alt_cost_index: alt_cost_index as u8,
                });
            }
        }
        options
    }

    fn may_play_land_options(
        game: &GameState,
        player: PlayerId,
        card_id: CardId,
        grants: usize,
    ) -> Vec<crate::agent::PlayOption> {
        let alt_cost_grants: Vec<u8> =
            crate::staticability::static_ability_continuous::may_play_alt_costs(
                game,
                player,
                game.card(card_id),
            )
            .iter()
            .enumerate()
            .filter(|(_, cost)| {
                !crate::staticability::static_ability_continuous::is_mana_alt_cost(cost)
            })
            .map(|(index, _)| index as u8)
            .collect();
        let normal = std::iter::repeat_n(
            crate::agent::PlayOption {
                card_id,
                mode: crate::agent::PlayCardMode::Normal,
                alt_cost_index: 0,
            },
            grants.saturating_sub(alt_cost_grants.len()),
        );
        normal
            .chain(
                alt_cost_grants
                    .into_iter()
                    .map(|alt_cost_index| crate::agent::PlayOption {
                        card_id,
                        mode: crate::agent::PlayCardMode::MayPlay(None),
                        alt_cost_index,
                    }),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Card;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    fn card(
        name: &str,
        owner: PlayerId,
        type_line: &str,
        mana_cost: &str,
        color: ColorSet,
        power: Option<i32>,
        toughness: Option<i32>,
        abilities: Vec<&str>,
    ) -> Card {
        Card::new(
            CardId(0),
            name.to_string(),
            owner,
            CardTypeLine::parse(type_line),
            ManaCost::parse(mana_cost),
            color,
            power,
            toughness,
            vec![],
            abilities.into_iter().map(str::to_string).collect(),
        )
    }

    #[test]
    fn phyrexian_spell_with_generic_is_playable_from_off_color_sources_and_life() {
        let player = PlayerId(1);
        let opponent = PlayerId(0);
        let mut game = GameState::new(&["Alice", "Bob"], 20);

        for name in ["Forest", "Mountain"] {
            let mut land = card(
                name,
                player,
                &format!("Basic Land - {name}"),
                "",
                ColorSet::COLORLESS,
                None,
                None,
                vec![],
            );
            land.zone = ZoneType::Battlefield;
            let land_id = game.create_card(land);
            game.add_card_to_zone(ZoneType::Battlefield, player, land_id);
        }

        let mut target = card(
            "Raging Goblin",
            opponent,
            "Creature - Goblin",
            "R",
            ColorSet::RED,
            Some(1),
            Some(1),
            vec![],
        );
        target.zone = ZoneType::Battlefield;
        let target_id = game.create_card(target);
        game.add_card_to_zone(ZoneType::Battlefield, opponent, target_id);

        let mut dismember = card(
            "Dismember",
            player,
            "Instant",
            "1 BP BP",
            ColorSet::BLACK,
            None,
            None,
            vec!["SP$ Pump | IsCurse$ True | ValidTgts$ Creature | NumAtt$ -5 | NumDef$ -5"],
        );
        dismember.zone = ZoneType::Hand;
        let dismember_id = game.create_card(dismember);
        game.add_card_to_zone(ZoneType::Hand, player, dismember_id);

        let game_loop = GameLoop::new(2);
        let sa =
            crate::spellability::build_spell_ability_for_card_cast(&game, dismember_id, player);
        let valid_targets = crate::card::card_util::get_valid_cards_to_target(&game, &sa);
        assert_eq!(
            valid_targets.len(),
            1,
            "Dismember should have the opposing creature as a valid target"
        );
        let available_mana =
            crate::mana::calculate_available_mana(game_loop.pool(player), &game, player);
        assert!(
            available_mana.can_pay_with_phyrexian_life(&ManaCost::parse("1 BP BP"), 20),
            "available off-color sources should cover generic while life covers phyrexian shards"
        );
        let playable = game_loop.get_playable_cards(&game, player, true);

        assert!(
            playable.iter().any(|option| option.card_id == dismember_id),
            "Dismember should be instant-speed playable using one off-color generic source and 4 life"
        );
    }
}
