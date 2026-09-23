use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use forge_foundation::PhaseType;
use manabrew_engine::agent::{
    BinaryChoiceKind, GameEntity, ManaCostAction, PlayCardMode, PlayOption, PlayerAgent,
    PriorityActionSpace, TargetChoice,
};
use manabrew_engine::card::Card;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::player_action::STATIC_ALTERNATIVE_ABILITY_INDEX;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
use manabrew_engine::spellability::AlternativeCost;
use manabrew_engine::spellability::{MagicStack, SpellAbility, StackEntry};

use crate::choice_space;
use crate::combat_choice_space;
use crate::gui_repro;
use crate::java_random::JavaRandom;
use crate::parity_card_map::ParityCardMap;
use crate::parity_order;

#[allow(dead_code)]
const ANSI_RESET: &str = "\x1b[0m";
#[allow(dead_code)]
const ANSI_DIM_GRAY: &str = "\x1b[90m";
#[allow(dead_code)]
const ANSI_YELLOW: &str = "\x1b[33m";
const PREFER_ACTION_WEIGHT: usize = 3;
const STACK_ACTION_SPACE_SKIP_THRESHOLD: usize = 20;

#[derive(Clone, Debug)]
pub enum VerboseMode {
    Off,
    All,
    Turns(Vec<u32>),
}

impl VerboseMode {
    /// Parse from an optional CLI value.
    /// `None` / not present → `Off`, `Some(None)` (bare `--verbose`) → `All`,
    /// `Some(Some("21,22"))` → `Turns([21, 22])`.
    pub fn from_flag(present: bool, value: Option<&str>) -> Self {
        if !present {
            return VerboseMode::Off;
        }
        match value {
            None => VerboseMode::All,
            Some("") => VerboseMode::All,
            Some(s) => {
                let turns: Vec<u32> = s.split(',').filter_map(|t| t.trim().parse().ok()).collect();
                if turns.is_empty() {
                    VerboseMode::All
                } else {
                    VerboseMode::Turns(turns)
                }
            }
        }
    }

    pub fn is_active(&self, current_turn: u32) -> bool {
        match self {
            VerboseMode::Off => false,
            VerboseMode::All => true,
            VerboseMode::Turns(turns) => turns.contains(&current_turn),
        }
    }

    /// True only for bare `--verbose` (all turns). Turn-specific modes
    /// should not trigger general progress logging.
    pub fn is_any(&self) -> bool {
        matches!(self, VerboseMode::All)
    }

    pub fn to_java_arg(&self) -> Option<String> {
        match self {
            VerboseMode::Off => None,
            VerboseMode::All => Some(String::new()),
            VerboseMode::Turns(turns) => Some(
                turns
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        }
    }
}

pub struct DeterministicAgent {
    player_id: PlayerId,
    pub log: Vec<String>,
    pub verbose: VerboseMode,
    current_turn: u32,
    last_game_snapshot: Option<GameSnapshot>,
    /// Players, cards, zones, stack and turn as of the last `snapshot_state`, refreshed in
    /// place; `CapturingAgent` reads it too, so a decision refreshes one copy of the game.
    snapshot_game: Option<GameState>,
    rng: Rc<RefCell<JavaRandom>>,
    game_rng: Rc<RefCell<JavaRandom>>,
    prefer_actions: bool,
    parity_map: Arc<ParityCardMap>,
    parity_sync_pending: Cell<bool>,
    parity_observer: Option<Arc<crate::runner::ParityObserver>>,
    choosing_targets: bool,
    target_loop_drew_continue: bool,
}

struct GameSnapshot {
    player_names: Vec<(PlayerId, String)>,
    card_names: Vec<(CardId, String)>,
    card_is_land: Vec<(CardId, bool)>,
    card_owner_controller: Vec<(CardId, (u32, u32))>,
    ability_is_mana: Vec<((CardId, usize), bool)>,
    ability_texts: Vec<((CardId, usize), String)>,
    stack_sources: Vec<(u32, CardId)>,
    phase: PhaseType,
    stack_depth: usize,
}

/// Refill a snapshot lookup table in place, keeping the `Vec` and each `String` buffer, so a
/// decision that changes nothing allocates nothing.
fn refill_named<'a, K>(dst: &mut Vec<(K, String)>, src: impl Iterator<Item = (K, &'a str)>) {
    let mut len = 0;
    for (key, value) in src {
        match dst.get_mut(len) {
            Some(slot) => {
                slot.0 = key;
                if slot.1 != value {
                    slot.1.clear();
                    slot.1.push_str(value);
                }
            }
            None => dst.push((key, value.to_string())),
        }
        len += 1;
    }
    dst.truncate(len);
}

fn refill<T>(dst: &mut Vec<T>, src: impl Iterator<Item = T>) {
    dst.clear();
    dst.extend(src);
}

#[derive(Clone, Copy)]
enum ActionChoice {
    Card(PlayOption),
    Ability(CardId, usize),
}

#[allow(private_interfaces)]
impl DeterministicAgent {
    fn shallow_snapshot_card(card: &Card) -> Card {
        card.clone_for_parity_snapshot()
    }

    fn shallow_cards(game: &GameState) -> Vec<Card> {
        game.cards.iter().map(Self::shallow_snapshot_card).collect()
    }

    /// Refreshes a snapshot of `cards` kept from the last decision; most cards have not
    /// changed, so this reuses their allocations instead of cloning every card again.
    pub(crate) fn refresh_snapshot_cards(snapshot: &mut Vec<Card>, cards: &[Card]) {
        snapshot.truncate(cards.len());
        for (out, card) in snapshot.iter_mut().zip(cards) {
            if out.id == card.id
                && out.zone == forge_foundation::ZoneType::Library
                && card.zone == forge_foundation::ZoneType::Library
            {
                continue;
            }
            card.refresh_parity_snapshot(out);
        }
        let kept = snapshot.len();
        snapshot.extend(cards[kept..].iter().map(Self::shallow_snapshot_card));
    }

    pub(crate) fn shallow_stack_entry(entry: &StackEntry) -> StackEntry {
        let mut spell_ability = SpellAbility::new_simple(
            entry.spell_ability.source,
            entry.spell_ability.activating_player,
            &entry.spell_ability.ability_text,
        );
        spell_ability.id = entry.spell_ability.id;
        StackEntry {
            id: entry.id,
            spell_ability,
            is_pending_cast: entry.is_pending_cast,
            is_creature_spell: entry.is_creature_spell,
            is_permanent_spell: entry.is_permanent_spell,
            cast_from_zone: entry.cast_from_zone,
            optional_trigger_decider: entry.optional_trigger_decider,
            optional_trigger_description: entry.optional_trigger_description.clone(),
            optional_trigger_source_name: entry.optional_trigger_source_name.clone(),
        }
    }

    fn shallow_game_state(previous: Option<GameState>, game: &GameState) -> GameState {
        let mut sim = previous.unwrap_or_else(|| {
            let player_names: Vec<String> = game.players.iter().map(|p| p.name.clone()).collect();
            let player_name_refs: Vec<&str> = player_names.iter().map(String::as_str).collect();
            let starting_life = game.players.first().map(|p| p.life).unwrap_or(20);
            GameState::new(&player_name_refs, starting_life)
        });
        sim.players.clone_from(&game.players);
        Self::refresh_snapshot_cards(&mut sim.cards, &game.cards);
        sim.replace_zone_store(game.zone_store_snapshot());
        let mut stack = MagicStack::new();
        for entry in game.stack.iter() {
            stack.push(Self::shallow_stack_entry(entry));
        }
        sim.stack = stack;
        sim.turn = game.turn.clone();
        sim.player_order = game.player_order.clone();
        sim.game_over = game.game_over;
        sim.winner = game.winner;
        sim
    }

    pub(crate) fn snapshot_game(&self) -> Option<&GameState> {
        self.snapshot_game.as_ref()
    }

    pub(crate) fn sync_parity_ids(&self) {
        if self.parity_sync_pending.replace(false) {
            if let Some(game) = self.snapshot_game() {
                self.parity_map.sync_with_game(game);
            }
        }
    }

    fn parity_id(&self, cid: CardId) -> u32 {
        self.sync_parity_ids();
        self.parity_map.id(cid)
    }

    pub(crate) fn snapshot_game_mut(&mut self) -> Option<&mut GameState> {
        self.snapshot_game.as_mut()
    }

    fn snapshot_cards(&self) -> &[Card] {
        self.snapshot_game
            .as_ref()
            .map(|game| game.cards.as_slice())
            .unwrap_or(&[])
    }

    fn shallow_replacement_game(game: &GameState) -> GameState {
        let player_names: Vec<String> = game.players.iter().map(|p| p.name.clone()).collect();
        let player_name_refs: Vec<&str> = player_names.iter().map(String::as_str).collect();
        let starting_life = game.players.first().map(|p| p.life).unwrap_or(20);
        let mut sim = GameState::new(&player_name_refs, starting_life);
        sim.players = game.players.clone();
        sim.cards = Self::shallow_cards(game);
        sim.turn = game.turn.clone();
        sim
    }

    pub fn new(
        player_id: PlayerId,
        verbose: VerboseMode,
        rng: Rc<RefCell<JavaRandom>>,
        game_rng: Rc<RefCell<JavaRandom>>,
        prefer_actions: bool,
        parity_map: Arc<ParityCardMap>,
        parity_observer: Option<Arc<crate::runner::ParityObserver>>,
    ) -> Self {
        Self {
            player_id,
            log: Vec::new(),
            verbose,
            current_turn: 0,
            last_game_snapshot: None,
            snapshot_game: None,
            rng,
            game_rng,
            prefer_actions,
            parity_map,
            parity_sync_pending: Cell::new(false),
            parity_observer,
            choosing_targets: false,
            target_loop_drew_continue: false,
        }
    }

