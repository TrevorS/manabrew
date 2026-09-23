//! Keyword-based ability and trigger generation for Card.
//!
//! These functions translate keywords like "Cycling", "Prowess", "Bushido", etc. into
//! concrete activated abilities and triggered abilities. They're called during card
//! initialization in `Card::from_rules()`.

use crate::ability::activated::parse_activated_ability;
use crate::card::svar_cache::ParsedSVarKind;
use crate::parsing::keys;
use crate::parsing::Params;
use crate::staticability::parse_static_ability;
use crate::trigger::parse_trigger;

use super::Card;

fn keyword_cost(keywords: &[String], name: &str) -> Option<String> {
    keywords
        .iter()
        .find_map(|kw| crate::keyword::extract_keyword_cost_str(kw, name))
        .map(str::to_string)
}

fn roman_chapter(mut chapter: usize) -> String {
    let mut result = String::new();
    for (value, numeral) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while chapter >= value {
            result.push_str(numeral);
            chapter -= value;
        }
    }
    result
}

/// Mirrors Java's `LibraryMovementCostVisitor`: the half of CR 605.1a that reads the cost.
fn cost_moves_card_to_or_from_library(cost: &crate::cost::Cost) -> bool {
    cost.parts.iter().any(|part| match part {
        crate::cost::CostPart::Mill(_)
        | crate::cost::CostPart::Draw(_)
        | crate::cost::CostPart::PutCardToLib { .. } => true,
        crate::cost::CostPart::Exile { from, .. } => *from == forge_foundation::ZoneType::Library,
        _ => false,
    })
}

impl Card {
    fn parsed_svar_params(&mut self, name: &str) -> Option<Params> {
        match self.parsed_s_var(name)?.kind {
            ParsedSVarKind::Ability { params, .. } | ParsedSVarKind::ParamRecord { params } => {
                Some(params)
            }
            ParsedSVarKind::Number { .. }
            | ParsedSVarKind::Count { .. }
            | ParsedSVarKind::NumericExpression { .. }
            | ParsedSVarKind::Raw { .. } => None,
        }
    }

    /// Mirrors Java's `SpellAbility.isManaAbility()` (CR 605.1a). `parse_activated_ability` only
    /// sees one ability's own text, so it answers from the root `AB$` alone; the mana part can sit
    /// on any link of the `SubAbility$` chain, and resolving those names needs the card's SVars.
    /// Java's answer is fixed by the card script rather than the board, so this runs once here.
    pub(crate) fn classify_mana_abilities(&mut self) {
        let verdicts: Vec<(usize, bool)> = self
            .activated_abilities
            .iter()
            .enumerate()
            .map(|(i, ab)| {
                let root = Params::from_raw(&ab.ability_text);
                if root.has(keys::VALID_TGTS)
                    || root.is_true(keys::PLANESWALKER)
                    || cost_moves_card_to_or_from_library(&ab.cost)
                {
                    return (i, false);
                }
                let mut adds_mana = false;
                let mut text = Some(ab.ability_text.clone());
                let mut seen: Vec<String> = Vec::new();
                while let Some(raw) = text {
                    let params = Params::from_raw(&raw);
                    if params
                        .get(keys::AB)
                        .or_else(|| params.get(keys::DB))
                        .is_some_and(|a| {
                            a.eq_ignore_ascii_case("Mana")
                                || a.eq_ignore_ascii_case("ManaReflected")
                        })
                    {
                        adds_mana = true;
                    }
                    if crate::ability::spell_ability_effect::moves_card_to_or_from_library(&raw) {
                        return (i, false);
                    }
                    text = params
                        .get(keys::SUB_ABILITY)
                        .map(str::to_string)
                        .filter(|name| !seen.contains(name))
                        .inspect(|name| seen.push(name.clone()))
                        .and_then(|name| self.svars.get(&name).cloned());
                }
                (i, adds_mana)
            })
            .collect();
        for (i, verdict) in verdicts {
            self.activated_abilities[i].is_mana_ability = verdict;
        }
    }

    /// Generate intrinsic mana abilities for basic land subtypes (Plains → {W}, etc.).
    /// Mirrors Java's `CardFactoryUtil.addIntrinsicAbilities()`.
    pub(crate) fn generate_basic_land_mana_abilities(&mut self) {
        const SUBTYPE_MANA: &[(&str, &str, &str)] = &[
            ("Plains", "W", "Add {W}."),
            ("Island", "U", "Add {U}."),
            ("Swamp", "B", "Add {B}."),
            ("Mountain", "R", "Add {R}."),
            ("Forest", "G", "Add {G}."),
        ];
        for &(subtype, letter, desc) in SUBTYPE_MANA {
            if self.type_line.has_subtype(subtype) {
                let already_produces = self.activated_abilities.iter().any(|ab| {
                    ab.is_mana_ability
                        && ab
                            .produced_ir
                            .as_ref()
                            .is_some_and(|ir| ir.as_script_text() == letter)
                });
                if !already_produces {
                    let raw = format!(
                        "AB$ Mana | Cost$ T | Produced$ {letter} | SpellDescription$ {desc}"
                    );
                    let idx = self.abilities.len();
                    self.abilities.push(raw.clone());
                    if let Some(ab) = parse_activated_ability(&raw, idx) {
                        self.activated_abilities.push(ab);
                    }
                }
            }
        }
    }

