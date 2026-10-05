use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;

const THIRD_TIME_SEEKER: &str = "Name:Third Time Seeker\nManaCost:R\nTypes:Creature Human\nPT:1/1\nA:AB$ GainLife | Cost$ 0 | LifeAmount$ 1 | Defined$ You | SubAbility$ DBDestroy | SpellDescription$ You gain 1 life. If this is the third time this ability has resolved this turn, destroy CARDNAME and you gain 10 life.\nSVar:DBDestroy:DB$ Destroy | Defined$ Self | ConditionCheckSVar$ X | ConditionSVarCompare$ EQ3 | SubAbility$ DBBonus\nSVar:DBBonus:DB$ GainLife | LifeAmount$ 10 | Defined$ You | ConditionCheckSVar$ X | ConditionSVarCompare$ EQ3\nSVar:X:Count$ResolvedThisTurn\nOracle:";

struct Activator {
    activations: usize,
}

impl PlayerAgent for Activator {
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
        let requested;
        let space = match s {
            Some(space) => space,
            None => {
                requested = pr.action_space();
                &requested
            }
        };
        if self.activations < 3 && pr.context().game.stack.is_empty() {
            if let Some(ability) = space.activatable.first() {
                self.activations += 1;
                return PlayerAction::ActivateAbility(AbilityRef {
                    card_id: ability.card_id,
                    ability_index: ability.ability_index,
                });
            }
        }
        PassAgent.choose_action(p, Some(space), pr)
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
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn an_ability_counts_its_resolutions_on_the_object_it_was_activated_from() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seeker = put(&mut game, THIRD_TIME_SEEKER, p0, ZoneType::Battlefield);
    let mut agents: Vec<Box<dyn PlayerAgent>> =
        vec![Box::new(Activator { activations: 0 }), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    for _ in 0..3 {
        game_loop.step_with_priority(&mut game, &mut agents, true);
    }
    assert_eq!(game.card(seeker).zone, ZoneType::Graveyard);
    assert_eq!(game.player(p0).life, 33);
}