    /// Keep in sync with `DeterministicController.chooseTargetsFor`: pick one candidate at a
    /// time; stop when no more targets may be added, and once the minimum is met draw one
    /// boolean per extra pick, continuing while it is true. Java's loop is
    /// `if (isMinTargetChosen() && !ChoiceSpace.pickBool(rng)) break;` — a true keeps going, so
    /// it can take more than the minimum.
    fn choose_targets_like_java(
        &mut self,
        remaining: Vec<CardId>,
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        let mut chosen = Vec::new();
        let mut rng = self.rng.borrow_mut();
        // Java's loop condition is `while (!isTargetNumberValid())`, which is false as soon
        // as the minimum is met, so the `pickBool` below draws but can never add another
        // target. Mirroring the condition, not just the body, is what keeps the count equal.
        // A target already chosen stays in the candidate list: `getAllCandidates` still
        // returns it and `TargetChoices.add` is a no-op on a duplicate, so the draw is over
        // the same option count every time and a repeat just costs another iteration.
        while !target_number_valid(chosen.len(), min, max)
            && remaining.iter().any(|cid| !chosen.contains(cid))
        {
            let Some(pick) = choice_space::pick_one(&remaining, &mut rng) else {
                break;
            };
            if !chosen.contains(&pick) {
                chosen.push(pick);
            }
            if chosen.len() >= max {
                break;
            }
            if chosen.len() >= min {
                self.target_loop_drew_continue = true;
                if !choice_space::pick_bool(&mut rng) {
                    break;
                }
            }
        }
        chosen
    }

    /// The loop above, with Java's re-filter: `chooseTargetsFor` rebuilds its candidate list
    /// every iteration and keeps only what `SpellAbility.canTarget` still accepts, so the
    /// relational restrictions see the targets chosen so far.
    fn choose_targets_relational(
        &mut self,
        mut remaining: Vec<CardId>,
        min: usize,
        max: usize,
        sa: &manabrew_engine::spellability::SpellAbility,
    ) -> Vec<CardId> {
        let Some(game) = self.snapshot_game.as_ref() else {
            return self.choose_targets_like_java(remaining, min, max);
        };
        let mut probe = sa.clone();
        let mut chosen: Vec<CardId> = Vec::new();
        let mut rng = self.rng.borrow_mut();
        while !target_number_valid(chosen.len(), min, max)
            && remaining.iter().any(|cid| !chosen.contains(cid))
        {
            let Some(pick) = choice_space::pick_one(&remaining, &mut rng) else {
                break;
            };
            if !chosen.contains(&pick) {
                chosen.push(pick);
                probe.target_chosen.add(Some(pick), None);
            }
            if chosen.len() >= max {
                break;
            }
            remaining.retain(|&cid| probe.relational_target_ok(cid, game));
            if chosen.len() >= min {
                self.target_loop_drew_continue = true;
                if !choice_space::pick_bool(&mut rng) {
                    break;
                }
            }
        }
        chosen
    }

    pub(crate) fn should_skip_priority_action_space(&self) -> bool {
        self.last_game_snapshot
            .as_ref()
            .is_some_and(|snap| snap.stack_depth >= STACK_ACTION_SPACE_SKIP_THRESHOLD)
    }

    pub fn rng_call_count(&self) -> u64 {
        self.rng.borrow().call_count
    }

    pub fn rng(&self) -> Rc<RefCell<JavaRandom>> {
        Rc::clone(&self.rng)
    }

    /// Look up a card name from the cached snapshot.
    fn card_name(&self, id: CardId) -> String {
        if let Some(ref snap) = self.last_game_snapshot {
            for (cid, name) in &snap.card_names {
                if *cid == id {
                    return name.clone();
                }
            }
        }
        format!("Card({})", id.0)
    }

    fn player_name(&self, id: PlayerId) -> String {
        if let Some(ref snap) = self.last_game_snapshot {
            for (pid, name) in &snap.player_names {
                if *pid == id {
                    return name.clone();
                }
            }
        }
        format!("Player{}", id.0 + 1)
    }

    fn defender_sort_key(&self, defender: DefenderId) -> (String, u32) {
        match defender {
            DefenderId::Player(pid) => (self.player_name(pid), pid.0),
            DefenderId::Permanent(cid) => (self.card_name(cid), self.parity_id(cid)),
        }
    }

    /// Check if a card is a land from the cached snapshot.
    fn is_land(&self, id: CardId) -> bool {
        if let Some(ref snap) = self.last_game_snapshot {
            for (cid, land) in &snap.card_is_land {
                if *cid == id {
                    return *land;
                }
            }
        }
        false
    }

    fn is_mana_ability(&self, card_id: CardId, ability_idx: usize) -> bool {
        if ability_idx == STATIC_ALTERNATIVE_ABILITY_INDEX {
            return false;
        }
        if let Some(ref snap) = self.last_game_snapshot {
            for ((cid, idx), is_mana) in &snap.ability_is_mana {
                if *cid == card_id && *idx == ability_idx {
                    return *is_mana;
                }
            }
        }
        false
    }

    /// Find the ability_index of the UnlockDoor activated ability on a Room card.
    /// Used by `action_sort_key` to produce the correct Java-matching sort key
    /// for Room unlock actions.
    fn unlock_door_ability_index(&self, card_id: CardId) -> usize {
        if let Some(ref snap) = self.last_game_snapshot {
            for ((cid, ability_idx), text) in &snap.ability_texts {
                if *cid != card_id {
                    continue;
                }
                if manabrew_engine::parsing::raw_get(text, manabrew_engine::parsing::keys::AB)
                    .map(|v| v.eq_ignore_ascii_case("UnlockDoor"))
                    .unwrap_or(false)
                {
                    return *ability_idx;
                }
            }
        }
        0
    }

    fn ability_sort_text(&self, card_id: CardId, ability_idx: usize) -> String {
        if ability_idx == STATIC_ALTERNATIVE_ABILITY_INDEX {
            return String::new();
        }
        if let Some(ref snap) = self.last_game_snapshot {
            for ((cid, idx), text) in &snap.ability_texts {
                if *cid == card_id && *idx == ability_idx {
                    return text.clone();
                }
            }
        }
        String::new()
    }

    fn target_owner_controller_key(&self, id: CardId) -> (u32, u32) {
        if let Some(ref snap) = self.last_game_snapshot {
            for (cid, owner_controller) in &snap.card_owner_controller {
                if *cid == id {
                    return *owner_controller;
                }
            }
            (u32::MAX, u32::MAX)
        } else {
            (u32::MAX, u32::MAX)
        }
    }

    fn target_sort_key(&self, id: CardId) -> String {
        let (owner, controller) = self.target_owner_controller_key(id);
        format!(
            "1|{}|O{owner:05}|C{controller:05}|I{:05}",
            self.card_name(id),
            self.parity_id(id)
        )
    }

    fn predicted_damage_to_card(
        &self,
        game: &GameState,
        target: CardId,
        amount: i32,
        source: CardId,
        is_combat: bool,
    ) -> i32 {
        if amount <= 0 {
            return 0;
        }
        let mut sim = Self::shallow_replacement_game(game);
        let mut event = ReplacementEvent::DamageToCard {
            target,
            amount,
            source: Some(source),
            is_combat,
        };
        let _ = apply_replacements(&mut sim, &mut event);
        match event {
            ReplacementEvent::DamageToCard { amount, .. } => amount.max(0),
            _ => 0,
        }
    }

    fn damage_needed_to_kill(
        &self,
        game: &GameState,
        target: CardId,
        max_damage: i32,
        source: CardId,
        is_combat: bool,
    ) -> i32 {
        let target_card = game.card(target);
        let source_card = game.card(source);
        let mut kill_damage = (target_card.toughness() - target_card.damage).max(0);

        if target_card.has_keyword("Indestructible")
            && !source_card.has_wither()
            && !source_card.has_infect()
        {
            return max_damage + 1;
        }
        if source_card.has_deathtouch() && target_card.is_creature() {
            kill_damage = 1;
        }

        for damage in 1..=max_damage {
            if self.predicted_damage_to_card(game, target, damage, source, is_combat) >= kill_damage
            {
                return damage;
            }
        }

        max_damage + 1
    }

    fn play_option_label(&self, play: PlayOption) -> String {
        let cast_face_down = matches!(play.mode, PlayCardMode::Alternative(alt) if alt.is_morph());
        if self.is_land(play.card_id) && play.mode != PlayCardMode::Secondary && !cast_face_down {
            return format!("LAND:{}", self.card_name(play.card_id));
        }
        // MDFC back-face land — Java buckets as LAND via isLandAbility().
        if play.mode == PlayCardMode::BackFaceLand {
            return format!("LAND:{}", self.card_name(play.card_id));
        }
        // Java harness: Room UnlockDoor is a StaticAbilityApiBased where
        // isSpell()=false, isLandAbility()=false, isManaAbility()=false,
        // so actionBaseLabel() classifies it as "AB:".
        if play.mode == PlayCardMode::UnlockDoor {
            return format!("AB:{}", self.card_name(play.card_id));
        }
        // Forge builds Foretell as an `AbilityStatic` (`CardFactoryUtil:2962`), so it is
        // not a spell and `ParityOrder.actionBaseLabel` buckets it as "AB:".
        if play.mode == PlayCardMode::ForetellExile {
            return format!("AB:{}", self.card_name(play.card_id));
        }
        let fb_tag = match play.mode {
            PlayCardMode::Alternative(AlternativeCost::Flashback) => "[FB]",
            _ => "",
        };
        format!("SPELL:{}{}", self.card_name(play.card_id), fb_tag)
    }