    /// Generate activated abilities from keywords (e.g. Cycling → AB$ Draw).
    /// Mirrors Java's `CardFactoryUtil.setupKeywordedAbilities()`.
    pub(super) fn generate_keyword_abilities(&mut self) {
        let mut keywords = self.keywords.as_string_list();
        keywords.extend(self.granted_keywords.as_string_list());
        self.generate_keyword_activated_abilities(&keywords);

        // Enlist: K:Enlist -> intrinsic optional attack cost static ability.
        if self
            .keywords
            .iter_strings()
            .chain(self.granted_keywords.iter_strings())
            .any(|k| k.eq_ignore_ascii_case("Enlist"))
        {
            let raw = "S:Mode$ OptionalAttackCost | ValidCard$ Card.Self | Cost$ Enlist<1/CARDNAME/creature> | Secondary$ True | Trigger$ TrigEnlist";
            if let Some(sa) = parse_static_ability(raw) {
                self.add_static_ability(sa);
            }
            self.svars.entry("TrigEnlist".to_string()).or_insert_with(|| {
                "DB$ Pump | NumAtt$ TriggerRemembered$CardPower | SpellDescription$ When you do, add its power to this creature's until end of turn.".to_string()
            });
        }

        if self
            .keywords
            .iter_strings()
            .chain(self.granted_keywords.iter_strings())
            .any(|k| k == "Decayed")
        {
            let raw = "S:Mode$ CantBlock | ValidCard$ Creature.Self | Secondary$ True | Description$ CARDNAME can't block.";
            if let Some(sa) = parse_static_ability(raw) {
                self.add_static_ability(sa);
            }
        }

        // Morph / Megamorph / Disguise: mark card as castable face-down for {3}.
        // The actual casting logic is in game_action_util (playable check + cost handling).
        if self
            .keywords
            .iter_strings()
            .chain(self.granted_keywords.iter_strings())
            .any(|k| {
                k.starts_with("Morph:") || k.starts_with("Megamorph:") || k.starts_with("Disguise:")
            })
        {
            self.has_morph = true;
        }

        // Class: K:Class:{level}:{cost}:{params} → AB$ ClassLevelUp.
        // Mirrors Java CardFactoryUtil lines 2789-2799.
        let class_keywords: Vec<String> = self
            .keywords
            .iter_strings()
            .chain(self.granted_keywords.iter_strings())
            .filter(|kw| kw.starts_with("Class:"))
            .map(|kw| kw.to_string())
            .collect();
        for kw in class_keywords {
            if let Some(rest) = kw.strip_prefix("Class:") {
                let mut parts = rest.splitn(3, ':');
                let level = parts.next().unwrap_or_default().trim();
                let cost = parts.next().unwrap_or_default().trim();
                let params = parts.next().unwrap_or_default().trim();

                let Ok(level_num) = level.parse::<i32>() else {
                    continue;
                };
                if cost.is_empty() {
                    continue;
                }

                if !params.is_empty() {
                    let parsed = Params::from_raw(params);
                    let mut desc_parts: Vec<String> = Vec::new();

                    if let Some(add_trigger) = parsed.get("AddTrigger") {
                        for svar_name in add_trigger
                            .split(" & ")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            if let Some(svar_params) = self.parsed_svar_params(svar_name) {
                                if let Some(desc) = svar_params.get(keys::TRIGGER_DESCRIPTION) {
                                    desc_parts.push(desc.to_string());
                                }
                            }
                        }
                    }

                    if let Some(add_static) = parsed.get("AddStaticAbility") {
                        for svar_name in add_static
                            .split(" & ")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            if let Some(svar_params) = self.parsed_svar_params(svar_name) {
                                if let Some(desc) = svar_params.get(keys::DESCRIPTION) {
                                    desc_parts.push(desc.to_string());
                                }
                            }
                        }
                    }

                    if let Some(add_replacement) = parsed.get("AddReplacementEffect") {
                        for svar_name in add_replacement
                            .split(" & ")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            if let Some(svar_params) = self.parsed_svar_params(svar_name) {
                                if let Some(desc) = svar_params.get(keys::DESCRIPTION) {
                                    desc_parts.push(desc.to_string());
                                }
                            }
                        }
                    }

                    let mut effect = format!(
                        "Mode$ Continuous | Affected$ Card.Self | ClassLevel$ {level_num} | {params}"
                    );
                    if !desc_parts.is_empty() {
                        effect.push_str(" | Description$ ");
                        effect.push_str(&desc_parts.join("\r\n"));
                    }
                    if let Some(st) = parse_static_ability(&effect) {
                        self.add_static_ability(st);
                    }
                }
            }
        }
    }

    pub(crate) fn generate_keyword_activated_abilities(&mut self, keywords: &[String]) {
        // Cycling: K:Cycling:{cost} → AB$ Draw | Cost$ {cost} Discard<1/CARDNAME> | ActivationZone$ Hand
        if let Some(cycling_cost) = keyword_cost(keywords, "Cycling") {
            let ab_text = format!(
                "AB$ Draw | Cost$ {cycling_cost} Discard<1/CARDNAME> | ActivationZone$ Hand | PrecostDesc$ Cycling | NumCards$ 1 | Defined$ You"
            );
            let next_idx = self.activated_abilities.len();
            if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                self.activated_abilities.push(ab);
            }
        }

        // TypeCycling: K:TypeCycling:{type}:{cost} → AB$ ChangeZone | Cost$ {cost} Discard<1/CARDNAME> | ActivationZone$ Hand
        // Mirrors Java CardFactoryUtil lines 3852-3864.
        for kw in keywords.iter().map(String::as_str) {
            if let Some(rest) = kw.strip_prefix("TypeCycling:") {
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let cycle_type = parts[0].trim(); // e.g., "Swamp"
                    let mana_cost = parts[1].trim(); // e.g., "1"
                                                     // getTitleWithoutCost() = capitalize(descType) + "cycling"
                    let precost_desc = format!(
                        "{}cycling",
                        cycle_type
                            .chars()
                            .next()
                            .map(|c| c.to_uppercase().to_string())
                            .unwrap_or_default()
                            + &cycle_type[1..]
                    );
                    let ab_text = format!(
                        "AB$ ChangeZone | Cost$ {mana_cost} Discard<1/CARDNAME> | ActivationZone$ Hand | PrecostDesc$ {precost_desc} | Origin$ Library | Destination$ Hand | ChangeType$ {cycle_type}"
                    );
                    let next_idx = self.activated_abilities.len();
                    if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                        self.activated_abilities.push(ab);
                    }
                }
            }
        }

        for equip_raw in keywords
            .iter()
            .map(String::as_str)
            .filter_map(|kw| crate::keyword::extract_keyword_cost_str(kw, "Equip"))
        {
            let payload = equip_raw
                .find(":Flavor ")
                .map_or(equip_raw, |idx| &equip_raw[..idx]);
            let k: Vec<&str> = payload.split(':').collect();
            let equip_cost = k[0].trim();
            let target_filter = k
                .get(1)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("Creature.YouCtrl");
            let extra = k.get(3).copied().unwrap_or("");
            if !equip_cost.is_empty() {
                let mut ab_text = format!(
                    "AB$ Attach | Cost$ {equip_cost} | ValidTgts$ {target_filter} | SorcerySpeed$ True | SpellDescription$ Equip {equip_cost}"
                );
                if !extra.is_empty() {
                    ab_text.push_str(" | ");
                    ab_text.push_str(extra);
                }
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }

        // Crew: K:Crew:N → AB$ Animate (tap creatures with total power ≥N).
        // Mirrors Java CardFactoryUtil lines 3820-3835.
        // Uses tapXType<Any/Creature.Other+withTotalPowerGE{N}> matching Java's format.
        for kw in keywords.iter().map(String::as_str) {
            if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Crew") {
                let mut k = n_str.split(':');
                let n = k.next().unwrap_or_default().trim();
                let mut ab_text = format!(
                    "AB$ Animate | Cost$ tapXType<Any/Creature.Other+withTotalPowerGE{{{n}}}> | Defined$ Self | Types$ Artifact,Creature | Secondary$ True | SpellDescription$ Crew {n}"
                );
                if let Some(extra) = k.next() {
                    ab_text.push_str(" | ");
                    ab_text.push_str(extra);
                }
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }

        for kw in keywords.iter().map(String::as_str) {
            if let Some(power) = crate::keyword::extract_keyword_cost_str(kw, "Saddle") {
                let power = power.trim();
                let ab_text = format!(
                    "AB$ AlterAttribute | Cost$ tapXType<Any/Creature.Other+withTotalPowerGE{{{power}}}> | CostDesc$ Saddle {power} | Attributes$ Saddle | Secondary$ True | Defined$ Self | SorcerySpeed$ True | SpellDescription$ Saddle {power}"
                );
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }

        // Station: K:Station:N → AB$ PutCounter (tap another creature to add charge counters).
        // Mirrors Java CardFactoryUtil lines 3587-3595.
        // The ability is sorcery-speed and puts charge counters equal to the tapped
        // creature's power onto this Spacecraft/Planet.
        for kw in keywords.iter().map(String::as_str) {
            if let Some(_n_str) = crate::keyword::extract_keyword_cost_str(kw, "Station") {
                let ab_text = "AB$ PutCounter | Cost$ tapXType<1/Creature.Other> | Defined$ Self | CounterType$ CHARGE | CounterNum$ StationX | SorcerySpeed$ True | CostDesc$ | SpellDescription$ Station";
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
                self.svars
                    .entry("StationX".to_string())
                    .or_insert_with(|| "TappedCards$TapPowerValue".to_string());
            }
        }

        // Embalm: K:Embalm:cost → AB$ CopyPermanent from graveyard.
        // Mirrors Java CardFactoryUtil lines 2879-2891.
        for kw in keywords.iter().map(String::as_str) {
            if let Some(cost_str) = crate::keyword::extract_keyword_cost_str(kw, "Embalm") {
                let cost = cost_str.trim();
                let ab_text = format!(
                    "AB$ CopyPermanent | Cost$ {cost} ExileFromGrave<1/CARDNAME> | ActivationZone$ Graveyard | SorcerySpeed$ True | Defined$ Self | SetColor$ White | AddTypes$ Zombie | SpellDescription$ Embalm"
                );
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }

        // Eternalize: K:Eternalize:cost → AB$ CopyPermanent from graveyard as 4/4.
        // Mirrors Java CardFactoryUtil lines 3023-3052.
        for kw in keywords.iter().map(String::as_str) {
            if let Some(cost_str) = crate::keyword::extract_keyword_cost_str(kw, "Eternalize") {
                let cost = cost_str.trim();
                let ab_text = format!(
                    "AB$ CopyPermanent | Cost$ {cost} ExileFromGrave<1/CARDNAME> | ActivationZone$ Graveyard | SorcerySpeed$ True | Defined$ Self | SetColor$ Black | SetPower$ 4 | SetToughness$ 4 | AddTypes$ Zombie | SpellDescription$ Eternalize"
                );
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }

        // Plot: K:Plot:{cost} → AB$ Plot | Cost$ {cost} | ActivationZone$ Hand | SorcerySpeed$ True
        // Mirrors Java CardFactoryUtil lines 3398-3449.
        // Exiles the card from hand; plotted cards can later be cast for free.
        if let Some(plot_cost) = keyword_cost(keywords, "Plot") {
            let ab_text = format!(
                "AB$ Plot | Cost$ {plot_cost} | ActivationZone$ Hand | SorcerySpeed$ True | Secondary$ True | SpellDescription$ Plot"
            );
            let next_idx = self.activated_abilities.len();
            if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                self.activated_abilities.push(ab);
            }
        }

        // Craft: K:Craft:{cost} → AB$ ChangeZone that exiles this artifact with the cost and
        // returns it transformed. Mirrors Java CardFactoryUtil (`inst instanceof Craft`).
        if let Some(craft) = keyword_cost(keywords, "Craft") {
            let cost = craft.split(':').next().unwrap_or_default().trim();
            let ab_text = format!(
                "AB$ ChangeZone | Cost$ Exile<1/CARDNAME> {cost} | Origin$ Exile | Destination$ Battlefield | Transformed$ True | Defined$ CorrectedSelf | SorcerySpeed$ True | SpellDescription$ Craft"
            );
            let next_idx = self.activated_abilities.len();
            if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                self.activated_abilities.push(ab);
            }
        }

        let class_keywords: Vec<String> = keywords
            .iter()
            .map(String::as_str)
            .filter(|kw| kw.starts_with("Class:"))
            .map(|kw| kw.to_string())
            .collect();
        for kw in class_keywords {
            if let Some(rest) = kw.strip_prefix("Class:") {
                let mut parts = rest.splitn(3, ':');
                let level = parts.next().unwrap_or_default().trim();
                let cost = parts.next().unwrap_or_default().trim();

                let Ok(level_num) = level.parse::<i32>() else {
                    continue;
                };
                if cost.is_empty() {
                    continue;
                }

                let ab_text = format!(
                    "AB$ ClassLevelUp | Cost$ {} | ClassLevel$ EQ{} | SorcerySpeed$ True | StackDescription$ SpellDescription | SpellDescription$ Level {}",
                    cost,
                    level_num - 1,
                    level_num
                );
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                }
            }
        }
    }

    pub fn ensure_crew_activated_ability(&mut self) {
        if self.activated_abilities.iter().any(|ab| {
            ab.spell_description
                .as_deref()
                .is_some_and(|desc| desc.starts_with("Crew"))
        }) {
            return;
        }
        for kw in self.keywords.iter_strings() {
            if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Crew") {
                let mut k = n_str.split(':');
                let n = k.next().unwrap_or_default().trim();
                let mut ab_text = format!(
                    "AB$ Animate | Cost$ tapXType<Any/Creature.Other+withTotalPowerGE{{{n}}}> | Defined$ Self | Types$ Artifact,Creature | Secondary$ True | SpellDescription$ Crew {n}"
                );
                if let Some(extra) = k.next() {
                    ab_text.push_str(" | ");
                    ab_text.push_str(extra);
                }
                let next_idx = self.activated_abilities.len();
                if let Some(ab) = parse_activated_ability(&ab_text, next_idx) {
                    self.activated_abilities.push(ab);
                    self.base_ability_count = self.activated_abilities.len();
                }
                return;
            }
        }
    }

    pub(crate) fn add_intrinsic_keyword_with_triggers(&mut self, kw: &str) {
        if !self.add_intrinsic_keyword(kw) {
            return;
        }
        let mut next_id = self.triggers.iter().map(|t| t.id + 1).max().unwrap_or(0);
        self.generate_keyword_trigger_combat(kw, &mut next_id);
        self.generate_keyword_trigger_zone(kw, &mut next_id);
        self.add_keyword_etb_counters(kw);
        self.generate_keyword_trigger_misc(kw, &mut next_id);
        self.base_trigger_count = self.triggers.len();
    }

    fn add_keyword_etb_counters(&mut self, kw: &str) {
        if let Some(n) = crate::keyword::extract_keyword_cost_str(kw, "Modular")
            .and_then(|n_str| n_str.parse::<i32>().ok())
        {
            self.add_etb_counter(None, crate::card::CounterType::P1P1, n);
        }
    }

    pub(crate) fn generate_keyword_triggers_for(&mut self, keywords: &[String]) {
        let mut next_id = self.triggers.iter().map(|t| t.id + 1).max().unwrap_or(0);
        for kw in keywords {
            self.generate_keyword_trigger_combat(kw, &mut next_id);
            self.generate_keyword_trigger_zone(kw, &mut next_id);
            self.generate_keyword_trigger_misc(kw, &mut next_id);
        }
    }

    /// Generate triggered abilities from keywords (e.g. Prowess, Bushido, Annihilator, etc.).
    /// Mirrors Java's `CardFactoryUtil.setupKeywordedTriggers()`.
    pub fn generate_keyword_triggers(&mut self) {
        let mut next_id = self.triggers.len() as u32;

        for kw in self.keywords.as_string_list() {
            self.generate_keyword_trigger_combat(&kw, &mut next_id);
            self.generate_keyword_trigger_zone(&kw, &mut next_id);
            self.add_keyword_etb_counters(&kw);
            self.generate_keyword_trigger_misc(&kw, &mut next_id);
        }
    }

    fn generate_keyword_trigger_combat(&mut self, kw: &str, next_id: &mut u32) {
        if kw == "Prowess" {
            let raw = "Mode$ SpellCast | ValidCard$ Card.nonCreature | ValidActivatingPlayer$ You | Execute$ TrigProwess | TriggerZones$ Battlefield | TriggerDescription$ Prowess";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigProwess".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigProwess".to_string())
                .or_insert_with(|| "DB$ Pump | Defined$ Self | NumAtt$ 1 | NumDef$ 1".to_string());
        }

        if kw == "Decayed" {
            let raw = "Mode$ Attacks | ValidCard$ Card.Self | Secondary$ True | Execute$ TrigDecayed | TriggerDescription$ When a creature with decayed attacks, sacrifice it at end of combat.";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigDecayed".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigDecayed".to_string())
                .or_insert_with(|| {
                    "DB$ DelayedTrigger | Mode$ Phase | Phase$ EndCombat | Execute$ TrigDecayedSac | TriggerDescription$ At end of combat, sacrifice CARDNAME.".to_string()
                });
            self.svars
                .entry("TrigDecayedSac".to_string())
                .or_insert_with(|| "DB$ Sacrifice".to_string());
        }

        if kw == "Storied" && self.is_permanent() {
            let raw = "Mode$ Always | TriggerZones$ Battlefield | Secondary$ True | Static$ True | EnduringStory$ False | IsPresent$ Permanent.YouCtrl+Historic | PresentCompare$ GE3 | Execute$ TrigStoried | TriggerDescription$ Storied";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigStoried".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigStoried".to_string())
                .or_insert_with(|| "DB$ InternalEnduringStory".to_string());
        }

        if kw == "Increment" {
            let raw = "Mode$ SpellCast | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Secondary$ True | Execute$ TrigIncrement | TriggerDescription$ Increment";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigIncrement".to_string();
                trig.base
                    .set_keyword(crate::keyword::keyword_interface::KeywordInterface::new(
                        crate::keyword::Keyword::Increment,
                        kw,
                    ));
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigIncrement".to_string())
                .or_insert_with(|| {
                    "DB$ PutCounter | CounterType$ P1P1 | CounterNum$ 1".to_string()
                });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Bushido") {
            if n_str.parse::<i32>().is_ok() {
                let raw1 = format!(
                    "Mode$ Blocks | ValidCard$ Card.Self | Execute$ TrigBushido | TriggerZones$ Battlefield | TriggerDescription$ Bushido {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw1, next_id) {
                    trig.execute = "TrigBushido".to_string();
                    self.add_trigger(trig);
                }
                let raw2 = format!(
                    "Mode$ AttackerBlocked | ValidCard$ Card.Self | Execute$ TrigBushido | TriggerZones$ Battlefield | TriggerDescription$ Bushido {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw2, next_id) {
                    trig.execute = "TrigBushido".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigBushido".to_string())
                    .or_insert_with(|| {
                        format!("DB$ Pump | Defined$ Self | NumAtt$ {n_str} | NumDef$ {n_str}")
                    });
            }
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Annihilator") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigAnnihilator | TriggerZones$ Battlefield | TriggerDescription$ Annihilator {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigAnnihilator".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigAnnihilator".to_string())
                    .or_insert_with(|| {
                        format!("DB$ Sacrifice | Defined$ TriggeredDefendingPlayer | SacValid$ Permanent | Amount$ {n_str}")
                    });
            }
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Mobilize") {
            let raw = format!(
                "Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigMobilize | TriggerDescription$ Mobilize {n_str}"
            );
            if let Some(mut trig) = parse_trigger(&raw, next_id) {
                trig.execute = "TrigMobilize".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigMobilize".to_string())
                .or_insert_with(|| {
                    format!("DB$ Token | TokenAmount$ {n_str} | TokenScript$ r_1_1_warrior | TokenTapped$ True | TokenAttacking$ True | AtEOT$ Sacrifice")
                });
        }

        if let Some(details) = crate::keyword::extract_keyword_cost_str(kw, "Firebending") {
            let mut parts = details.splitn(2, ':');
            let n_str = parts.next().unwrap_or_default();
            let desc = format!("Firebending {n_str}{}", parts.next().unwrap_or_default());
            let raw =
                format!("Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigFirebending | TriggerDescription$ {desc}");
            if let Some(mut trig) = parse_trigger(&raw, next_id) {
                trig.execute = "TrigFirebending".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigFirebending".to_string())
                .or_insert_with(|| {
                    format!("DB$ Mana | Defined$ You | CombatMana$ True | Produced$ R | Amount$ {n_str}")
                });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Afflict") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ AttackerBlocked | ValidCard$ Card.Self | TriggerZones$ Battlefield | Secondary$ True | Execute$ TrigAfflict | TriggerDescription$ Afflict {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigAfflict".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigAfflict".to_string())
                    .or_insert_with(|| {
                        format!("DB$ LoseLife | Defined$ TriggeredDefendingPlayer | LifeAmount$ {n_str}")
                    });
            }
        }

        if kw == "Battle cry" {
            let raw = "Mode$ Attacks | ValidCard$ Card.Self | TriggerZones$ Battlefield | Secondary$ True | Execute$ BattleCryPumpAll | TriggerDescription$ Battle cry";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "BattleCryPumpAll".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("BattleCryPumpAll".to_string())
                .or_insert_with(|| {
                    "DB$ PumpAll | ValidCards$ Creature.attacking+Other | NumAtt$ 1".to_string()
                });
        }

        if kw == "Exalted" {
            let raw = "Mode$ Attacks | ValidCard$ Creature.YouCtrl | Alone$ True | Execute$ TrigExalted | TriggerZones$ Battlefield | TriggerDescription$ Exalted";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigExalted".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigExalted".to_string())
                .or_insert_with(|| {
                    "DB$ Pump | Defined$ TriggeredAttacker | NumAtt$ +1 | NumDef$ +1".to_string()
                });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Renown") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player | CombatDamage$ True | Execute$ TrigRenown | TriggerZones$ Battlefield | TriggerDescription$ Renown {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigRenown".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigRenown".to_string())
                    .or_insert_with(|| {
                        format!("DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ {n_str} | Renown$ True")
                    });
            }
        }

        if kw == "Flanking" {
            let raw = "Mode$ AttackerBlockedByCreature | ValidCard$ Card.Self | ValidBlocker$ Creature.withoutFlanking | TriggerZones$ Battlefield | Secondary$ True | TriggerDescription$ Flanking";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigFlanking".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigFlanking".to_string())
                .or_insert_with(|| {
                    "DB$ Pump | Defined$ TriggeredBlockerLKICopy | NumAtt$ -1 | NumDef$ -1"
                        .to_string()
                });
        }

        if kw == "Extort" {
            let raw = "Mode$ SpellCast | ValidActivatingPlayer$ You | Execute$ TrigExtort | TriggerZones$ Battlefield | TriggerDescription$ Extort";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigExtort".to_string();
                trig.optional = true;
                self.add_trigger(trig);
            }
            self.svars.entry("TrigExtort".to_string()).or_insert_with(|| {
                "AB$ LoseLife | Cost$ WB | Defined$ Player.Opponent | LifeAmount$ 1 | SubAbility$ ExtortGain"
                    .to_string()
            });
            self.svars
                .entry("ExtortGain".to_string())
                .or_insert_with(|| {
                    "DB$ GainLife | Defined$ You | LifeAmount$ AFLifeLost".to_string()
                });
            self.svars
                .entry("AFLifeLost".to_string())
                .or_insert_with(|| "Number$0".to_string());
        }
    }

    fn generate_keyword_trigger_zone(&mut self, kw: &str, next_id: &mut u32) {
        self.generate_keyword_trigger_zone_graveyard(kw, next_id);
        self.generate_keyword_trigger_zone_battlefield(kw, next_id);
    }

    fn generate_keyword_trigger_zone_graveyard(&mut self, kw: &str, next_id: &mut u32) {
        if kw == "Undying" {
            let raw = "Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self+counters_EQ0_P1P1 | TriggerZones$ Battlefield | Execute$ TrigUndying | TriggerDescription$ Undying";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigUndying".to_string();
                self.add_trigger(trig);
            }
            self.svars.entry("TrigUndying".to_string()).or_insert_with(|| {
                "DB$ ChangeZone | Defined$ TriggeredNewCardLKICopy | Origin$ Graveyard | Destination$ Battlefield | WithCountersType$ P1P1".to_string()
            });
        }

        if kw == "Persist" {
            let raw = "Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self+counters_EQ0_M1M1 | TriggerZones$ Battlefield | Execute$ TrigPersist | TriggerDescription$ Persist";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigPersist".to_string();
                self.add_trigger(trig);
            }
            self.svars.entry("TrigPersist".to_string()).or_insert_with(|| {
                "DB$ ChangeZone | Defined$ TriggeredNewCardLKICopy | Origin$ Graveyard | Destination$ Battlefield | WithCountersType$ M1M1".to_string()
            });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Afterlife") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigAfterlife | TriggerDescription$ Afterlife {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigAfterlife".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigAfterlife".to_string())
                    .or_insert_with(|| {
                        format!(
                            "DB$ Token | TokenAmount$ {n_str} | TokenScript$ wb_1_1_spirit_flying"
                        )
                    });
            }
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Modular") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigModular | TriggerDescription$ Modular {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigModular".to_string();
                    trig.optional = true;
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigModular".to_string())
                    .or_insert_with(|| "SP$ Charm | Choices$ ModularMove".to_string());
                self.svars
                    .entry("ModularMove".to_string())
                    .or_insert_with(|| {
                        format!("DB$ PutCounter | Defined$ Targeted | CounterType$ P1P1 | CounterNum$ {n_str} | Modular$ true | ValidTgts$ Creature.Artifact | SpellDescription$ Put +1/+1 counter(s) on target artifact creature")
                    });
            }
        }
    }

    fn generate_keyword_trigger_zone_battlefield(&mut self, kw: &str, next_id: &mut u32) {
        if let Some(cost) = crate::keyword::extract_keyword_cost_str(kw, "Offspring") {
            let raw = format!(
                "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | CheckSVar$ Offspring | Secondary$ True | Execute$ TrigOffspring | TriggerDescription$ Offspring {cost}"
            );
            if let Some(mut trig) = parse_trigger(&raw, next_id) {
                trig.execute = "TrigOffspring".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("Offspring".to_string())
                .or_insert_with(|| "Count$OptionalKeywordAmount".to_string());
            self.svars
                .entry("TrigOffspring".to_string())
                .or_insert_with(|| {
                    "DB$ CopyPermanent | Defined$ TriggeredCardLKICopy | SetPower$ 1 | SetToughness$ 1"
                        .to_string()
                });
        }

        if kw.starts_with("Impending:") {
            let raw = "Mode$ Phase | Phase$ End of Turn | ValidPlayer$ You | TriggerZones$ Battlefield | IsPresent$ Card.Self+impended+counters_GE1_TIME | Secondary$ True | Execute$ TrigImpending | TriggerDescription$ At the beginning of your end step, remove a time counter from it.";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigImpending".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigImpending".to_string())
                .or_insert_with(|| {
                    "DB$ RemoveCounter | Defined$ Self | CounterType$ TIME | CounterNum$ 1"
                        .to_string()
                });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Hideaway") {
            let raw = format!(
                "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Secondary$ True | Execute$ TrigHideaway | TriggerDescription$ Hideaway {n_str}"
            );
            if let Some(mut trig) = parse_trigger(&raw, next_id) {
                trig.execute = "TrigHideaway".to_string();
                self.add_trigger(trig);
            }
            self.svars.entry("TrigHideaway".to_string()).or_insert_with(|| {
                format!("DB$ Dig | Defined$ You | DigNum$ {n_str} | DestinationZone$ Exile | ExileFaceDown$ True | RememberChanged$ True | RestRandomOrder$ True | SubAbility$ DBHideawayEffect")
            });
            self.svars
                .entry("DBHideawayEffect".to_string())
                .or_insert_with(|| {
                    "DB$ Effect | StaticAbilities$ STHideawayEffectLookAtCard | ForgetOnMoved$ Exile | RememberObjects$ Remembered | Duration$ Permanent | SubAbility$ DBHideawayCleanup".to_string()
                });
            self.svars
                .entry("STHideawayEffectLookAtCard".to_string())
                .or_insert_with(|| {
                    "Mode$ Continuous | Affected$ Card.IsRemembered | MayLookAt$ EffectSourceController | EffectZone$ Command | AffectedZone$ Exile | Description$ Any player who has controlled the permanent that exiled this card may look at this card in the exile zone.".to_string()
                });
            self.svars
                .entry("DBHideawayCleanup".to_string())
                .or_insert_with(|| "DB$ Cleanup | ClearRemembered$ True".to_string());
        }

        if kw == "Exploit" {
            let raw = "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigExploit | TriggerDescription$ Exploit";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigExploit".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigExploit".to_string())
                .or_insert_with(|| {
                    "DB$ Sacrifice | SacValid$ Creature | Optional$ True | Exploit$ True"
                        .to_string()
                });
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Fabricate") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigFabricate | Secondary$ True | TriggerDescription$ Fabricate {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigFabricate".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigFabricate".to_string())
                    .or_insert_with(|| {
                        format!(
                            "DB$ Token | TokenAmount$ {n_str} | TokenScript$ c_1_1_a_servo \
                             | UnlessCost$ AddCounter<{n_str}/P1P1> | UnlessPayer$ You \
                             | SpellDescription$ Fabricate {n_str}"
                        )
                    });
            }
        }

        if kw == "Living Weapon" {
            let raw = "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Secondary$ True | TriggerDescription$ Living Weapon";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigLivingWeapon".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigLivingWeapon".to_string())
                .or_insert_with(|| {
                    "DB$ Token | TokenScript$ b_0_0_phyrexian_germ | TokenOwner$ You | RememberTokens$ True | SubAbility$ DBLivingWeaponAttach".to_string()
                });
            self.svars
                .entry("DBLivingWeaponAttach".to_string())
                .or_insert_with(|| {
                    "DB$ Attach | Defined$ Remembered | SubAbility$ DBLivingWeaponCleanup"
                        .to_string()
                });
            self.svars
                .entry("DBLivingWeaponCleanup".to_string())
                .or_insert_with(|| "DB$ Cleanup | ClearRemembered$ True".to_string());
        }

        if kw == "Job select" {
            let raw = "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | TriggerDescription$ Job select";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigJobSelect".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigJobSelect".to_string())
                .or_insert_with(|| {
                    "DB$ Token | TokenScript$ c_1_1_hero | TokenOwner$ You | RememberTokens$ True | SubAbility$ DBJobSelectAttach".to_string()
                });
            self.svars
                .entry("DBJobSelectAttach".to_string())
                .or_insert_with(|| {
                    "DB$ Attach | Defined$ Remembered | SubAbility$ DBJobSelectCleanup".to_string()
                });
            self.svars
                .entry("DBJobSelectCleanup".to_string())
                .or_insert_with(|| "DB$ Cleanup | ClearRemembered$ True".to_string());
        }

        if let Some(n_str) = crate::keyword::extract_keyword_cost_str(kw, "Bloodthirst") {
            if n_str.parse::<i32>().is_ok() {
                let raw = format!(
                    "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigBloodthirst | TriggerDescription$ Bloodthirst {n_str}"
                );
                if let Some(mut trig) = parse_trigger(&raw, next_id) {
                    trig.execute = "TrigBloodthirst".to_string();
                    self.add_trigger(trig);
                }
                self.svars
                    .entry("TrigBloodthirst".to_string())
                    .or_insert_with(|| {
                        format!("DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ {n_str} | Bloodthirst$ True")
                    });
            }
        }

        if kw == "Unleash" {
            let raw = "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigUnleash | TriggerDescription$ Unleash";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigUnleash".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigUnleash".to_string())
                .or_insert_with(|| {
                    "DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1".to_string()
                });
        }
    }

    pub(crate) fn generate_keyword_paradigm(&mut self) {
        if !self.keywords.iter_strings().any(|kw| kw == "Paradigm") {
            return;
        }
        let Some(idx) = self
            .abilities
            .iter()
            .position(|a| crate::parsing::raw_has_key(a, keys::SP))
        else {
            return;
        };
        let mut last = None;
        let mut text = self.abilities[idx].clone();
        while let Some(sub) = crate::parsing::raw_get(&text, keys::SUB_ABILITY).map(str::to_string)
        {
            if sub == "ParadigmExile" || last.as_ref() == Some(&sub) {
                return;
            }
            let Some(next) = self.svars.get(&sub).cloned() else {
                return;
            };
            last = Some(sub);
            text = next;
        }
        let appended = format!("{text} | SubAbility$ ParadigmExile");
        match last {
            Some(name) => {
                self.svars.insert(name, appended);
            }
            None => self.abilities[idx] = appended,
        }
        self.svars.insert(
            "ParadigmExile".to_string(),
            "DB$ ChangeZone | Defined$ Self | Origin$ Stack | Destination$ Exile | SubAbility$ ParadigmEffect".to_string(),
        );
        self.svars.insert(
            "ParadigmEffect".to_string(),
            format!(
                "DB$ Effect | Triggers$ ParadigmTrigger | Duration$ Permanent | Unique$ True | Name$ {}' Paradigm",
                self.card_name
            ),
        );
        self.svars.insert(
            "ParadigmTrigger".to_string(),
            "Mode$ Phase | Phase$ Main1 | ValidPlayer$ You | OptionalDecider$ You | Execute$ ParadigmCopy | TriggerDescription$ Paradigm".to_string(),
        );
        self.svars.insert(
            "ParadigmCopy".to_string(),
            "DB$ Play | Defined$ EffectSource | ValidSA$ Spell | ZoneRegardless$ True | WithoutManaCost$ True | Optional$ True | CopyCard$ True".to_string(),
        );
    }

    pub(crate) fn generate_keyword_chapter_triggers(&mut self) {
        if !self.has_subtype("Saga") {
            return;
        }

        let mut next_id = self.triggers.len() as u32;
        for kw in self.keywords.as_string_list() {
            if !kw.starts_with("Chapter") {
                continue;
            }
            let Some((count, svars)) = kw
                .strip_prefix("Chapter:")
                .and_then(|value| value.split_once(':'))
            else {
                panic!("invalid Chapter keyword: {kw}");
            };
            let count = count
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("invalid Chapter count: {count}"));
            let svars: Vec<&str> = svars.split(',').collect();
            assert!(
                !svars.iter().any(|svar| svar.is_empty()),
                "Chapter ability list must not contain empty SVars"
            );
            assert_eq!(svars.len(), count, "Saga max differ from Ability amount");

            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            for (chapter, svar) in svars.iter().enumerate() {
                let chapter = chapter + 1;
                if let Some((_, chapters)) = groups
                    .iter_mut()
                    .find(|(existing_svar, _)| existing_svar == svar)
                {
                    chapters.push(chapter);
                } else {
                    groups.push(((*svar).to_string(), vec![chapter]));
                }
            }

            for (svar, chapters) in groups {
                let description = self
                    .get_s_var(&svar)
                    .and_then(|raw| {
                        raw.split('|').find_map(|param| {
                            param
                                .trim()
                                .strip_prefix("SpellDescription$")
                                .map(str::trim)
                        })
                    })
                    .map(str::to_string)
                    .unwrap_or_default();
                let grouped_chapters = chapters
                    .iter()
                    .map(|chapter| roman_chapter(*chapter))
                    .collect::<Vec<_>>()
                    .join(", ");

                for (index, chapter) in chapters.iter().enumerate() {
                    let mut trigger = format!(
                        "Mode$ CounterAdded | ValidCard$ Card.Self | TriggerZones$ Battlefield | Chapter$ {chapter} | CounterType$ LORE | CounterAmount$ EQ{chapter} | Execute$ {svar}"
                    );
                    if index > 0 {
                        trigger.push_str(" | Secondary$ True");
                    }
                    trigger.push_str(&format!(
                        " | TriggerDescription$ {grouped_chapters} — {description}"
                    ));
                    let Some(trigger) = parse_trigger(&trigger, &mut next_id) else {
                        panic!("invalid Chapter trigger");
                    };
                    self.add_trigger(trigger);
                }
            }
        }
    }

    fn generate_keyword_trigger_misc(&mut self, kw: &str, next_id: &mut u32) {
        if kw == "Storm" {
            let raw = "Mode$ SpellCast | ValidCard$ Card.Self | TriggerZones$ Stack | Secondary$ True | TriggerDescription$ Storm";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigStorm".to_string();
                self.add_trigger(trig);
            }
            self.svars.entry("TrigStorm".to_string()).or_insert_with(|| {
                "DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | Amount$ StormCount | MayChooseTarget$ True".to_string()
            });
            self.svars
                .entry("StormCount".to_string())
                .or_insert_with(|| "TriggerCount$CurrentStormCount/Minus.1".to_string());
        }

        if let Some(cost_str) = crate::keyword::extract_keyword_cost_str(kw, "Ward") {
            let raw = "Mode$ BecomesTarget | ValidSource$ SpellAbility.OppCtrl | ValidTarget$ Card.Self | Secondary$ True | TriggerZones$ Battlefield | TriggerDescription$ Ward";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigWard".to_string();
                self.add_trigger(trig);
            }
            self.svars.entry("TrigWard".to_string()).or_insert_with(|| {
                format!("DB$ Counter | Defined$ TriggeredSourceSA | UnlessCost$ {cost_str} | UnlessPayer$ TriggeredSourceSAController")
            });
        }

        if let Some(rest) = kw.strip_prefix("Cumulative upkeep:") {
            let cost_spec = rest.split(':').next().unwrap_or(rest);
            let raw = "Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | TriggerDescription$ Cumulative upkeep";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigCumulativeUpkeep".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigCumulativeUpkeep".to_string())
                .or_insert_with(|| {
                    format!("DB$ Sacrifice | SacValid$ Self | CumulativeUpkeep$ {cost_spec}")
                });
        }

        if let Some(cost_str) = crate::keyword::extract_keyword_cost_str(kw, "Echo") {
            let cost_spec = cost_str.split(':').next().unwrap_or(cost_str);
            let raw = "Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | IsPresent$ Card.Self+cameUnderControlSinceLastUpkeep | Secondary$ True | TriggerDescription$ Echo";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigEcho".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigEcho".to_string())
                .or_insert_with(|| format!("DB$ Sacrifice | SacValid$ Self | Echo$ {cost_spec}"));
        }

        if let Some(madness_cost) = crate::keyword::extract_keyword_cost_str(kw, "Madness") {
            let raw = "Mode$ ChangesZone | Origin$ Hand | Destination$ Exile | ValidCard$ Card.Self | Secondary$ True | TriggerZones$ Exile | TriggerDescription$ You may cast this card for its madness cost.";
            if let Some(mut trig) = parse_trigger(raw, next_id) {
                trig.execute = "TrigMadnessPlay".to_string();
                self.add_trigger(trig);
            }
            self.svars
                .entry("TrigMadnessPlay".to_string())
                .or_insert_with(|| {
                    format!(
                        "DB$ Play | Defined$ Self | ValidSA$ Spell | PlayCost$ {madness_cost} | Optional$ True | RememberPlayed$ True | SubAbility$ MadnessMoveToYard"
                    )
                });
            self.svars
                .entry("MadnessMoveToYard".to_string())
                .or_insert_with(|| {
                    "DB$ ChangeZone | Defined$ Self | Origin$ Exile | Destination$ Graveyard | TrackDiscarded$ True | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ EQ0 | SubAbility$ MadnessCleanup".to_string()
                });
            self.svars
                .entry("MadnessCleanup".to_string())
                .or_insert_with(|| "DB$ Cleanup | ClearRemembered$ True".to_string());
        }

        if let Some(partner_name) = kw.strip_prefix("Partner with:") {
            let mut partner_name = partner_name.trim().to_string();
            let raw = format!(
                "Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Secondary$ True | TriggerDescription$ Partner with {partner_name}"
            );
            if let Some(mut trig) = parse_trigger(&raw, next_id) {
                trig.execute = "TrigPartnerWith".to_string();
                self.add_trigger(trig);
            }
            partner_name = partner_name.replace(',', ";");
            self.svars
                .entry("TrigPartnerWith".to_string())
                .or_insert_with(|| {
                    format!(
                        "DB$ ChangeZone | ValidTgts$ Player | Origin$ Library | Destination$ Hand | ChangeType$ Card.named{partner_name} | Hidden$ True | Chooser$ Targeted | Optional$ True"
                    )
                });
        }
    }
}
