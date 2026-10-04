use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Once, OnceLock};

use forge_carddb::CardDatabase;
use forge_foundation::ZoneType;
use serde::{Deserialize, Serialize};

use crate::agent::GameEntity;
use crate::card::card_damage_history::TrackedEntity;
use crate::card::card_damage_map::CardDamageMap;
use crate::card::card_zone_table::CardZoneTable;
use crate::card::Card;
use crate::card::CounterType;
use crate::ids::{CardId, PlayerId};
use crate::phase::ExtraTurn;
use crate::phase::Phase;
use crate::phase::TurnState;
use crate::player::PlayerState;
use crate::spellability::MagicStack;
use crate::zone::{CostPaymentStack, Zone, ZoneKey, ZoneStore};

/// Global registry of type lists loaded from `TypeLists.txt`.
///
/// Mirrors Java's `CardType.Constant.CREATURE_TYPES` etc., populated once by
/// `FModel.loadDynamicGamedata()` → `CardType.Helper.parseTypes()`.
///
/// Call [`TypeRegistry::load`] once at startup with the contents of
/// `TypeLists.txt`. All subsequent calls to [`TypeRegistry::creature_types`]
/// return the loaded data without any per-game copying.
pub struct TypeRegistry;

static CREATURE_TYPES: OnceLock<Vec<String>> = OnceLock::new();
static SUBTYPE_SECTIONS: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();

impl TypeRegistry {
    /// Load creature types from the raw contents of `TypeLists.txt` and of every
    /// edition file.
    ///
    /// Parses each `[CreatureTypes]` section. Each line is either `TypeName` or
    /// `TypeName:PluralName`; only the singular (left of `:`) is kept, and a type
    /// already listed is skipped. Edition files add their own types (the Un-sets'
    /// `[CreatureTypes]`), as `CardEdition.Reader` passes them to `parseTypes`.
    ///
    /// Mirrors Java's `FileSection.parseSections()` + `CardType.Helper.parseTypes()`.
    ///
    /// This must be called once before any game starts. Subsequent calls are
    /// silently ignored (first write wins).
    pub fn load<'a>(type_lists_content: &str, edition_texts: impl IntoIterator<Item = &'a str>) {
        let mut types = Vec::new();
        Self::parse_creature_types(type_lists_content, &mut types);
        for edition in edition_texts {
            Self::parse_creature_types(edition, &mut types);
        }
        let _ = CREATURE_TYPES.set(types);
        let _ = SUBTYPE_SECTIONS.set(Self::parse_subtype_sections(type_lists_content));
    }

    /// Whether `subtype` is listed under `[section]` of `TypeLists.txt` (`LandTypes`,
    /// `ArtifactTypes`, ...). False before [`TypeRegistry::load`].
    pub fn is_subtype_in(section: &str, subtype: &str) -> bool {
        SUBTYPE_SECTIONS
            .get()
            .and_then(|sections| sections.get(section))
            .is_some_and(|types| types.iter().any(|ty| ty.eq_ignore_ascii_case(subtype)))
    }

    pub fn subtype_sections_loaded() -> bool {
        SUBTYPE_SECTIONS.get().is_some()
    }

    /// Java `CardType.isALandType`: a basic land type or another land type.
    pub fn is_land_type(subtype: &str) -> bool {
        Self::is_subtype_in("BasicTypes", subtype) || Self::is_subtype_in("LandTypes", subtype)
    }

    fn parse_subtype_sections(content: &str) -> BTreeMap<String, Vec<String>> {
        let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = Some(line[1..line.len() - 1].to_string());
                continue;
            }
            if let Some(section) = &current {
                let singular = line.split(':').next().unwrap_or(line);
                if !singular.is_empty() {
                    sections
                        .entry(section.clone())
                        .or_default()
                        .push(singular.to_string());
                }
            }
        }
        sections
    }

    /// Return the loaded creature types.
    ///
    /// # Panics
    /// Panics if [`TypeRegistry::load`] has not been called.
    pub fn creature_types() -> &'static [String] {
        CREATURE_TYPES.get().expect(
            "TypeRegistry: creature types not loaded. \
             Call TypeRegistry::load() with the contents of TypeLists.txt before starting a game.",
        )
    }

    /// Return whether `creature_type` is a known creature subtype.
    ///
    /// Unlike [`TypeRegistry::creature_types`], this is safe to call in unit
    /// tests that haven't loaded type data yet; it simply returns `false`.
    pub fn is_creature_type(creature_type: &str) -> bool {
        CREATURE_TYPES.get().is_some_and(|types| {
            types
                .iter()
                .any(|ty| ty.eq_ignore_ascii_case(creature_type))
        })
    }

    fn parse_creature_types(content: &str, types: &mut Vec<String>) {
        let mut in_creature_section = false;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_creature_section = &line[1..line.len() - 1] == "CreatureTypes";
                continue;
            }
            if in_creature_section {
                // "TypeName" or "TypeName:PluralName" — keep singular only
                let singular = line.split(':').next().unwrap_or(line);
                if !singular.is_empty() && !types.iter().any(|ty| ty == singular) {
                    types.push(singular.to_string());
                }
            }
        }
    }
}

static CARD_DATABASE: OnceLock<Arc<CardDatabase>> = OnceLock::new();
static CARD_DATABASE_PARSED: Once = Once::new();

pub struct CardDatabaseRegistry;

impl CardDatabaseRegistry {
    pub fn load(database: Arc<CardDatabase>) {
        let _ = CARD_DATABASE.set(database);
    }

    pub fn get() -> Option<&'static CardDatabase> {
        CARD_DATABASE.get().map(Arc::as_ref)
    }

    pub fn all() -> Option<&'static CardDatabase> {
        let database = Self::get()?;
        CARD_DATABASE_PARSED.call_once(|| {
            database.force_parse_all();
        });
        Some(database)
    }
}

/// The complete, serializable game state.
/// All game entities live here — nothing holds references, everything uses IDs.
#[derive(Debug, Clone)]
pub struct DamageThisTurnLki {
    pub history: CardId,
    pub index: usize,
    pub source: Arc<Card>,
    pub target: DamageLkiTarget,
}

