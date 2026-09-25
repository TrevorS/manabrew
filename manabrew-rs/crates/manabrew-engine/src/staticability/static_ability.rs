//! Static ability parsing and types.
//!
//! Mirrors the Java Forge `forge/game/staticability/` package, specifically
//! `StaticAbility.java` and `StaticAbilityContinuous.java`.
//!
//! Card scripts encode static abilities as `S$`-prefixed lines, e.g.:
//! ```text
//! S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1 | Description$ Creatures you control get +1/+1.
//! S$ Mode$ ETBTapped | Description$ This permanent enters the battlefield tapped.
//! S$ Mode$ CantAttack | Affected$ Creature.YouControl | Description$ Creatures you control can't attack.
//! ```

use crate::HashMap;

use forge_foundation::ZoneType;
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use super::static_ability_mode;
use crate::card::valid_filter::CardTraitRequirementsIr;
use crate::card::Card;
use crate::card::CounterType;
use crate::card_trait_base::{CardTrait, CardTraitBase, CardTraitIrOwner};
use crate::core::HasSVars;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::parsing::keys;
use crate::parsing::{CompiledSelector, Params};

const STATIC_ZONE_KEYS: &[&str] = &[keys::ACTIVE_ZONES, keys::EFFECT_ZONE];

const STATIC_CONDITION_KEYS: &[&str] = &[
    keys::PHASES,
    keys::CONDITION,
    keys::PLAYER_TURN,
    "TopCardOfLibraryIs",
    "ClassLevel",
    "CheckSecondSVar",
    "CheckThirdSVar",
    "CheckFourthSVar",
];

// ── Mode ────────────────────────────────────────────────────────────────────

/// The mode of a static ability.
///
/// Mirrors Java `StaticAbilityMode` enum. Each variant corresponds to a
/// `Mode$ <Value>` entry in the card script.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, EnumString,
)]
#[strum(ascii_case_insensitive)]
pub enum StaticMode {
    /// `Mode$ Continuous` — layer-based continuous effects (anthems, keyword
    /// grants, P/T setting). The most common category; handled by the layer
    /// system in `layer.rs`.
    Continuous,

    /// `Mode$ CantAttack` — affected creatures cannot attack.
    CantAttack,

    /// `Mode$ CantBlock` — affected creatures cannot block.
    CantBlock,

    /// `Mode$ ETBTapped` — this permanent (or matching permanents) enters the
    /// battlefield tapped. Applied once at ETB time, not continuously.
    ETBTapped,

    /// `Mode$ CantBeCast` — matching spells cannot be cast.
    CantBeCast,
    /// `Mode$ CantBeActivated` — matching abilities cannot be activated.
    CantBeActivated,
    /// `Mode$ CantPlayLand` — matching lands cannot be played.
    CantPlayLand,

    /// `Mode$ ReduceCost` — reduce the mana cost of matching spells.
    ReduceCost,

    /// `Mode$ RaiseCost` — increase the mana cost of matching spells.
    RaiseCost,

    /// `Mode$ SetCost` — raise cost to a minimum (Trinisphere). Used with `RaiseTo$`.
    SetCost,
    CantTarget,
    CantAttach,
    MustAttack,
    MustBlock,
    Panharmonicon,
    CantGainLosePayLife,
    CantDraw,
    CantExile,
    CantSacrifice,
    CantRegenerate,
    DisableTriggers,
    CantPutCounter,
    CastWithFlash,
    BlockRestrict,
    AttackRestrict,
    CanAttackDefender,
    IgnoreHexproof,
    IgnoreShroud,
    IgnoreLegendRule,
    MustTarget,
    AssignCombatDamageAsUnblocked,
    AssignNoCombatDamage,
    CombatDamageToughness,
    NoCleanupDamage,
    InfectDamage,
    WitherDamage,
    ColorlessDamageSource,
    CountersRemain,
    MaxCounter,
    /// `Mode$ CantAttackUnless` — attacker must pay a cost to attack (Propaganda, Ghostly Prison).
    CantAttackUnless,
    /// `Mode$ OptionalAttackCost` — optional attack payment like Exert/Enlist.
    OptionalAttackCost,
    /// `Mode$ OptionalCost` — generic optional cost on a spell/ability (GameActionUtil).
    OptionalCost,
    /// `Mode$ AttackRequirement` — adds an attack requirement (StaticAbilityAttackRequirement).
    AttackRequirement,
    /// `Mode$ PlayerMustAttack` — controller of affected creatures must attack.
    PlayerMustAttack,
    /// `Mode$ CantBlockUnless` — blocker must pay a cost to block (War Cadence).
    CantBlockUnless,
    /// `Mode$ CantBlockBy` — restricts which blockers can block an attacker
    /// (Flying, Fear, Intimidate, Skulk, or card-specific restrictions).
    CantBlockBy,
    /// `Mode$ ManaConvert` — spend mana as though it were mana of any color/type.
    ManaConvert,
    /// `Mode$ UnspentMana` — mana of specified type doesn't empty from pool.
    UnspentMana,
    /// `Mode$ ManaBurn` — losing unspent mana causes life loss (Yurlok of Scorch Thrash).
    ManaBurn,
    ActivateAbilityAsIfHaste,
    CanAdapt,
    AlternativeCost,
    CantBeCopied,
    CantBeSuspected,
    CantBecomeMonarch,
    CantChangeDayTime,
    CantCrew,
    CantDiscard,
    CantPhaseIn,
    CantPhaseOut,
    CantTransform,
    CantVenture,
    Devotion,
    CanExhaust,
    Activations,
    FlipCoinMod,
    GainLifeRadiation,
    IgnoreLandwalk,
    NumLoyaltyAct,
    PlotZone,
    SurveilNum,
    TapPowerValue,
    TurnReversed,
    PhaseReversed,
    UntapOtherPlayer,
    CanBlockIfReach,
    BlockTapped,
    CanAttackIfHaste,
    MinMaxBlocker,
    AttackVigilance,
    CantPreventDamage,
    CantGainLife,
    CantLoseLife,
    CantPayLife,
    CantChangeLife,

