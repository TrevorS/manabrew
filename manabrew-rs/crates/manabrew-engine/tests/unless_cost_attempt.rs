use std::cell::Cell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{ManaAtom, PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, GameEntity, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::{ActionSpaceManaProbe, Mana, ManaPool};
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const TABLET: &str = "Name:Spell Tablet\nManaCost:2\nTypes:Artifact\nA:AB$ Mana | Cost$ T | Produced$ R | SpellDescription$ Add {R}.\nA:AB$ Mana | Cost$ T | Produced$ R | Amount$ 2 | RestrictValid$ Spell.Instant,Spell.Sorcery | SpellDescription$ Add {R}{R}. Spend this mana only to cast instant and sorcery spells.\nOracle:";
const TAX_INSTANT: &str = "Name:Tax Instant\nManaCost:R\nTypes:Instant\nA:SP$ LoseLife | Defined$ Opponent | LifeAmount$ 3 | UnlessCost$ 1 | UnlessPayer$ Opponent | SpellDescription$ Each opponent loses 3 life unless they pay {1}.\nOracle:";
const BEAR: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const EFFECT_SOURCE: &str = "Name:Effect Source\nManaCost:no cost\nTypes:Artifact\nOracle:";

struct WillingPayer {
    mana_asked: Rc<Cell<usize>>,
    prevent_asked: Rc<Cell<usize>>,
}

impl PlayerAgent for WillingPayer {
    fn choose_targets_for(&mut self, _: &mut SpellAbility, _: &GameState, _: &[ManaPool]) -> bool {
        true
    }
    fn mulligan_decision(
        &mut self,
        c: DecisionContext<'_>,
        p: PlayerId,
        h: &[CardId],
        n: u32,
    ) -> bool {
        PassAgent.mulligan_decision(c, p, h, n)
    }
    fn choose_action(
        &mut self,
        p: PlayerId,
        s: Option<&PriorityActionSpace>,
        pr: &mut dyn PriorityContext,
    ) -> PlayerAction {
        PassAgent.choose_action(p, s, pr)
    }
    fn choose_attackers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        Vec::new()
    }
    fn choose_blockers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[CardId],
        _: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        Vec::new()
    }
    fn choose_target_player(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        None
    }
    fn choose_target_card(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> Option<CardId> {
        None
    }
    fn choose_target_any(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        TargetChoice::None
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
    fn choose_mana_from_pool(&mut self, _: DecisionContext<'_>, _: PlayerId, _: &[Mana]) -> usize {
        self.mana_asked.set(self.mana_asked.get() + 1);
        0
    }
    #[allow(clippy::too_many_arguments)]
    fn pay_mana_cost(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: CardId,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: bool,
        _: &[CardId],
        _: &[ManaAbilityOption],
        _: &[CardId],
        _: &[CardId],
        _: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
    #[allow(clippy::too_many_arguments)]
    fn pay_cost_to_prevent_effect(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &str,
        _: &str,
        _: Option<CardId>,
        _: Option<manabrew_engine::ability::api_type::ApiType>,
        _: bool,
        _: &[GameEntity],
        _: &str,
    ) -> bool {
        self.prevent_asked.set(self.prevent_asked.get() + 1);
        true
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

struct Outcome {
    payer_permanents_tapped: Vec<bool>,
    payer_life: i32,
}

fn resolve_unless(
    unless_cost: &str,
    x_paid: u32,
    restricted_floating_mana: bool,
    harness_mirror: bool,
) -> Outcome {
    resolve_unless_with(
        &[FOREST],
        unless_cost,
        x_paid,
        restricted_floating_mana,
        harness_mirror,
    )
}

fn resolve_unless_with_asked(
    payer_permanents: &[&str],
    unless_cost: &str,
    x_paid: u32,
    restricted_floating_mana: bool,
    harness_mirror: bool,
) -> (Outcome, usize) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.mirror_forge_bugs = harness_mirror;
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    let permanents: Vec<CardId> = payer_permanents
        .iter()
        .map(|script| put(&mut game, script, p1, ZoneType::Battlefield))
        .collect();
    let source = put(&mut game, EFFECT_SOURCE, p0, ZoneType::Command);
    let mut sa = SpellAbility::new_simple(
        Some(source),
        p0,
        &format!(
            "DB$ LoseLife | Defined$ Opponent | LifeAmount$ 3 | UnlessCost$ {unless_cost} | UnlessPayer$ Opponent"
        ),
    );
    sa.x_mana_cost_paid = x_paid;
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut game_loop = GameLoop::new(2);
    if restricted_floating_mana {
        let mut mana = Mana::simple(ManaAtom::BLUE);
        mana.restriction = Some("Spell.Instant,Spell.Sorcery".to_string());
        game_loop.mana_pools[1].add_mana(mana);
    }
    let prevent_asked = Rc::new(Cell::new(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(PassAgent),
        Box::new(WillingPayer {
            mana_asked: Rc::new(Cell::new(0)),
            prevent_asked: Rc::clone(&prevent_asked),
        }),
    ];
    game_loop.step_with_priority(&mut game, &mut agents, false);
    (
        Outcome {
            payer_permanents_tapped: permanents
                .iter()
                .map(|&card| game.card(card).tapped)
                .collect(),
            payer_life: game.player(p1).life,
        },
        prevent_asked.get(),
    )
}

fn resolve_unless_with(
    payer_permanents: &[&str],
    unless_cost: &str,
    x_paid: u32,
    restricted_floating_mana: bool,
    harness_mirror: bool,
) -> Outcome {
    resolve_unless_with_asked(
        payer_permanents,
        unless_cost,
        x_paid,
        restricted_floating_mana,
        harness_mirror,
    )
    .0
}

#[test]
fn an_unpayable_unless_cost_is_attempted_and_keeps_its_taps_only_under_the_harness_mirror() {
    let mirrored = resolve_unless("2", 0, true, true);
    assert_eq!(mirrored.payer_life, 17);
    assert_eq!(mirrored.payer_permanents_tapped, vec![true]);
    let rules = resolve_unless("2", 0, true, false);
    assert_eq!(rules.payer_life, 17);
    assert_eq!(rules.payer_permanents_tapped, vec![false]);
}

#[test]
fn an_unpayable_x_unless_cost_taps_nothing_outside_the_harness_mirror() {
    let mirrored = resolve_unless("X", 2, true, true);
    assert_eq!(mirrored.payer_life, 17);
    assert_eq!(mirrored.payer_permanents_tapped, vec![true]);
    let rules = resolve_unless("X", 2, true, false);
    assert_eq!(rules.payer_life, 17);
    assert_eq!(rules.payer_permanents_tapped, vec![false]);
}

#[test]
fn the_harness_mirror_does_not_attempt_an_unless_cost_only_restricted_mana_could_pay() {
    for harness_mirror in [true, false] {
        let outcome = resolve_unless_with(&[TABLET], "2", 0, false, harness_mirror);
        assert_eq!(outcome.payer_life, 17);
        assert_eq!(outcome.payer_permanents_tapped, vec![false]);
    }
}

#[test]
fn the_harness_unless_probe_cannot_spend_restricted_mana_for_a_resolving_spell() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.mirror_forge_bugs = true;
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    let instant = put(&mut game, TAX_INSTANT, p0, ZoneType::Stack);
    let mut tax = SpellAbility::new_simple(
        Some(instant),
        p0,
        "SP$ LoseLife | Defined$ Opponent | LifeAmount$ 3 | UnlessCost$ 1 | UnlessPayer$ Opponent",
    );
    tax.is_spell = true;
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: tax,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: Some(ZoneType::Hand),
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut game_loop = GameLoop::new(2);
    for color in [ManaAtom::BLUE, ManaAtom::RED] {
        let mut restricted = Mana::simple(color);
        restricted.restriction = Some("Spell.Instant,Spell.Sorcery".to_string());
        game_loop.mana_pools[1].add_mana(restricted);
    }
    let asked = Rc::new(Cell::new(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(PassAgent),
        Box::new(WillingPayer {
            mana_asked: Rc::clone(&asked),
            prevent_asked: Rc::new(Cell::new(0)),
        }),
    ];
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.player(p1).life, 17);
    assert_eq!(asked.get(), 0);
}

#[test]
fn the_harness_mirror_does_not_attempt_a_waterbend_unless_cost_with_no_mana_to_pay_it() {
    for harness_mirror in [true, false] {
        // A Waterbend unless cost is checked under ComputerUtilMana as mana only
        // (no tap-creatures-for-{1} mechanic), so an untapped Bear sitting beside it
        // must not let the harness mirror attempt the cost.
        let (outcome, asked) =
            resolve_unless_with_asked(&[BEAR], "Waterbend<2>", 0, false, harness_mirror);
        assert_eq!(outcome.payer_life, 17);
        assert_eq!(outcome.payer_permanents_tapped, vec![false]);
        assert_eq!(asked, 0, "the harness must not even ask when it cannot pay");
    }
}

#[test]
fn the_harness_mirror_attempts_a_waterbend_unless_cost_payable_from_untapped_lands() {
    // Waterbend<2> paid with 2 untapped Islands' mana, not floating pool mana: the
    // pre-check has to use available mana (land taps included), not the bare pool.
    let (outcome, asked) =
        resolve_unless_with_asked(&[FOREST, FOREST], "Waterbend<2>", 0, false, true);
    assert_eq!(
        asked, 1,
        "the harness must attempt when untapped lands can pay it"
    );
    assert_eq!(outcome.payer_life, 20);
}

#[test]
fn the_harness_mirror_attempts_a_waterbend_unless_cost_payable_from_floating_mana() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.mirror_forge_bugs = true;
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    let source = put(&mut game, EFFECT_SOURCE, p0, ZoneType::Command);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(
            Some(source),
            p0,
            "DB$ LoseLife | Defined$ Opponent | LifeAmount$ 3 | UnlessCost$ Waterbend<2> | UnlessPayer$ Opponent",
        ),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut game_loop = GameLoop::new(2);
    game_loop.mana_pools[1].add(ManaAtom::COLORLESS, 2);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(PassAgent),
        Box::new(WillingPayer {
            mana_asked: Rc::new(Cell::new(0)),
            prevent_asked: Rc::new(Cell::new(0)),
        }),
    ];
    game_loop.step_with_priority(&mut game, &mut agents, false);
    assert_eq!(game.player(p1).life, 20);
}

#[test]
fn the_harness_mirror_does_not_count_the_unless_ability_host_toward_a_waterbend_cost() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p1 = PlayerId(1);
    game.turn.active_player = p1;
    game.new_turn_for_player(p1);
    game.turn.phase = PhaseType::Main1;
    game.mirror_forge_bugs = true;
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    let host = put(&mut game, FOREST, p1, ZoneType::Battlefield);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(
            Some(host),
            p1,
            "DB$ LoseLife | Defined$ You | LifeAmount$ 3 | UnlessCost$ Waterbend<1> | UnlessPayer$ You",
        ),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let prevent_asked = Rc::new(Cell::new(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(PassAgent),
        Box::new(WillingPayer {
            mana_asked: Rc::new(Cell::new(0)),
            prevent_asked: Rc::clone(&prevent_asked),
        }),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert_eq!(prevent_asked.get(), 0);
    assert_eq!(game.player(p1).life, 17);
    assert!(!game.card(host).tapped);
}

#[test]
fn an_unless_cost_only_restricted_battlefield_mana_could_pay_is_not_offered() {
    let (outcome, asked) = resolve_unless_with_asked(&[TABLET], "2", 0, false, false);
    assert_eq!(asked, 0);
    assert_eq!(outcome.payer_life, 17);
    assert_eq!(outcome.payer_permanents_tapped, vec![false]);
}

#[test]
fn an_unless_cost_only_restricted_floating_mana_could_pay_is_not_offered() {
    let (outcome, asked) = resolve_unless_with_asked(&[], "1", 0, true, false);
    assert_eq!(asked, 0);
    assert_eq!(outcome.payer_life, 17);
}

#[test]
fn a_payable_unless_cost_is_still_offered_and_paid() {
    let (outcome, asked) = resolve_unless_with_asked(&[FOREST], "1", 0, false, false);
    assert_eq!(asked, 1);
    assert_eq!(outcome.payer_life, 20);
    assert_eq!(outcome.payer_permanents_tapped, vec![true]);
}