#[derive(Debug, Clone)]
pub enum DamageLkiTarget {
    Player(PlayerId),
    Card(Arc<Card>),
}

/// Every mutable borrow takes a stamp no other borrow on any thread has taken, so an
/// unchanged stamp proves the value is unchanged (`apply_continuous_effects` relies on it).
#[derive(Clone)]
pub struct Tracked<T> {
    value: T,
    stamp: u64,
    #[cfg(feature = "layer-skip-stats")]
    writer: Option<&'static std::panic::Location<'static>>,
}

fn next_layer_stamp() -> u64 {
    const BLOCK: u64 = 1 << 32;
    static NEXT_BLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    thread_local! {
        static NEXT: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    }
    NEXT.with(|next| {
        let (mut stamp, mut end) = next.get();
        if stamp == end {
            stamp = NEXT_BLOCK.fetch_add(BLOCK, std::sync::atomic::Ordering::Relaxed);
            end = stamp + BLOCK;
        }
        next.set((stamp + 1, end));
        stamp
    })
}

impl<T> Tracked<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            stamp: next_layer_stamp(),
            #[cfg(feature = "layer-skip-stats")]
            writer: None,
        }
    }

    pub fn stamp(&self) -> u64 {
        self.stamp
    }
}

impl<T> std::ops::Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> std::ops::DerefMut for Tracked<T> {
    #[cfg_attr(feature = "layer-skip-stats", track_caller)]
    fn deref_mut(&mut self) -> &mut T {
        self.stamp = next_layer_stamp();
        #[cfg(feature = "layer-skip-stats")]
        {
            self.writer = Some(std::panic::Location::caller());
        }
        &mut self.value
    }
}

impl<'a, T> IntoIterator for &'a Tracked<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        (&self.value).into_iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Tracked<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;

    #[cfg_attr(feature = "layer-skip-stats", track_caller)]
    fn into_iter(self) -> Self::IntoIter {
        (&mut **self).into_iter()
    }
}

impl<T: Default> Default for Tracked<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Tracked<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: Serialize> Serialize for Tracked<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Tracked<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}

/// Everything `apply_continuous_effects` reads. `priority_player` is left out because no
/// static ability, condition or count reads it, and it changes on every pass of priority.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerKey {
    stamps: [u64; 11],
    turn_number: u32,
    active_player: PlayerId,
    player_previous_turn: Option<PlayerId>,
    phase: forge_foundation::PhaseType,
    is_extra_turn: bool,
    num_players: u32,
    combat_attackers_declared: bool,
    combat_blockers_declared: bool,
    combat_block_assignments: Vec<(CardId, CardId)>,
    drawn_for_turn: bool,
    n_upkeeps_this_turn: i32,
    n_combats_this_turn: i32,
    n_end_of_turns_this_turn: i32,
    extra_phases: usize,
    is_night: bool,
    day_night_started: bool,
    player_order: Vec<PlayerId>,
    game_over: bool,
    winner: Option<PlayerId>,
    extra_turns: usize,
    prevent_all_combat_damage: bool,
    monarch: Option<PlayerId>,
    initiative_holder: Option<PlayerId>,
    end_turn_requested: bool,
    end_combat_requested: bool,
    mirror_forge_bugs: bool,
    next_card_id: u32,
    next_zone_timestamp: u64,
    card_names_unchanged: bool,
    last_sacrificed_card: Option<CardId>,
}

/// A cache, not game state: its Debug prints nothing of it, so two equal games format the
/// same whether or not their layer pass has run (checkpoint digests, the layer-skip verify).
#[derive(Clone, Default)]
pub struct LayerKeyCache(
    pub Option<LayerKey>,
    #[cfg(feature = "layer-skip-stats")]
    pub  Option<Arc<crate::staticability::layer_skip_stats::LayerStatsSnapshot>>,
);

#[cfg(feature = "layer-skip-stats")]
pub const LAYER_KEY_FIELDS: [&str; 11] = [
    "cards",
    "players",
    "zones",
    "stack",
    "last_state_battlefield",
    "last_state_battlefield_combat_lki",
    "change_zone_lki_info",
    "counter_added_this_turn",
    "left_battlefield_this_turn",
    "left_graveyard_this_turn",
    "damage_this_turn_lki",
];

#[cfg(feature = "layer-skip-stats")]
impl LayerKey {
    pub fn moved_fields(&self, other: &Self) -> ([bool; 11], bool) {
        let moved = std::array::from_fn(|index| self.stamps[index] != other.stamps[index]);
        let scalars = LayerKey {
            stamps: [0; 11],
            ..self.clone()
        } != LayerKey {
            stamps: [0; 11],
            ..other.clone()
        };
        (moved, scalars)
    }

    pub fn moved_scalars(&self, other: &Self) -> Vec<&'static str> {
        let mut moved = Vec::new();
        macro_rules! check {
            ($($field:ident),*) => {
                $(if self.$field != other.$field {
                    moved.push(stringify!($field));
                })*
            };
        }
        check!(
            turn_number,
            active_player,
            player_previous_turn,
            phase,
            is_extra_turn,
            num_players,
            combat_attackers_declared,
            combat_blockers_declared,
            combat_block_assignments,
            drawn_for_turn,
            n_upkeeps_this_turn,
            n_combats_this_turn,
            n_end_of_turns_this_turn,
            extra_phases,
            is_night,
            day_night_started,
            player_order,
            game_over,
            winner,
            extra_turns,
            prevent_all_combat_damage,
            monarch,
            initiative_holder,
            end_turn_requested,
            end_combat_requested,
            mirror_forge_bugs,
            next_card_id,
            next_zone_timestamp,
            card_names_unchanged,
            last_sacrificed_card
        );
        moved
    }
}

