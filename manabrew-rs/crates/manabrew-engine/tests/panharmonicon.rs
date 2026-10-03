use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayCardMode, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{AlternativeCost, SpellAbility};
use rand::SeedableRng;

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const WARP_SCOUT: &str =
    "Name:Warp Scout\nManaCost:2 U\nTypes:Creature Human Scout\nPT:1/1\nK:Warp:U\nOracle:";
const DELNEY: &str = "Name:Delney, Streetwise Lookout\nManaCost:2 W\nTypes:Legendary Creature Human Scout\nPT:2/2\nS:Mode$ Panharmonicon | ValidCard$ Creature.YouCtrl+powerLE2 | Description$ If an ability of a creature you control with power 2 or less triggers, that ability triggers an additional time.\nOracle:";
const PANHARMONICON: &str = "Name:Panharmonicon\nManaCost:4\nTypes:Artifact\nS:Mode$ Panharmonicon | ValidMode$ ChangesZone,ChangesZoneAll | ValidCard$ Permanent.YouCtrl | ValidCause$ Artifact,Creature | Destination$ Battlefield | Description$ If an artifact or creature entering causes a triggered ability of a permanent you control to trigger, that ability triggers an additional time.\nOracle:";
const STATIC_BEACON: &str = "Name:Static Beacon\nManaCost:1\nTypes:Artifact\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature | Static$ True | Execute$ TrigGainLife | TriggerDescription$ Whenever a creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const BEACON: &str = "Name:Beacon\nManaCost:1\nTypes:Artifact\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature | Execute$ TrigGainLife | TriggerDescription$ Whenever a creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    end_step_triggers: usize,
}

struct CastOnce {
    seen: Rc<RefCell<Seen>>,
    mode: PlayCardMode,
}

impl PlayerAgent for CastOnce {
    fn choose_targets_for(
        &mut self,
        sa: &mut SpellAbility,
        game: &GameState,
        pools: &[ManaPool],
    ) -> bool {
        PassAgent.choose_targets_for(sa, game, pools)
    }
    fn mulligan_decision(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        hand: &[CardId],
        count: u32,
    ) -> bool {
        PassAgent.mulligan_decision(context, player, hand, count)
    }
    fn choose_action(
        &mut self,
        player: PlayerId,
        space: Option<&PriorityActionSpace>,
        priority: &mut dyn PriorityContext,
    ) -> PlayerAction {
        {
            let game = priority.context().game;
            if game.turn.phase == PhaseType::EndOfTurn {
                let triggers = game
                    .stack
                    .iter()
                    .filter(|entry| entry.spell_ability.is_trigger)
                    .count();
                let mut seen = self.seen.borrow_mut();
                seen.end_step_triggers = seen.end_step_triggers.max(triggers);
            }
        }
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let play = space
            .playable
            .iter()
            .find(|play| play.mode == self.mode)
            .copied();
        match play {
            Some(play) if !self.seen.borrow().cast => {
                self.seen.borrow_mut().cast = true;
                PlayerAction::CastSpell(play)
            }
            _ => PassAgent.choose_action(player, Some(space), priority),
        }
    }
    fn choose_attackers(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _available: &[CardId],
        _defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        Vec::new()
    }
    fn choose_blockers(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _attackers: &[CardId],
        _blockers: &[CardId],
        _max: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        Vec::new()
    }
    fn choose_target_player(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[PlayerId],
        sa: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        PassAgent.choose_target_player(context, player, valid, sa)
    }
    fn choose_target_card(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        PassAgent.choose_target_card(context, player, valid, sa)
    }
    fn choose_target_any(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        players: &[PlayerId],
        cards: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        PassAgent.choose_target_any(context, player, players, cards, sa)
    }
    fn pay_mana_cost(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _card_id: CardId,
        _card_name: &str,
        _mana_cost: &str,
        _mana_cost_display: &str,
        _mana_cost_checkpoint: &str,
        _can_confirm_from_pool: bool,
        _allow_reserved_source_reuse: bool,
        _reserved_sacrifices: &[CardId],
        _mana_ability_options: &[ManaAbilityOption],
        _tappable_lands: &[CardId],
        _untappable_lands: &[CardId],
        _mana_pool: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        PassAgent.choose_land_or_spell(context, player)
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn warp_a_scout(with_delney: bool) -> (usize, ZoneType, bool) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let scout = put(&mut game, WARP_SCOUT, p0, ZoneType::Hand);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    if with_delney {
        put(&mut game, DELNEY, p0, ZoneType::Battlefield);
    }
    for player in [p0, p1] {
        for _ in 0..3 {
            put(&mut game, ISLAND, player, ZoneType::Library);
        }
    }
    game.turn.active_player = p0;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            seen: Rc::clone(&seen),
            mode: PlayCardMode::Alternative(AlternativeCost::Warp),
        }),
        Box::new(PassAgent),
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    GameLoop::new(2).run_turn(&mut game, &mut agents, &mut rng);
    assert!(seen.borrow().cast);
    let triggers = seen.borrow().end_step_triggers;
    (triggers, game.card(scout).zone, game.card(scout).warped)
}

#[test]
fn a_warped_creature_is_exiled_at_end_step() {
    assert_eq!(warp_a_scout(false), (1, ZoneType::Exile, true));
}

#[test]
fn delney_does_not_double_the_warp_exile_trigger() {
    assert_eq!(warp_a_scout(true), (1, ZoneType::Exile, true));
}

fn bears_enter_beside(trigger_host: &str) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, BEARS, p0, ZoneType::Hand);
    put(&mut game, FOREST, p0, ZoneType::Battlefield);
    put(&mut game, FOREST, p0, ZoneType::Battlefield);
    put(&mut game, PANHARMONICON, p0, ZoneType::Battlefield);
    put(&mut game, trigger_host, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            seen: Rc::clone(&seen),
            mode: PlayCardMode::Normal,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert!(seen.borrow().cast);
    game.players[0].life
}

#[test]
fn panharmonicon_doubles_an_enters_trigger() {
    assert_eq!(bears_enter_beside(BEACON), 22);
}

#[test]
fn panharmonicon_does_not_double_a_static_trigger() {
    assert_eq!(bears_enter_beside(STATIC_BEACON), 21);
}