    /// Any mode not yet recognised — stored but not applied.
    #[strum(default)]
    Other(String),
}

// ── Layer ────────────────────────────────────────────────────────────────────

/// CR 613 layer ordering for continuous effects.
///
/// Effects are applied in ascending numeric order. Timestamp ordering within
/// the same layer is preserved by the order in which effects are collected
/// (battlefield entry order in `GameState.cards`).
///
/// Reference: <https://magic.wizards.com/en/rules> CR 613
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Layer {
    /// Layer 2 — control-changing effects.
    Control = 2,
    /// Layer 3 — text-changing effects.
    Text = 3,
    /// Layer 4 — type-changing effects.
    Type = 4,
    /// Layer 5 — color-changing effects.
    Color = 5,
    /// Layer 6 — ability-adding / removing.
    Ability = 6,
    /// Layer 7a — characteristic-defining P/T (CDAs).
    Characteristic = 70,
    /// Layer 7b — P/T set to an absolute value.
    SetPT = 71,
    /// Layer 7c — P/T modifications: bonuses and penalties.
    ModifyPT = 72,
    /// Layer 7d / rules-modifying — MayPlay, AdjustLandPlays, Goad, etc.
    Rules = 80,
}

// ── StaticAbility ────────────────────────────────────────────────────────────

/// A parsed static ability from an `S$` line in a card script.
///
/// Params are stored exactly as they appear in the script so that new param
/// types can be added without changing this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticAbility {
    #[serde(default)]
    pub base: Box<CardTraitBase>,
    pub modes: Vec<StaticMode>,
    pub ignore_effect_cards: Vec<CardId>,
    pub ignore_effect_players: Vec<PlayerId>,
    pub may_play_turn: i32,
    /// Mirrors `CardTraitBase.sVars` in Java. Populated by the card factory
    /// with the host card's SVar map so that `$`-expressions evaluated under
    /// the ability resolve against the card's SVars (not the ability's
    /// mapParams).
    #[serde(default)]
    pub svars: HashMap<String, String>,
    #[serde(skip)]
    pub ir: StaticAbilityIr,
}

