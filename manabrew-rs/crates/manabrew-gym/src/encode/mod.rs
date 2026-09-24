pub mod action;
pub mod vocab;

use forge_foundation::{ManaAtom, PhaseType, ZoneType};
use manabrew_engine::agent::PlayCardMode;
use manabrew_engine::card::{Card, CounterType};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::spellability::SpellAbility;

use crate::decision::{DecisionKind, PriorityOption, TargetOption};
use action::{candidates, Candidate};
use vocab::{CardVocab, HIDDEN, UNKNOWN};

pub const CARD_INTS: &[&str] = &["vocab_id", "attached_to_row", "blocking_row"];

pub const CARD_FEATURES: &[&str] = &[
    "owner_self",
    "controller_self",
    "zone_battlefield",
    "zone_hand",
    "zone_graveyard",
    "zone_exile",
    "zone_stack",
    "zone_command",
    "zone_other",
    "tapped",
    "summoning_sick",
    "power",
    "toughness",
    "damage",
    "mana_value",
    "counters_p1p1",
    "counters_m1m1",
    "counters_loyalty",
    "counters_lore",
    "counters_other",
    "face_down",
    "token",
    "creature",
    "land",
    "artifact",
    "enchantment",
    "planeswalker",
    "instant",
    "sorcery",
    "legendary",
    "white",
    "blue",
    "black",
    "red",
    "green",
    "attacking",
    "blocking",
    "flying",
    "reach",
    "first_strike",
    "double_strike",
    "deathtouch",
    "trample",
    "lifelink",
    "vigilance",
    "menace",
    "indestructible",
    "hexproof",
    "haste",
    "defender",
];

pub const GLOBAL_FEATURES: &[&str] = &[
    "turn",
    "active_self",
    "priority_self",
    "phase_untap",
    "phase_upkeep",
    "phase_draw",
    "phase_main1",
    "phase_combat_begin",
    "phase_declare_attackers",
    "phase_declare_blockers",
    "phase_first_strike_damage",
    "phase_combat_damage",
    "phase_combat_end",
    "phase_main2",
    "phase_end_of_turn",
    "phase_cleanup",
    "stack_size",
    "day",
    "night",
    "cards_dropped",
    "candidates_dropped",
    "self_life",
    "self_poison",
    "self_hand",
    "self_library",
    "self_graveyard",
    "self_exile",
    "self_battlefield",
    "self_lands_played",
    "self_land_drop_available",
    "self_speed",
    "self_mana_w",
    "self_mana_u",
    "self_mana_b",
    "self_mana_r",
    "self_mana_g",
    "self_mana_c",
    "opp_life",
    "opp_poison",
    "opp_hand",
    "opp_library",
    "opp_graveyard",
    "opp_exile",
    "opp_battlefield",
    "opp_lands_played",
    "opp_land_drop_available",
    "opp_speed",
    "opp_mana_w",
    "opp_mana_u",
    "opp_mana_b",
    "opp_mana_r",
    "opp_mana_g",
    "opp_mana_c",
];

pub const STACK_INTS: &[&str] = &[
    "vocab_id",
    "source_row",
    "target_card_row",
    "target_stack_row",
];

pub const STACK_FEATURES: &[&str] = &[
    "controller_self",
    "spell",
    "trigger",
    "ability",
    "targets_self",
    "targets_opp",
    "target_count",
    "pending",
];

pub const CANDIDATE_INTS: &[&str] = &["card_row", "other_row", "stack_row", "index", "vocab_id"];

pub const CANDIDATE_FEATURES: &[&str] = &[
    "pass",
    "play",
    "activate",
    "land",
    "spell",
    "ability",
    "attack",
    "block",
    "target_player",
    "target_card",
    "target_stack",
    "card",
    "mode",
    "yes",
    "no",
    "player_self",
    "player_opp",
    "mode_normal",
    "mode_back_face_land",
    "mode_alternative",
    "mode_secondary",
    "mode_other",
];

