use std::path::Path;

use forge_carddb::{CardDatabase, CardFace, CardRules};
use forge_foundation::color::{Color, ColorSet};
use forge_foundation::CoreType;

use crate::deck_generator::{format_inline, parse_inline, DeckSpec};
use crate::probe::{coverage_names, DEFAULT_OPPONENT, DEFAULT_PARTNERS};
use crate::script_index::{keyword_names, parse_params, trait_lines, TraitKind, TraitLine};

const RAMP_MANA_VALUE: i32 = 6;
const LANDS: usize = 24;
const RAMP_LANDS: usize = 36;
const COPIES: usize = 12;
const COPIES_WITH_PARTNERS: usize = 8;
const NAMES_PER_GROUP: usize = 2;
const CARDS_PER_GROUP: usize = 8;
const GRAVEYARD_FILLERS: usize = 16;
const GROUPS_PER_SIDE: usize = 2;
const MAX_PARTNER_MANA_VALUE: i32 = 4;
const PASSIVE_OPPONENT: &str = "Plains*60";
const TRADE_TRIGGERS: [&str; 2] = ["AttackerBlocked", "Blocks"];
const CONNECT_TRIGGERS: [&str; 3] = ["Attacks", "DamageDone", "DamageDoneOnce"];

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Battlefield,
    Graveyard,
    Stack,
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Own,
    Opponent,
}

#[derive(PartialEq)]
enum Wants {
    Ramp,
    Tokens,
    Cards {
        zone: Zone,
        filter: &'static str,
        target_type: Option<&'static str>,
        stat: Option<&'static str>,
    },
}

struct Need {
    reason: String,
    side: Side,
    wants: Wants,
    presence: bool,
    cards: usize,
}

#[derive(Default)]
struct ScriptNeeds {
    needs: Vec<Need>,
    ramp: bool,
    slow: bool,
    trades: bool,
    connects: bool,
}

impl ScriptNeeds {
    fn push(&mut self, reason: String, side: Side, wants: Wants, presence: bool) {
        let fills_graveyard = !presence && wants.zone() == Some(Zone::Graveyard);
        self.slow |= fills_graveyard || wants == Wants::Ramp;
        if !self
            .needs
            .iter()
            .any(|n| n.side == side && n.wants == wants)
        {
            self.needs.push(Need {
                reason,
                side,
                wants,
                presence,
                cards: if fills_graveyard {
                    GRAVEYARD_FILLERS
                } else {
                    CARDS_PER_GROUP
                },
            });
        }
    }
}

pub struct SupportPool {
    cards: Vec<(String, &'static CardRules)>,
}

impl SupportPool {
    pub fn read(db: &CardDatabase, tsv: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(tsv).map_err(|e| format!("{}: {e}", tsv.display()))?;
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
        let column = |name: &str| {
            header
                .iter()
                .position(|h| *h == name)
                .ok_or_else(|| format!("{}: no {name} column", tsv.display()))
        };
        let (card, result) = (column("card")?, column("result")?);
        let mut cards: Vec<_> = lines
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .filter(|fields| fields.get(result) == Some(&"PASS"))
            .filter_map(|fields| {
                let rules = db.get_by_card_name(fields.get(card)?)?;
                let single_faced = rules.other_part.is_none() && rules.specialized_parts.is_empty();
                (single_faced && !rules.main_part.type_line.is_land())
                    .then(|| (rules.name(), rules))
            })
            .collect();
        cards.sort_by(|a, b| a.0.cmp(&b.0));
        cards.dedup_by(|a, b| a.0 == b.0);
        Ok(Self { cards })
    }

