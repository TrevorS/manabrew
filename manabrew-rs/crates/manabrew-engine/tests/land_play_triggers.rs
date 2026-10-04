use std::cell::Cell;
use std::rc::Rc;

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
use manabrew_engine::spellability::SpellAbility;

const TEMPLE_GARDEN: &str = "Name:Temple Garden\nManaCost:no cost\nTypes:Land Forest Plains\nR:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplaceWith$ DBTap | ReplacementResult$ Updated | Description$ As CARDNAME enters, you may pay 2 life. If you don't, it enters tapped.\nSVar:DBTap:DB$ Tap | ETB$ True | Defined$ Self | UnlessCost$ PayLife<2> | UnlessPayer$ You | StackDescription$ enters tapped.\nOracle:";
const LANDFALL_PINGER: &str = "Name:Landfall Pinger\nManaCost:1 R\nTypes:Creature Elemental\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | TriggerZones$ Battlefield | ValidCard$ Land.YouCtrl | Execute$ TrigLose | TriggerDescription$ Landfall — Whenever a land you control enters, target player loses 1 life.\nSVar:TrigLose:DB$ LoseLife | ValidTgts$ Player | LifeAmount$ 1\nOracle:";

struct LandPlayer {
    targeted: Rc<Cell<usize>>,
    played: bool,
}

impl PlayerAgent for LandPlayer {
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
        let requested;
        let space = match s {
            Some(space) => space,
            None => {
                requested = pr.action_space();
                &requested
            }
        };
        if !self.played {
            if let Some(&play) = space.playable.first() {
                self.played = true;
                return PlayerAction::CastSpell(play);
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
        p: PlayerId,
        valid: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        self.targeted.set(self.targeted.get() + 1);
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

#[test]
fn a_player_paying_life_to_zero_for_a_land_loses_before_landfall_targets_are_chosen() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.player_mut(p0).life = 2;
    put(&mut game, LANDFALL_PINGER, p0, ZoneType::Battlefield);
    let garden = put(&mut game, TEMPLE_GARDEN, p0, ZoneType::Hand);
    let targeted = Rc::new(Cell::new(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(LandPlayer {
            targeted: Rc::clone(&targeted),
            played: false,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(garden).zone, ZoneType::Battlefield);
    assert!(game.player(p0).has_lost);
    assert_eq!(targeted.get(), 0);
}
