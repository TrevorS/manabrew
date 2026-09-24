//! TokenEffectBase — shared token creation machinery.
//!
//! Mirrors Java's `TokenEffectBase.java`.  Rust effects are generated as
//! resolver functions rather than subclasses, so this module exposes a trait
//! with Java-parity default methods plus a concrete stateless implementation
//! used by effects that need the common token path.
use crate::HashMap;

use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

use super::{emit_zone_trigger, EffectContext};
use crate::card::card_zone_table::CardZoneTable;
use crate::card::Card;
use crate::card_trait_base::CardTrait;
use crate::event::RunParams;
use crate::ids::{CardId, PlayerId};
use crate::parsing::{keys, split_param_list_value};
use crate::replacement::replacement_handler::{
    apply_replacements_with_agents_and_runtime, ReplacementEvent, ReplacementRuntime,
};
use crate::replacement::replacement_result::ReplacementResult;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

#[derive(Clone, Debug)]
pub struct TokenTableCell {
    pub owner: PlayerId,
    pub prototype: Card,
    pub amount: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TokenCreateTable {
    cells: Vec<TokenTableCell>,
}

impl TokenCreateTable {
    pub fn put(&mut self, owner: PlayerId, prototype: Card, amount: usize) {
        let at = self
            .cells
            .iter()
            .rposition(|cell| cell.owner == owner)
            .map_or(self.cells.len(), |i| i + 1);
        self.cells.insert(
            at,
            TokenTableCell {
                owner,
                prototype,
                amount,
            },
        );
    }

    pub fn cells(&self) -> &[TokenTableCell] {
        &self.cells
    }

    pub fn cells_mut(&mut self) -> &mut Vec<TokenTableCell> {
        &mut self.cells
    }

    pub fn row_key_set(&self) -> Vec<PlayerId> {
        let mut rows = Vec::new();
        for cell in &self.cells {
            if !rows.contains(&cell.owner) {
                rows.push(cell.owner);
            }
        }
        rows
    }

    pub fn get_filter_amount(
        &self,
        valid_owner: Option<&str>,
        valid_token: Option<&str>,
        ctb: &impl CardTrait,
        host: &Card,
    ) -> usize {
        let filtered_player = valid_owner.map(|valid| {
            self.row_key_set()
                .into_iter()
                .filter(|&player| ctb.matches_valid_player(valid, player, host))
                .collect::<Vec<_>>()
        });
        if filtered_player.as_ref().is_some_and(Vec::is_empty) {
            return 0;
        }
        let filtered_cards = valid_token.map(|valid| {
            self.cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| ctb.matches_valid_card(valid, &cell.prototype, host))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        });
        if filtered_cards.as_ref().is_some_and(Vec::is_empty) {
            return 0;
        }
        match (filtered_player, filtered_cards) {
            (Some(players), Some(cards)) => self
                .cells
                .iter()
                .enumerate()
                .filter(|(i, cell)| players.contains(&cell.owner) && cards.contains(i))
                .map(|(_, cell)| cell.amount)
                .sum(),
            _ => self.cells.iter().map(|cell| cell.amount).sum(),
        }
    }

    pub fn retain_players(&mut self, keep: impl Fn(PlayerId) -> bool) {
        self.cells.retain(|cell| keep(cell.owner));
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct TokenCreateResult {
    pub created: Vec<CardId>,
    pub combat_changed: bool,
}

#[derive(Clone, Copy)]
pub enum TokenAttachmentTarget {
    Card(CardId),
    Player(PlayerId),
}

pub trait TokenEffectBase {
    fn token_scripts(&self, sa: &SpellAbility) -> Vec<String> {
        split_param_list_value(sa.ir.token_script.as_deref(), ",")
    }

    fn get_token_template<'a>(
        &self,
        templates: &'a HashMap<String, Card>,
        script: &str,
    ) -> Option<&'a Card> {
        templates.get(script).or_else(|| {
            let lower = script.to_ascii_lowercase();
            templates
                .iter()
                .find(|(key, _)| key.to_ascii_lowercase() == lower)
                .map(|(_, value)| value)
        })
    }

    fn require_token_template(&self, templates: &HashMap<String, Card>, script: &str) -> Card {
        self.get_token_template(templates, script)
            .cloned()
            .unwrap_or_else(|| panic!("don't find Token for TokenScript: {script}"))
    }