impl std::fmt::Debug for LayerKeyCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LayerKeyCache")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    // Arenas
    pub cards: Tracked<Vec<Arc<Card>>>,
    pub players: Tracked<Vec<PlayerState>>,

    // Zones: keyed by (ZoneType, PlayerId)
    #[serde(skip)]
    zones: Tracked<ZoneStore>,

    // The stack
    pub stack: Tracked<MagicStack>,

    /// Cost payment tracking stack — used by triggers to inspect cost payments.
    /// Mirrors Java's `Game.costPaymentStack`.
    #[serde(skip)]
    pub cost_payment_stack: CostPaymentStack,

    // Day/Night cycle (Innistrad DFC mechanic)
    pub is_night: bool,
    pub day_night_started: bool,

    // Turn/phase state
    pub turn: TurnState,
    pub begin_of_combat: Phase,
    pub end_of_combat: Phase,
    #[serde(default)]
    pub end_of_turn: Phase,
    #[serde(default)]
    pub last_copied_replacement_id: i32,
    #[serde(default)]
    pub copied_replacement_ids: Vec<((CardId, i32), i32)>,
    pub cleanup: Phase,
    #[serde(default)]
    pub leaves_play_commands: Vec<(CardId, crate::phase::PhaseCommand)>,
    #[serde(default)]
    pub untap_commands: Vec<(CardId, crate::phase::PhaseCommand)>,
    #[serde(default)]
    pub change_controller_commands: Vec<(CardId, crate::phase::PhaseCommand)>,
    #[serde(default)]
    pub phase_out_commands: Vec<(CardId, crate::phase::PhaseCommand)>,
    #[serde(default)]
    pub facedown_commands: Vec<(CardId, crate::phase::PhaseCommand)>,

    // Player order (for turn sequence)
    pub player_order: Vec<PlayerId>,

    // Game over flag
    pub game_over: bool,
    pub winner: Option<PlayerId>,

    // Extra turns queue — players who get extra turns (issue #22, AddTurn effect).
    // After cleanup, the game pops from here instead of advancing to the next player.
    #[serde(skip)]
    pub extra_turns: VecDeque<ExtraTurn>,

    // Fog — prevent all combat damage this turn (issue #22, Fog effect).
    // Reset at end of turn cleanup.
    pub prevent_all_combat_damage: bool,

    // Monarch designation (issue #22, BecomeMonarch effect).
    pub monarch: Option<PlayerId>,

    // Initiative holder (issue #22, TakeInitiative effect).
    pub initiative_holder: Option<PlayerId>,

    // End turn requested — skip remaining phases, jump to cleanup (issue #22, EndTurn effect).
    pub end_turn_requested: bool,

    // End combat requested — skip remaining combat steps (issue #22, EndCombatPhase effect).
    pub end_combat_requested: bool,

    #[serde(default)]
    pub action_space_mana_probe: crate::mana::ActionSpaceManaProbe,

    #[serde(default)]
    pub mirror_forge_bugs: bool,

    // Next card ID counter
    next_card_id: u32,

    /// Monotonically increasing counter for zone-entry timestamps.
    /// Each time a card enters a zone, it gets the next value.
    /// Used to order same-player triggers by zone entry order,
    /// matching Java's `Zone.cardList` insertion order.
    next_zone_timestamp: u64,
    /// Shared damage aggregation map for Java-style `DamageMap` flows.
    /// Used across sub-ability chains and consumed by `DamageResolve`.
    #[serde(skip)]
    pub pending_damage_map: Option<CardDamageMap>,
    /// Shared prevention map paired with `pending_damage_map`.
    #[serde(skip)]
    pub pending_prevent_map: Option<CardDamageMap>,
    /// Shared zone-change aggregation table for Java-style `ChangeZoneTable` flows.
    /// Used across sub-ability chains and consumed by `ChangeZoneResolve`.
    #[serde(skip)]
    pub pending_change_zone_table: Option<CardZoneTable>,
    /// Open batch of cards each player discarded for one event; `DiscardedAll` fires once per
    /// player when it closes, as Java's `SpellAbilityEffect.discard`/`CostDiscard` do.
    #[serde(skip)]
    pub pending_discard_batch: Option<crate::HashMap<PlayerId, Vec<CardId>>>,
    /// Keep in sync with `ReplacementEffect.hasRun`: a replacement is skipped by any event
    /// raised while its own replacement runs.
    #[serde(skip)]
    pub replacements_running: crate::HashSet<(CardId, usize, i32)>,
    #[serde(skip)]
    pub hold_checking_static_abilities: bool,
    /// Keep in sync with `Card::add_changed_name`, the only writer of the `OriginalName`
    /// SVar: while this holds, no card carries one.
    #[serde(skip)]
    pub card_names_unchanged: bool,
    /// The last state-based check applied static abilities and its final pass changed nothing.
    #[serde(skip)]
    pub statics_current_after_sba: bool,

    #[serde(skip)]
    pub pending_remove_from_combat: Vec<CardId>,

    #[serde(skip)]
    pub token_edition_pins: std::collections::BTreeMap<String, String>,

    /// Periodic LKI snapshot of battlefield cards.
    /// Mirrors Java's `Game.lastStateBattlefield`.
    /// Updated by `copy_last_state()` at key game checkpoints.
    #[serde(skip)]
    pub last_state_battlefield: Tracked<Vec<crate::lki::CardSnapshot>>,

    #[serde(skip)]
    pub last_state_graveyard: Vec<CardId>,

    #[serde(skip)]
    pub last_state_battlefield_combat_lki: Tracked<Vec<(CardId, Option<bool>)>>,

    /// Snapshot of cards on the battlefield at the start of the current SBA check.
    /// Used by `DisableTriggers` (Hushbringer) to check LKI — a creature that dies
    /// in the same batch as another creature still suppresses the other's death trigger.
    /// Mirrors Java's `LastStateBattlefield` passed through `RunParams`.
    /// Set at the start of `check_state_based_actions_with_triggers`, cleared after.
    #[serde(skip)]
    pub pre_sba_battlefield: Vec<CardId>,

    #[serde(skip)]
    pub replacement_last_state_battlefield: Option<Vec<CardId>>,

    #[serde(skip)]
    pub change_zone_lki_info: Tracked<crate::HashMap<CardId, Arc<Card>>>,

    /// Last card sacrificed as a cost (for `Sacrificed$CardPower` SVar resolution).
    /// Mirrors Java's `sa.getPaidList("SacrificedCards")`.
    #[serde(skip)]
    pub last_sacrificed_card: Option<CardId>,
    #[serde(skip)]
    pub counter_added_this_turn:
        Tracked<BTreeMap<(GameEntity, Option<u64>, CounterType, Option<PlayerId>), i32>>,
    #[serde(skip)]
    pub left_battlefield_this_turn: Tracked<Vec<CardId>>,
    #[serde(skip)]
    pub left_graveyard_this_turn: Tracked<Vec<CardId>>,
    #[serde(skip)]
    pub damage_this_turn_lki: Tracked<Vec<DamageThisTurnLki>>,
    #[serde(skip)]
    pub granted_trigger_ids: crate::HashMap<(CardId, u64, Option<CardId>, u64, String), u32>,
    #[serde(skip)]
    pub layer_key_after_pass: LayerKeyCache,
}