    fn choose(
        &self,
        need: &Need,
        identity: ColorSet,
        exclude: &[String],
    ) -> Vec<&(String, &'static CardRules)> {
        let mut candidates: Vec<_> = self
            .cards
            .iter()
            .filter(|(name, rules)| !exclude.contains(name) && need.wants.accepts(rules))
            .collect();
        candidates.sort_by_cached_key(|(name, rules)| {
            let face = &rules.main_part;
            let colors = face.mana_cost.color_set();
            let off_colors = match need.side {
                Side::Own => colors.count_colors() - colors.intersection(identity).count_colors(),
                Side::Opponent => colors.count_colors(),
            };
            (
                has_type(face, "Legendary"),
                off_colors,
                need.wants.rank(rules),
                face.mana_cost.cmc(),
                name.clone(),
            )
        });
        candidates.truncate(NAMES_PER_GROUP);
        candidates
    }
}

pub struct Support {
    pub deck: DeckSpec,
    pub opponent: DeckSpec,
    pub reasons: Vec<String>,
}

fn has_type(face: &CardFace, ty: &str) -> bool {
    let line = &face.type_line;
    match ty {
        "Card" => true,
        "Permanent" => line.is_permanent(),
        _ => {
            line.core_types.iter().any(|t| t.name() == ty)
                || line.supertypes.iter().any(|t| t.name() == ty)
                || line.subtypes.iter().any(|t| t == ty)
        }
    }
}

fn has_property(face: &CardFace, property: &str) -> bool {
    if property == "token" {
        return false;
    }
    if let Some(ty) = property.strip_prefix("non") {
        return !has_type(face, ty);
    }
    let stats = [
        ("cmc", Some(face.mana_cost.cmc())),
        ("power", face.int_power),
        ("toughness", face.int_toughness),
    ];
    for (stat, value) in stats {
        let Some(comparison) = property.strip_prefix(stat) else {
            continue;
        };
        let (op, amount) = comparison.split_at(comparison.len().min(2));
        let Ok(amount) = amount.parse::<i32>() else {
            return true;
        };
        return value.is_some_and(|v| match op {
            "GE" => v >= amount,
            "LE" => v <= amount,
            "GT" => v > amount,
            "LT" => v < amount,
            "EQ" => v == amount,
            _ => v != amount,
        });
    }
    let is_type = CoreType::ALL.iter().any(|t| t.name() == property) || property == "Legendary";
    !is_type || has_type(face, property)
}

fn matches(face: &CardFace, filter: &str) -> bool {
    filter.split(',').any(|alternative| {
        let mut parts = alternative.split(['.', '+']);
        parts.next().is_some_and(|ty| has_type(face, ty)) && parts.all(|p| has_property(face, p))
    })
}

fn spell_targets(face: &CardFace) -> impl Iterator<Item = Option<&str>> {
    face.abilities
        .iter()
        .map(|raw| parse_params(raw))
        .filter(|params| params.iter().any(|(key, _)| *key == "SP"))
        .map(|params| {
            params
                .into_iter()
                .find(|(key, _)| *key == "ValidTgts")
                .map(|(_, value)| value)
        })
}

fn casts_without_board(face: &CardFace) -> bool {
    !has_type(face, "Aura")
        && spell_targets(face)
            .all(|tgts| tgts.is_none_or(|v| v.contains("Any") || v.contains("Player")))
}

fn accepts_target_type(face: &CardFace, target_type: &str) -> bool {
    target_type.split(',').any(|entry| {
        let (kind, properties) = entry.split_once('.').unwrap_or((entry, ""));
        match kind {
            "Spell" => {
                !face.type_line.is_land()
                    && properties
                        .split('+')
                        .filter(|p| !p.is_empty())
                        .all(|p| has_property(face, p))
            }
            "Instant" | "Sorcery" => has_type(face, kind),
            "SpellAbility" => spell_targets(face).any(|tgts| tgts.is_some()),
            _ => false,
        }
    })
}

fn taps_for_mana(raw: &str) -> bool {
    let params = parse_params(raw);
    params.contains(&("AB", "Mana"))
        && params.contains(&("Cost", "T"))
        && params.iter().all(|(key, value)| match *key {
            "Produced" => !value.starts_with("Special"),
            _ => !matches!(*key, "RestrictValid" | "IsPresent" | "ActivationLimit"),
        })
}

impl Wants {
    fn zone(&self) -> Option<Zone> {
        match self {
            Wants::Cards { zone, .. } => Some(*zone),
            _ => None,
        }
    }