    fn create_token_table(
        &self,
        ctx: &EffectContext,
        players: &[PlayerId],
        token_scripts: &[String],
        final_amount: usize,
        sa: &SpellAbility,
    ) -> TokenCreateTable {
        let mut token_table = TokenCreateTable::default();
        for &owner in players {
            if !ctx.game.player(owner).is_alive() {
                continue;
            }
            for script in token_scripts {
                let result = self.get_proto_type(ctx, script, sa, owner);
                token_table.put(owner, result, final_amount);
            }
        }
        token_table
    }

    fn get_proto_type(
        &self,
        ctx: &EffectContext,
        script: &str,
        sa: &SpellAbility,
        owner: PlayerId,
    ) -> Card {
        let mut result = self.require_token_template(ctx.token_templates, script);
        result.set_owner(owner);
        result.set_controller(owner);
        result.set_is_token(true);
        result.set_s_var("TokenScript", script);
        result.set_s_var("TokenSpawningAbility", sa.ability_text.clone());
        self.apply_token_power_toughness(ctx, sa, &mut result);
        result
    }

    fn make_token_table_internal_from_script(
        &self,
        ctx: &EffectContext,
        owner: PlayerId,
        script: &str,
        final_amount: usize,
        sa: &SpellAbility,
    ) -> TokenCreateTable {
        let result = self.get_proto_type(ctx, script, sa, owner);
        self.make_token_table_internal(owner, result, final_amount)
    }

    fn apply_token_power_toughness(
        &self,
        ctx: &EffectContext,
        sa: &SpellAbility,
        result: &mut Card,
    ) {
        if let Some(power) = sa.ir.token_power_text.as_deref() {
            result.set_base_power(Some(crate::svar::resolve_numeric_value(
                ctx.game, sa, power, 0,
            )));
        }
        if let Some(toughness) = sa.ir.token_toughness_text.as_deref() {
            result.set_base_toughness(Some(crate::svar::resolve_numeric_value(
                ctx.game, sa, toughness, 0,
            )));
        }
    }

    fn make_token_table_internal(
        &self,
        owner: PlayerId,
        result: Card,
        final_amount: usize,
    ) -> TokenCreateTable {
        let mut token_table = TokenCreateTable::default();
        token_table.put(owner, result, final_amount);
        token_table
    }

    fn has_inline_token_params(&self, sa: &SpellAbility) -> bool {
        sa.ir.token_power.is_some()
            || sa.ir.token_toughness.is_some()
            || sa.ir.token_types_text.is_some()
            || sa.ir.token_name_text.is_some()
    }

    fn build_inline_token(&self, sa: &SpellAbility, owner: PlayerId) -> Card {
        let name = sa
            .ir
            .token_name_text
            .clone()
            .unwrap_or_else(|| "Token".to_string());
        let power = sa.ir.token_power;
        let toughness = sa.ir.token_toughness;
        let type_line = sa
            .ir
            .token_types_text
            .as_deref()
            .map(CardTypeLine::parse)
            .unwrap_or_else(|| CardTypeLine::parse("Creature"));
        let colors = sa
            .ir
            .token_colors_text
            .as_deref()
            .map(|s| {
                if s.eq_ignore_ascii_case("Colorless") {
                    ColorSet::COLORLESS
                } else {
                    ColorSet::from_names(s)
                }
            })
            .unwrap_or(ColorSet::COLORLESS);
        let keywords = split_param_list_value(sa.ir.token_keywords_text.as_deref(), "&");

        Card::new(
            CardId(0),
            name,
            owner,
            type_line,
            ManaCost::parse(""),
            colors,
            power,
            toughness,
            keywords,
            vec![],
        )
    }

    fn make_token_table_from_scripts(
        &self,
        ctx: &mut EffectContext,
        players: &[PlayerId],
        token_scripts: &[String],
        final_amount: usize,
        clone_origin: bool,
        trigger_list: &mut CardZoneTable,
        sa: &SpellAbility,
    ) -> TokenCreateResult {
        let token_table = self.create_token_table(ctx, players, token_scripts, final_amount, sa);
        self.make_token_table(ctx, token_table, clone_origin, trigger_list, sa)
    }

