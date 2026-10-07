use std::cell::RefCell;
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
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;

const SHOCK: &str = "Name:Shock\nManaCost:R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";
const MOUNTAIN: &str = "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const WARDING: &str = "Name:Warding Banner\nManaCost:1 W\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddKeyword$ Ward:1 | Description$ Creatures you control have ward {1}.\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const TORPOR_ORB: &str = "Name:Torpor Orb\nManaCost:2\nTypes:Artifact\nS:Mode$ DisableTriggers | ValidCause$ Creature | ValidMode$ ChangesZone,ChangesZoneAll | Destination$ Battlefield | Description$ Creatures entering don't cause abilities to trigger.\nOracle:";
const ELESH_NORN: &str = "Name:Elesh Norn, Mother of Machines\nManaCost:4 W\nTypes:Legendary Creature Phyrexian Praetor\nPT:4/7\nK:Vigilance\nS:Mode$ DisableTriggers | ValidCause$ Permanent | ValidMode$ ChangesZone,ChangesZoneAll | Destination$ Battlefield | ValidCard$ Permanent.OppCtrl+inZoneBattlefield | Description$ Permanents entering don't cause abilities of permanents your opponents control to trigger.\nOracle:";
const LOOKOUT: &str = "Name:Lookout Totem\nManaCost:1\nTypes:Artifact\nA:AB$ DelayedTrigger | Cost$ T | Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Creature | ThisTurn$ True | Execute$ TrigGainLife | SpellDescription$ When a creature enters this turn, you gain 2 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 2\nOracle:";
const BEACON: &str = "Name:Static Beacon\nManaCost:1\nTypes:Artifact\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature | Static$ True | Execute$ TrigGainLife | TriggerDescription$ Whenever a creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const NOWHERE: &str = "Name:Nowhere to Run\nManaCost:1 B\nTypes:Enchantment\nS:Mode$ IgnoreHexproof | ValidEntity$ Creature.OppCtrl | Description$ Creatures your opponents control can be the targets of spells and abilities as though they didn't have hexproof. Ward abilities of those creatures don't trigger.\nS:Mode$ DisableTriggers | Secondary$ True | ValidTrigger$ Triggered.Ward | ValidCard$ Creature.OppCtrl+inZoneBattlefield | Description$ Ward abilities of those creatures don't trigger.\nOracle:";

#[derive(Default)]
struct Seen {
    activated: bool,
    cast: bool,
    ward_prompts: usize,
}

struct ActivateThenCast(Rc<RefCell<Seen>>);

impl PlayerAgent for ActivateThenCast {
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
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let mut seen = self.0.borrow_mut();
        if !seen.activated {
            if let Some(ability) = space.activatable.first() {
                seen.activated = true;
                return PlayerAction::ActivateAbility(AbilityRef {
                    card_id: ability.card_id,
                    ability_index: ability.ability_index,
                });
            }
        }
        match space.playable.first() {
            Some(&play) if !seen.cast => {
                seen.cast = true;
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
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _valid: &[PlayerId],
        _sa: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        None
    }
    fn choose_target_card(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        valid
            .iter()
            .copied()
            .find(|&cid| context.game.card(cid).controller != player)
    }
    fn choose_target_any(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        _players: &[PlayerId],
        cards: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        match self.choose_target_card(context, player, cards, sa) {
            Some(card) => TargetChoice::Card(card),
            None => TargetChoice::None,
        }
    }
    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        PassAgent.choose_land_or_spell(context, player)
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
    fn pay_cost_to_prevent_effect(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _cost_kind: &str,
        _message: &str,
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
        _can_pay: bool,
        _targets: &[GameEntity],
        _effect_text: &str,
    ) -> bool {
        self.0.borrow_mut().ward_prompts += 1;
        false
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn shock_a_warded_bear(nowhere_to_run: bool) -> (usize, ZoneType) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, SHOCK, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    if nowhere_to_run {
        put(&mut game, NOWHERE, p0, ZoneType::Battlefield);
    }
    put(&mut game, WARDING, p1, ZoneType::Battlefield);
    let bears = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(ActivateThenCast(Rc::clone(&seen))),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let prompts = seen.borrow().ward_prompts;
    (prompts, game.card(bears).zone)
}

#[test]
fn a_granted_ward_counters_an_unpaid_spell() {
    let (prompts, bears) = shock_a_warded_bear(false);

    assert_eq!(prompts, 1);
    assert_eq!(bears, ZoneType::Battlefield);
}

#[test]
fn nowhere_to_run_stops_a_granted_ward_from_triggering() {
    let (prompts, bears) = shock_a_warded_bear(true);

    assert_eq!(prompts, 0);
    assert_eq!(bears, ZoneType::Graveyard);
}

fn bears_enter(p0_cards: &[&str], p1_cards: &[&str]) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, BEARS, p0, ZoneType::Hand);
    put(&mut game, FOREST, p0, ZoneType::Battlefield);
    put(&mut game, FOREST, p0, ZoneType::Battlefield);
    for script in p0_cards {
        put(&mut game, script, p0, ZoneType::Battlefield);
    }
    for script in p1_cards {
        put(&mut game, script, p1, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(ActivateThenCast(Rc::clone(&seen))),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert!(seen.borrow().cast);
    game.players[0].life
}

#[test]
fn a_delayed_enters_trigger_fires_without_a_disabling_static() {
    assert_eq!(bears_enter(&[LOOKOUT], &[]), 22);
}

#[test]
fn torpor_orb_stops_a_delayed_enters_trigger() {
    assert_eq!(bears_enter(&[LOOKOUT], &[TORPOR_ORB]), 20);
}

#[test]
fn elesh_norn_does_not_stop_a_delayed_trigger_of_an_opponents_permanent() {
    assert_eq!(bears_enter(&[LOOKOUT], &[ELESH_NORN]), 22);
}

#[test]
fn torpor_orb_does_not_stop_a_static_trigger() {
    assert_eq!(bears_enter(&[BEACON], &[TORPOR_ORB]), 21);
}

const TWIN_HEXER: &str = "Name:Twin Hexer\nManaCost:2\nTypes:Creature Human\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigDestroy | TriggerDescription$ When this enters, destroy target artifact an opponent controls.\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigDestroy | Secondary$ True | TriggerDescription$ When this enters, destroy target artifact an opponent controls.\nSVar:TrigDestroy:DB$ Destroy | ValidTgts$ Artifact.OppCtrl | TgtPrompt$ Select target artifact an opponent controls\nOracle:";
const WARDED_RELIC: &str = "Name:Warded Relic\nManaCost:2\nTypes:Artifact\nK:Ward:2\nOracle:";

#[test]
fn each_ward_trigger_counters_the_ability_that_targeted_it() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, TWIN_HEXER, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let relic = put(&mut game, WARDED_RELIC, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(ActivateThenCast(Rc::clone(&seen))),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(seen.borrow().cast);
    assert_eq!(game.card(relic).zone, ZoneType::Battlefield);
}