pub const DECISION_INTS: &[&str] = &["kind", "min", "max", "candidates"];

const CARD_BLOCKING: usize = 36;
const GLOBAL_CARDS_DROPPED: usize = 19;
const GLOBAL_CANDIDATES_DROPPED: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub max_cards: usize,
    pub max_stack: usize,
    pub max_candidates: usize,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        EncoderConfig {
            max_cards: 160,
            max_stack: 16,
            max_candidates: 128,
        }
    }
}

impl EncoderConfig {
    pub fn shapes(&self) -> [(&'static str, [usize; 2]); 11] {
        [
            ("card_ints", [self.max_cards, CARD_INTS.len()]),
            ("card_floats", [self.max_cards, CARD_FEATURES.len()]),
            ("card_mask", [self.max_cards, 1]),
            ("global", [1, GLOBAL_FEATURES.len()]),
            ("stack_ints", [self.max_stack, STACK_INTS.len()]),
            ("stack_floats", [self.max_stack, STACK_FEATURES.len()]),
            ("stack_mask", [self.max_stack, 1]),
            (
                "candidate_ints",
                [self.max_candidates, CANDIDATE_INTS.len()],
            ),
            (
                "candidate_floats",
                [self.max_candidates, CANDIDATE_FEATURES.len()],
            ),
            ("candidate_mask", [self.max_candidates, 1]),
            ("decision", [1, DECISION_INTS.len()]),
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub card_ints: Vec<i32>,
    pub card_floats: Vec<f32>,
    pub card_mask: Vec<bool>,
    pub global: Vec<f32>,
    pub stack_ints: Vec<i32>,
    pub stack_floats: Vec<f32>,
    pub stack_mask: Vec<bool>,
    pub candidate_ints: Vec<i32>,
    pub candidate_floats: Vec<f32>,
    pub candidate_mask: Vec<bool>,
    pub decision: Vec<i32>,
}

impl Observation {
    pub fn zeroed(config: &EncoderConfig) -> Self {
        Observation {
            card_ints: vec![0; config.max_cards * CARD_INTS.len()],
            card_floats: vec![0.0; config.max_cards * CARD_FEATURES.len()],
            card_mask: vec![false; config.max_cards],
            global: vec![0.0; GLOBAL_FEATURES.len()],
            stack_ints: vec![0; config.max_stack * STACK_INTS.len()],
            stack_floats: vec![0.0; config.max_stack * STACK_FEATURES.len()],
            stack_mask: vec![false; config.max_stack],
            candidate_ints: vec![0; config.max_candidates * CANDIDATE_INTS.len()],
            candidate_floats: vec![0.0; config.max_candidates * CANDIDATE_FEATURES.len()],
            candidate_mask: vec![false; config.max_candidates],
            decision: vec![0; DECISION_INTS.len()],
        }
    }
}

struct Writer<'a> {
    out: &'a mut [f32],
    at: usize,
}

impl Writer<'_> {
    fn push(&mut self, value: f32) {
        self.out[self.at] = value;
        self.at += 1;
    }

    fn flag(&mut self, value: bool) {
        self.push(if value { 1.0 } else { 0.0 });
    }

    fn one_hot(&mut self, index: usize, len: usize) {
        for i in 0..len {
            self.flag(i == index);
        }
    }

    fn skip(&mut self, n: usize) {
        self.at += n;
    }
}

const ZONES: [ZoneType; 6] = [
    ZoneType::Battlefield,
    ZoneType::Hand,
    ZoneType::Graveyard,
    ZoneType::Exile,
    ZoneType::Stack,
    ZoneType::Command,
];

pub struct Encoder {
    config: EncoderConfig,
    vocab: &'static CardVocab,
    viewer: PlayerId,
    state: Observation,
    rows: Vec<i32>,
    names: Vec<i32>,
    row_cards: Vec<CardId>,
    stack_ids: Vec<u32>,
}

impl Encoder {
    pub fn new(config: EncoderConfig) -> Self {
        Encoder {
            config,
            vocab: CardVocab::get(),
            viewer: PlayerId(0),
            state: Observation::zeroed(&config),
            rows: Vec::new(),
            names: Vec::new(),
            row_cards: Vec::new(),
            stack_ids: Vec::new(),
        }
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    pub fn observe(&mut self, game: &GameState, pools: &[ManaPool], viewer: PlayerId) {
        self.viewer = viewer;
        let opponent = game
            .player_order
            .iter()
            .copied()
            .find(|&p| p != viewer)
            .unwrap_or(viewer);
        self.rows.clear();
        self.rows.resize(game.cards.len(), -1);
        for card in &game.cards[self.names.len().min(game.cards.len())..] {
            self.names.push(self.vocab.id(&card.card_name));
        }
        self.row_cards.clear();
        let mut dropped = 0;
        let order = [
            (ZoneType::Battlefield, viewer),
            (ZoneType::Battlefield, opponent),
            (ZoneType::Hand, viewer),
            (ZoneType::Stack, viewer),
            (ZoneType::Stack, opponent),
            (ZoneType::Graveyard, viewer),
            (ZoneType::Graveyard, opponent),
            (ZoneType::Exile, viewer),
            (ZoneType::Exile, opponent),
            (ZoneType::Command, viewer),
            (ZoneType::Command, opponent),
        ];
        for (zone, owner) in order {
            for &id in game.cards_in_zone(zone, owner) {
                if self.row_cards.len() < self.config.max_cards {
                    self.rows[id.index()] = self.row_cards.len() as i32;
                    self.row_cards.push(id);
                } else {
                    dropped += 1;
                }
            }
        }

        self.state.card_ints.fill(0);
        self.state.card_floats.fill(0.0);
        self.state.card_mask.fill(false);
        for row in 0..self.row_cards.len() {
            let card = game.card(self.row_cards[row]);
            let named = match card.effect_source {
                Some(source) if card.zone == ZoneType::Command => game.card(source),
                _ => card,
            };
            self.encode_card(row, card, &named.card_name);
        }
        for &(blocker, attacker) in &game.turn.combat_block_assignments {
            if let Some(row) = self.row_of(blocker) {
                self.state.card_ints[row * CARD_INTS.len() + 2] = self.row_index(attacker);
                self.state.card_floats[row * CARD_FEATURES.len() + CARD_BLOCKING] = 1.0;
            }
        }

        self.encode_stack(game);
        self.encode_global(game, pools, opponent, dropped);
    }

    pub fn observation(&self, kind: &DecisionKind) -> Observation {
        let mut obs = self.state.clone();
        let list = candidates(kind);
        let k = list.len().min(self.config.max_candidates);
        for (i, candidate) in list.iter().take(k).enumerate() {
            self.encode_candidate(&mut obs, i, candidate, kind);
        }
        obs.global[GLOBAL_CANDIDATES_DROPPED] = (list.len() - k) as f32;
        let (min, max) = picks(kind);
        obs.decision
            .copy_from_slice(&[kind.id() as i32, min, max, list.len() as i32]);
        obs
    }

    fn row_of(&self, id: CardId) -> Option<usize> {
        self.rows
            .get(id.index())
            .and_then(|&r| (r >= 0).then_some(r as usize))
    }

    fn row_index(&self, id: CardId) -> i32 {
        self.rows.get(id.index()).copied().unwrap_or(-1)
    }

    fn hidden(&self, card: &Card) -> bool {
        card.face_down && card.controller != self.viewer
    }

    fn vocab_id(&self, id: CardId) -> i32 {
        match self.row_of(id) {
            Some(row) => self.state.card_ints[row * CARD_INTS.len()],
            None => self.names.get(id.index()).copied().unwrap_or(UNKNOWN),
        }
    }

    fn encode_card(&mut self, row: usize, card: &Card, name: &str) {
        let hidden = self.hidden(card);
        let ints = &mut self.state.card_ints[row * CARD_INTS.len()..(row + 1) * CARD_INTS.len()];
        ints[0] = if hidden {
            HIDDEN
        } else {
            let id = self.vocab.id(name);
            self.names[card.id.index()] = id;
            id
        };
        ints[1] = card
            .attached_to
            .map_or(-1, |a| self.rows.get(a.index()).copied().unwrap_or(-1));
        ints[2] = -1;
        self.state.card_mask[row] = true;

        let n = CARD_FEATURES.len();
        let mut w = Writer {
            out: &mut self.state.card_floats[row * n..(row + 1) * n],
            at: 0,
        };
        let on_battlefield = card.zone == ZoneType::Battlefield;
        w.flag(card.owner == self.viewer);
        w.flag(card.controller == self.viewer);
        w.one_hot(
            ZONES
                .iter()
                .position(|&z| z == card.zone)
                .unwrap_or(ZONES.len()),
            ZONES.len() + 1,
        );
        w.flag(card.tapped);
        if hidden && !on_battlefield {
            w.skip(10);
            w.flag(true);
            return;
        }
        w.flag(card.summoning_sick);
        let creature = card.is_creature();
        w.push(if creature { card.power() as f32 } else { 0.0 });
        w.push(if creature {
            card.toughness() as f32
        } else {
            0.0
        });
        w.push(card.damage as f32);
        w.push(card.mana_value() as f32);
        let mut counters = [0i32; 5];
        for (kind, &count) in &card.counters {
            let slot = match kind {
                CounterType::P1P1 => 0,
                CounterType::M1M1 => 1,
                CounterType::Loyalty => 2,
                CounterType::Lore => 3,
                _ => 4,
            };
            counters[slot] += count;
        }
        for count in counters {
            w.push(count as f32);
        }
        w.flag(card.face_down);
        w.flag(card.is_token);
        let types = &card.type_line;
        w.flag(creature);
        w.flag(types.is_land());
        w.flag(types.is_artifact());
        w.flag(types.is_enchantment());
        w.flag(types.is_planeswalker());
        w.flag(types.is_instant());
        w.flag(types.is_sorcery());
        w.flag(types.is_legendary());
        let colors = card.color.mask();
        for bit in [1u8, 2, 4, 8, 16] {
            w.flag(colors & bit != 0);
        }
        w.flag(card.attacking_player.is_some());
        w.skip(1);
        if on_battlefield && creature {
            w.flag(card.has_flying());
            w.flag(card.has_reach());
            w.flag(card.has_first_strike());
            w.flag(card.has_double_strike());
            w.flag(card.has_deathtouch());
            w.flag(card.has_trample());
            w.flag(card.has_lifelink());
            w.flag(card.has_vigilance());
            w.flag(card.has_menace());
            w.flag(card.has_indestructible());
            w.flag(card.has_hexproof());
            w.flag(card.has_haste());
            w.flag(card.has_defender());
        } else {
            w.skip(13);
        }
        debug_assert_eq!(w.at, n);
    }

    fn encode_stack(&mut self, game: &GameState) {
        let (si, sf) = (STACK_INTS.len(), STACK_FEATURES.len());
        self.state.stack_ints.fill(0);
        self.state.stack_floats.fill(0.0);
        self.state.stack_mask.fill(false);
        let entries: Vec<_> = game.stack.iter().collect();
        self.stack_ids.clear();
        self.stack_ids.extend(
            entries
                .iter()
                .rev()
                .take(self.config.max_stack)
                .map(|e| e.id),
        );
        for (row, entry) in entries.iter().rev().take(self.config.max_stack).enumerate() {
            let sa = &entry.spell_ability;
            let (target_card, target_stack, players, count) = targets(sa);
            let vocab_id = sa.source.map_or(UNKNOWN, |source| {
                if self.hidden(game.card(source)) {
                    HIDDEN
                } else {
                    self.vocab_id(source)
                }
            });
            let values = [
                vocab_id,
                sa.source.map_or(-1, |s| self.row_index(s)),
                target_card.map_or(-1, |c| self.row_index(c)),
                target_stack.map_or(-1, |id| self.stack_row(id)),
            ];
            self.state.stack_ints[row * si..(row + 1) * si].copy_from_slice(&values);
            self.state.stack_mask[row] = true;
            let mut w = Writer {
                out: &mut self.state.stack_floats[row * sf..(row + 1) * sf],
                at: 0,
            };
            w.flag(sa.activating_player == self.viewer);
            w.flag(sa.is_spell);
            w.flag(sa.is_trigger);
            w.flag(!sa.is_spell && !sa.is_trigger);
            w.flag(players.contains(&self.viewer));
            w.flag(players.iter().any(|&p| p != self.viewer));
            w.push(count as f32);
            w.flag(entry.is_pending_cast);
        }
    }

    fn stack_row(&self, id: u32) -> i32 {
        self.stack_ids
            .iter()
            .position(|&s| s == id)
            .map_or(-1, |r| r as i32)
    }

    fn encode_global(
        &mut self,
        game: &GameState,
        pools: &[ManaPool],
        opponent: PlayerId,
        dropped: usize,
    ) {
        let viewer = self.viewer;
        let mut w = Writer {
            out: &mut self.state.global,
            at: 0,
        };
        w.push(game.turn.turn_number as f32);
        w.flag(game.turn.active_player == viewer);
        w.flag(game.turn.priority_player == viewer);
        w.one_hot(
            PhaseType::TURN_ORDER
                .iter()
                .position(|&p| p == game.turn.phase)
                .unwrap_or(0),
            PhaseType::TURN_ORDER.len(),
        );
        w.push(game.stack.len() as f32);
        w.flag(game.is_day());
        w.flag(game.day_night_started && game.is_night);
        w.push(dropped as f32);
        w.push(0.0);
        for player in [viewer, opponent] {
            let state = game.player(player);
            w.push(state.life as f32);
            w.push(state.poison_counters as f32);
            for zone in [
                ZoneType::Hand,
                ZoneType::Library,
                ZoneType::Graveyard,
                ZoneType::Exile,
                ZoneType::Battlefield,
            ] {
                w.push(game.cards_in_zone(zone, player).len() as f32);
            }
            w.push(state.lands_played_this_turn as f32);
            w.flag(state.can_play_land());
            w.push(state.speed as f32);
            match pools.get(player.index()) {
                Some(pool) => {
                    for atom in [
                        ManaAtom::WHITE,
                        ManaAtom::BLUE,
                        ManaAtom::BLACK,
                        ManaAtom::RED,
                        ManaAtom::GREEN,
                        ManaAtom::COLORLESS,
                    ] {
                        w.push(pool.count_color(atom) as f32);
                    }
                }
                None => w.skip(6),
            }
        }
        debug_assert_eq!(w.at, GLOBAL_FEATURES.len());
        debug_assert_eq!(GLOBAL_FEATURES[GLOBAL_CARDS_DROPPED], "cards_dropped");
    }

    fn encode_candidate(
        &self,
        obs: &mut Observation,
        i: usize,
        candidate: &Candidate,
        kind: &DecisionKind,
    ) {
        let (ki, kf) = (CANDIDATE_INTS.len(), CANDIDATE_FEATURES.len());
        let source = match kind {
            DecisionKind::Target { source, .. }
            | DecisionKind::Cards { source, .. }
            | DecisionKind::Modes { source, .. }
            | DecisionKind::Confirm { source, .. } => *source,
            _ => None,
        };
        let mut card = source;
        let mut other_row = -1;
        let mut stack_row = -1;
        let mut index = 0;
        let mut player = None;
        let mut play_mode = None;
        let slot = match *candidate {
            Candidate::Priority(PriorityOption::Pass) => 0,
            Candidate::Priority(PriorityOption::Play(play)) => {
                card = Some(play.card_id);
                index = i32::from(play.alt_cost_index);
                play_mode = Some(play.mode);
                1
            }
            Candidate::Priority(PriorityOption::Activate(ability)) => {
                card = Some(ability.card_id);
                index = ability.ability_index as i32;
                2
            }
            Candidate::LandOrSpell(choice) => [3, 4, 0][choice],
            Candidate::Ability { index: n, source } => {
                card = source;
                index = n as i32;
                5
            }
            Candidate::Attack {
                slot,
                attacker,
                target,
                ..
            } => {
                card = Some(attacker);
                index = slot as i32;
                match target {
                    DefenderId::Player(p) => player = Some(p),
                    DefenderId::Permanent(c) => other_row = self.row_index(c),
                }
                6
            }
            Candidate::Block {
                slot,
                blocker,
                attacker_id,
                ..
            } => {
                card = Some(blocker);
                index = slot as i32;
                other_row = self.row_index(attacker_id);
                7
            }
            Candidate::Target(TargetOption::Player(p)) => {
                player = Some(p);
                8
            }
            Candidate::Target(TargetOption::Card(c)) => {
                card = Some(c);
                9
            }
            Candidate::Target(TargetOption::Stack(id)) => {
                stack_row = self.stack_row(id);
                10
            }
            Candidate::Card(c) => {
                card = Some(c);
                11
            }
            Candidate::Mode(n) => {
                index = n as i32;
                12
            }
            Candidate::Confirm(yes) => {
                if yes {
                    13
                } else {
                    14
                }
            }
        };
        let ints = &mut obs.candidate_ints[i * ki..(i + 1) * ki];
        ints[0] = card.map_or(-1, |c| self.row_index(c));
        ints[1] = other_row;
        ints[2] = stack_row;
        ints[3] = index;
        ints[4] = card.map_or(0, |c| self.vocab_id(c));
        obs.candidate_mask[i] = true;
        let mut w = Writer {
            out: &mut obs.candidate_floats[i * kf..(i + 1) * kf],
            at: 0,
        };
        w.one_hot(slot, 15);
        w.flag(player == Some(self.viewer));
        w.flag(player.is_some_and(|p| p != self.viewer));
        match play_mode {
            Some(mode) => w.one_hot(play_mode_slot(mode), 5),
            None => w.skip(5),
        }
        debug_assert_eq!(w.at, kf);
    }
}

fn play_mode_slot(mode: PlayCardMode) -> usize {
    match mode {
        PlayCardMode::Normal => 0,
        PlayCardMode::BackFaceLand => 1,
        PlayCardMode::Alternative(_)
        | PlayCardMode::StaticAlternative
        | PlayCardMode::MayPlay(_) => 2,
        PlayCardMode::Secondary | PlayCardMode::RoomRightSplit => 3,
        PlayCardMode::ForetellExile | PlayCardMode::UnlockDoor => 4,
    }
}

fn picks(kind: &DecisionKind) -> (i32, i32) {
    match kind {
        DecisionKind::Cards { min, max, .. } | DecisionKind::Modes { min, max, .. } => {
            (*min as i32, *max as i32)
        }
        DecisionKind::Attackers { attackers, .. } => (0, attackers.len() as i32),
        DecisionKind::Blockers { blockers, max, .. } => {
            (0, max.unwrap_or(blockers.len()).min(blockers.len()) as i32)
        }
        _ => (1, 1),
    }
}

fn targets(sa: &SpellAbility) -> (Option<CardId>, Option<u32>, Vec<PlayerId>, usize) {
    let mut card = None;
    let mut stack = None;
    let mut players = Vec::new();
    let mut count = 0;
    let mut node = Some(sa);
    while let Some(sa) = node {
        let chosen = &sa.target_chosen;
        let cards = chosen.all_target_cards();
        count += cards.len();
        card = card.or(cards.first().copied());
        if let Some(id) = chosen.target_stack_entry {
            stack = stack.or(Some(id));
            count += 1;
        }
        for p in chosen.all_target_players() {
            count += 1;
            players.push(p);
        }
        node = sa.sub_ability.as_deref();
    }
    (card, stack, players, count)
}