    fn make_token_table(
        &self,
        ctx: &mut EffectContext,
        mut token_table: TokenCreateTable,
        clone_origin: bool,
        trigger_list: &mut CardZoneTable,
        sa: &SpellAbility,
    ) -> TokenCreateResult {
        for cell in token_table.cells_mut() {
            let script = cell.prototype.get_s_var("TokenScript").map(str::to_owned);
            if let Some(script) = script.as_deref() {
                if cell.prototype.copied_permanent.is_some() {
                    // Java builds a copy from the original's PaperToken; its image key is the only draw.
                    ctx.rng.next_int(1);
                } else {
                    cell.prototype.set_code = Some(ctx.sync_token_art_rng(script, sa));
                }
            }
        }
        self.apply_create_token_replacements(ctx, &mut token_table);

        let original_tokens: Vec<Card> = token_table
            .cells()
            .iter()
            .map(|cell| cell.prototype.clone())
            .collect();
        let pump_keywords = self.pump_keywords(sa);
        let mut result = TokenCreateResult::default();

        for cell in token_table.cells().iter().cloned() {
            let controller = cell.prototype.controller;
            for _ in 0..cell.amount {
                let Some(token_id) = self.create_single_token(
                    ctx,
                    sa,
                    &cell.prototype,
                    cell.owner,
                    controller,
                    clone_origin,
                    &pump_keywords,
                    trigger_list,
                    &original_tokens,
                ) else {
                    // Java records a None->None table entry with a null moved card here.
                    // Rust CardZoneTable is CardId-only, so there is no valid value to store.
                    continue;
                };
                result.created.push(token_id);
                if self.add_token_to_combat(ctx, sa, token_id) {
                    result.combat_changed = true;
                }
            }
        }

        if let Some(action) = sa.ir.at_eot.as_deref() {
            crate::ability::spell_ability_effect::register_at_eot(
                ctx.trigger_handler,
                ctx.game,
                sa,
                action,
                result.created.clone(),
            );
        }

        result
    }

    fn create_single_token(
        &self,
        ctx: &mut EffectContext,
        sa: &SpellAbility,
        prototype: &Card,
        creator: PlayerId,
        controller: PlayerId,
        clone_origin: bool,
        pump_keywords: &[String],
        trigger_list: &mut CardZoneTable,
        original_tokens: &[Card],
    ) -> Option<CardId> {
        let attachment_before_move = if sa.ir.attach_after_text.is_none() {
            self.attachment_target(ctx, sa)
        } else {
            None
        };
        if sa.ir.attached_to.is_some()
            && attachment_before_move.is_none()
            && prototype.type_line.has_subtype("Aura")
        {
            return None;
        }

        let mut token = prototype.clone();
        token.set_owner(creator);
        token.set_controller(controller);
        token.set_is_token(true);

        if let Some(counter_type) = sa.ir.with_counters_type.as_ref() {
            let amount = super::resolve_numeric_svar(ctx.game, sa, keys::WITH_COUNTERS_AMOUNT, 1);
            token.add_counter(counter_type, amount.max(0));
        }

        if let Some(add_triggers_from) = sa.ir.add_triggers_from_text.as_deref() {
            if let Some(source_id) = sa.source {
                let cards = crate::ability::ability_utils::get_defined_cards(
                    ctx.game,
                    Some(source_id),
                    add_triggers_from,
                    Some(sa.activating_player),
                );
                for card_id in cards {
                    for trigger in ctx.game.card(card_id).copiable_triggers() {
                        token.add_trigger(trigger);
                    }
                }
            }
        }

        let token_id = ctx.game.create_card(token);
        self.after_token_created(ctx, token_id);
        if let Some(attachment) = attachment_before_move {
            if !self.attach_token_to(ctx, token_id, attachment)
                && ctx.game.card(token_id).type_line.has_subtype("Aura")
            {
                return None;
            }
        }

        let outer_change_zone_table = ctx.game.pending_change_zone_table.take();
        ctx.move_card(token_id, ZoneType::Battlefield, controller);
        ctx.game.pending_change_zone_table = outer_change_zone_table;

        if sa.ir.token_tapped {
            ctx.game.tap(token_id);
        }

        if clone_origin {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(token_id).clone_origin = Some(source_id);
            }
        }
        if clone_origin || prototype.copied_permanent.is_some() {
            ctx.game.card_mut(token_id).copied_permanent =
                prototype.copied_permanent.or_else(|| {
                    if prototype.id != CardId(0) {
                        Some(prototype.id)
                    } else {
                        None
                    }
                });
        }