    fn play_option_sort_text(play: PlayOption) -> &'static str {
        match play.mode {
            PlayCardMode::Normal => "0",
            PlayCardMode::BackFaceLand => "0",
            PlayCardMode::RoomRightSplit => "0",
            PlayCardMode::Secondary => "0",
            PlayCardMode::Alternative(AlternativeCost::Flashback) => "Flashback",
            PlayCardMode::Alternative(AlternativeCost::Spectacle) => "Spectacle",
            PlayCardMode::Alternative(AlternativeCost::Evoke) => "Evoke",
            PlayCardMode::Alternative(AlternativeCost::Dash) => "Dash",
            PlayCardMode::Alternative(AlternativeCost::Blitz) => "Blitz",
            PlayCardMode::Alternative(AlternativeCost::Escape) => "Escape",
            PlayCardMode::Alternative(AlternativeCost::Overload) => "Overload",
            PlayCardMode::Alternative(AlternativeCost::Madness) => "Madness",
            PlayCardMode::Alternative(AlternativeCost::Foretell) => "Foretell",
            PlayCardMode::Alternative(AlternativeCost::Emerge) => "Emerge",
            PlayCardMode::Alternative(AlternativeCost::Suspend) => "Suspend",
            PlayCardMode::Alternative(AlternativeCost::Morph)
            | PlayCardMode::Alternative(AlternativeCost::Megamorph) => "Morph",
            PlayCardMode::Alternative(AlternativeCost::Bestow) => "Bestow",
            PlayCardMode::Alternative(AlternativeCost::Warp) => "0",
            PlayCardMode::Alternative(AlternativeCost::SacrificeAlt) => "0",
            PlayCardMode::Alternative(AlternativeCost::Plot) => "Plot",
            PlayCardMode::Alternative(AlternativeCost::Awaken) => "Awaken",
            PlayCardMode::Alternative(AlternativeCost::Disturb) => "Disturb",
            PlayCardMode::Alternative(AlternativeCost::Harmonize) => "Harmonize",
            PlayCardMode::Alternative(AlternativeCost::Freerunning) => "Freerunning",
            PlayCardMode::Alternative(AlternativeCost::Impending) => "Impending",
            PlayCardMode::Alternative(AlternativeCost::Mayhem) => "Mayhem",
            PlayCardMode::Alternative(AlternativeCost::MTMtE) => "MTMtE",
            PlayCardMode::Alternative(AlternativeCost::Mutate) => "Mutate",
            PlayCardMode::Alternative(AlternativeCost::Prowl) => "Prowl",
            PlayCardMode::Alternative(AlternativeCost::Sneak) => "Sneak",
            PlayCardMode::Alternative(AlternativeCost::Surge) => "Surge",
            PlayCardMode::Alternative(AlternativeCost::WebSlinging) => "WebSlinging",
            PlayCardMode::Alternative(AlternativeCost::Plotted) => "Plotted",
            // Host-card `Mode$ AlternativeCost` actions are represented in Rust
            // as `StaticAlternative`; parity uses the same explicit label.
            PlayCardMode::StaticAlternative => "StaticAlternative",
            PlayCardMode::ForetellExile => "ForetellExile",
            PlayCardMode::UnlockDoor => "0",
            PlayCardMode::MayPlay(base) => Self::play_option_sort_text(PlayOption {
                mode: base.map_or(PlayCardMode::Normal, PlayCardMode::Alternative),
                ..play
            }),
        }
    }

    /// Fallback tiebreaker for card play modes. Mirrors Java's use of
    /// `sa.toUnsuppressedString()` as the 5th sort key field.
    /// When variant is the same (e.g., Normal and Warp both return "0"),
    /// this ensures a deterministic ordering.
    fn play_option_fallback(&self, play: PlayOption) -> String {
        if let PlayCardMode::MayPlay(base) = play.mode {
            let base_play = PlayOption {
                mode: base.map_or(PlayCardMode::Normal, PlayCardMode::Alternative),
                alt_cost_index: 0,
                ..play
            };
            return format!(
                "{} by:{:03}",
                self.play_option_fallback(base_play),
                play.alt_cost_index
            );
        }
        // Disambiguate multi-cost alt entries (e.g. intrinsic vs granted
        // Evoke) so the stable sort places them in a predictable order that
        // matches Java's SA text ordering.
        let idx_suffix = if play.alt_cost_index > 0 {
            format!(":{:03}", play.alt_cost_index)
        } else {
            String::new()
        };
        // Split cards expose one playable per face — Java's
        // `sa.toUnsuppressedString()` therefore differs per face (front vs
        // back oracle text). Mirror that by feeding the per-face name into
        // the fallback. We read it from `Card::full_name` (the canonical
        // "Front // Back" form for split-type cards) so the lookup stays
        // generic — no Room-specific SVars.
        let base: String = match play.mode {
            PlayCardMode::Normal => self
                .split_face_spell_text(play)
                .or_else(|| self.play_option_face_name(play))
                .or_else(|| self.secondary_face_texts(play).map(|(front, _)| front))
                .or_else(|| self.permanent_spell_text(play))
                .unwrap_or_else(|| "0".to_string()),
            PlayCardMode::BackFaceLand => "1".to_string(),
            PlayCardMode::RoomRightSplit => self
                .split_face_spell_text(play)
                .or_else(|| self.play_option_face_name(play))
                .unwrap_or_else(|| "2".to_string()),
            PlayCardMode::Secondary => self
                .secondary_face_texts(play)
                .map(|(_, secondary)| secondary)
                .unwrap_or_else(|| "1".to_string()),
            PlayCardMode::Alternative(AlternativeCost::Warp) => "Warp".to_string(),
            PlayCardMode::StaticAlternative => "StaticAlternative".to_string(),
            // Other modes already have unique variant strings, so fallback rarely matters.
            _ => String::new(),
        };
        format!("{base}{idx_suffix}")
    }

    /// The two `toUnsuppressedString()` texts Java compares for a card with a Secondary
    /// face (Adventure, Omen): the permanent spell reads `Name - Type ...`, the secondary
    /// spell reads its `SpellDescription$` with `CARDNAME` as the secondary face's name
    /// (`CardTraitBase.getHostName`). A modal back face is a permanent spell too, so both
    /// texts read `Name - Type ...`. Which sorts first depends on the card.
    fn secondary_face_texts(&self, play: PlayOption) -> Option<(String, String)> {
        self.last_game_snapshot.as_ref()?;
        let card = self
            .snapshot_cards()
            .iter()
            .find(|c| c.id == play.card_id)?;
        let other = card.other_part.as_ref()?;
        let front = format!("{} - ", card.card_name);
        match other.state_name {
            forge_foundation::CardStateName::Secondary => {
                let description = other.abilities.first()?.split('|').find_map(|param| {
                    param
                        .trim()
                        .strip_prefix("SpellDescription$")
                        .map(str::trim)
                })?;
                Some((front, description.replace("CARDNAME", &other.name)))
            }
            forge_foundation::CardStateName::Backside if other.is_modal => {
                Some((front, format!("{} - ", other.name)))
            }
            _ => None,
        }
    }

    /// A permanent spell's `toUnsuppressedString()` starts `Name - Type ...`; a land play does
    /// not, and a keyword cast of the same card (Warp reads `Warp {R} (...)`) sorts against it.
    fn permanent_spell_text(&self, play: PlayOption) -> Option<String> {
        self.last_game_snapshot.as_ref()?;
        let card = self
            .snapshot_cards()
            .iter()
            .find(|c| c.id == play.card_id)?;
        if card.type_line.is_land() || !card.is_permanent() {
            return None;
        }
        Some(format!("{} - ", card.card_name))
    }

    /// For a playable on a split card (`"Front // Back"` `full_name`), return
    /// the name of the face being cast — front for `Normal`, back for
    /// `RoomRightSplit`. Returns `None` for non-split cards or modes that
    /// don't pick a face.
    fn play_option_face_name(&self, play: PlayOption) -> Option<String> {
        self.last_game_snapshot.as_ref()?;
        let card = self
            .snapshot_cards()
            .iter()
            .find(|c| c.id == play.card_id)?;
        let (front, back) = card.full_name.split_once(" // ")?;
        Some(match play.mode {
            PlayCardMode::Normal => front.trim().to_string(),
            PlayCardMode::RoomRightSplit => back.trim().to_string(),
            _ => return None,
        })
    }

    fn split_face_spell_text(&self, play: PlayOption) -> Option<String> {
        self.last_game_snapshot.as_ref()?;
        let card = self
            .snapshot_cards()
            .iter()
            .find(|c| c.id == play.card_id)?;
        let other = card.other_part.as_ref()?;
        if other.state_name != forge_foundation::CardStateName::RightSplit {
            return None;
        }
        match play.mode {
            PlayCardMode::Normal if !card.type_line.is_permanent() => {
                Some(card.oracle_text.clone())
            }
            PlayCardMode::RoomRightSplit if !other.type_line.is_permanent() => {
                Some(other.oracle_text.clone())
            }
            _ => None,
        }
    }

    /// Java's `ParityOrder.abilityDeclarationIndex` walks `Card.getSpellAbilities()`,
    /// which lists the card's own spell first and the keyword-made Foretell after it.
    fn foretell_declaration_index(&self, card_id: CardId) -> usize {
        self.snapshot_cards()
            .iter()
            .find(|card| card.id == card_id)
            .map(|card| card.activated_abilities.len().max(1))
            .unwrap_or(1)
    }

    fn action_sort_key(&self, choice: &ActionChoice) -> String {
        match *choice {
            ActionChoice::Card(play) => {
                // Room UnlockDoor: Java models this as a StaticAbilityApiBased
                // where isSpell()=false, so it sorts in the ability bucket (|1|)
                // with abilityDeclarationIndex as the variant, not the spell bucket.
                if play.mode == PlayCardMode::UnlockDoor {
                    let ability_idx = self.unlock_door_ability_index(play.card_id);
                    let sort_idx = self
                        .last_game_snapshot
                        .as_ref()
                        .map(|snap| {
                            parity_order::ability_declaration_sort_key(
                                self.snapshot_cards(),
                                &snap.ability_texts,
                                play.card_id,
                                ability_idx,
                            )
                        })
                        .unwrap_or_else(|| format!("{ability_idx:05}"));
                    return format!(
                        "AB:{}|1|{}|{}|{}",
                        self.card_name(play.card_id),
                        self.parity_id(play.card_id),
                        sort_idx,
                        self.ability_sort_text(play.card_id, ability_idx),
                    );
                }
                if play.mode == PlayCardMode::ForetellExile {
                    return format!(
                        "AB:{}|1|{}|{:05}|{}",
                        self.card_name(play.card_id),
                        self.parity_id(play.card_id),
                        self.foretell_declaration_index(play.card_id),
                        self.play_option_fallback(play),
                    );
                }
                let label = self.play_option_label(play);
                format!(
                    "{}|0|{}|{}|{}",
                    label,
                    self.parity_id(play.card_id),
                    Self::play_option_sort_text(play),
                    self.play_option_fallback(play),
                )
            }
            ActionChoice::Ability(card_id, ability_idx) => {
                let sort_idx = self
                    .last_game_snapshot
                    .as_ref()
                    .map(|snap| {
                        parity_order::ability_declaration_sort_key(
                            self.snapshot_cards(),
                            &snap.ability_texts,
                            card_id,
                            ability_idx,
                        )
                    })
                    .unwrap_or_else(|| {
                        if ability_idx == STATIC_ALTERNATIVE_ABILITY_INDEX {
                            "-0001".to_string()
                        } else {
                            format!("{ability_idx:05}")
                        }
                    });
                format!(
                    "AB:{}|1|{}|{}|{}",
                    self.card_name(card_id),
                    self.parity_id(card_id),
                    sort_idx,
                    self.ability_sort_text(card_id, ability_idx),
                )
            }
        }
    }

    fn sorted_priority_action_choices(
        &self,
        action_space: &PriorityActionSpace,
    ) -> Vec<(String, ActionChoice)> {
        // Match Java harness ActionSpace: omit explicit mana abilities.
        // Activated-ability payability comes directly from engine action-space.
        let filtered_activatable = action_space
            .activatable
            .iter()
            .map(|a| (a.card_id, a.ability_index))
            .filter(|(card_id, ability_idx)| !self.is_mana_ability(*card_id, *ability_idx));
        let choices = action_space
            .playable
            .iter()
            .copied()
            .map(ActionChoice::Card)
            .chain(filtered_activatable.map(|(card_id, idx)| ActionChoice::Ability(card_id, idx)));
        let mut choices: Vec<(String, ActionChoice)> = choices
            .map(|choice| (self.action_sort_key(&choice), choice))
            .collect();
        choices.sort_by(|a, b| a.0.cmp(&b.0));
        choices
    }

    fn format_action_choice_for_log(&self, choice: ActionChoice) -> String {
        match choice {
            ActionChoice::Card(play) => format!(
                "CastSpell(PlayOption {{ card: {}@{}, mode: Normal }})",
                self.card_name(play.card_id),
                self.parity_id(play.card_id),
            ),
            ActionChoice::Ability(card_id, ability_idx) => format!(
                "ActivateAbility(AbilityRef {{ card: {}@{}, ability_index: {} }})",
                self.card_name(card_id),
                self.parity_id(card_id),
                if ability_idx == STATIC_ALTERNATIVE_ABILITY_INDEX {
                    "-1".to_string()
                } else {
                    ability_idx.to_string()
                },
            ),
        }
    }

    pub(crate) fn format_action_space_for_log(
        &self,
        action_space: &PriorityActionSpace,
    ) -> Option<String> {
        let mut rendered: Vec<String> = self
            .sorted_priority_action_choices(action_space)
            .into_iter()
            .enumerate()
            .map(|(idx, (_, choice))| {
                format!("#{idx} {}", self.format_action_choice_for_log(choice))
            })
            .collect();
        if rendered.is_empty() {
            return None;
        }
        rendered.push("PASS".to_string());
        Some(format!("[{}]", rendered.join(" | ")))
    }

    fn snapshot_card(&self, _snap: &GameSnapshot, id: CardId) -> Option<&Card> {
        self.snapshot_cards().iter().find(|c| c.id == id)
    }

    fn snapshot_can_creature_block(
        &self,
        snap: &GameSnapshot,
        blocker_id: CardId,
        attacker_id: CardId,
    ) -> bool {
        let Some(attacker) = self.snapshot_card(snap, attacker_id) else {
            return false;
        };
        let Some(blocker) = self.snapshot_card(snap, blocker_id) else {
            return false;
        };

        if !blocker.can_block() {
            return false;
        }
        if attacker.has_flying() && !blocker.has_flying() && !blocker.has_reach() {
            return false;
        }
        if attacker.has_fear() && !blocker.type_line.is_artifact() && !blocker.color.has_black() {
            return false;
        }
        if attacker.has_intimidate()
            && !blocker.type_line.is_artifact()
            && !blocker.color.shares_color_with(attacker.color)
        {
            return false;
        }
        if attacker.has_shadow() != blocker.has_shadow() {
            return false;
        }
        if attacker.has_horsemanship() && !blocker.has_horsemanship() {
            return false;
        }
        if attacker.has_skulk() && blocker.power() > attacker.power() {
            return false;
        }
        if attacker.is_protected_from(blocker) {
            return false;
        }

        for source in self.snapshot_cards().iter().filter(|c| {
            c.zone == forge_foundation::ZoneType::Battlefield
                || c.zone == forge_foundation::ZoneType::Command
        }) {
            for sa in &source.static_abilities {
                if !sa.check_mode(&manabrew_engine::staticability::StaticMode::CantBlockBy) {
                    continue;
                }
                if let Some(game) = self.snapshot_game() {
                    if !sa.check_conditions(source, game) {
                        continue;
                    }
                }

                if let Some(valid_attacker) = sa.ir.valid_attacker.as_ref() {
                    if !manabrew_engine::card::valid_filter::matches_valid_card_selector(
                        valid_attacker,
                        attacker,
                        source,
                    ) {
                        continue;
                    }
                }

                if let Some(valid_blocker) = sa.ir.valid_blocker_text.as_deref() {
                    let blocker_matches = valid_blocker.split(',').any(|v| {
                        manabrew_engine::card::valid_filter::matches_valid_card(
                            v.trim(),
                            blocker,
                            source,
                        )
                    });
                    if !blocker_matches {
                        continue;
                    }
                }

                return false;
            }
        }

        true
    }

    fn legal_attackers_for_blocker(&self, blocker: CardId, attackers: &[CardId]) -> Vec<CardId> {
        let Some(ref snap) = self.last_game_snapshot else {
            return attackers.to_vec();
        };
        attackers
            .iter()
            .copied()
            .filter(|&attacker| self.snapshot_can_creature_block(snap, blocker, attacker))
            .collect()
    }

    fn snapshot_max_blockers_for_attacker(&self, snap: &GameSnapshot, attacker: CardId) -> usize {
        let Some(attacker_card) = self.snapshot_card(snap, attacker) else {
            return usize::MAX;
        };
        let mut max = usize::MAX;
        for source in self.snapshot_cards().iter().filter(|c| {
            c.zone == forge_foundation::ZoneType::Battlefield
                || c.zone == forge_foundation::ZoneType::Command
        }) {
            for st_ab in &source.static_abilities {
                if !st_ab.check_mode(&manabrew_engine::staticability::StaticMode::MinMaxBlocker) {
                    continue;
                }
                if !manabrew_engine::card::valid_filter::matches_valid_card_selector_opt(
                    st_ab.ir.valid_card.as_ref(),
                    attacker_card,
                    source,
                ) {
                    continue;
                }
                if let Some(max_text) = st_ab.ir.max_text.as_deref() {
                    if let Ok(value) = max_text.trim().parse::<usize>() {
                        max = max.min(value);
                    }
                }
            }
        }
        max
    }

    /// Pick a random index in [0, len) from the shared RNG.
    fn pick(&self, len: usize) -> usize {
        choice_space::pick_index(len, &mut self.rng.borrow_mut())
    }

    fn is_verbose(&self) -> bool {
        self.verbose.is_active(self.current_turn)
    }

    /// Java's `choose_targets_for(candidates)` row. It must not assign a parity id.
    fn log_target_candidates(&self, players: &[PlayerId], cards: &[CardId]) {
        if !self.choosing_targets {
            return;
        }
        let names: Vec<String> = players
            .iter()
            .map(|player| format!("Player({})", player.0))
            .chain(cards.iter().map(|&card| {
                let id = self
                    .parity_map
                    .peek(card)
                    .map_or_else(|| "?".to_string(), |id| id.to_string());
                format!("Card({}@{id})", self.card_name(card))
            }))
            .collect();
        self.emit_callback(
            "choose_targets_for(candidates)",
            &format!("[{}]", names.join(", ")),
        );
    }

    fn emit_callback(&self, name: &str, outcome: &str) {
        if let Some(ref observer) = self.parity_observer {
            observer.on_callback(
                name,
                outcome,
                self.player_id.0,
                self.current_turn,
                &format!(
                    "{:?}",
                    self.last_game_snapshot
                        .as_ref()
                        .map(|s| &s.phase)
                        .unwrap_or(&PhaseType::Untap)
                ),
                Vec::new(),
            );
        }
    }
}