    fn accepts(&self, rules: &'static CardRules) -> bool {
        let face = &rules.main_part;
        let cmc = face.mana_cost.cmc();
        match self {
            Wants::Ramp => {
                face.type_line.is_permanent()
                    && cmc <= 2
                    && face.abilities.iter().any(|raw| taps_for_mana(raw))
            }
            Wants::Tokens => cmc <= 3 && trait_lines(rules).iter().any(|l| l.owner() == "Token"),
            Wants::Cards {
                zone,
                filter,
                target_type,
                ..
            } => {
                cmc <= MAX_PARTNER_MANA_VALUE
                    && matches(face, filter)
                    && match zone {
                        Zone::Battlefield => face.type_line.is_permanent(),
                        Zone::Graveyard => true,
                        Zone::Stack => target_type.is_none_or(|t| accepts_target_type(face, t)),
                    }
            }
        }
    }

    fn rank(&self, rules: &'static CardRules) -> (bool, bool, bool, i32) {
        let face = &rules.main_part;
        let line = &face.type_line;
        let spell = line.is_instant() || line.is_sorcery();
        let (preferred, stat) = match self {
            Wants::Ramp => (line.is_creature(), None),
            Wants::Tokens => (spell, None),
            Wants::Cards { zone, stat, .. } => (*zone != Zone::Graveyard || spell, *stat),
        };
        let graveyard = self.zone() == Some(Zone::Graveyard);
        let mills_itself = trait_lines(rules).iter().any(|l| {
            matches!(l.owner().as_str(), "Mill" | "Surveil")
                && l.param("ValidTgts").is_none()
                && l.param("Defined").is_none_or(|d| d == "You")
        });
        let stat = match stat {
            Some("CardPower") => -face.int_power.unwrap_or(0),
            Some("CardToughness") => -face.int_toughness.unwrap_or(0),
            _ => 0,
        };
        (
            !preferred,
            graveyard && !mills_itself,
            !casts_without_board(face),
            stat,
        )
    }
}

fn angle_costs<'a>(cost: &'a str, kind: &str) -> Vec<&'a str> {
    cost.split(' ')
        .filter_map(|part| {
            part.strip_prefix(kind)?
                .strip_prefix('<')?
                .strip_suffix('>')
        })
        .filter_map(|inner| inner.split('/').nth(1))
        .collect()
}

fn counted_cards(svar: &str) -> Option<(Zone, &str, Option<&str>)> {
    let (zone, rest) = svar.strip_prefix("Count$Valid")?.split_once(' ')?;
    let zone = match zone {
        "" => Zone::Battlefield,
        "Graveyard" => Zone::Graveyard,
        _ => return None,
    };
    let (filter, stat) = match rest.split_once('$') {
        Some((filter, stat)) => (filter, Some(stat)),
        None => (rest, None),
    };
    Some((zone, filter, stat))
}

fn side_of(filter: &str, zone: Zone) -> Side {
    let every = |marks: [&str; 2]| {
        filter
            .split(',')
            .all(|alt| marks.iter().any(|m| alt.contains(m)))
    };
    if every(["YouCtrl", "YouOwn"]) {
        Side::Own
    } else if every(["OppCtrl", "OppOwn"]) || zone != Zone::Graveyard {
        Side::Opponent
    } else {
        Side::Own
    }
}

fn wants_cards(
    zone: Zone,
    filter: &'static str,
    target_type: Option<&'static str>,
    stat: Option<&'static str>,
) -> Wants {
    if filter
        .split(',')
        .any(|alt| alt.split(['.', '+']).any(|p| p == "token"))
    {
        return Wants::Tokens;
    }
    Wants::Cards {
        zone,
        filter,
        target_type,
        stat,
    }
}

fn target_zone(line: &TraitLine) -> Option<Zone> {
    let zone = line
        .param("TgtZone")
        .or_else(|| line.param("Origin"))
        .unwrap_or("Battlefield");
    if line.param("TargetType").is_some() || zone == "Stack" {
        Some(Zone::Stack)
    } else if zone.contains("Graveyard") {
        Some(Zone::Graveyard)
    } else {
        (zone == "Battlefield").then_some(Zone::Battlefield)
    }
}

