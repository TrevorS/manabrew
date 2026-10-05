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
const EFFECT_SOURCE: &str = "Name:Effect Source\nManaCost:no cost\nTypes:Artifact\nOracle:";

struct WillingPayer;

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

fn resolve_unless_with(
    payer_permanents: &[&str],
    unless_cost: &str,
    x_paid: u32,
    restricted_floating_mana: bool,
    harness_mirror: bool,
) -> Outcome {
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
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(WillingPayer)];
    game_loop.step_with_priority(&mut game, &mut agents, false);
    Outcome {
        payer_permanents_tapped: permanents
            .iter()
            .map(|&card| game.card(card).tapped)
            .collect(),
        payer_life: game.player(p1).life,
    }
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