impl PlayerAgent for DeterministicAgent {
    fn snapshot_state(&mut self, game: &GameState, _mana_pools: &[ManaPool]) {
        let split_priority_snapshot = manabrew_engine::perf::current_params_lookup_scope()
            == Some(manabrew_engine::perf::ParamsLookupScope::PrioritySnapshot);
        self.parity_sync_pending.set(true);

        let (
            mut player_names,
            mut card_names,
            mut card_is_land,
            mut card_owner_controller,
            mut ability_is_mana,
            mut ability_texts,
            mut stack_sources,
        ) = match self.last_game_snapshot.take() {
            Some(prev) => (
                prev.player_names,
                prev.card_names,
                prev.card_is_land,
                prev.card_owner_controller,
                prev.ability_is_mana,
                prev.ability_texts,
                prev.stack_sources,
            ),
            None => Default::default(),
        };

        refill_named(
            &mut player_names,
            game.players
                .iter()
                .map(|player| (player.id, player.name.as_str())),
        );
        {
            let _perf_scope = split_priority_snapshot
                .then(|| {
                    manabrew_engine::perf::ParamsLookupScopeGuard::enter(
                        manabrew_engine::perf::ParamsLookupScope::PrioritySnapshotMetadata,
                    )
                })
                .flatten();
            refill_named(
                &mut card_names,
                game.cards.iter().map(|c| {
                    // Keep in sync with FmtCtx::card: the action-space log names come
                    // from this snapshot, so both must match Java's getName().
                    let split_off_battlefield = c.zone != forge_foundation::ZoneType::Battlefield
                        && c.other_part.as_ref().is_some_and(|o| {
                            o.state_name == forge_foundation::CardStateName::RightSplit
                        });
                    let name = if c.face_down {
                        ""
                    } else if split_off_battlefield {
                        c.full_name.as_str()
                    } else {
                        c.card_name.as_str()
                    };
                    (c.id, name)
                }),
            );
            refill(
                &mut card_is_land,
                game.cards.iter().map(|c| (c.id, c.is_land())),
            );
            refill(
                &mut card_owner_controller,
                game.cards
                    .iter()
                    .map(|c| (c.id, (c.owner.0, c.controller.0))),
            );
        };
        {
            let _perf_scope = split_priority_snapshot
                .then(|| {
                    manabrew_engine::perf::ParamsLookupScopeGuard::enter(
                        manabrew_engine::perf::ParamsLookupScope::PrioritySnapshotAbility,
                    )
                })
                .flatten();
            refill(
                &mut ability_is_mana,
                game.cards.iter().flat_map(|c| {
                    c.activated_abilities
                        .iter()
                        .map(move |ab| ((c.id, ab.ability_index), ab.is_mana_ability))
                }),
            );
            refill_named(
                &mut ability_texts,
                game.cards.iter().flat_map(|c| {
                    c.activated_abilities
                        .iter()
                        .map(move |ab| ((c.id, ab.ability_index), ab.ability_text.as_str()))
                }),
            );
        };
        {
            let _perf_scope = split_priority_snapshot
                .then(|| {
                    manabrew_engine::perf::ParamsLookupScopeGuard::enter(
                        manabrew_engine::perf::ParamsLookupScope::PrioritySnapshotCardClone,
                    )
                })
                .flatten();
            self.snapshot_game = Some(Self::shallow_game_state(self.snapshot_game.take(), game));
        }
        refill(
            &mut stack_sources,
            game.stack
                .iter()
                .filter_map(|entry| entry.spell_ability.source.map(|source| (entry.id, source))),
        );
        self.last_game_snapshot = Some(GameSnapshot {
            stack_sources,
            player_names,
            card_names,
            card_is_land,
            card_owner_controller,
            ability_is_mana,
            ability_texts,
            phase: game.turn.phase,
            stack_depth: game.stack.len(),
        });
    }