fn script_needs(rules: &'static CardRules) -> ScriptNeeds {
    let face = &rules.main_part;
    let lines = trait_lines(rules);
    let mut out = ScriptNeeds::default();
    if face.mana_cost.cmc() >= RAMP_MANA_VALUE {
        out.ramp = true;
        let reason = format!("mana value {}", face.mana_cost.cmc());
        out.push(reason, Side::Own, Wants::Ramp, false);
    }
    for line in lines.iter().filter(|l| l.kind == TraitKind::Static) {
        if line.param("Mode") != Some("ReduceCost") || line.param("ValidCard") != Some("Card.Self")
        {
            continue;
        }
        out.ramp = true;
        out.push(
            "reduces its own cost".to_string(),
            Side::Own,
            Wants::Ramp,
            false,
        );
        let counted = line
            .param("Amount")
            .and_then(|name| face.svars.get(name))
            .and_then(|svar| counted_cards(svar));
        if let Some((zone, filter, stat)) = counted {
            let wants = wants_cards(zone, filter, None, stat);
            out.push(format!("its cost counts {filter}"), Side::Own, wants, false);
        }
    }
    let delve = keyword_names(rules)
        .into_iter()
        .filter(|k| matches!(*k, "Delve" | "Escape"))
        .map(|_| "Card");
    let costs: Vec<&'static str> = lines.iter().filter_map(|l| l.param("Cost")).collect();
    for filter in delve.chain(costs.iter().flat_map(|c| angle_costs(c, "ExileFromGrave"))) {
        let wants = wants_cards(Zone::Graveyard, filter, None, None);
        out.push(
            format!("exiles {filter} from its graveyard"),
            Side::Own,
            wants,
            false,
        );
    }
    for filter in costs
        .iter()
        .flat_map(|c| [angle_costs(c, "Sac"), angle_costs(c, "tapXType")].concat())
    {
        let wants = wants_cards(Zone::Battlefield, filter, None, None);
        out.push(format!("costs {filter}"), Side::Own, wants, true);
    }
    for line in &lines {
        let (Some(filter), Some(zone)) = (line.param("ValidTgts"), target_zone(line)) else {
            continue;
        };
        if line.param("TargetMin") == Some("0") {
            continue;
        }
        let target_type = line.param("TargetType");
        let place = match zone {
            Zone::Battlefield => "on the battlefield",
            Zone::Graveyard => "in a graveyard",
            Zone::Stack => "on the stack",
        };
        let kind = target_type.map(|t| format!(" ({t})")).unwrap_or_default();
        let wants = wants_cards(zone, filter, target_type, None);
        let reason = format!("targets {filter}{kind} {place}");
        out.push(reason, side_of(filter, zone), wants, true);
    }
    for line in &lines {
        let mode = line.param("Mode").unwrap_or_default();
        let trigger = line.kind == TraitKind::Trigger;
        let dies = mode == "ChangesZone"
            && line.param("Origin") == Some("Battlefield")
            && line.param("Destination") == Some("Graveyard");
        out.trades |= trigger && (TRADE_TRIGGERS.contains(&mode) || dies);
        out.connects |=
            (trigger && CONNECT_TRIGGERS.contains(&mode)) || line.raw.contains("dealtDamage");
    }
    out
}

fn basics(weights: &[(Color, usize)], total: usize) -> DeckSpec {
    if weights.is_empty() {
        return vec![("Plains".to_string(), total)];
    }
    let sum: usize = weights.iter().map(|(_, w)| w).sum();
    let mut counts: Vec<usize> = weights.iter().map(|(_, w)| total * w / sum).collect();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(weights[i].1));
    let left = total - counts.iter().sum::<usize>();
    for &i in order.iter().cycle().take(left) {
        counts[i] += 1;
    }
    weights
        .iter()
        .zip(counts)
        .map(|((color, _), count)| (color.basic_land_type().to_string(), count))
        .collect()
}

fn nonland_entries(db: &CardDatabase, spec: &str) -> Vec<(String, usize, &'static CardRules)> {
    parse_inline(spec)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, count)| Some((name.clone(), count, db.get_by_card_name(&name)?)))
        .filter(|(_, _, rules)| !rules.main_part.type_line.is_land())
        .collect()
}

