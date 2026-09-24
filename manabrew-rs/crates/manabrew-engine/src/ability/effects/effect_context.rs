//! `EffectContext` — bundle of subsystem refs threaded through effect resolution.
//!
//! Rust-specific concession: Java reaches `Game.getTriggerHandler()`, agents,
//! combat, mana pools via chained getters from `SpellAbility.getHostCard()`.
//! The Rust engine deliberately owns those subsystems outside `GameState`
//! (see `trigger_handler.rs` top comment), so every resolver needs a handful
//! of mutable references. This struct packs them.

use crate::HashMap;

use forge_foundation::ZoneType;

use crate::agent::GameEntity;
use crate::agent::PlayerAgent;
use crate::card::{Card, CounterType};
use crate::event::RunParams;
use crate::game::GameState;
use crate::game_entity_counter_table::GameEntityCounterTable;
use crate::ids::{CardId, PlayerId};
use crate::mana::ManaPool;
use crate::spellability::SpellAbility;
use crate::trigger::handler::TriggerHandler;

/// Everything an effect needs to resolve.
pub struct EffectContext<'a> {
    pub game: &'a mut GameState,
    pub combat: Option<&'a mut crate::combat::CombatState>,
    pub agents: &'a mut [Box<dyn PlayerAgent>],
    pub trigger_handler: &'a mut TriggerHandler,
    pub token_templates: &'a HashMap<String, Card>,
    /// Token art variant counts for game-RNG parity with Java.
    pub token_art_variants: &'a HashMap<(String, String), usize>,
    /// Token fallback codes: edition_code → fallback_edition_code.
    pub token_fallback: &'a HashMap<String, String>,
    /// Edition release dates: edition_code → "YYYY-MM-DD". Used to sort
    /// editions newest-first for token fallback (Java parity).
    pub edition_dates: &'a HashMap<String, String>,
    pub mana_pools: &'a mut Vec<ManaPool>,
    /// CardId of the parent SA's chosen target card, propagated through the
    /// sub-ability chain so that `Defined$ ParentTarget` effects can resolve it.
    /// Mirrors Java's `SpellAbility.getParentTargetCard()` (via getRootAbility()).
    pub parent_target_card: Option<CardId>,
    /// Pluggable RNG for game effects (shuffles, coin flips, dice rolls).
    /// Parity tests inject a JavaRandom-backed implementation; normal gameplay
    /// uses the default ThreadRngAdapter.
    pub rng: &'a mut dyn crate::game_rng::GameRng,
}

pub(crate) fn add_counter_with_context(
    game: &mut GameState,
    trigger_handler: Option<&mut TriggerHandler>,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    card_id: CardId,
    counter_type: &CounterType,
    amount: i32,
    params: RunParams,
    is_effect: bool,
) -> i32 {
    let source = params.source_player.or(params.cause_player);
    let cause = params.cause.clone();
    let mut table = GameEntityCounterTable::default();
    table.put(
        source,
        GameEntity::Card(card_id),
        counter_type.clone(),
        amount,
    );
    table
        .replace_counter_effect(
            game,
            trigger_handler,
            agents,
            cause.as_ref(),
            is_effect,
            params,
        )
        .get(source, GameEntity::Card(card_id), counter_type)
}