    fn choose_targets_for(
        &mut self,
        sa: &mut manabrew_engine::spellability::SpellAbility,
        game: &GameState,
        mana_pools: &[ManaPool],
    ) -> bool {
        self.snapshot_state(game, mana_pools);
        if let Some(tr) = sa.target_restrictions.as_ref() {
            let min_targets = tr.get_min_targets(game, sa);
            let current_targets = sa.target_chosen.all_target_cards().len() as i32
                + sa.target_chosen.all_target_players().len() as i32
                + i32::from(sa.target_chosen.target_stack_entry.is_some());
            if current_targets == 0 && min_targets <= 0 {
                return true;
            }
        }
        self.choosing_targets = true;
        self.target_loop_drew_continue = false;
        let result =
            manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, mana_pools);
        self.choosing_targets = false;
        // `DeterministicController.chooseTargetsFor` draws a stop-or-continue bool after a
        // pick that meets the minimum while another target could still be added, and its
        // loop then ends either way. Kinds that do not go through
        // `choose_targets_like_java` take one target and owe the same draw.
        if !self.target_loop_drew_continue {
            if let Some(tr) = sa.target_restrictions.as_ref() {
                let chosen = sa.target_chosen.all_target_cards().len() as i32
                    + sa.target_chosen.all_target_players().len() as i32
                    + i32::from(sa.target_chosen.target_stack_entry.is_some());
                if chosen > 0
                    && chosen >= tr.get_min_targets(game, sa)
                    && chosen < tr.get_max_targets(game, sa)
                {
                    choice_space::pick_bool(&mut self.rng.borrow_mut());
                }
            }
        }