impl GameState {
    pub fn new(player_names: &[&str], starting_life: i32) -> Self {
        let mut players = Vec::new();
        let mut player_order = Vec::new();

        for (i, name) in player_names.iter().enumerate() {
            let pid = PlayerId(i as u32);
            players.push(PlayerState::new(pid, name.to_string(), starting_life));
            player_order.push(pid);
        }

        let zones = ZoneStore::new(&player_order);

        GameState {
            cards: Tracked::new(Vec::new()),
            players: Tracked::new(players),
            zones: Tracked::new(zones),
            stack: Tracked::new(MagicStack::new()),
            cost_payment_stack: CostPaymentStack::new(),
            is_night: false,
            day_night_started: false,
            turn: TurnState::new(player_order[0], player_order.len() as u32),
            begin_of_combat: Phase::new(forge_foundation::PhaseType::CombatBegin),
            end_of_combat: Phase::new(forge_foundation::PhaseType::CombatEnd),
            end_of_turn: Phase::new(forge_foundation::PhaseType::EndOfTurn),
            last_copied_replacement_id: 1 << 24,
            copied_replacement_ids: Vec::new(),
            cleanup: Phase::new(forge_foundation::PhaseType::Cleanup),
            leaves_play_commands: Vec::new(),
            untap_commands: Vec::new(),
            change_controller_commands: Vec::new(),
            phase_out_commands: Vec::new(),
            facedown_commands: Vec::new(),
            player_order,
            game_over: false,
            winner: None,
            extra_turns: VecDeque::new(),
            prevent_all_combat_damage: false,
            monarch: None,
            initiative_holder: None,
            end_turn_requested: false,
            end_combat_requested: false,
            action_space_mana_probe: crate::mana::ActionSpaceManaProbe::default(),
            mirror_forge_bugs: false,
            next_card_id: 0,
            next_zone_timestamp: 0,
            pending_damage_map: None,
            pending_prevent_map: None,
            pending_change_zone_table: None,
            pending_discard_batch: None,
            replacements_running: crate::HashSet::default(),
            hold_checking_static_abilities: false,
            card_names_unchanged: true,
            statics_current_after_sba: false,
            pending_remove_from_combat: Vec::new(),
            token_edition_pins: std::collections::BTreeMap::new(),
            last_state_battlefield: Tracked::default(),
            last_state_graveyard: Vec::new(),
            last_state_battlefield_combat_lki: Tracked::default(),
            pre_sba_battlefield: Vec::new(),
            replacement_last_state_battlefield: None,
            change_zone_lki_info: Tracked::default(),
            last_sacrificed_card: None,
            counter_added_this_turn: Tracked::default(),
            left_battlefield_this_turn: Tracked::default(),
            left_graveyard_this_turn: Tracked::default(),
            damage_this_turn_lki: Tracked::default(),
            granted_trigger_ids: crate::HashMap::default(),
            layer_key_after_pass: LayerKeyCache::default(),
        }
    }

    /// Create a new card instance and return its ID. Does NOT place it in a zone.
    pub fn create_card(&mut self, mut card: Card) -> CardId {
        let id = CardId(self.next_card_id);
        self.next_card_id += 1;
        card.id = id;
        for trigger in &mut card.triggers {
            trigger.bind_host_card_id(id);
        }
        for static_ability in &mut card.static_abilities {
            static_ability.base.set_host_card_id(id);
        }
        for replacement_effect in &mut card.replacement_effects {
            replacement_effect.base.set_host_card_id(id);
        }
        if let Some(other) = card.other_part.as_mut() {
            for trigger in &mut other.triggers {
                trigger.bind_host_card_id(id);
            }
            for static_ability in &mut other.static_abilities {
                static_ability.base.set_host_card_id(id);
            }
            for replacement_effect in &mut other.replacement_effects {
                replacement_effect.base.set_host_card_id(id);
            }
        }
        card.bind_trait_base_hosts(id);
        self.cards.push(Arc::new(card));
        id
    }

    // --- Accessors ---

    pub fn card(&self, id: CardId) -> &Card {
        &self.cards[id.index()]
    }

    #[cfg_attr(feature = "layer-skip-stats", track_caller)]
    pub fn card_mut(&mut self, id: CardId) -> &mut Card {
        Arc::make_mut(&mut self.cards[id.index()])
    }

    pub fn player(&self, id: PlayerId) -> &PlayerState {
        &self.players[id.index()]
    }

    #[cfg_attr(feature = "layer-skip-stats", track_caller)]
    pub fn player_mut(&mut self, id: PlayerId) -> &mut PlayerState {
        &mut self.players[id.index()]
    }

    pub fn zone(&self, zone_type: ZoneType, owner: PlayerId) -> &Zone {
        self.zones.get(zone_type, owner).expect("Zone not found")
    }

    #[cfg_attr(feature = "layer-skip-stats", track_caller)]
    pub fn zone_mut(&mut self, zone_type: ZoneType, owner: PlayerId) -> &mut Zone {
        self.zones
            .get_mut(zone_type, owner)
            .expect("Zone not found")
    }