fn mana_colors(db: &CardDatabase, spec: &DeckSpec) -> ColorSet {
    spec.iter()
        .filter_map(|(name, _)| db.get_by_card_name(name))
        .fold(ColorSet::COLORLESS, |acc, rules| {
            acc.union(rules.main_part.mana_cost.color_set())
        })
}

impl Support {
    pub fn for_card(db: &CardDatabase, pool: &SupportPool, card: &str) -> Result<Self, String> {
        let rules = db
            .get_by_card_name(card)
            .ok_or_else(|| "not in the card database".to_string())?;
        let identity = rules.color_identity;
        let script = script_needs(rules);
        let own_default = nonland_entries(db, DEFAULT_PARTNERS);
        let opponent_default = nonland_entries(db, DEFAULT_OPPONENT);
        let names = coverage_names(db, card);
        let mut reasons = Vec::new();
        let (mut own, mut theirs): (Vec<DeckSpec>, Vec<DeckSpec>) = (vec![], vec![]);
        let mut trades = script.trades;
        for need in script.needs {
            let (defaults, groups) = match need.side {
                Side::Own => (&own_default, &mut own),
                Side::Opponent => (&opponent_default, &mut theirs),
            };
            if need.presence
                && defaults
                    .iter()
                    .any(|(_, _, rules)| need.wants.accepts(rules))
            {
                continue;
            }
            if groups.len() == GROUPS_PER_SIDE {
                reasons.push(format!("{} (no room)", need.reason));
                continue;
            }
            let chosen = pool.choose(&need, identity, &names);
            if chosen.is_empty() {
                reasons.push(format!("{} (no PASS card provides it)", need.reason));
                continue;
            }
            trades |= need.wants.zone() == Some(Zone::Graveyard)
                && chosen
                    .iter()
                    .any(|(_, r)| r.main_part.type_line.is_creature());
            let each = need.cards / chosen.len();
            let group: DeckSpec = chosen
                .iter()
                .map(|(name, _)| (name.clone(), each))
                .collect();
            let side = if need.side == Side::Own {
                "own"
            } else {
                "opponent"
            };
            reasons.push(format!(
                "{} -> {side} {}",
                need.reason,
                format_inline(&group)
            ));
            groups.push(group);
        }
        let (own, theirs) = (own.concat(), theirs.concat());
        let partner_colors = mana_colors(db, &own);
        let weights: Vec<(Color, usize)> = Color::ALL
            .into_iter()
            .filter(|&c| identity.has_color(c) || partner_colors.has_color(c))
            .map(|c| {
                let pips = rules.main_part.mana_cost.shards().iter();
                (c, pips.filter(|s| s.color().has_color(c)).count().max(1))
            })
            .collect();
        let copies = if own.is_empty() {
            COPIES
        } else {
            COPIES_WITH_PARTNERS
        };
        let mut deck: DeckSpec = vec![(card.to_string(), copies)];
        if own.is_empty() {
            deck.extend(parse_inline(DEFAULT_PARTNERS)?);
        }
        deck.extend(own);
        let lands = if script.ramp { RAMP_LANDS } else { LANDS };
        deck.extend(basics(&weights, lands));

        let passive = !trades && (script.slow || script.connects);
        if trades {
            reasons.push("its creatures must trade: default opponent".to_string());
        } else if script.connects {
            reasons.push("its creatures must connect: passive opponent".to_string());
        } else if script.slow {
            reasons.push("it needs time: passive opponent".to_string());
        }
        let mut opponent: DeckSpec = if passive {
            vec![]
        } else {
            opponent_default
                .into_iter()
                .map(|(name, count, _)| (name, count))
                .collect()
        };
        let opponent_colors = mana_colors(db, &theirs);
        opponent.extend(theirs);
        if opponent.is_empty() {
            opponent = parse_inline(PASSIVE_OPPONENT)?;
        } else {
            let weights: Vec<(Color, usize)> = opponent_colors.iter().map(|c| (c, 1)).collect();
            opponent.extend(basics(&weights, LANDS));
        }
        Ok(Self {
            deck,
            opponent,
            reasons,
        })
    }
}