impl EffectContext<'_> {
    fn token_set(&self, token_script: &str, edition_code: &str) -> Option<String> {
        let code = edition_code.to_uppercase();
        if self
            .token_art_variants
            .contains_key(&(token_script.to_lowercase(), code.clone()))
        {
            return Some(code);
        }
        let fallback = self.token_fallback.get(&code)?;
        self.token_set(token_script, fallback)
    }

    fn get_token(&self, token_script: &str, edition_code: &str) -> (String, usize) {
        let script = token_script.to_lowercase();
        let code = edition_code.to_uppercase();
        if let Some(&count) = self.token_art_variants.get(&(script.clone(), code.clone())) {
            return (code, count);
        }
        self.fallback_token(&script).unwrap_or((code, 1))
    }

    /// Keep in sync with `TokenDb.getTokenFromEditions` with no edition filter: editions
    /// iterate in case-insensitive code order and the first that has the token wins.
    fn fallback_token(&self, script: &str) -> Option<(String, usize)> {
        self.token_art_variants
            .iter()
            .filter(|((name, _), _)| name == script)
            .min_by_key(|((_, code), _)| code.to_lowercase())
            .map(|((_, code), &count)| (code.clone(), count))
    }

    pub fn sync_token_art_rng(&mut self, token_script: &str, sa: &SpellAbility) -> String {
        let host_edition = sa
            .original_host
            .or(sa.source)
            .and_then(|cid| self.game.card(cid).set_code.clone())
            .unwrap_or_default();
        let pinned = self.game.token_edition_pins.get(token_script).cloned();
        let edition = pinned.clone().unwrap_or_else(|| {
            self.token_set(token_script, &host_edition)
                .unwrap_or(host_edition)
        });
        let (token_edition, art_count) = self.get_token(token_script, &edition);
        if art_count > 1 {
            self.rng.next_int(art_count as i32);
        }
        if pinned.is_none() {
            self.game
                .token_edition_pins
                .insert(token_script.to_string(), token_edition.clone());
        }
        self.rng.next_int(1);
        token_edition
    }

    pub fn move_card(&mut self, card_id: CardId, dest_zone: ZoneType, dest_owner: PlayerId) {
        let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
            trigger_handler: self.trigger_handler,
            token_templates: self.token_templates,
            token_art_variants: self.token_art_variants,
            token_fallback: self.token_fallback,
            edition_dates: self.edition_dates,
            mana_pools: self.mana_pools,
            rng: self.rng,
        };
        self.game.move_card_with_agents_and_replacement_runtime(
            card_id,
            dest_zone,
            dest_owner,
            self.agents,
            &mut runtime,
        );
    }

    pub(crate) fn deal_damage(
        &mut self,
        source: CardId,
        target: crate::card::card_damage_map::DamageTarget,
        amount: i32,
    ) -> (GameEntity, i32) {
        let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
            trigger_handler: self.trigger_handler,
            token_templates: self.token_templates,
            token_art_variants: self.token_art_variants,
            token_fallback: self.token_fallback,
            edition_dates: self.edition_dates,
            mana_pools: self.mana_pools,
            rng: self.rng,
        };
        self.game
            .deal_damage(source, target, amount, self.agents, &mut runtime)
    }

    pub(crate) fn sacrifice_destroy(
        &mut self,
        card_id: CardId,
        lki_p1p1: i32,
        lki_power: i32,
        lki_toughness: i32,
    ) {
        let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
            trigger_handler: self.trigger_handler,
            token_templates: self.token_templates,
            token_art_variants: self.token_art_variants,
            token_fallback: self.token_fallback,
            edition_dates: self.edition_dates,
            mana_pools: self.mana_pools,
            rng: self.rng,
        };
        self.game.sacrifice_destroy(
            card_id,
            self.agents,
            &mut runtime,
            lki_p1p1,
            lki_power,
            lki_toughness,
        );
    }

    pub fn discard_card(
        &mut self,
        card_id: CardId,
        discard_player: PlayerId,
        sa: Option<&SpellAbility>,
    ) {
        let mut runtime = crate::replacement::replacement_handler::ReplacementRuntime {
            trigger_handler: self.trigger_handler,
            token_templates: self.token_templates,
            token_art_variants: self.token_art_variants,
            token_fallback: self.token_fallback,
            edition_dates: self.edition_dates,
            mana_pools: self.mana_pools,
            rng: self.rng,
        };
        self.game
            .discard_card(card_id, discard_player, sa, Some(self.agents), &mut runtime);
    }

    pub(crate) fn add_counter(
        &mut self,
        card_id: CardId,
        counter_type: &CounterType,
        amount: i32,
        sa: &SpellAbility,
        mut params: RunParams,
    ) -> i32 {
        params.source_player.get_or_insert(sa.activating_player);
        params.cause.get_or_insert_with(|| sa.clone());
        add_counter_with_context(
            self.game,
            Some(self.trigger_handler),
            Some(self.agents),
            card_id,
            counter_type,
            amount,
            params,
            true,
        )
    }

    pub(crate) fn add_player_counter(
        &mut self,
        player: PlayerId,
        counter_type: &CounterType,
        amount: i32,
        sa: &SpellAbility,
        mut params: RunParams,
    ) -> i32 {
        params.source_player.get_or_insert(sa.activating_player);
        params.cause.get_or_insert_with(|| sa.clone());
        let source = params.source_player;
        let mut table = GameEntityCounterTable::default();
        table.put(
            source,
            GameEntity::Player(player),
            counter_type.clone(),
            amount,
        );
        table
            .replace_counter_effect(
                self.game,
                Some(self.trigger_handler),
                Some(self.agents),
                Some(sa),
                true,
                params,
            )
            .get(source, GameEntity::Player(player), counter_type)
    }
}