    pub fn zone_store_snapshot(&self) -> ZoneStore {
        (*self.zones).clone()
    }

    pub fn replace_zone_store(&mut self, zones: ZoneStore) {
        self.zones = Tracked::new(zones);
    }

    #[cfg(feature = "layer-skip-stats")]
    pub fn layer_key_writers(&self) -> [Option<&'static std::panic::Location<'static>>; 11] {
        [
            self.cards.writer,
            self.players.writer,
            self.zones.writer,
            self.stack.writer,
            self.last_state_battlefield.writer,
            self.last_state_battlefield_combat_lki.writer,
            self.change_zone_lki_info.writer,
            self.counter_added_this_turn.writer,
            self.left_battlefield_this_turn.writer,
            self.left_graveyard_this_turn.writer,
            self.damage_this_turn_lki.writer,
        ]
    }

    pub fn verify_sample_key(&self) -> u64 {
        let mut key = u64::from(self.turn.turn_number)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(self.turn.phase as u64)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(self.next_zone_timestamp)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(u64::from(self.next_card_id));
        key ^= key >> 31;
        key
    }

    pub fn layer_key(&self) -> LayerKey {
        let turn = &self.turn;
        LayerKey {
            stamps: [
                self.cards.stamp(),
                self.players.stamp(),
                self.zones.stamp(),
                self.stack.stamp(),
                self.last_state_battlefield.stamp(),
                self.last_state_battlefield_combat_lki.stamp(),
                self.change_zone_lki_info.stamp(),
                self.counter_added_this_turn.stamp(),
                self.left_battlefield_this_turn.stamp(),
                self.left_graveyard_this_turn.stamp(),
                self.damage_this_turn_lki.stamp(),
            ],
            turn_number: turn.turn_number,
            active_player: turn.active_player,
            player_previous_turn: turn.player_previous_turn,
            phase: turn.phase,
            is_extra_turn: turn.is_extra_turn,
            num_players: turn.num_players,
            combat_attackers_declared: turn.combat_attackers_declared,
            combat_blockers_declared: turn.combat_blockers_declared,
            combat_block_assignments: turn.combat_block_assignments.clone(),
            drawn_for_turn: turn.drawn_for_turn,
            n_upkeeps_this_turn: turn.n_upkeeps_this_turn,
            n_combats_this_turn: turn.n_combats_this_turn,
            n_end_of_turns_this_turn: turn.n_end_of_turns_this_turn,
            extra_phases: turn.extra_phases.len(),
            is_night: self.is_night,
            day_night_started: self.day_night_started,
            player_order: self.player_order.clone(),
            game_over: self.game_over,
            winner: self.winner,
            extra_turns: self.extra_turns.len(),
            prevent_all_combat_damage: self.prevent_all_combat_damage,
            monarch: self.monarch,
            initiative_holder: self.initiative_holder,
            end_turn_requested: self.end_turn_requested,
            end_combat_requested: self.end_combat_requested,
            mirror_forge_bugs: self.mirror_forge_bugs,
            next_card_id: self.next_card_id,
            next_zone_timestamp: self.next_zone_timestamp,
            card_names_unchanged: self.card_names_unchanged,
            last_sacrificed_card: self.last_sacrificed_card,
        }
    }

    pub fn iter_zones(&self) -> impl Iterator<Item = (ZoneKey, &Zone)> {
        self.zones.iter()
    }

    pub fn zones_stamp(&self) -> u64 {
        self.zones.stamp()
    }

