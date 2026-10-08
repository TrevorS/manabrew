use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, GameEntity, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const ONCE_WARDEN: &str = "Name:Once Warden\nManaCost:1 W\nTypes:Creature Human\nPT:2/2\nT:Mode$ DamageDoneOnce | ValidTarget$ Creature.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | OptionalDecider$ You | ResolvedLimit$ 1 | TriggerDescription$ Whenever a creature you control is dealt damage, you may gain 1 life. If you do, CARDNAME deals 1 damage to itself. Do this only once each turn.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You | SubAbility$ DBHurt\nSVar:DBHurt:DB$ DealDamage | NumDmg$ 1 | Defined$ Self\nOracle:";
const COUNTING_WARDEN: &str = "Name:Counting Warden\nManaCost:1 W\nTypes:Creature Human\nPT:2/2\nT:Mode$ DamageDoneOnce | ValidTarget$ Creature.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever a creature you control is dealt damage, you gain 1 life if this is the first time this ability has resolved this turn. If it's the second time, you gain 2 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You | ConditionCheckSVar$ X | ConditionSVarCompare$ EQ1 | SubAbility$ DBGainMore\nSVar:DBGainMore:DB$ GainLife | LifeAmount$ 2 | Defined$ You | ConditionCheckSVar$ X | ConditionSVarCompare$ EQ2\nSVar:X:Count$ResolvedThisTurn\nOracle:";
const TAPPED_WARDEN: &str = "Name:Tapped Warden\nManaCost:1 W\nTypes:Creature Human\nPT:2/2\nT:Mode$ DamageDoneOnce | ValidTarget$ Creature.YouCtrl | TriggerZones$ Battlefield | PresentDefined$ Self | IsPresent$ Card.tapped | Execute$ TrigGain | TriggerDescription$ Whenever a creature you control is dealt damage, if CARDNAME is tapped, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 1 | Defined$ You\nOracle:";
const HILL_GIANT: &str = "Name:Hill Giant\nManaCost:3 R\nTypes:Creature Giant\nPT:3/3\nOracle:";
const DAMAGE_SOURCE: &str = "Name:Damage Source\nManaCost:no cost\nTypes:Artifact\nOracle:";

struct TargetOpponent;

impl PlayerAgent for TargetOpponent {
    fn choose_targets_for(
        &mut self,
        sa: &mut SpellAbility,
        game: &GameState,
        pools: &[ManaPool],
    ) -> bool {
        manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, pools)
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
        p: PlayerId,
        valid: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        valid.iter().copied().find(|&v| v != p)
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
        p: PlayerId,
        players: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        match players.iter().copied().find(|&v| v != p) {
            Some(v) => TargetChoice::Player(v),
            None => TargetChoice::None,
        }
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
        false
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn damage_all_with_warden(companion: &str, damage: u32) -> (GameState, CardId) {
    damage_all_with(ONCE_WARDEN, companion, damage)
}

fn damage_all_with(warden_script: &str, companion: &str, damage: u32) -> (GameState, CardId) {
    damage_all_with_host(warden_script, companion, damage, false)
}

fn damage_all_with_host(
    warden_script: &str,
    companion: &str,
    damage: u32,
    tapped: bool,
) -> (GameState, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    game.turn.active_player = p1;
    game.new_turn_for_player(p1);
    game.turn.phase = PhaseType::Main1;
    let warden = put(&mut game, warden_script, p0, ZoneType::Battlefield);
    game.card_mut(warden).tapped = tapped;
    put(&mut game, companion, p0, ZoneType::Battlefield);
    put(&mut game, companion, p0, ZoneType::Battlefield);
    let source = put(&mut game, DAMAGE_SOURCE, p1, ZoneType::Command);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(
            Some(source),
            p1,
            &format!("DB$ DamageAll | ValidCards$ Creature | NumDmg$ {damage}"),
        ),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    let mut agents: Vec<Box<dyn PlayerAgent>> =
        vec![Box::new(TargetOpponent), Box::new(TargetOpponent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    assert!(game.stack.is_empty());
    (game, warden)
}

#[test]
fn a_resolved_limit_still_holds_after_the_trigger_killed_its_host() {
    let (game, warden) = damage_all_with_warden(BEARS, 1);
    assert_eq!(game.card(warden).zone, ZoneType::Graveyard);
    assert_eq!(game.player(PlayerId(0)).life, 21);
}

#[test]
fn a_resolved_limit_holds_when_the_host_died_before_any_trigger_resolved() {
    let (game, warden) = damage_all_with_warden(HILL_GIANT, 2);
    assert_eq!(game.card(warden).zone, ZoneType::Graveyard);
    assert_eq!(game.player(PlayerId(0)).life, 21);
}

#[test]
fn resolved_this_turn_counts_the_resolutions_of_a_host_that_died_before_they_resolved() {
    let (game, warden) = damage_all_with(COUNTING_WARDEN, HILL_GIANT, 2);
    assert_eq!(game.card(warden).zone, ZoneType::Graveyard);
    assert_eq!(game.player(PlayerId(0)).life, 23);
}

#[test]
fn a_tapped_host_that_died_before_its_trigger_resolved_still_meets_its_self_condition() {
    let (game, warden) = damage_all_with_host(TAPPED_WARDEN, HILL_GIANT, 2, true);
    assert_eq!(game.card(warden).zone, ZoneType::Graveyard);
    assert_eq!(game.player(PlayerId(0)).life, 23);
}