        // Log the actual targets chosen for parity debugging.
        let mut target_names = Vec::new();
        for pid in sa.target_chosen.all_target_players() {
            target_names.push(format!("Player({})", pid.0));
        }
        if let Some(cid) = sa.target_chosen.target_card {
            target_names.push(format!("{}@{}", self.card_name(cid), self.parity_id(cid)));
        }
        for &cid in sa.target_chosen.divided_map.keys() {
            target_names.push(format!("{}@{}", self.card_name(cid), self.parity_id(cid)));
        }
        if let Some(stack_id) = sa.target_chosen.target_stack_entry {
            target_names.push(format!("Stack({stack_id})"));
        }
        if !target_names.is_empty() {
            self.emit_callback(
                "choose_targets_for(inner)",
                &format!("[{}]", target_names.join(", ")),
            );
        }
        result
    }

    fn mulligan_decision(
        &mut self,
        _player: PlayerId,
        _hand: &[CardId],
        _mulligan_count: u32,
    ) -> bool {
        true
    }

    fn choose_action(
        &mut self,
        _player: PlayerId,
        action_space: Option<&PriorityActionSpace>,
        request_action_space: &mut dyn FnMut() -> PriorityActionSpace,
    ) -> PlayerAction {
        if self.should_skip_priority_action_space() {
            return PlayerAction::PassPriority;
        }
        let requested_action_space;
        let action_space = match action_space {
            Some(action_space) => action_space,
            None => {
                requested_action_space = request_action_space();
                &requested_action_space
            }
        };
        let playable = &action_space.playable;
        let activatable = &action_space.activatable;
        if self.is_verbose() {
            let raw_playable: Vec<String> = playable
                .iter()
                .map(|play| {
                    format!(
                        "{} [{}]",
                        self.action_sort_key(&ActionChoice::Card(*play)),
                        match play.mode {
                            PlayCardMode::Normal => "Normal",
                            PlayCardMode::BackFaceLand => "BackFaceLand",
                            PlayCardMode::RoomRightSplit => "RoomRightSplit",
                            PlayCardMode::Secondary => "Secondary",
                            PlayCardMode::UnlockDoor => "UnlockDoor",
                            PlayCardMode::StaticAlternative => "StaticAlternative",
                            PlayCardMode::ForetellExile => "ForetellExile",
                            PlayCardMode::Alternative(_) => "Alternative",
                            PlayCardMode::MayPlay(_) => "MayPlay",
                        }
                    )
                })
                .collect();
            let raw_activatable: Vec<String> = activatable
                .iter()
                .map(|a| {
                    format!(
                        "AB:{}@{}:{} mana={}",
                        self.card_name(a.card_id),
                        self.parity_id(a.card_id),
                        a.ability_index,
                        self.is_mana_ability(a.card_id, a.ability_index)
                    )
                })
                .collect();
            eprintln!(
                "[parity-agent p{}] raw playable({}): {}",
                self.player_id.0,
                playable.len(),
                raw_playable.join(" | ")
            );
            eprintln!(
                "[parity-agent p{}] raw activatable({}): {}",
                self.player_id.0,
                activatable.len(),
                raw_activatable.join(" | ")
            );
        }
        if playable.is_empty() && activatable.is_empty() {
            return PlayerAction::PassPriority;
        }

        let choices = self.sorted_priority_action_choices(action_space);
        if self.is_verbose() {
            let rendered: Vec<String> = choices
                .iter()
                .enumerate()
                .map(|(idx, (sort_key, choice))| match *choice {
                    ActionChoice::Card(play) => format!(
                        "#{idx}: {sort_key} [{}]",
                        match play.mode {
                            PlayCardMode::Normal => "Normal",
                            PlayCardMode::BackFaceLand => "BackFaceLand",
                            PlayCardMode::RoomRightSplit => "RoomRightSplit",
                            PlayCardMode::Secondary => "Secondary",
                            PlayCardMode::UnlockDoor => "UnlockDoor",
                            PlayCardMode::StaticAlternative => "StaticAlternative",
                            PlayCardMode::ForetellExile => "ForetellExile",
                            PlayCardMode::Alternative(_) => "Alternative",
                            PlayCardMode::MayPlay(_) => "MayPlay",
                        }
                    ),
                    ActionChoice::Ability(card_id, ability_idx) => format!(
                        "#{idx}: AB:{}@{}:{}",
                        self.card_name(card_id),
                        self.parity_id(card_id),
                        ability_idx
                    ),
                })
                .collect();
            eprintln!(
                "[parity-agent p{}] actions({}): {}",
                self.player_id.0,
                choices.len(),
                rendered.join(" | ")
            );
        }
        if choices.is_empty() {
            return PlayerAction::PassPriority;
        }
        // Pick randomly:
        // - default: each action + pass are equally likely
        // - prefer-actions: each action has weight PREFER_ACTION_WEIGHT, pass has weight 1
        let chosen_idx = if self.prefer_actions {
            let idx = choice_space::pick_weighted_index_with_pass(
                choices.len(),
                PREFER_ACTION_WEIGHT,
                &mut self.rng.borrow_mut(),
            );
            if idx >= choices.len() {
                return PlayerAction::PassPriority;
            }
            idx
        } else {
            let idx = choice_space::pick_index_with_pass(choices.len(), &mut self.rng.borrow_mut());
            if idx >= choices.len() {
                return PlayerAction::PassPriority;
            }
            idx
        };

        match choices[chosen_idx].1 {
            ActionChoice::Card(chosen) => PlayerAction::CastSpell(chosen),
            ActionChoice::Ability(card_id, ability_idx) => {
                PlayerAction::ActivateAbility(AbilityRef {
                    card_id,
                    ability_index: ability_idx,
                })
            }
        }
    }

    fn pay_mana_cost(
        &mut self,
        _player: PlayerId,
        _card_id: CardId,
        _card_name: &str,
        _mana_cost: &str,
        _mana_cost_display: &str,
        _mana_cost_checkpoint: &str,
        _can_confirm_from_pool: bool,
        _allow_reserved_source_reuse: bool,
        _reserved_sacrifices: &[CardId],
        _mana_ability_options: &[manabrew_engine::agent::ManaAbilityOption],
        _tappable_lands: &[CardId],
        _untappable_lands: &[CardId],
        _mana_pool: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }

    fn pay_combat_cost(
        &mut self,
        _player: PlayerId,
        _attacker: CardId,
        _cost: i32,
        _description: &str,
        _mana_ability_options: &[manabrew_engine::agent::ManaAbilityOption],
        _tappable_lands: &[CardId],
        _untappable_lands: &[CardId],
        _mana_pool_total: i32,
    ) -> manabrew_engine::agent::CombatCostAction {
        manabrew_engine::agent::CombatCostAction::AutoPay
    }

    fn choose_attackers(
        &mut self,
        _player: PlayerId,
        available: &[CardId],
        possible_defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        let mut attackers = Vec::new();
        if !possible_defenders.is_empty() {
            let sorted_defenders = choice_space::sort_native(possible_defenders, |a, b| {
                self.defender_sort_key(*a).cmp(&self.defender_sort_key(*b))
            });
            let sorted_available = choice_space::sort_native(available, |a, b| {
                let an = self.card_name(*a);
                let bn = self.card_name(*b);
                an.cmp(&bn)
                    .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
            });
            for &id in &sorted_available {
                let roll = choice_space::pick_index(2, &mut self.rng.borrow_mut());
                if self.is_verbose() {
                    eprintln!(
                        "[parity-agent p{}] atk roll {} -> {}",
                        self.player_id.0,
                        self.card_name(id),
                        roll
                    );
                }
                if roll == 1 {
                    let def_idx = choice_space::pick_index(
                        sorted_defenders.len(),
                        &mut self.rng.borrow_mut(),
                    );
                    if self.is_verbose() {
                        eprintln!(
                            "[parity-agent p{}] atk defender {} idx={}/{}",
                            self.player_id.0,
                            self.card_name(id),
                            def_idx,
                            sorted_defenders.len()
                        );
                    }
                    attackers.push((id, sorted_defenders[def_idx]));
                }
            }
        }
        if !attackers.is_empty() {
            let names: Vec<String> = attackers
                .iter()
                .map(|&(id, _)| self.card_name(id))
                .collect();
            let _joined = names.join(", ");
        }
        attackers
    }

    fn exert_attackers(&mut self, _player: PlayerId, attackers: &[CardId]) -> Vec<CardId> {
        if attackers.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        let mut rng = self.rng.borrow_mut();
        for &attacker in attackers {
            if gui_repro::pick_bool(&mut rng) {
                out.push(attacker);
            }
        }
        out
    }

    fn enlist_attackers(&mut self, _player: PlayerId, attackers: &[CardId]) -> Vec<CardId> {
        if attackers.is_empty() {
            return vec![];
        }
        choice_space::pick_one(attackers, &mut self.rng.borrow_mut())
            .into_iter()
            .collect()
    }

    fn choose_blockers(
        &mut self,
        player: PlayerId,
        attackers: &[CardId],
        available_blockers: &[CardId],
        max_blockers: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        let sorted_attackers = choice_space::sort_native(attackers, |a, b| {
            let an = self.card_name(*a);
            let bn = self.card_name(*b);
            an.cmp(&bn)
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        let sorted_blockers = choice_space::sort_native(available_blockers, |a, b| {
            let an = self.card_name(*a);
            let bn = self.card_name(*b);
            an.cmp(&bn)
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });

        let mut pairs = Vec::new();
        let mut blocker_counts_by_attacker: HashMap<CardId, usize> = HashMap::new();
        let mut combat = manabrew_engine::combat::CombatState::new();
        for &attacker in attackers {
            combat.declare_attacker(attacker, DefenderId::Player(player), 0);
        }
        for &blocker in &sorted_blockers {
            // When BlockRestrict limit is reached, Java still iterates remaining
            // blockers with 0 legal options (consuming RNG for forced PASS).
            // Mirror this by continuing iteration but with empty legal attackers.
            // Java builds its blocker list with `CombatChoiceSpace.legalBlockers`, which
            // leaves out a creature that can block none of the attackers, so such a
            // creature draws nothing. A listed blocker whose options run out later
            // (block limit, max blockers per attacker) still draws its forced PASS.
            let initial_attackers = self.legal_attackers_for_blocker(blocker, &sorted_attackers);
            if initial_attackers.is_empty() {
                continue;
            }
            let at_limit = max_blockers.is_some_and(|max| pairs.len() >= max);
            let mut legal_attackers = if at_limit {
                Vec::new() // no legal targets → forced PASS (consumes RNG)
            } else {
                initial_attackers
            };
            if let Some(ref snap) = self.last_game_snapshot {
                legal_attackers.retain(|attacker| {
                    let current = blocker_counts_by_attacker
                        .get(attacker)
                        .copied()
                        .unwrap_or(0);
                    current < self.snapshot_max_blockers_for_attacker(snap, *attacker)
                });
            }
            if let Some(game) = self.snapshot_game.as_ref() {
                legal_attackers.retain(|&attacker| {
                    !manabrew_engine::combat::combat_util::lure_forbids_block(
                        game, &combat, attacker, blocker,
                    )
                });
            }
            let choice = choice_space::pick_index_with_pass(
                legal_attackers.len(),
                &mut self.rng.borrow_mut(),
            );
            if choice > 0 && choice <= legal_attackers.len() {
                let attacker = legal_attackers[choice - 1];
                *blocker_counts_by_attacker.entry(attacker).or_default() += 1;
                combat.declare_blocker(blocker, attacker, 0);
                pairs.push((blocker, attacker));
            }
        }
        if pairs.is_empty() {
            return pairs;
        }
        pairs
    }

    fn choose_blocker_for(
        &mut self,
        _player: PlayerId,
        attackers: &[CardId],
        blocker: CardId,
    ) -> Option<CardId> {
        let sorted_attackers = choice_space::sort_native(attackers, |a, b| {
            let an = self.card_name(*a);
            let bn = self.card_name(*b);
            an.cmp(&bn)
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        let legal_attackers = self.legal_attackers_for_blocker(blocker, &sorted_attackers);
        if legal_attackers.is_empty() {
            // Java DeterministicController always rolls `nextInt(options.size() + 1)`.
            // When options is empty, that's `nextInt(1)` (consumes RNG, always 0).
            let _ = choice_space::pick_index_with_pass(0, &mut self.rng.borrow_mut());
            return None;
        }
        let attacker = combat_choice_space::pick_single_blocker_target(
            &legal_attackers,
            &mut self.rng.borrow_mut(),
        );
        attacker?;
        let attacker = attacker.unwrap();
        Some(attacker)
    }

    fn choose_damage_assignment_order(
        &mut self,
        _player: PlayerId,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> Vec<CardId> {
        parity_order::sort_cards_by_name_then_id(
            blockers,
            |cid| self.card_name(cid),
            |cid| self.parity_id(cid),
        )
    }

    fn assign_combat_damage(
        &mut self,
        game: &GameState,
        _player: PlayerId,
        attacker: CardId,
        blockers_in_order: &[CardId],
        defender_id: Option<DefenderId>,
        damage_to_assign: i32,
    ) -> Vec<(Option<CardId>, i32)> {
        let mut out: Vec<(Option<CardId>, i32)> = Vec::new();
        if damage_to_assign <= 0 {
            return out;
        }

        let has_trample = game.card(attacker).has_trample();
        let can_assign_defender = has_trample && defender_id.is_some();
        let mut damage_left = damage_to_assign;
        let mut last_target: Option<CardId> = None;

        for &blocker in blockers_in_order {
            if damage_left <= 0 {
                break;
            }
            if game.card(blocker).zone != forge_foundation::ZoneType::Battlefield {
                continue;
            }
            if manabrew_engine::staticability::static_ability_colorless_damage_source::target_is_protected_from_source(
                &game.cards,
                game.card(blocker),
                game.card(attacker),
            ) {
                continue;
            }
            last_target = Some(blocker);
            let blocker_card = game.card(blocker);
            let lethal = if blocker_card.type_line.is_planeswalker() {
                blocker_card.counter_count(&manabrew_engine::card::CounterType::Loyalty)
            } else {
                self.damage_needed_to_kill(game, blocker, damage_left, attacker, true)
            };
            let assign = lethal.min(damage_left);
            if assign > 0 {
                out.push((Some(blocker), assign));
                damage_left -= assign;
            }
        }

        if damage_left > 0 {
            if can_assign_defender {
                out.push((None, damage_left));
            } else if let Some(last) = last_target {
                if let Some((_, d)) = out
                    .iter_mut()
                    .find(|(assignee, _)| assignee.map(|id| id == last).unwrap_or(false))
                {
                    *d += damage_left;
                } else {
                    out.push((Some(last), damage_left));
                }
            }
        }

        out
    }

    fn choose_target_card_or_stack(
        &mut self,
        _player: PlayerId,
        cards: &[CardId],
        stack: &[(u32, CardId)],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> manabrew_engine::agent::CardOrStackTarget {
        use manabrew_engine::agent::CardOrStackTarget;
        // `chooseTargetsFor` lists the cards, then the stack candidates, and sorts them with
        // `ParityOrder.targetSortKey` on each one's card (a stack item's host), a stable sort.
        let mut entries: Vec<(CardOrStackTarget, CardId)> = cards
            .iter()
            .map(|&cid| (CardOrStackTarget::Card(cid), cid))
            .chain(
                stack
                    .iter()
                    .map(|&(id, host)| (CardOrStackTarget::Stack(id), host)),
            )
            .collect();
        entries.sort_by_key(|(_, a)| self.target_sort_key(*a));
        let hosts: Vec<CardId> = entries.iter().map(|(_, host)| *host).collect();
        self.log_target_candidates(&[], &hosts);
        let choices: Vec<CardOrStackTarget> = entries.iter().map(|(choice, _)| *choice).collect();
        choice_space::pick_one(&choices, &mut self.rng.borrow_mut())
            .unwrap_or(CardOrStackTarget::None)
    }

    fn choose_target_spell(
        &mut self,
        _player: PlayerId,
        valid: &[u32],
        _source: Option<CardId>,
    ) -> Option<u32> {
        if valid.is_empty() {
            return None;
        }
        // Java sorts every target candidate with `ParityOrder.targetSortKey`; a spell on
        // the stack is its card there: name, owner and controller, parity id.
        let source_of = |stack_id: u32| {
            self.last_game_snapshot.as_ref().and_then(|snap| {
                snap.stack_sources
                    .iter()
                    .find(|(id, _)| *id == stack_id)
                    .map(|(_, source)| *source)
            })
        };
        let sorted =
            choice_space::sort_native(valid, |a, b| match (source_of(*a), source_of(*b)) {
                (Some(ca), Some(cb)) => self.target_sort_key(ca).cmp(&self.target_sort_key(cb)),
                _ => a.cmp(b),
            });
        let spell_cards: Vec<CardId> = sorted.iter().filter_map(|&id| source_of(id)).collect();
        self.log_target_candidates(&[], &spell_cards);
        let target = choice_space::pick_one(&sorted, &mut self.rng.borrow_mut())?;
        Some(target)
    }

    fn choose_target_player(
        &mut self,
        _player: PlayerId,
        valid: &[PlayerId],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<PlayerId> {
        if valid.is_empty() {
            return None;
        }
        self.log_target_candidates(valid, &[]);
        let target = choice_space::pick_one(valid, &mut self.rng.borrow_mut())?;
        Some(target)
    }

    fn choose_target_card(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<CardId> {
        if valid.is_empty() {
            return None;
        }
        // Keep target ordering aligned with Java parity harness:
        // sort by card name, then owner/controller, then parity id.
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.target_sort_key(*a).cmp(&self.target_sort_key(*b))
        });
        self.log_target_candidates(&[], &sorted);
        let target = choice_space::pick_one(&sorted, &mut self.rng.borrow_mut())?;
        Some(target)
    }

    fn choose_target_card_from_zone(
        &mut self,
        _player: PlayerId,
        _zone: forge_foundation::ZoneType,
        valid: &[CardId],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<CardId> {
        if valid.is_empty() {
            return None;
        }
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.target_sort_key(*a).cmp(&self.target_sort_key(*b))
        });
        self.log_target_candidates(&[], &sorted);
        choice_space::pick_one(&sorted, &mut self.rng.borrow_mut())
    }

    fn choose_target_any(
        &mut self,
        _player: PlayerId,
        valid_players: &[PlayerId],
        valid_cards: &[CardId],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> TargetChoice {
        let mut sorted: Vec<TargetChoice> = valid_players
            .iter()
            .copied()
            .map(TargetChoice::Player)
            .chain(valid_cards.iter().copied().map(TargetChoice::Card))
            .collect();
        // Keep target ordering aligned with Java parity harness:
        // players first by id/name, then cards by name, owner/controller, parity id.
        sorted.sort_by(|a, b| match (a, b) {
            (TargetChoice::Player(pa), TargetChoice::Player(pb)) => pa.0.cmp(&pb.0),
            (TargetChoice::Player(_), TargetChoice::Card(_)) => std::cmp::Ordering::Less,
            (TargetChoice::Card(_), TargetChoice::Player(_)) => std::cmp::Ordering::Greater,
            (TargetChoice::Card(ca), TargetChoice::Card(cb)) => {
                self.target_sort_key(*ca).cmp(&self.target_sort_key(*cb))
            }
            _ => std::cmp::Ordering::Equal,
        });
        let (candidate_players, candidate_cards): (Vec<PlayerId>, Vec<CardId>) = (
            sorted
                .iter()
                .filter_map(|choice| match choice {
                    TargetChoice::Player(player) => Some(*player),
                    _ => None,
                })
                .collect(),
            sorted
                .iter()
                .filter_map(|choice| match choice {
                    TargetChoice::Card(card) => Some(*card),
                    _ => None,
                })
                .collect(),
        );
        self.log_target_candidates(&candidate_players, &candidate_cards);

        let total = sorted.len();

        if total == 0 {
            return TargetChoice::None;
        }

        let idx = self.pick(total);
        match sorted[idx] {
            TargetChoice::Player(pid) => TargetChoice::Player(pid),
            TargetChoice::Card(cid) => TargetChoice::Card(cid),
            TargetChoice::None => TargetChoice::None,
        }
    }

    fn choose_optional_trigger(
        &mut self,
        _player: PlayerId,
        _description: &str,
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let accept = choice_space::pick_bool(&mut self.rng.borrow_mut());
        accept
    }

    fn confirm_action(
        &mut self,
        _player: PlayerId,
        _mode: Option<&str>,
        _message: &str,
        _options: &[String],
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let accept = choice_space::pick_bool(&mut self.rng.borrow_mut());
        accept
    }

    fn confirm_replacement_effect(
        &mut self,
        _player: PlayerId,
        _question: &str,
        _effect_description: &str,
        _source: Option<CardId>,
    ) -> bool {
        let accept = choice_space::pick_bool(&mut self.rng.borrow_mut());
        accept
    }

    fn confirm_payment(
        &mut self,
        _player: PlayerId,
        _cost_kind: &str,
        _message: &str,
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let accept = choice_space::pick_bool(&mut self.rng.borrow_mut());
        accept
    }

    fn pay_cost_to_prevent_effect(
        &mut self,
        _player: PlayerId,
        _cost_kind: &str,
        _message: &str,
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
        can_pay: bool,
        _targets: &[GameEntity],
        _effect_text: &str,
    ) -> bool {
        // Java DeterministicController.payCostToPreventEffect short-circuits
        // to false when ComputerUtilCost.canPayCost reports the cost as
        // unpayable; otherwise it enters deterministic cost payment directly
        // (no separate boolean RNG). Mirror that gate here.
        can_pay
    }

    fn choose_binary(
        &mut self,
        _player: PlayerId,
        _question: &str,
        _kind: BinaryChoiceKind,
        _default_choice: Option<bool>,
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        let chosen_left = choice_space::pick_bool(&mut self.rng.borrow_mut());
        chosen_left
    }

    // ── Fixed overrides that sort alphabetically (matching Java) but use no RNG ──

    fn choose_legend_keep(&mut self, _player: PlayerId, duplicates: &[CardId]) -> CardId {
        // Sort by (card_name, parity_id) for deterministic cross-engine parity.
        // Both Java and Rust sort identically to avoid HashMap ordering issues.
        let sorted = choice_space::sort_native(duplicates, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        choice_space::pick_one(&sorted, &mut self.rng.borrow_mut()).unwrap_or(duplicates[0])
    }

    fn choose_sacrifice(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        _source: Option<CardId>,
    ) -> Option<CardId> {
        if valid.is_empty() {
            return None;
        }
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        // Match Java `choosePermanentsToSacrifice` which calls
        // `ChoiceSpace.pickManyCards(sorted, min=1, max=1, rng)`. The RNG
        // trajectory must match, so walk the same `pick_count` + `pick_index`
        // + `pick_many_unique` sequence even though we only return one card.
        let picked = gui_repro::pick_many_unique(&sorted, 1, 1, &mut self.rng.borrow_mut());
        picked.into_iter().next()
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _player: PlayerId,
        min: usize,
        max: usize,
        valid: &[CardId],
        _source: Option<CardId>,
    ) -> Vec<CardId> {
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        gui_repro::pick_many_unique(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn choose_discard(&mut self, _player: PlayerId, hand: &[CardId], num: usize) -> Vec<CardId> {
        if hand.is_empty() || num == 0 {
            return vec![];
        }
        // Sort by (card_name, parity_id) for deterministic cross-engine parity.
        let sorted = choice_space::sort_native(hand, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        gui_repro::pick_many_unique(&sorted, num, num, &mut self.rng.borrow_mut())
    }

    fn choose_discard_any_number(
        &mut self,
        _player: PlayerId,
        hand: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        if hand.is_empty() {
            return vec![];
        }
        let sorted = choice_space::sort_native(hand, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        let clamped_max = max.min(sorted.len());
        gui_repro::pick_many_unique(&sorted, min, clamped_max, &mut self.rng.borrow_mut())
    }

    fn choose_random_discard(
        &mut self,
        _player: PlayerId,
        hand: &[CardId],
        num: usize,
    ) -> Vec<CardId> {
        if hand.is_empty() || num == 0 {
            return vec![];
        }
        // Reservoir sampling with the game RNG, mirroring Java's Aggregates.random()
        // which uses MyRandom.getRandom().nextInt(i) for reservoir replacement.
        // We use game_rng (not agent rng) to match Java's architecture where
        // Aggregates.random() uses MyRandom (the game-level RNG) rather than
        // the agent's decision RNG.
        // IMPORTANT: Do NOT sort — Java iterates cards in zone order (the order
        // they were added to hand), not alphabetically. Sorting would change the
        // reservoir sampling input sequence and produce different results.
        let count = num.min(hand.len());
        let mut rng = self.game_rng.borrow_mut();
        let mut result: Vec<CardId> = hand[..count].to_vec();
        for (offset, &card) in hand[count..].iter().enumerate() {
            let i = count + offset;
            let j = choice_space::pick_index(i + 1, &mut rng);
            if j < count {
                result[j] = card;
            }
        }
        result
    }

    fn choose_dig(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        valid: &[CardId],
        max: usize,
        optional: bool,
    ) -> Vec<CardId> {
        if valid.is_empty() || max == 0 {
            return vec![];
        }
        // Java DigEffect: min = (anyNumber || optional) ? 0 : max
        // When not optional, the player must take exactly `max` cards.
        let min = if optional { 0 } else { max };
        // Sort by (card_name, parity_id) for deterministic cross-engine parity.
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        gui_repro::pick_many_unique(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn vote(&mut self, _player: PlayerId, options: &[String], optional: bool) -> Option<usize> {
        let mut rng = self.rng.borrow_mut();
        if optional && choice_space::pick_bool(&mut rng) {
            return None;
        }
        Some(choice_space::pick_index(options.len(), &mut rng))
    }

    fn choose_cards_to_reveal(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        choice_space::pick_many_unique(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn choose_cards_pile(
        &mut self,
        _player: PlayerId,
        _pile1: &[CardId],
        _pile2: &[CardId],
        _face_down: &str,
    ) -> bool {
        choice_space::pick_bool(&mut self.rng.borrow_mut())
    }

    fn choose_cards_for_effect_multiple(
        &mut self,
        _player: PlayerId,
        pools: &[Vec<CardId>],
        _optional: bool,
    ) -> Vec<CardId> {
        let mut chosen: Vec<CardId> = Vec::new();
        for pool in pools {
            let remaining: Vec<CardId> = pool
                .iter()
                .copied()
                .filter(|card| !chosen.contains(card))
                .collect();
            if remaining.is_empty() {
                continue;
            }
            let sorted = choice_space::sort_native(&remaining, |a, b| {
                self.card_name(*a)
                    .cmp(&self.card_name(*b))
                    .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
            });
            if let Some(pick) = choice_space::pick_one(&sorted, &mut self.rng.borrow_mut()) {
                chosen.push(pick);
            }
        }
        chosen
    }

    fn choose_land_or_spell(&mut self, _player: PlayerId) -> Option<bool> {
        // TODO: engine does not currently expose a typed choice list here.
        None
    }

    fn choose_color(&mut self, _player: PlayerId, valid_colors: &[String]) -> Option<String> {
        let sorted = parity_order::sort_color_names_like_java(valid_colors);
        gui_repro::choose_color(&sorted, &mut self.rng.borrow_mut())
    }

    fn choose_colors(
        &mut self,
        _player: PlayerId,
        valid_colors: &[String],
        min: usize,
        max: usize,
    ) -> Vec<String> {
        let sorted = parity_order::sort_color_names_like_java(valid_colors);
        gui_repro::choose_colors(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn choose_type(
        &mut self,
        _player: PlayerId,
        _type_category: &str,
        valid_types: &[String],
    ) -> Option<String> {
        gui_repro::choose_type(valid_types, &mut self.rng.borrow_mut())
    }

    fn choose_card_name(&mut self, _player: PlayerId, valid_names: &[String]) -> Option<String> {
        gui_repro::choose_card_name(valid_names, &mut self.rng.borrow_mut())
    }

    fn choose_counter_type(
        &mut self,
        _player: PlayerId,
        options: &[manabrew_engine::card::CounterType],
        _prompt: &str,
    ) -> Option<manabrew_engine::card::CounterType> {
        if options.is_empty() {
            return None;
        }
        let idx = choice_space::pick_index(options.len(), &mut self.rng.borrow_mut());
        Some(options[idx].clone())
    }

    fn choose_number(
        &mut self,
        _player: PlayerId,
        _source: Option<CardId>,
        _title: &str,
        _description: Option<&str>,
        min: i32,
        max: i32,
    ) -> Option<i32> {
        Some(gui_repro::choose_number(
            min,
            max,
            &mut self.rng.borrow_mut(),
        ))
    }

    fn choose_number_for_keyword_cost(
        &mut self,
        _player: PlayerId,
        max: i32,
        _prompt: &str,
        _source: Option<CardId>,
    ) -> i32 {
        gui_repro::choose_number(0, max, &mut self.rng.borrow_mut())
    }

    /// Always pay life for phyrexian mana — matches Java's
    /// ComputerUtilMana.payManaCost() which auto-pays phyrexian
    /// shards with life when no colored mana source is available.
    fn choose_phyrexian_pay_life(
        &mut self,
        _player: PlayerId,
        _color: &str,
        _source: Option<CardId>,
    ) -> bool {
        true
    }

    fn choose_cards_for_effect(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
    ) -> Vec<CardId> {
        if valid.is_empty() {
            return vec![];
        }
        // Sort valid cards by (card_name, parity_id) for deterministic cross-engine parity.
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        if self.choosing_targets {
            self.log_target_candidates(&[], &sorted);
            return self.choose_targets_like_java(sorted, min, max);
        }
        gui_repro::pick_many_unique(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn choose_target_cards(
        &mut self,
        player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
        sa: &manabrew_engine::spellability::SpellAbility,
    ) -> Vec<CardId> {
        if valid.is_empty() {
            return vec![];
        }
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.target_sort_key(*a).cmp(&self.target_sort_key(*b))
        });
        if self.choosing_targets {
            self.log_target_candidates(&[], &sorted);
            return self.choose_targets_relational(sorted, min, max, sa);
        }
        self.choose_cards_for_effect(player, valid, min, max)
    }

    fn choose_tap_type_for_cost(
        &mut self,
        _player: PlayerId,
        valid: &[CardId],
        min_total_power: i32,
        card_powers: &[(CardId, i32)],
        card_sort_powers: &[(CardId, i32)],
        _sa: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Vec<CardId> {
        let mut candidates: Vec<(usize, CardId, i32, i32)> = valid
            .iter()
            .enumerate()
            .map(|(idx, &cid)| {
                let power = card_powers
                    .iter()
                    .find(|(card_id, _)| *card_id == cid)
                    .map(|(_, power)| *power)
                    .unwrap_or(0);
                let sort_power = card_sort_powers
                    .iter()
                    .find(|(card_id, _)| *card_id == cid)
                    .map(|(_, power)| *power)
                    .unwrap_or(power);
                (idx, cid, power, sort_power)
            })
            .collect();
        candidates.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));

        let mut chosen = Vec::new();
        let mut total = 0;
        for (_, cid, power, _) in candidates {
            chosen.push(cid);
            total += power;
            if total >= min_total_power {
                break;
            }
        }
        chosen
    }

    fn choose_entities_for_effect(
        &mut self,
        _player: PlayerId,
        candidates: &[GameEntity],
        min: usize,
        max: usize,
    ) -> Vec<GameEntity> {
        if candidates.is_empty() {
            return vec![];
        }
        // Sort entities canonically: players first (by id), cards second (by name + parity_id).
        let mut sorted = candidates.to_vec();
        sorted.sort_by(|a, b| {
            let key = |e: &GameEntity| -> (u8, String, u32) {
                match e {
                    GameEntity::Player(pid) => (0, format!("P{}", pid.0), 0),
                    GameEntity::Card(cid) => (1, self.card_name(*cid), self.parity_id(*cid)),
                }
            };
            key(a).cmp(&key(b))
        });
        gui_repro::pick_many_unique(&sorted, min, max, &mut self.rng.borrow_mut())
    }

    fn choose_single_card_for_zone_change(
        &mut self,
        _game: &GameState,
        player: PlayerId,
        valid: &[CardId],
        _select_prompt: &str,
        _is_optional: bool,
    ) -> Option<CardId> {
        if valid.is_empty() {
            return None;
        }
        // Parity: Java DeterministicController calls ChoiceSpace.pickOne (single RNG draw).
        // Must NOT go through pick_many_unique (pick_count + pick_index) which consumes
        // multiple RNG values and desyncs subsequent picks.
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        let _ = player;
        choice_space::pick_one(&sorted, &mut self.rng.borrow_mut())
    }

    fn choose_cards_for_zone_change(
        &mut self,
        _game: &GameState,
        player: PlayerId,
        valid: &[CardId],
        min: usize,
        max: usize,
        _select_prompt: &str,
    ) -> Vec<CardId> {
        let sorted = choice_space::sort_native(valid, |a, b| {
            self.card_name(*a)
                .cmp(&self.card_name(*b))
                .then_with(|| self.parity_id(*a).cmp(&self.parity_id(*b)))
        });
        self.choose_cards_for_effect(player, &sorted, min, max)
    }

    fn choose_keyword_for_pump(
        &mut self,
        _player: PlayerId,
        options: &[String],
        _source_card_id: Option<CardId>,
    ) -> Option<usize> {
        let indices: Vec<usize> = (0..options.len()).collect();
        choice_space::pick_one(&indices, &mut self.rng.borrow_mut())
    }

    fn choose_mode(
        &mut self,
        _player: PlayerId,
        descriptions: &[String],
        min: usize,
        max: usize,
        _source_card_id: Option<CardId>,
    ) -> Vec<usize> {
        if descriptions.is_empty() {
            return vec![];
        }
        let mut rng = self.rng.borrow_mut();
        let count = gui_repro::pick_count(min, max, descriptions.len(), &mut rng);
        let mut pool: Vec<usize> = (0..descriptions.len()).collect();
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if pool.is_empty() {
                break;
            }
            let idx = choice_space::pick_index(pool.len(), &mut rng);
            out.push(pool.remove(idx));
        }
        out
    }

    fn choose_spell_abilities_for_effect(
        &mut self,
        _player: PlayerId,
        abilities: &[SpellAbility],
        num: usize,
    ) -> Vec<usize> {
        if abilities.is_empty() || num == 0 {
            return vec![];
        }
        let count = num.min(abilities.len());
        let mut pool: Vec<usize> = (0..abilities.len()).collect();
        let mut out = Vec::with_capacity(count);
        let mut rng = self.rng.borrow_mut();
        for _ in 0..count {
            if pool.is_empty() {
                break;
            }
            let idx = choice_space::pick_index(pool.len(), &mut rng);
            out.push(pool.remove(idx));
        }
        out
    }

    fn choose_single_entity_for_effect(
        &mut self,
        _player: PlayerId,
        valid: &[GameEntity],
        _is_optional: bool,
    ) -> Option<GameEntity> {
        if valid.is_empty() {
            return None;
        }
        // Sort to match Java's deterministic ordering for the harness picker:
        // players first (by player_id), then cards by (name, parity_id).
        // Mirrors how `chooseSingleEntityForEffect` iterates a Java
        // `FCollectionView` whose insertion order Java's harness preserves.
        let sorted = choice_space::sort_native(valid, |a, b| {
            let key = |entity: &GameEntity| -> (u8, String, u32) {
                match entity {
                    GameEntity::Player(p) => (0, format!("P{}", p.0), 0),
                    GameEntity::Card(c) => (1, self.card_name(*c), self.parity_id(*c)),
                }
            };
            key(a).cmp(&key(b))
        });
        choice_space::pick_one(&sorted, &mut self.rng.borrow_mut())
    }

    fn get_ability_to_play(
        &mut self,
        _player: PlayerId,
        abilities: &[SpellAbility],
    ) -> Option<usize> {
        if abilities.is_empty() {
            return None;
        }
        let idx = choice_space::pick_index(abilities.len(), &mut self.rng.borrow_mut());
        Some(idx)
    }

    fn choose_scry(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        _source: Option<CardId>,
        cards: &[CardId],
    ) -> Vec<Vec<CardId>> {
        let mut top = Vec::new();
        let mut bottom = Vec::new();
        let mut rng = self.rng.borrow_mut();
        for &cid in cards {
            if gui_repro::pick_bool(&mut rng) {
                bottom.push(cid);
            } else {
                top.push(cid);
            }
        }
        vec![top, bottom]
    }

    fn choose_surveil(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        _source: Option<CardId>,
        cards: &[CardId],
    ) -> Vec<Vec<CardId>> {
        let mut top = Vec::new();
        let mut graveyard = Vec::new();
        let mut rng = self.rng.borrow_mut();
        for &cid in cards {
            if gui_repro::pick_bool(&mut rng) {
                graveyard.push(cid);
            } else {
                top.push(cid);
            }
        }
        vec![top, graveyard]
    }

    fn choose_reorder_library(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        cards: &[CardId],
    ) -> Vec<CardId> {
        // Java's DeterministicController.orderMoveToZoneList returns cards as-is
        // (no RNG consumed), so we must do the same to stay in sync.
        cards.to_vec()
    }

    fn notify(&mut self, event: manabrew_engine::agent::notification::GameNotification) {
        use manabrew_engine::agent::notification::GameNotification;
        match &event {
            GameNotification::Event(log_event) => {
                if self.log.len() >= 500 {
                    self.log.remove(0);
                }
                self.log.push(log_event.message.clone());
                if self.is_verbose() {
                    eprintln!(
                        "[parity-agent-rust p{}] notify: {}",
                        self.player_id.0, log_event.message
                    );
                }
            }
            GameNotification::TurnChanged {
                active_player,
                turn_number,
            } => {
                self.current_turn = *turn_number;
                if self.is_verbose() {
                    eprintln!(
                        "[parity-agent-rust p{}] === Turn {} (P{} active) ===",
                        self.player_id.0, turn_number, active_player.0
                    );
                }
            }
            GameNotification::PhaseChanged { phase } if self.is_verbose() => {
                eprintln!(
                    "[parity-agent-rust p{}] --- Phase: {:?} ---",
                    self.player_id.0, phase
                );
            }
            _ => {}
        }
    }

    fn choose_single_replacement_effect(
        &mut self,
        _player: PlayerId,
        descriptions: &[String],
    ) -> usize {
        let sorted = parity_order::sort_replacement_descriptions_with_indices(descriptions);
        if sorted.is_empty() {
            return 0;
        }
        let picked = choice_space::pick_index(sorted.len(), &mut self.rng.borrow_mut());
        sorted[picked].0
    }

    fn reveal_cards(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        _cards: &[CardId],
        _zone: forge_foundation::ZoneType,
        _owner: PlayerId,
        _message_prefix: Option<&str>,
    ) {
    }
}

/// Java `SpellAbility.isTargetNumberValid`: the minimum is chosen and the maximum is not
/// exceeded. `chooseTargetsFor` loops while this is false.
fn target_number_valid(chosen: usize, min: usize, max: usize) -> bool {
    chosen >= min && chosen <= max
}