    pub fn cards_in_all_zones(&self, zone_type: ZoneType) -> impl Iterator<Item = CardId> + '_ {
        self.iter_zones()
            .filter(move |(key, _)| key.zone_type == zone_type)
            .flat_map(|(_, zone)| zone.cards.iter().copied())
    }

    pub fn card_zone_location(&self, card: CardId) -> Option<ZoneKey> {
        self.zones.card_location(card)
    }

    pub fn card_zone(&self, card: CardId) -> Option<ZoneType> {
        self.card_zone_location(card)
            .map(|location| location.zone_type)
    }

    pub fn card_current_zone(&self, card: CardId) -> ZoneType {
        self.card_zone(card).unwrap_or_else(|| self.card(card).zone)
    }

    pub fn card_is_in_zone(&self, card: CardId, zone: ZoneType) -> bool {
        self.card_current_zone(card) == zone
    }

    pub fn card_zone_owner(&self, card: CardId) -> Option<PlayerId> {
        self.card_zone_location(card).map(|location| location.owner)
    }

    pub fn card_zone_location_matches_card(&self, card: CardId) -> bool {
        let card_ref = self.card(card);
        match self.card_zone_location(card) {
            Some(location) => {
                location.zone_type == card_ref.zone && location.owner == card_ref.controller
            }
            None => card_ref.zone == ZoneType::None,
        }
    }

    pub fn add_left_battlefield_this_turn(&mut self, lki: CardId) {
        self.left_battlefield_this_turn.push(lki);
    }

    pub fn add_left_graveyard_this_turn(&mut self, lki: CardId) {
        self.left_graveyard_this_turn.push(lki);
    }

    pub fn clear_left_battlefield_this_turn(&mut self) {
        self.left_battlefield_this_turn.clear();
    }

    pub fn clear_left_graveyard_this_turn(&mut self) {
        self.left_graveyard_this_turn.clear();
    }

    pub fn register_damage(
        &mut self,
        source: CardId,
        damage: i32,
        is_combat: bool,
        target: TrackedEntity,
    ) {
        if damage <= 0 {
            return;
        }
        self.card_mut(source).damage_history.register_damage(
            damage,
            is_combat,
            Some(source),
            target,
        );
        let index = self.card(source).damage_history.damage_done_this_turn.len() - 1;
        self.add_global_damage_history(source, index, target);
    }

    fn add_global_damage_history(&mut self, history: CardId, index: usize, target: TrackedEntity) {
        let target = match target {
            TrackedEntity::Player(player) => DamageLkiTarget::Player(player),
            TrackedEntity::Card(card) => {
                DamageLkiTarget::Card(Arc::clone(&self.cards[card.index()]))
            }
        };
        self.damage_this_turn_lki.push(DamageThisTurnLki {
            history,
            index,
            source: Arc::clone(&self.cards[history.index()]),
            target,
        });
    }

    pub fn clear_global_damage_history(&mut self) {
        self.damage_this_turn_lki.clear();
    }

    pub fn get_damage_done_this_turn(
        &self,
        is_combat: Option<bool>,
        valid_source_card: &str,
        valid_target_entity: &str,
        source: CardId,
        source_controller: PlayerId,
    ) -> Vec<i32> {
        let source_card = self.card(source);
        let source_selectors: Vec<_> = valid_source_card
            .split(',')
            .map(crate::parsing::cached_compiled_selector)
            .collect();
        let target_selectors: Vec<_> = valid_target_entity
            .split(',')
            .map(crate::parsing::cached_compiled_selector)
            .collect();
        let is_valid = |lki: &DamageThisTurnLki| {
            source_selectors.iter().any(|selector| {
                crate::card::valid_filter::matches_valid_card_selector_in_game(
                    selector,
                    &lki.source,
                    source_card,
                    self,
                )
            }) && match &lki.target {
                DamageLkiTarget::Player(player) => {
                    crate::card::valid_filter::matches_valid_player_selector_in_game(
                        &crate::parsing::cached_compiled_selector(valid_target_entity),
                        *player,
                        source_card,
                        source_controller,
                        self,
                    )
                }
                DamageLkiTarget::Card(card) => target_selectors.iter().any(|selector| {
                    crate::card::valid_filter::matches_valid_card_selector_in_game(
                        selector,
                        card,
                        source_card,
                        self,
                    )
                }),
            }
        };
        let mut per_history: Vec<((CardId, u64), i32)> = Vec::new();
        for lki in &self.damage_this_turn_lki {
            let damage = &lki.source.damage_history.damage_done_this_turn[lki.index];
            if is_combat.is_some_and(|combat| damage.is_combat != combat) || !is_valid(lki) {
                continue;
            }
            let history = (lki.history, lki.source.zone_timestamp);
            match per_history.iter_mut().find(|(key, _)| *key == history) {
                Some((_, dmg)) => *dmg += damage.amount,
                None => per_history.push((history, damage.amount)),
            }
        }
        per_history
            .into_iter()
            .filter_map(|(_, dmg)| (dmg != 0).then_some(dmg))
            .collect()
    }

    pub fn is_void(&self) -> bool {
        self.left_battlefield_this_turn
            .iter()
            .any(|&card| !self.card(card).is_land())
            || self.stack.get_spells_cast_this_turn().iter().any(|&card| {
                self.card(card).cast_sa.as_ref().is_some_and(|sa| {
                    sa.alt_cost == Some(crate::spellability::AlternativeCost::Warp)
                })
            })
    }

    pub fn reset_zone_turn_tracking(&mut self) {
        for zone in self.zones.values_mut() {
            zone.reset_cards_added_this_turn();
        }
    }

    pub fn reset_card_turn_tracking(&mut self) {
        self.counter_added_this_turn.clear();
        for card in &mut self.cards {
            if card.turn_tracking_is_clear() {
                continue;
            }
            let card = Arc::make_mut(card);
            card.reset_activations_per_turn();
            card.reset_ability_resolved_this_turn();
        }
    }

    pub fn counter_added_this_turn(
        &self,
        entity: GameEntity,
        counter_type: Option<&CounterType>,
    ) -> i32 {
        self.counter_added_this_turn
            .iter()
            .filter(|((entry_entity, timestamp, entry_type, _), _)| {
                *entry_entity == entity
                    && *timestamp == self.counter_entity_timestamp(entity)
                    && counter_type.is_none_or(|ct| ct == entry_type)
            })
            .map(|(_, amount)| *amount)
            .fold(0, |total, amount| self.add_counter_totals(total, amount))
    }

    pub fn get_counter_added_this_turn(
        &self,
        counter_type: Option<&CounterType>,
        valid_player: &str,
        valid_card: &str,
        source: CardId,
        source_controller: PlayerId,
    ) -> i32 {
        let source_card = self.card(source);
        let card_selectors: Vec<_> = valid_card
            .split(',')
            .map(crate::parsing::cached_compiled_selector)
            .collect();
        self.counter_added_this_turn
            .iter()
            .filter(|((entity, _, entry_type, putter), _)| {
                counter_type.is_none_or(|ct| ct == entry_type)
                    && putter.is_some_and(|putter| {
                        crate::card::valid_filter::matches_valid_player_selector_in_game(
                            &crate::parsing::cached_compiled_selector(valid_player),
                            putter,
                            source_card,
                            source_controller,
                            self,
                        )
                    })
                    && match entity {
                        GameEntity::Card(card) => card_selectors.iter().any(|selector| {
                            crate::card::valid_filter::matches_valid_card_selector_in_game(
                                selector,
                                self.card(*card),
                                source_card,
                                self,
                            )
                        }),
                        GameEntity::Player(_) => false,
                    }
            })
            .map(|(_, amount)| *amount)
            .fold(0, |total, amount| self.add_counter_totals(total, amount))
    }

    /// Java sums counter totals in an `int`, which wraps (FORGE BUG, mirrored under
    /// `mirror_forge_bugs`); a total otherwise stops at `i32::MAX`.
    fn add_counter_totals(&self, total: i32, amount: i32) -> i32 {
        if self.mirror_forge_bugs {
            total.wrapping_add(amount)
        } else {
            total.saturating_add(amount)
        }
    }

    pub fn record_counter_added(
        &mut self,
        putter: Option<PlayerId>,
        entity: GameEntity,
        counter_type: &CounterType,
        amount: i32,
    ) {
        let key = (
            entity,
            self.counter_entity_timestamp(entity),
            counter_type.clone(),
            putter,
        );
        let total = self.counter_added_this_turn.entry(key).or_default();
        *total = if self.mirror_forge_bugs {
            total.wrapping_add(amount)
        } else {
            total.saturating_add(amount)
        };
    }

    fn counter_entity_timestamp(&self, entity: GameEntity) -> Option<u64> {
        match entity {
            GameEntity::Card(card) => Some(self.card(card).zone_timestamp),
            GameEntity::Player(_) => None,
        }
    }

    pub(crate) fn remove_card_from_zone(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        card: CardId,
    ) -> bool {
        if crate::game_loop::GameLoop::zone_trace_enabled()
            && self.cards[card.index()].card_name == "Mind Stone"
        {
            eprintln!(
                "[zone-rust] T{} remove {:?} {} from {:?} owner={:?}",
                self.turn.turn_number,
                card,
                self.cards[card.index()].card_name,
                zone_type,
                owner
            );
        }
        self.zones.remove_card(zone_type, owner, card)
    }

    pub(crate) fn add_card_to_zone(&mut self, zone_type: ZoneType, owner: PlayerId, card: CardId) {
        if crate::game_loop::GameLoop::zone_trace_enabled()
            && self.cards[card.index()].card_name == "Mind Stone"
        {
            eprintln!(
                "[zone-rust] T{} add {:?} {} -> {:?} owner={:?}",
                self.turn.turn_number,
                card,
                self.cards[card.index()].card_name,
                zone_type,
                owner
            );
        }
        self.zones.add_card_to_top(zone_type, owner, card);
    }

    pub(crate) fn add_card_to_zone_bottom(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        card: CardId,
    ) {
        self.zones.add_card_to_bottom(zone_type, owner, card);
    }

    pub fn take_top_card_from_zone(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
    ) -> Option<CardId> {
        self.zones.take_top_card(zone_type, owner)
    }

    pub fn take_top_cards_from_zone(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        count: usize,
    ) -> Vec<CardId> {
        let mut cards = Vec::with_capacity(count);
        for _ in 0..count {
            let Some(card) = self.take_top_card_from_zone(zone_type, owner) else {
                break;
            };
            cards.push(card);
        }
        cards.reverse();
        cards
    }

    pub fn reorder_card_in_zone(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        card: CardId,
        index: usize,
    ) {
        self.zones.reorder_card(zone_type, owner, card, index);
    }

    pub fn move_cards_to_zone_top(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        cards: &[CardId],
    ) {
        self.zones.move_cards_to_top(zone_type, owner, cards);
    }

    pub fn move_cards_to_zone_bottom(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        cards: &[CardId],
    ) {
        self.zones.move_cards_to_bottom(zone_type, owner, cards);
    }

    pub fn replace_zone_cards(&mut self, zone_type: ZoneType, owner: PlayerId, cards: Vec<CardId>) {
        self.zones.replace_cards(zone_type, owner, cards);
    }

    pub fn shuffle_zone_cards(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        rng: &mut dyn crate::game_rng::GameRng,
    ) {
        self.zones.shuffle_cards(zone_type, owner, rng);
    }

    pub fn shuffle_zone_cards_with_rand<R: rand::Rng + ?Sized>(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        rng: &mut R,
    ) {
        self.zones.shuffle_cards_with_rand(zone_type, owner, rng);
    }

    pub(crate) fn save_zone_lki(
        &mut self,
        zone_type: ZoneType,
        owner: PlayerId,
        card: CardId,
        from: ZoneType,
        latest_state: Option<crate::zone::LatestState>,
    ) {
        self.zones
            .save_lki(zone_type, owner, card, from, latest_state);
    }

    pub fn active_player(&self) -> PlayerId {
        self.turn.active_player
    }

    pub fn is_day(&self) -> bool {
        self.day_night_started && !self.is_night
    }

    pub fn is_neither_day_nor_night(&self) -> bool {
        !self.day_night_started
    }

    pub fn get_day_time(&self) -> Option<bool> {
        self.day_night_started.then_some(self.is_night)
    }

    pub fn set_day_time(
        &mut self,
        value: Option<bool>,
        trigger_handler: &mut crate::trigger::handler::TriggerHandler,
    ) {
        if crate::staticability::static_ability_cant_change_day_time::cant_change_day(self, value) {
            return;
        }
        let previous = self.get_day_time();
        self.day_night_started = value.is_some();
        self.is_night = value == Some(true);
        if previous.is_some() && value.is_some() && previous != value {
            trigger_handler.run_trigger(
                crate::trigger::TriggerType::DayTimeChanges,
                crate::event::RunParams::default(),
                false,
            );
        }
    }

    pub fn next_player(&self, player: PlayerId) -> PlayerId {
        let current_idx = self
            .player_order
            .iter()
            .position(|&p| p == player)
            .unwrap_or(0);
        for i in 1..self.player_order.len() {
            let next_idx = (current_idx + i) % self.player_order.len();
            let next_pid = self.player_order[next_idx];
            if self.player(next_pid).is_alive() {
                return next_pid;
            }
        }
        player
    }

    pub fn opponent_of(&self, player: PlayerId) -> PlayerId {
        for &pid in &self.player_order {
            if pid != player && self.player(pid).is_alive() {
                return pid;
            }
        }
        player // no opponent found (shouldn't happen in normal games)
    }

    pub fn alive_players(&self) -> Vec<PlayerId> {
        self.player_order
            .iter()
            .filter(|&&pid| self.player(pid).is_alive())
            .copied()
            .collect()
    }

    /// Get all cards in a specific zone for a player.
    pub fn cards_in_zone(&self, zone_type: ZoneType, owner: PlayerId) -> &[CardId] {
        &self.zone(zone_type, owner).cards
    }

    /// Get all creatures on the battlefield for a player.
    pub fn creatures_on_battlefield(&self, player: PlayerId) -> Vec<CardId> {
        self.cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .filter(|&&cid| self.card(cid).is_creature())
            .copied()
            .collect()
    }

    /// Assign the next zone timestamp to a card, returning the value.
    /// Called whenever a card enters a new zone to track insertion order.
    pub fn assign_zone_timestamp(&mut self, card_id: CardId) -> u64 {
        let ts = self.next_timestamp();
        let card = self.card_mut(card_id);
        card.zone_timestamp = ts;
        card.layer_timestamp = ts;
        ts
    }

    pub fn next_timestamp(&mut self) -> u64 {
        let ts = self.next_zone_timestamp;
        self.next_zone_timestamp += 1;
        ts
    }

    /// Return the next monotonic effect timestamp.
    /// A copied replacement effect is a new object in Java, so it can apply to an event the
    /// original already replaced; the fresh id keeps `ReplacementHandler.has_run` from matching it.
    pub fn next_copied_replacement_id(&mut self) -> i32 {
        self.last_copied_replacement_id += 1;
        self.last_copied_replacement_id
    }

    /// CR 616.1f: a replacement effect applies to an event at most once, and a copy of the
    /// same card's effect is that effect again. Java gives every copy a new id, so Superior
    /// Spider-Man copying another Superior Spider-Man card was offered its Mind Swap without end
    /// (FORGE BUG, mirrored under `mirror_forge_bugs`).
    pub fn copied_replacement_id(&mut self, source: CardId, original: i32) -> i32 {
        if self.mirror_forge_bugs {
            return self.next_copied_replacement_id();
        }
        if let Some(&(_, id)) = self
            .copied_replacement_ids
            .iter()
            .find(|(key, _)| *key == (source, original))
        {
            return id;
        }
        let id = self.next_copied_replacement_id();
        self.copied_replacement_ids.push(((source, original), id));
        id
    }

    pub fn next_effect_timestamp(&mut self) -> i64 {
        self.next_timestamp() as i64
    }

    /// Ensure shared damage/prevent maps exist for this resolution scope.
    pub fn ensure_pending_damage_maps(&mut self) {
        if self.pending_damage_map.is_none() {
            self.pending_damage_map = Some(CardDamageMap::default());
        }
        if self.pending_prevent_map.is_none() {
            self.pending_prevent_map = Some(CardDamageMap::default());
        }
    }

    /// Clear shared damage/prevent maps.
    pub fn clear_pending_damage_maps(&mut self) {
        self.pending_damage_map = None;
        self.pending_prevent_map = None;
    }

    /// Ensure a shared zone-change table exists for this resolution scope.
    pub fn ensure_pending_change_zone_table(&mut self) {
        if self.pending_change_zone_table.is_none() {
            self.pending_change_zone_table = Some(CardZoneTable::default());
        }
    }

    /// Clear the shared zone-change table.
    pub fn clear_pending_change_zone_table(&mut self) {
        self.pending_change_zone_table = None;
    }

    /// Get all lands on the battlefield for a player.
    pub fn lands_on_battlefield(&self, player: PlayerId) -> Vec<CardId> {
        self.cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .filter(|&&cid| self.card(cid).is_land())
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost};

    #[test]
    fn create_game() {
        let game = GameState::new(&["Alice", "Bob"], 20);
        assert_eq!(game.players.len(), 2);
        assert_eq!(game.player(PlayerId(0)).name, "Alice");
        assert_eq!(game.player(PlayerId(1)).name, "Bob");
        assert_eq!(game.player(PlayerId(0)).life, 20);
        assert!(game.zone(ZoneType::Sideboard, PlayerId(0)).is_empty());
        assert!(game.zone(ZoneType::AttractionDeck, PlayerId(0)).is_empty());
        assert!(game.zone(ZoneType::ContraptionDeck, PlayerId(0)).is_empty());
    }

    #[test]
    fn a_games_debug_ignores_its_layer_cache() {
        let game = GameState::new(&["Alice", "Bob"], 20);
        let mut cached = game.clone();
        cached.layer_key_after_pass.0 = Some(cached.layer_key());

        assert_eq!(format!("{cached:?}"), format!("{game:?}"));
    }

    #[test]
    fn create_card_and_zone() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let card = Card::new(
            CardId(0),
            "Grizzly Bears".to_string(),
            PlayerId(0),
            CardTypeLine::parse("Creature Bear"),
            ManaCost::parse("1 G"),
            ColorSet::GREEN,
            Some(2),
            Some(2),
            vec![],
            vec![],
        );
        let cid = game.create_card(card);
        game.add_card_to_zone(ZoneType::Library, PlayerId(0), cid);
        game.card_mut(cid).zone = ZoneType::Library;
        assert_eq!(game.zone(ZoneType::Library, PlayerId(0)).len(), 1);
        assert_eq!(game.card_zone(cid), Some(ZoneType::Library));
    }

    #[test]
    fn opponent_lookup() {
        let game = GameState::new(&["Alice", "Bob"], 20);
        assert_eq!(game.opponent_of(PlayerId(0)), PlayerId(1));
        assert_eq!(game.opponent_of(PlayerId(1)), PlayerId(0));
    }

    #[test]
    fn lki_snapshot_captures_battlefield_state() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);

        // Create a 3/3 creature on the battlefield
        let mut card = Card::new(
            CardId(0),
            "Grizzly Bears".to_string(),
            PlayerId(0),
            CardTypeLine::parse("Creature Bear"),
            ManaCost::parse("1 G"),
            ColorSet::GREEN,
            Some(3),
            Some(3),
            vec![],
            vec![],
        );
        card.zone = ZoneType::Battlefield;
        let cid = game.create_card(card);

        // Take LKI snapshot
        game.copy_last_state();

        // Verify snapshot captured the correct power/toughness
        let snapshot = game.get_lki_snapshot(cid).expect("snapshot should exist");
        assert_eq!(snapshot.power, 3);
        assert_eq!(snapshot.toughness, 3);
        assert_eq!(snapshot.card_name, "Grizzly Bears");

        // Move card to graveyard and verify snapshot still exists
        game.card_mut(cid).zone = ZoneType::Graveyard;
        let snapshot = game
            .get_lki_snapshot(cid)
            .expect("snapshot should still exist");
        assert_eq!(snapshot.power, 3);

        // Snapshot preserves stale entries for LKI (cards that left the battlefield).
        // This matches Java's behavior where LKI persists through resolution chains.
        game.copy_last_state();
        assert!(
            game.get_lki_snapshot(cid).is_some(),
            "stale LKI should persist"
        );
    }
}
