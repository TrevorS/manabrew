use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use forge_carddb::{CardDatabase, CardFace, CardRules};
use forge_foundation::CardSplitType;

use super::{matches_valid_cards_for_sa, EffectContext};
use crate::agent::DecisionContext;
use crate::card::Card;
use crate::game::{CardDatabaseRegistry, GameState};
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;

fn insert_name(names: &mut BTreeSet<String>, game: &GameState, sa: &SpellAbility, card: &Card) {
    if matches_valid_cards_for_sa(game, sa, card, sa.ir.valid_cards_selector.as_ref(), "Card") {
        names.insert(card.card_name.clone());
    }
}

fn card_from_face(face: &CardFace, owner: PlayerId) -> Card {
    let mut card = Card::new(
        CardId(u32::MAX),
        face.name.clone(),
        owner,
        face.type_line.clone(),
        face.mana_cost.clone(),
        face.resolved_color(),
        face.int_power,
        face.int_toughness,
        face.keywords.clone(),
        Vec::new(),
    );
    card.oracle_text = face.oracle_text.replace("\\n", "\n");
    card.initial_loyalty = face.initial_loyalty.clone();
    card
}

fn insert_face(names: &mut BTreeSet<String>, game: &GameState, sa: &SpellAbility, face: &CardFace) {
    insert_name(names, game, sa, &card_from_face(face, sa.activating_player));
}

struct AllFaces {
    faces_by_name: Vec<(&'static str, Vec<&'static CardFace>)>,
    flavor_faces: Vec<String>,
}

static ALL_FACES: OnceLock<AllFaces> = OnceLock::new();

fn all_faces(database: &'static CardDatabase) -> &'static AllFaces {
    ALL_FACES.get_or_init(|| {
        let mut faces_by_name: BTreeMap<&'static str, Vec<&'static CardFace>> = BTreeMap::new();
        for face in database
            .iter()
            .into_iter()
            .filter(|(_, rules)| !rules.is_variant())
            .flat_map(|(_, rules)| {
                std::iter::once(&rules.main_part)
                    .chain(rules.other_part.iter())
                    .chain(rules.specialized_parts.values())
            })
        {
            faces_by_name
                .entry(face.name.as_str())
                .or_default()
                .push(face);
        }
        AllFaces {
            faces_by_name: faces_by_name.into_iter().collect(),
            flavor_faces: database.flavor_name_faces(),
        }
    })
}

fn insert_defined_rules_faces(
    names: &mut BTreeSet<String>,
    game: &GameState,
    sa: &SpellAbility,
    rules: &CardRules,
) {
    insert_face(names, game, sa, &rules.main_part);
    if rules.split_type == CardSplitType::Split {
        if let Some(other) = rules.other_part.as_ref() {
            insert_face(names, game, sa, other);
        }
    }
}

fn insert_game_card_faces(
    names: &mut BTreeSet<String>,
    game: &GameState,
    sa: &SpellAbility,
    card: &Card,
    include_other: bool,
) {
    insert_name(names, game, sa, card);
    if include_other && card.other_part.is_some() {
        let mut other_face = card.clone();
        other_face.transform();
        insert_name(names, game, sa, &other_face);
    }
}

fn valid_face_predicates(ctx: &EffectContext, sa: &SpellAbility) -> Option<Vec<String>> {
    let valid = sa.ir.valid_cards_text.as_deref()?;
    let host = sa.source.map(|source| ctx.game.card(source));
    Some(
        valid
            .split(',')
            .map(|v| {
                let mut v = v.to_string();
                if let Some(s) = v.split("cmcEQ").nth(1) {
                    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                        let amount = super::resolve_numeric_value(ctx.game, sa, s, 0);
                        v = v.replace(s, &amount.to_string());
                    }
                }
                if v.contains("ManaCost=") {
                    let replaced = if v.contains("ManaCost=Equipped") {
                        host.and_then(|host| host.attached_to)
                            .map(|equipping| ("=Equipped", equipping))
                    } else if v.contains("ManaCost=Imprinted") {
                        sa.source
                            .and_then(|source| {
                                ctx.game
                                    .host_object(source, sa)
                                    .imprinted_cards
                                    .first()
                                    .copied()
                            })
                            .map(|imprinted| ("=Imprinted", imprinted))
                    } else {
                        None
                    };
                    if let Some((from, card)) = replaced {
                        v = v.replace(from, &ctx.game.card(card).mana_cost.short_string());
                    }
                }
                v
            })
            .collect(),
    )
}

fn valid_names(ctx: &EffectContext, sa: &SpellAbility) -> Vec<String> {
    if let Some(list) = sa.ir.choose_from_list_text.as_deref() {
        return list
            .split(',')
            .map(|name| name.trim().replace(';', ","))
            .collect();
    }

    let database = CardDatabaseRegistry::get();
    let mut names = BTreeSet::new();
    if sa.ir.choose_from_defined_cards {
        if let Some(source_id) = sa.source {
            for card_id in ctx
                .game
                .host_object(source_id, sa)
                .remembered_cards
                .iter()
                .copied()
            {
                let card = ctx.game.card(card_id);
                if let Some(rules) =
                    database.and_then(|database| database.get_by_card_name(&card.full_name))
                {
                    insert_defined_rules_faces(&mut names, ctx.game, sa, rules);
                } else {
                    insert_game_card_faces(&mut names, ctx.game, sa, card, false);
                }
            }
        }
    } else {
        let database =
            CardDatabaseRegistry::all().expect("card database must be loaded for card naming");
        let all = all_faces(database);
        let valid_cards = valid_face_predicates(ctx, sa);
        let names: Vec<&str> = all
            .faces_by_name
            .iter()
            .filter(|(_, faces)| {
                valid_cards.as_ref().is_none_or(|valid_cards| {
                    faces.iter().any(|face| {
                        valid_cards
                            .iter()
                            .any(|v| forge_carddb::card_face_predicates::valid(face, v))
                    })
                })
            })
            .map(|(name, _)| *name)
            .collect();
        let flavor_faces = all
            .flavor_faces
            .iter()
            .filter(|face| names.binary_search(&face.as_str()).is_ok())
            .cloned();
        let mut valid: Vec<String> = names
            .iter()
            .map(|name| name.to_string())
            .chain(flavor_faces)
            .collect();
        valid.sort();
        return valid;
    }
    names.into_iter().collect()
}

#[manabrew_engine_macros::spell_effect(NameCardEffect)]
fn resolve(ctx: &mut EffectContext, sa: &SpellAbility) {
    let controller = sa.activating_player;
    let mut valid_names = valid_names(ctx, sa);
    let chosen = if sa.ir.at_random && sa.ir.choose_from_list_text.is_some() {
        (!valid_names.is_empty()).then(|| {
            let index = ctx.rng.next_int(valid_names.len() as i32) as usize;
            valid_names[index].clone()
        })
    } else if sa.ir.at_random && !sa.ir.choose_from_defined_cards {
        valid_names.sort_by_cached_key(|name| name.to_lowercase());
        let mut chosen = None;
        for (index, name) in valid_names.into_iter().enumerate() {
            if index == 0 || ctx.rng.next_int((index + 1) as i32) == 0 {
                chosen = Some(name);
            }
        }
        chosen
    } else {
        ctx.agents[controller.index()].choose_card_name(
            DecisionContext::new(ctx.game, ctx.mana_pools),
            controller,
            &valid_names,
        )
    };

    if let (Some(chosen_name), Some(source_id)) = (chosen, sa.source) {
        ctx.game.card_mut(source_id).add_named_card(&chosen_name);
    }
}