        if !pump_keywords.is_empty() {
            for keyword in pump_keywords {
                ctx.game.card_mut(token_id).add_pump_keyword(keyword);
            }
            self.add_pump_until(ctx, sa, token_id);
        }

        if let Some(location) = sa.ir.at_eot_trig_text.as_deref() {
            crate::ability::spell_ability_effect::add_self_trigger_at_eot(
                ctx.trigger_handler,
                ctx.game,
                location,
                token_id,
            );
        }

        if sa.ir.remember_tokens {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).add_remembered_card(token_id);
            }
        }
        if sa.ir.remember_original_tokens
            && original_tokens.iter().any(|original| {
                original.card_name == prototype.card_name
                    && original.type_line == prototype.type_line
            })
        {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).add_remembered_card(token_id);
            }
        }
        if sa.ir.imprint_tokens {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).add_imprinted_card(token_id);
            }
        }
        if sa.ir.remember_source {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(token_id).add_remembered_card(source_id);
            }
        }
        if let Some(defined) = sa.ir.token_remembered.as_deref() {
            if let Some(source_id) = sa.source {
                let remembered_cards = crate::ability::ability_utils::get_defined_cards(
                    ctx.game,
                    Some(source_id),
                    defined,
                    Some(sa.activating_player),
                );
                ctx.game
                    .card_mut(token_id)
                    .add_remembered_cards(remembered_cards);
            }
        }
        if sa.ir.cleanup_for_each {
            let remembered = prototype.remembered_cards.clone();
            for card_id in remembered {
                ctx.game.card_mut(token_id).remove_remembered(card_id);
            }
        }

        if sa.ir.attach_after_text.is_some() {
            if let Some(attachment) = self.attachment_target(ctx, sa) {
                let _ = self.attach_token_to(ctx, token_id, attachment);
            }
        }

        ctx.trigger_handler
            .register_active_trigger(ctx.game, token_id);
        ctx.trigger_handler.run_trigger(
            TriggerType::TokenCreated,
            RunParams {
                card: Some(token_id),
                player: Some(creator),
                ..Default::default()
            },
            false,
        );
        emit_zone_trigger(
            ctx.trigger_handler,
            token_id,
            ZoneType::None,
            ZoneType::Battlefield,
        );

        let token_lki = crate::card::card_copy_service::get_lki_copy(ctx.game.card(token_id));
        trigger_list.put(Some(ZoneType::None), Some(ZoneType::Battlefield), token_id);
        let first_time = ctx.game.player(creator).tokens_created_this_turn == 0;
        // TODO(parity): Java CardZoneTable stores the token LKI copy here. Rust
        // CardZoneTable currently stores CardId only, so consumers see the live token.
        let _ = token_lki;
        trigger_list.add_token(token_id, creator, first_time);
        crate::player::add_tokens_created_this_turn(ctx.game, creator, 1);

        Some(token_id)
    }

    fn after_token_created(&self, ctx: &mut EffectContext, token_id: CardId) {
        let token = ctx.game.card_mut(token_id);
        token.update_spell_abilities();

        let bound_host = ctx.game.card(token_id).clone();
        let token = ctx.game.card_mut(token_id);
        for trigger in &mut token.triggers {
            trigger.bind_host_card_id(bound_host.id);
        }
        for static_ability in &mut token.static_abilities {
            static_ability.base.set_host_card_id(bound_host.id);
        }
        for replacement_effect in &mut token.replacement_effects {
            replacement_effect.base.set_host_card_id(bound_host.id);
        }
    }

    fn attachment_target(
        &self,
        ctx: &mut EffectContext,
        sa: &SpellAbility,
    ) -> Option<TokenAttachmentTarget> {
        let attached_to = sa.ir.attached_to.as_deref()?;
        let (players, cards) =
            crate::ability::ability_utils::get_defined_entities(attached_to, sa, ctx.game);
        if let Some(&player_id) = players.first() {
            return Some(TokenAttachmentTarget::Player(player_id));
        }
        cards
            .first()
            .map(|&card_id| TokenAttachmentTarget::Card(card_id))
    }

    fn attach_token_to(
        &self,
        ctx: &mut EffectContext,
        token_id: CardId,
        target: TokenAttachmentTarget,
    ) -> bool {
        match target {
            TokenAttachmentTarget::Card(target_id) => {
                if ctx.game.card(target_id).zone != ZoneType::Battlefield {
                    return false;
                }
                if crate::staticability::static_ability_cant_attach::cant_attach(
                    &ctx.game.cards,
                    ctx.game.card(token_id),
                    ctx.game.card(target_id),
                    false,
                ) {
                    return false;
                }
                ctx.game.attach_to(token_id, target_id);
                true
            }
            TokenAttachmentTarget::Player(player_id) => {
                if !ctx.game.player(player_id).is_alive() {
                    return false;
                }
                ctx.game.attach_to_player(token_id, player_id);
                true
            }
        }
    }

    fn add_token_to_combat(
        &self,
        ctx: &mut EffectContext,
        sa: &SpellAbility,
        token_id: CardId,
    ) -> bool {
        if super::add_to_combat(ctx, sa, token_id, keys::TOKEN_ATTACKING) {
            return true;
        }
        self.add_token_blocking(ctx, sa, token_id)
    }

    fn add_token_blocking(
        &self,
        ctx: &mut EffectContext,
        sa: &SpellAbility,
        token_id: CardId,
    ) -> bool {
        if !ctx.game.turn.is_combat() || !ctx.game.card(token_id).is_creature() {
            return false;
        }
        let Some(blocking) = sa.ir.token_blocking_text.as_deref() else {
            return false;
        };
        let attackers = crate::ability::ability_utils::get_defined_cards(
            ctx.game,
            sa.source,
            blocking,
            Some(sa.activating_player),
        );
        let Some(combat) = ctx.combat.as_deref_mut() else {
            return false;
        };
        let Some(attacker_id) = attackers
            .into_iter()
            .find(|&attacker_id| combat.is_attacking(attacker_id))
        else {
            return false;
        };
        if !crate::combat::combat_util::can_creature_block(ctx.game, token_id, attacker_id) {
            return false;
        }
        if combat
            .blockers
            .iter()
            .any(|&(blocker, attacker)| blocker == token_id && attacker == attacker_id)
        {
            return false;
        }
        combat.declare_blocker(
            token_id,
            attacker_id,
            ctx.game.card(token_id).zone_timestamp,
        );
        true
    }

    fn pump_keywords(&self, sa: &SpellAbility) -> Vec<String> {
        split_param_list_value(sa.ir.pump_keywords.as_deref(), " & ")
    }

    fn add_pump_until(&self, ctx: &mut EffectContext, sa: &SpellAbility, token_id: CardId) {
        if let Some(duration) = sa.ir.pump_duration_text.as_deref() {
            ctx.game
                .card_mut(token_id)
                .set_s_var("PumpDuration", duration);
        }
    }

    fn apply_create_token_replacements(
        &self,
        ctx: &mut EffectContext,
        token_table: &mut TokenCreateTable,
    ) {
        let mut to_remove = Vec::new();
        for player in token_table.row_key_set() {
            let mut event = ReplacementEvent::CreateToken {
                player,
                token_table: std::mem::take(token_table),
                is_effect: true,
            };
            let mut runtime = ReplacementRuntime {
                trigger_handler: ctx.trigger_handler,
                token_templates: ctx.token_templates,
                token_art_variants: ctx.token_art_variants,
                token_fallback: ctx.token_fallback,
                edition_dates: ctx.edition_dates,
                mana_pools: ctx.mana_pools,
                rng: ctx.rng,
            };
            let result = apply_replacements_with_agents_and_runtime(
                &mut *ctx.game,
                ctx.agents,
                &mut runtime,
                &mut event,
            );
            if let ReplacementEvent::CreateToken {
                token_table: replaced,
                ..
            } = event
            {
                *token_table = replaced;
            }
            if !matches!(
                result,
                ReplacementResult::NotReplaced | ReplacementResult::Updated
            ) {
                to_remove.push(player);
            }
        }
        token_table
            .cells
            .retain(|cell| !to_remove.contains(&cell.owner));
    }
}

pub struct TokenEffectBaseImpl;

impl TokenEffectBase for TokenEffectBaseImpl {}

pub const TOKEN_EFFECT_BASE: TokenEffectBaseImpl = TokenEffectBaseImpl;