impl StaticAbility {
    pub fn clone_for_parity_snapshot(&self) -> Self {
        let StaticAbility {
            base: _,
            modes,
            ignore_effect_cards,
            ignore_effect_players,
            may_play_turn,
            svars,
            ir,
        } = self;
        StaticAbility {
            base: Box::default(),
            modes: modes.clone(),
            ignore_effect_cards: ignore_effect_cards.clone(),
            ignore_effect_players: ignore_effect_players.clone(),
            may_play_turn: *may_play_turn,
            svars: svars.clone(),
            ir: ir.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticAbilityIr {
    pub card_trait_requirements: CardTraitRequirementsIr,
    pub valid_card: Option<CompiledSelector>,
    pub valid_card_text: Option<String>,
    pub valid_cards_text: Option<String>,
    pub valid_player: Option<CompiledSelector>,
    pub affected: Option<CompiledSelector>,
    pub affected_text: Option<String>,
    pub affected_defined: Option<String>,
    pub affected_zone: Option<ZoneType>,
    pub affected_zone_text: Option<String>,
    pub affected_zones: Vec<ZoneType>,
    pub valid_zone: Vec<ZoneType>,
    pub may_play: bool,
    /// Cost string from `MayPlayAltManaCost$` (Airbend's `2`, `CollectEvidence<10>`).
    /// When `may_play` is true and this is set, the granted cast option pays
    /// this cost in place of the card's mana cost.
    pub may_play_alt_mana_cost: Option<String>,
    /// `MayPlayWithoutManaCost$`. Java builds a separate `copyWithNoManaCost` option for it;
    /// here it rides the alt-cost path as a zero cost, so the two stay one code path.
    pub may_play_without_mana_cost: bool,
    pub may_play_grants_zone_permissions: bool,
    pub may_play_player: Option<String>,
    pub may_play_limit: Option<i32>,
    pub may_play_text: Option<String>,
    pub may_play_with_flash: bool,
    pub counter_type_text: Option<String>,
    pub counter_type: Option<CounterType>,
    pub caster: Option<CompiledSelector>,
    pub activator: Option<CompiledSelector>,
    pub player: Option<CompiledSelector>,
    pub valid_sa: Option<String>,
    pub valid_mode: Option<String>,
    pub valid_trigger: Option<String>,
    pub valid_cause: Option<CompiledSelector>,
    pub valid_cause_text: Option<String>,
    pub valid_source: Option<CompiledSelector>,
    pub valid_activator: Option<CompiledSelector>,
    pub for_cost: Option<bool>,
    pub is_combat: Option<bool>,
    pub origin_zones: Vec<ZoneType>,
    pub destination_zones: Vec<ZoneType>,
    pub combat_damage: Option<bool>,
    pub sorcery_speed: bool,
    pub only_sorcery_speed: bool,
    pub cmc_gt: Option<String>,
    pub num_limit_each_turn: Option<i32>,
    pub activator_raw: Option<String>,
    pub cost: Option<String>,
    pub gain_control_text: Option<String>,
    pub add_power_text: Option<String>,
    pub add_toughness_text: Option<String>,
    pub add_type_text: Option<String>,
    pub set_power_text: Option<String>,
    pub set_toughness_text: Option<String>,
    pub add_keyword_text: Option<String>,
    pub set_name_text: Option<String>,
    pub remove_card_types: bool,
    pub remove_land_types: bool,
    pub remove_artifact_types: bool,
    pub remove_creature_types: bool,
    pub remove_all_abilities: bool,
    pub add_ability_text: Option<String>,
    pub gains_abilities_of: Option<String>,
    pub gains_abilities_of_zones: Vec<ZoneType>,
    pub gains_abilities_of_defined: Option<String>,
    pub gains_abilities_limit_per_turn: Option<String>,
    pub gains_trigger_abs_of: Option<String>,
    pub add_trigger_text: Option<String>,
    pub add_replacement_effect_text: Option<String>,
    pub add_static_ability_text: Option<String>,
    pub adjust_land_plays_text: Option<String>,
    pub type_filter: Option<String>,
    pub mana_conversion: Option<String>,
    pub except_cause_text: Option<String>,
    pub restriction_text: Option<String>,
    pub x_alternative_text: Option<String>,
    pub announce_text: Option<String>,
    pub mana_restriction_text: Option<String>,
    pub stack_description_text: Option<String>,
    pub cost_desc_text: Option<String>,
    pub description_text: Option<String>,
    pub draw_limit: Option<i32>,
    /// `SetMaxHandSize$ Unlimited` (Reliquary Tower) or a numeric override.
    /// Stored as a raw string because Forge accepts both `Unlimited` and a
    /// number (and deck-builder cards use SVar expressions, though no
    /// staticability_test card currently does).
    pub set_max_hand_size: Option<String>,
    /// `RaiseMaxHandSize$ N` (Spellbook etc.) — additive to the player's
    /// max hand size.
    pub raise_max_hand_size: Option<String>,
    pub named_text: Option<String>,
    pub trigger_text: Option<String>,
    pub valid_defender: Option<CompiledSelector>,
    pub valid_defender_text: Option<String>,
    pub max_attackers: Option<String>,
    pub max_blockers: Option<String>,
    pub valid_attacked: Option<CompiledSelector>,
    pub valid_creature: Option<CompiledSelector>,
    pub must_attack: Option<String>,
    pub valid_entity: Option<CompiledSelector>,
    pub is_present: Option<CompiledSelector>,
    pub valid_attacker: Option<CompiledSelector>,
    pub valid_blocker: Option<CompiledSelector>,
    pub valid_blocker_text: Option<String>,
    pub valid_attacker_relative: Option<CompiledSelector>,
    pub has_valid_attacker_relative: bool,
    pub valid_blocker_relative: Option<CompiledSelector>,
    pub has_valid_blocker_relative: bool,
    pub unless_defender_text: Option<String>,
    pub kw_text: Option<String>,
    pub valid_keyword_text: Option<String>,
    pub value_text: Option<String>,
    pub mana_type_text: Option<String>,
    pub result_text: Option<String>,
    pub new_time_text: Option<String>,
    pub present_compare_text: Option<String>,
    pub check_svar_text: Option<String>,
    pub svar_compare_text: Option<String>,
    pub min_text: Option<String>,
    pub max_text: Option<String>,
    pub additional_text: Option<String>,
    pub min_limit_text: Option<String>,
    pub attacker_text: Option<String>,
    pub target_text: Option<String>,
    pub trigger: bool,
    pub twice: bool,
    pub only_source_abs: bool,
    pub optional: bool,
    pub num_value: Option<i32>,
    pub defender_not_nearest_to_you_in_chosen_direction: bool,
    pub effect_zone_all: bool,
    pub effect_zones: Vec<ZoneType>,
    pub valid_target: Option<CompiledSelector>,
    pub valid_target_text: Option<String>,
    pub valid_spell: Option<String>,
    pub for_each_shard: Option<String>,
    pub amount: Option<String>,
    pub min_mana: Option<i32>,
    pub raise_to: bool,
    pub may_play_ignore_type: bool,
    pub may_play_ignore_color: bool,
    pub may_play_snow_ignore_color: bool,
    pub color: Option<String>,
    pub ignore_generic: bool,
    pub only_first_spell: bool,
    pub unless_valid_target: bool,
    pub relative: bool,
    pub target: Option<CompiledSelector>,
    pub valid_card_to_target: Option<CompiledSelector>,
    pub exception_sba: bool,
    pub exceptions: Option<CompiledSelector>,
    pub has_valid_card: bool,
    pub has_valid_player: bool,
    pub max_num: Option<i32>,
    pub active_zones: Vec<ZoneType>,
    pub phases_text: Option<String>,
    pub condition_text: Option<String>,
    pub player_turn_text: Option<String>,
    pub top_card_of_library_is: Option<String>,
    pub valid_after_stack: Option<String>,
    pub may_play_raise_cost: Option<String>,
    pub class_level_min: Option<i32>,
    pub check_third_svar: Option<String>,
    pub third_svar_compare: Option<String>,
    pub check_fourth_svar: Option<String>,
    pub fourth_svar_compare: Option<String>,
    pub add_power: bool,
    pub add_toughness: bool,
    pub set_power: bool,
    pub set_toughness: bool,
    pub add_keyword: bool,
    pub gain_control_param: bool,
    pub add_type: bool,
    pub remove_type: bool,
    pub add_color: bool,
    pub remove_color: bool,
    pub set_color: bool,
    pub characteristic_defining: bool,
    pub has_text_layer_key: bool,
    pub has_type_layer_key: bool,
    pub has_color_layer_key: bool,
    pub has_ability_layer_key: bool,
    pub has_rules_layer_key: bool,
    pub has_zone_keys: bool,
    pub has_condition_keys: bool,
}

impl StaticAbilityIr {
    fn from_params(params: &Params) -> Self {
        let raw = params.inner();
        Self {
            card_trait_requirements: CardTraitRequirementsIr::from_key_values(
                params.iter(),
                params.selector_untracked(keys::IS_PRESENT).cloned(),
                params.selector_untracked("IsPresent2").cloned(),
            ),
            valid_card: params.selector_untracked(keys::VALID_CARD).cloned(),
            valid_card_text: raw.get(keys::VALID_CARD).map(String::to_string),
            valid_cards_text: raw.get(keys::VALID_CARDS).map(String::to_string),
            valid_player: params.selector_untracked(keys::VALID_PLAYER).cloned(),
            affected: params.selector_untracked(keys::AFFECTED).cloned(),
            affected_text: raw.get(keys::AFFECTED).map(String::to_string),
            affected_defined: raw.get(keys::AFFECTED_DEFINED).map(String::to_string),
            affected_zone: raw
                .get(keys::AFFECTED_ZONE)
                .map(String::as_str)
                .and_then(ZoneType::from_str_compat),
            affected_zone_text: raw.get(keys::AFFECTED_ZONE).map(String::to_string),
            affected_zones: zone_list(raw.get(keys::AFFECTED_ZONE).map(String::as_str)),
            valid_zone: zone_list(raw.get(keys::VALID_ZONE).map(String::as_str)),
            may_play: raw
                .get(keys::MAY_PLAY)
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            may_play_alt_mana_cost: raw.get(keys::MAY_PLAY_ALT_MANA_COST).map(String::to_string),
            may_play_without_mana_cost: raw.contains_key(keys::MAY_PLAY_WITHOUT_MANA_COST),
            may_play_grants_zone_permissions: !raw
                .contains_key(keys::MAY_PLAY_DONT_GRANT_ZONE_PERMISSIONS),
            may_play_player: raw.get(keys::MAY_PLAY_PLAYER).map(String::to_string),
            may_play_with_flash: raw.contains_key("MayPlayWithFlash"),
            may_play_limit: raw
                .get(keys::MAY_PLAY_LIMIT)
                .and_then(|value| value.trim().parse().ok()),
            may_play_text: raw.get(keys::MAY_PLAY_TEXT).map(String::to_string),
            counter_type_text: raw.get(keys::COUNTER_TYPE).map(String::to_string),
            counter_type: raw
                .get(keys::COUNTER_TYPE)
                .map(|value| crate::card::counter_type::parse_counter_type(value)),
            caster: params.selector_untracked(keys::CASTER).cloned(),
            activator: params.selector_untracked(keys::ACTIVATOR).cloned(),
            player: params.selector_untracked(keys::PLAYER).cloned(),
            valid_sa: raw.get(keys::VALID_SA).map(String::to_string),
            valid_mode: raw.get(keys::VALID_MODE).map(String::to_string),
            valid_trigger: raw.get(keys::VALID_TRIGGER).map(String::to_string),
            valid_cause: params.selector_untracked(keys::VALID_CAUSE).cloned(),
            valid_cause_text: raw.get(keys::VALID_CAUSE).map(String::to_string),
            valid_source: params.selector_untracked(keys::VALID_SOURCE).cloned(),
            valid_activator: params.selector_untracked(keys::VALID_ACTIVATOR).cloned(),
            for_cost: raw
                .get(keys::FOR_COST)
                .map(|value| value.eq_ignore_ascii_case("True")),
            is_combat: raw
                .get(keys::IS_COMBAT)
                .map(|value| value.eq_ignore_ascii_case("True")),
            origin_zones: zone_list(raw.get(keys::ORIGIN).map(String::as_str)),
            destination_zones: zone_list(raw.get(keys::DESTINATION).map(String::as_str)),
            combat_damage: raw
                .get(keys::COMBAT_DAMAGE)
                .map(|value| value.eq_ignore_ascii_case("True")),
            sorcery_speed: raw.get(keys::SORCERY_SPEED).is_some(),
            only_sorcery_speed: raw.get("OnlySorcerySpeed").is_some(),
            cmc_gt: raw.get("cmcGT").map(String::to_string),
            num_limit_each_turn: raw
                .get("NumLimitEachTurn")
                .and_then(|value| value.parse().ok()),
            activator_raw: raw.get(keys::ACTIVATOR).map(String::to_string),
            cost: raw.get(keys::COST).map(String::to_string),
            gain_control_text: raw.get(keys::GAIN_CONTROL).map(String::to_string),
            add_power_text: raw.get(keys::ADD_POWER).map(String::to_string),
            add_toughness_text: raw.get(keys::ADD_TOUGHNESS).map(String::to_string),
            add_type_text: raw.get(keys::ADD_TYPE).map(String::to_string),
            set_power_text: raw.get(keys::SET_POWER).map(String::to_string),
            set_toughness_text: raw.get(keys::SET_TOUGHNESS).map(String::to_string),
            add_keyword_text: raw.get(keys::ADD_KEYWORD).map(String::to_string),
            set_name_text: raw.get(keys::SET_NAME).map(String::to_string),
            remove_card_types: raw.contains_key(keys::REMOVE_CARD_TYPES),
            remove_land_types: raw.contains_key(keys::REMOVE_LAND_TYPES),
            remove_artifact_types: raw.contains_key(keys::REMOVE_ARTIFACT_TYPES),
            remove_creature_types: raw.contains_key(keys::REMOVE_CREATURE_TYPES),
            remove_all_abilities: raw
                .get(keys::REMOVE_ALL_ABILITIES)
                .is_some_and(|value| value.eq_ignore_ascii_case("true")),
            add_ability_text: raw.get(keys::ADD_ABILITY).map(String::to_string),
            gains_abilities_of: raw.get(keys::GAINS_ABILITIES_OF).map(String::to_string),
            gains_abilities_of_zones: zone_list(
                raw.get(keys::GAINS_ABILITIES_OF_ZONES).map(String::as_str),
            ),
            gains_abilities_of_defined: raw
                .get(keys::GAINS_ABILITIES_OF_DEFINED)
                .map(String::to_string),
            gains_abilities_limit_per_turn: raw
                .get(keys::GAINS_ABILITIES_LIMIT_PER_TURN)
                .map(String::to_string),
            gains_trigger_abs_of: raw.get(keys::GAINS_TRIGGER_ABS_OF).map(String::to_string),
            add_trigger_text: raw.get(keys::ADD_TRIGGER).map(String::to_string),
            add_replacement_effect_text: raw
                .get(keys::ADD_REPLACEMENT_EFFECT)
                .map(String::to_string),
            add_static_ability_text: raw.get("AddStaticAbility").map(String::to_string),
            adjust_land_plays_text: raw.get(keys::ADJUST_LAND_PLAYS).map(String::to_string),
            type_filter: raw.get(keys::TYPE).map(String::to_string),
            mana_conversion: raw.get(keys::MANA_CONVERSION).map(String::to_string),
            except_cause_text: raw.get(keys::EXCEPT_CAUSE).map(String::to_string),
            restriction_text: raw.get(keys::RESTRICTION).map(String::to_string),
            x_alternative_text: raw.get("XAlternative").map(String::to_string),
            announce_text: raw.get("Announce").map(String::to_string),
            mana_restriction_text: raw.get("ManaRestriction").map(String::to_string),
            stack_description_text: raw.get("StackDescription").map(String::to_string),
            cost_desc_text: raw.get("CostDesc").map(String::to_string),
            description_text: raw.get(keys::DESCRIPTION).map(String::to_string),
            draw_limit: raw
                .get(keys::DRAW_LIMIT)
                .and_then(|value| value.parse().ok()),
            set_max_hand_size: raw.get("SetMaxHandSize").map(String::to_string),
            raise_max_hand_size: raw.get("RaiseMaxHandSize").map(String::to_string),
            named_text: raw.get("Named").map(String::to_string),
            trigger_text: raw.get(keys::TRIGGER).map(String::to_string),
            valid_defender: params.selector_untracked(keys::VALID_DEFENDER).cloned(),
            valid_defender_text: raw.get(keys::VALID_DEFENDER).map(String::to_string),
            max_attackers: raw.get(keys::MAX_ATTACKERS).map(String::to_string),
            max_blockers: raw.get(keys::MAX_BLOCKERS).map(String::to_string),
            valid_attacked: params.selector_untracked(keys::VALID_ATTACKED).cloned(),
            valid_creature: params.selector_untracked(keys::VALID_CREATURE).cloned(),
            must_attack: params.get("MustAttack").map(str::to_string),
            valid_entity: params.selector_untracked(keys::VALID_ENTITY).cloned(),
            is_present: params.selector_untracked(keys::IS_PRESENT).cloned(),
            valid_attacker: params.selector_untracked(keys::VALID_ATTACKER).cloned(),
            valid_blocker: params.selector_untracked(keys::VALID_BLOCKER).cloned(),
            valid_blocker_text: raw.get(keys::VALID_BLOCKER).map(String::to_string),
            valid_attacker_relative: params
                .selector_untracked(keys::VALID_ATTACKER_RELATIVE)
                .cloned(),
            has_valid_attacker_relative: raw.contains_key(keys::VALID_ATTACKER_RELATIVE),
            valid_blocker_relative: params
                .selector_untracked(keys::VALID_BLOCKER_RELATIVE)
                .cloned(),
            has_valid_blocker_relative: raw.contains_key(keys::VALID_BLOCKER_RELATIVE),
            unless_defender_text: raw.get(keys::UNLESS_DEFENDER).map(String::to_string),
            kw_text: raw.get(keys::KW).map(String::to_string),
            valid_keyword_text: raw.get(keys::VALID_KEYWORD).map(String::to_string),
            value_text: raw.get(keys::VALUE).map(String::to_string),
            mana_type_text: raw.get(keys::MANA_TYPE).map(String::to_string),
            result_text: raw.get(keys::RESULT).map(String::to_string),
            new_time_text: raw.get(keys::NEW_TIME).map(String::to_string),
            present_compare_text: raw.get(keys::PRESENT_COMPARE).map(String::to_string),
            check_svar_text: raw.get(keys::CHECK_SVAR).map(String::to_string),
            svar_compare_text: raw.get(keys::SVAR_COMPARE).map(String::to_string),
            min_text: raw.get(keys::MIN).map(String::to_string),
            max_text: raw.get(keys::MAX).map(String::to_string),
            additional_text: raw.get(keys::ADDITIONAL).map(String::to_string),
            min_limit_text: raw.get(keys::MIN_LIMIT).map(String::to_string),
            attacker_text: raw.get(keys::ATTACKER).map(String::to_string),
            target_text: raw.get(keys::TARGET).map(String::to_string),
            trigger: raw.contains_key(keys::TRIGGER),
            twice: raw.contains_key(keys::TWICE),
            only_source_abs: raw.contains_key(keys::ONLY_SOURCE_ABS),
            optional: raw.contains_key(keys::OPTIONAL),
            num_value: raw.get(keys::NUM).and_then(|value| value.parse().ok()),
            defender_not_nearest_to_you_in_chosen_direction: raw
                .contains_key(keys::DEFENDER_NOT_NEAREST_TO_YOU_IN_CHOSEN_DIRECTION),
            effect_zone_all: raw
                .get(keys::EFFECT_ZONE)
                .or_else(|| raw.get(keys::AFFECTED_ZONE))
                .is_some_and(|value| value.eq_ignore_ascii_case("All")),
            effect_zones: zone_list(
                raw.get(keys::EFFECT_ZONE)
                    .or_else(|| raw.get(keys::AFFECTED_ZONE))
                    .map(String::as_str),
            ),
            valid_target: params.selector_untracked(keys::VALID_TARGET).cloned(),
            valid_target_text: raw.get(keys::VALID_TARGET).map(String::to_string),
            valid_spell: raw.get(keys::VALID_SPELL).map(String::to_string),
            for_each_shard: raw.get(keys::FOR_EACH_SHARD).map(String::to_string),
            amount: raw.get(keys::AMOUNT).map(String::to_string),
            min_mana: raw.get(keys::MIN_MANA).and_then(|value| value.parse().ok()),
            raise_to: raw
                .get("RaiseTo")
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            may_play_ignore_type: raw.contains_key("MayPlayIgnoreType"),
            may_play_ignore_color: raw.contains_key("MayPlayIgnoreColor"),
            may_play_snow_ignore_color: raw.contains_key("MayPlaySnowIgnoreColor"),
            color: raw.get(keys::COLOR).map(String::to_string),
            ignore_generic: raw
                .get("IgnoreGeneric")
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            only_first_spell: raw
                .get("OnlyFirstSpell")
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            unless_valid_target: raw
                .get("UnlessValidTarget")
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            relative: raw
                .get("Relative")
                .is_some_and(|value| value.eq_ignore_ascii_case("True")),
            target: params.selector_untracked(keys::TARGET).cloned(),
            valid_card_to_target: params
                .selector_untracked(keys::VALID_CARD_TO_TARGET)
                .cloned(),
            exception_sba: raw.get(keys::EXCEPTION_SBA).is_some(),
            exceptions: params.selector_untracked(keys::EXCEPTIONS).cloned(),
            has_valid_card: raw.contains_key(keys::VALID_CARD),
            has_valid_player: raw.contains_key(keys::VALID_PLAYER),
            max_num: raw.get(keys::MAX_NUM).and_then(|value| value.parse().ok()),
            active_zones: zone_list(raw.get(keys::ACTIVE_ZONES).map(String::as_str)),
            phases_text: raw.get(keys::PHASES).map(String::to_string),
            condition_text: raw.get(keys::CONDITION).map(String::to_string),
            player_turn_text: raw.get(keys::PLAYER_TURN).map(String::to_string),
            top_card_of_library_is: raw.get("TopCardOfLibraryIs").map(String::to_string),
            valid_after_stack: raw.get("ValidAfterStack").map(String::to_string),
            may_play_raise_cost: raw.get("RaiseCost").map(String::to_string),
            class_level_min: raw.get("ClassLevel").and_then(|value| value.parse().ok()),
            check_third_svar: raw.get("CheckThirdSVar").map(String::to_string),
            third_svar_compare: raw.get("ThirdSVarCompare").map(String::to_string),
            check_fourth_svar: raw.get("CheckFourthSVar").map(String::to_string),
            fourth_svar_compare: raw.get("FourthSVarCompare").map(String::to_string),
            add_power: raw.contains_key(keys::ADD_POWER),
            add_toughness: raw.contains_key(keys::ADD_TOUGHNESS),
            set_power: raw.contains_key(keys::SET_POWER),
            set_toughness: raw.contains_key(keys::SET_TOUGHNESS),
            add_keyword: raw.contains_key(keys::ADD_KEYWORD),
            gain_control_param: raw.contains_key(keys::GAIN_CONTROL),
            add_type: raw.contains_key(keys::ADD_TYPE),
            remove_type: raw.contains_key(keys::REMOVE_TYPE),
            add_color: raw.contains_key(keys::ADD_COLOR),
            remove_color: raw.contains_key(keys::REMOVE_COLOR),
            set_color: raw.contains_key(keys::SET_COLOR),
            characteristic_defining: raw
                .get(keys::CHARACTERISTIC_DEFINING)
                .is_some_and(|v| v.eq_ignore_ascii_case("True")),
            has_text_layer_key: params.contains_any_key(&[
                keys::CHANGE_COLOR_WORDS_TO,
                keys::GAIN_TEXT_OF,
                keys::ADD_NAMES,
                keys::SET_NAME,
                keys::INCORPORATE,
                keys::MANA_COST,
            ]),
            has_type_layer_key: params.contains_any_key(&[
                keys::ADD_TYPE,
                keys::REMOVE_TYPE,
                keys::ADD_ALL_CREATURE_TYPES,
                keys::REMOVE_CARD_TYPES,
                keys::REMOVE_SUB_TYPES,
                keys::REMOVE_SUPER_TYPES,
                keys::REMOVE_LAND_TYPES,
                keys::REMOVE_CREATURE_TYPES,
                keys::REMOVE_ARTIFACT_TYPES,
                keys::REMOVE_ENCHANTMENT_TYPES,
            ]),
            has_color_layer_key: params.contains_any_key(&[
                keys::ADD_COLOR,
                keys::REMOVE_COLOR,
                keys::SET_COLOR,
            ]),
            has_ability_layer_key: params.contains_any_key(&[
                keys::REMOVE_ALL_ABILITIES,
                keys::REMOVE_NON_MANA_ABILITIES,
                keys::GAINS_ABILITIES_OF,
                keys::GAINS_ABILITIES_OF_DEFINED,
                keys::GAINS_TRIGGER_ABS_OF,
                keys::ADD_KEYWORD,
                keys::ADD_ABILITY,
                keys::ADD_TRIGGER,
                keys::ADD_REPLACEMENT_EFFECT,
                keys::ADD_STATIC_ABILITY,
                keys::ADD_SVAR,
                keys::CANT_HAVE_KEYWORD,
                keys::SHARE_REMEMBERED_KEYWORDS,
                keys::REMOVE_KEYWORD,
            ]),
            has_rules_layer_key: params.contains_any_key(&[
                keys::ADD_HIDDEN_KEYWORD,
                keys::MAY_PLAY,
                keys::IGNORE_EFFECT_COST,
                keys::GOAD,
                keys::CAN_BLOCK_ANY,
                keys::CAN_BLOCK_AMOUNT,
                keys::ADJUST_LAND_PLAYS,
                keys::CONTROL_VOTE,
                keys::ADDITIONAL_VOTE,
                keys::ADDITIONAL_OPTIONAL_VOTE,
                keys::DECLARES_ATTACKERS,
                keys::DECLARES_BLOCKERS,
            ]),
            has_zone_keys: params.contains_any_key(STATIC_ZONE_KEYS),
            has_condition_keys: params.contains_any_key(STATIC_CONDITION_KEYS),
        }
    }
}

fn zone_list(raw: Option<&str>) -> Vec<ZoneType> {
    raw.map(|zones| {
        zones
            .split(',')
            .filter_map(|zone| ZoneType::from_str_compat(zone.trim()))
            .collect()
    })
    .unwrap_or_default()
}

impl StaticAbility {
    fn sync_trait_base_params(&mut self) {
        self.base.set_svars(self.svars.clone());
    }

    pub fn check_mode(&self, mode: &StaticMode) -> bool {
        match mode {
            StaticMode::Other(query) => self
                .modes
                .iter()
                .any(|m| matches!(m, StaticMode::Other(s) if s.eq_ignore_ascii_case(query))),
            other => self.modes.iter().any(|m| m == other),
        }
    }

    pub fn unknown_modes(&self) -> impl Iterator<Item = &str> {
        self.modes.iter().filter_map(|mode| match mode {
            StaticMode::Other(value) => Some(value.as_str()),
            _ => None,
        })
    }

    /// Modes that parse with Java parity but do not yet have a Rust runtime
    /// implementation. This keeps them distinct from `Other`, which means the
    /// parser has not recognised the Java mode at all.
    pub fn parsed_but_unimplemented_modes(&self) -> impl Iterator<Item = &StaticMode> {
        self.modes.iter().filter(|mode| {
            matches!(
                mode,
                StaticMode::OptionalCost
                    | StaticMode::AttackRequirement
                    | StaticMode::PlayerMustAttack
            )
        })
    }

    pub fn zones_check(&self, source_zone: ZoneType) -> bool {
        if self.ir.characteristic_defining {
            return true;
        }
        if !self.ir.has_zone_keys {
            return source_zone == ZoneType::Battlefield;
        }

        let _perf_scope = crate::perf::ParamsLookupScopeGuard::enter(
            crate::perf::ParamsLookupScope::StaticAbility,
        );
        if !self.ir.active_zones.is_empty() {
            return self.ir.active_zones.contains(&source_zone);
        }
        if self.ir.effect_zone_all {
            return true;
        }
        if !self.ir.effect_zones.is_empty() {
            return self.ir.effect_zones.contains(&source_zone);
        }
        source_zone == ZoneType::Battlefield
    }

    pub fn check_conditions(&self, source: &Card, game: &GameState) -> bool {
        let _perf_scope = crate::perf::ParamsLookupScopeGuard::enter(
            crate::perf::ParamsLookupScope::StaticAbility,
        );
        if !self.zones_check(source.zone) {
            return false;
        }
        if source.phased_out {
            return false;
        }
        if let Some(state) = self.base.get_card_state_name() {
            if matches!(
                state,
                forge_foundation::CardStateName::LeftSplit
                    | forge_foundation::CardStateName::RightSplit
            ) && !source.room_door_unlocked(state)
            {
                return false;
            }
        }
        if !self.meets_card_trait_requirements(game, source, self) {
            return false;
        }

        if !self.ir.has_condition_keys {
            return true;
        }

        if let Some(phases) = self.ir.phases_text.as_deref() {
            if !forge_foundation::PhaseType::parse_range(phases).contains(&game.turn.phase) {
                return false;
            }
        }

        if let Some(condition) = self.ir.condition_text.as_deref() {
            if condition.eq_ignore_ascii_case("MaxSpeed")
                && game.player(source.controller).speed != 4
            {
                return false;
            }
        }

        if let Some(player_turn) = self.ir.player_turn_text.as_deref() {
            let active = game.turn.active_player;
            let defined = crate::ability::effects::helpers::resolve_defined_players(
                player_turn,
                source.controller,
                game,
            );
            let ok = defined.contains(&active);
            if !ok {
                return false;
            }
        }

        if let Some(valid_top) = self.ir.top_card_of_library_is.as_deref() {
            let top = game
                .zone(ZoneType::Library, source.controller)
                .peek_top()
                .map(|cid| game.card(cid));
            let Some(top_card) = top else {
                return false;
            };
            if !crate::card::valid_filter::matches_valid_card_opt(Some(valid_top), top_card, source)
            {
                return false;
            }
        }

        if let Some(min) = self.ir.class_level_min {
            if source.class_level < min {
                return false;
            }
        }

        // Java `CardTraitBase.meetsCommonRequirements` gates every trait on `CheckSVar$`, not just
        // the infect-damage static that reads it with a hypothetical life total.
        if let Some(check_name) = self.ir.check_svar_text.as_deref() {
            let compare = self.ir.svar_compare_text.as_deref().unwrap_or("GE1");
            if !crate::card::valid_filter::check_svar_requirement(
                game, source, source, check_name, compare,
            ) {
                return false;
            }
        }
        if let Some(check_name) = self.ir.check_third_svar.as_deref() {
            let compare = self.ir.third_svar_compare.as_deref().unwrap_or("GE1");
            if !crate::card::valid_filter::check_svar_requirement(
                game, source, source, check_name, compare,
            ) {
                return false;
            }
        }
        if let Some(check_name) = self.ir.check_fourth_svar.as_deref() {
            let compare = self.ir.fourth_svar_compare.as_deref().unwrap_or("GE1");
            if !crate::card::valid_filter::check_svar_requirement(
                game, source, source, check_name, compare,
            ) {
                return false;
            }
        }

        true
    }

    pub fn check_conditions_full(
        &self,
        mode: &StaticMode,
        source: &Card,
        game: &GameState,
    ) -> bool {
        self.check_mode(mode) && self.check_conditions(source, game)
    }

    pub fn is_active_for(&self, mode: StaticMode, source_zone: ZoneType) -> bool {
        self.check_mode(&mode) && self.zones_check(source_zone)
    }

    pub fn add_ignore_effect_players(&mut self, player: PlayerId) {
        if !self.ignore_effect_players.contains(&player) {
            self.ignore_effect_players.push(player);
        }
    }

    pub fn clear_ignore_effects(&mut self) {
        self.ignore_effect_cards.clear();
        self.ignore_effect_players.clear();
    }

    pub fn inc_may_play_turn(&mut self) {
        self.may_play_turn += 1;
    }

    pub fn reset_may_play_turn(&mut self) {
        self.may_play_turn = 0;
    }

    pub fn copy(&self) -> Self {
        self.clone()
    }
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Parse a raw `S$` (or `S:`) ability line from a card script into a
/// [`StaticAbility`].
///
/// Returns `None` if the line does not start with the `S$` / `S:` prefix or
/// has no recognisable `Mode$` param.
///
/// # Format
///
/// ```text
/// S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1
/// S$ Mode$ ETBTapped | Description$ Enters tapped.
/// ```
///
/// Reference: Java `StaticAbility.java` in `forge/game/staticability/`.
pub fn parse_static_ability(raw: &str) -> Option<StaticAbility> {
    let trimmed = raw.trim();
    // Accept "S$ ..." or "S: ..." prefixes from card script static lines,
    // plus bare "Mode$ ..." SVar payloads used by AddStaticAbility$.
    let body = if let Some(rest) = trimmed.strip_prefix("S$ ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("S:") {
        rest.trim_start()
    } else if trimmed.starts_with("Mode$ ") {
        trimmed
    } else {
        return None;
    };

    // Parse "|"-separated "Key$ Value" pairs using central parser.
    let params = Params::from_raw(body);

    let modes = {
        let value = params.get(keys::MODE)?;
        static_ability_mode::set_value_of(value)
    };
    if modes.is_empty() {
        return None;
    }

    let ir = StaticAbilityIr::from_params(&params);
    let mut st_ab = StaticAbility {
        base: Box::new(CardTraitBase::default()),
        modes,
        ignore_effect_cards: Vec::new(),
        ignore_effect_players: Vec::new(),
        may_play_turn: 0,
        svars: HashMap::default(),
        ir,
    };
    st_ab.sync_trait_base_params();
    Some(st_ab)
}

impl HasSVars for StaticAbility {
    fn get_svar(&self, name: &str) -> Option<&str> {
        self.svars.get(name).map(String::as_str)
    }

    fn set_svar(&mut self, name: String, value: String) {
        self.svars.insert(name.clone(), value.clone());
        self.base.set_svar(name, value);
    }

    fn set_svars(&mut self, new_svars: HashMap<String, String>) {
        self.svars = new_svars.clone();
        self.base.set_svars(new_svars);
    }

    fn get_svars(&self) -> &HashMap<String, String> {
        &self.svars
    }

    fn remove_svar(&mut self, var: &str) {
        self.svars.remove(var);
        self.base.remove_svar(var);
    }
}

impl CardTrait for StaticAbility {
    fn base(&self) -> &CardTraitBase {
        &self.base
    }
}

impl CardTraitIrOwner for StaticAbility {
    type Ir = StaticAbilityIr;

    fn ir(&self) -> &Self::Ir {
        &self.ir
    }

    fn card_trait_requirements(&self) -> &CardTraitRequirementsIr {
        &self.ir.card_trait_requirements
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parser tests ─────────────────────────────────────────────────────

    #[test]
    fn parse_continuous_anthem() {
        let raw = "S$ Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 1 | AddToughness$ 1 | Description$ Creatures you control get +1/+1.";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(sa.modes, vec![StaticMode::Continuous]);
        assert_eq!(sa.ir.add_power_text.as_deref(), Some("1"));
        assert_eq!(sa.ir.add_toughness_text.as_deref(), Some("1"));
        assert_eq!(
            crate::staticability::layer::classify_static_layers(&sa),
            vec![Layer::ModifyPT]
        );
    }

    #[test]
    fn parse_etb_tapped() {
        let raw = "S$ Mode$ ETBTapped | Description$ This permanent enters the battlefield tapped.";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(sa.modes, vec![StaticMode::ETBTapped]);
        assert!(crate::staticability::layer::classify_static_layers(&sa).is_empty());
    }

    #[test]
    fn parse_cant_attack() {
        let raw = "S$ Mode$ CantAttack | Affected$ Creature.YouControl | Description$ Creatures you control can't attack.";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(sa.modes, vec![StaticMode::CantAttack]);
    }

    #[test]
    fn parse_keyword_grant() {
        let raw = "S$ Mode$ Continuous | Affected$ Creature.YouControl | AddKeyword$ Flying | Description$ Creatures you control have flying.";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(
            crate::staticability::layer::classify_static_layers(&sa),
            vec![Layer::Ability]
        );
        assert_eq!(sa.ir.add_keyword_text.as_deref(), Some("Flying"));
    }

    #[test]
    fn parse_set_pt() {
        let raw =
            "S$ Mode$ Continuous | Affected$ Creature.YouControl | SetPower$ 0 | SetToughness$ 1";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(
            crate::staticability::layer::classify_static_layers(&sa),
            vec![Layer::SetPT]
        );
    }

    #[test]
    fn parse_s_colon_prefix() {
        let raw =
            "S: Mode$ Continuous | Affected$ Creature.YouControl | AddPower$ 2 | AddToughness$ 2";
        let sa = parse_static_ability(raw).expect("should parse S: prefix");
        assert_eq!(sa.modes, vec![StaticMode::Continuous]);
    }

    #[test]
    fn parse_multi_mode_cant_attack_block() {
        let raw = "S$ Mode$ CantAttack,CantBlock | Affected$ Creature.YouControl";
        let sa = parse_static_ability(raw).expect("should parse");
        assert_eq!(
            sa.modes,
            vec![StaticMode::CantAttack, StaticMode::CantBlock]
        );
        assert!(sa.check_mode(&StaticMode::CantAttack));
        assert!(sa.check_mode(&StaticMode::CantBlock));
        assert!(!sa.check_mode(&StaticMode::CantBeActivated));
    }

    #[test]
    fn parse_multi_mode_three_restrictions() {
        let raw = "S$ Mode$ CantAttack,CantBlock,CantBeActivated | Affected$ Creature.YouControl";
        let sa = parse_static_ability(raw).expect("should parse");
        assert!(sa.check_mode(&StaticMode::CantAttack));
        assert!(sa.check_mode(&StaticMode::CantBlock));
        assert!(sa.check_mode(&StaticMode::CantBeActivated));
    }

    #[test]
    fn parse_continuous_with_restrictions() {
        let raw = "S$ Mode$ Continuous,CantPlayLand,CantBeCast | Affected$ Card.YouControl";
        let sa = parse_static_ability(raw).expect("should parse");
        assert!(sa.check_mode(&StaticMode::Continuous));
        assert!(sa.check_mode(&StaticMode::CantPlayLand));
        assert!(sa.check_mode(&StaticMode::CantBeCast));
    }

    #[test]
    fn non_static_line_returns_none() {
        assert!(parse_static_ability("AB$ Mana | Cost$ T | Produced$ G").is_none());
        assert!(parse_static_ability("T$ Mode$ ChangesZone").is_none());
        assert!(parse_static_ability("").is_none());
    }
}
