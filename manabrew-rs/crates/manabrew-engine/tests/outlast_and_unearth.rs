use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::{CardInstance, CounterType};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use rand::SeedableRng;

const PLAINS: &str = "Name:Plains\nManaCost:no cost\nTypes:Basic Land Plains\nOracle:";
const SWAMP: &str = "Name:Swamp\nManaCost:no cost\nTypes:Basic Land Swamp\nOracle:";
const OUTLASTER: &str =
    "Name:Outlast Squire\nManaCost:1 W\nTypes:Creature Human Warrior\nPT:1/2\nK:Outlast:W\nOracle:";
const UNEARTHER: &str =
    "Name:Unearth Ghoul\nManaCost:2 B\nTypes:Creature Zombie\nPT:1/1\nK:Unearth:B\nOracle:";
const WITHERING: &str = "Name:Withering Field\nManaCost:2 B\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature | AddPower$ -1 | AddToughness$ -1 | Description$ Creatures get -1/-1.\nOracle:";

#[derive(Default)]
struct Seen {
    activated: bool,
    haste_on_battlefield: bool,
}

struct ActivateOnce {
    seen: Rc<RefCell<Seen>>,
    card: CardId,
}

impl PlayerAgent for ActivateOnce {
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
            let card = game.card(self.card);
            if card.zone == ZoneType::Battlefield && card.has_keyword("Haste") {
                self.seen.borrow_mut().haste_on_battlefield = true;
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
        let ability = space
            .activatable
            .iter()
            .find(|action| action.card_id == self.card && !action.is_mana_ability);
        match ability {
            Some(action) if !self.seen.borrow().activated => {
                self.seen.borrow_mut().activated = true;
                PlayerAction::ActivateAbility(AbilityRef {
                    card_id: action.card_id,
                    ability_index: action.ability_index,
                })
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

struct Turn {
    game: GameState,
    card: CardId,
    seen: Seen,
}

fn play_a_turn(card_script: &str, card_zone: ZoneType, others: &[(&str, ZoneType)]) -> Turn {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let card = put(&mut game, card_script, p0, card_zone);
    for &(script, zone) in others {
        put(&mut game, script, p0, zone);
    }
    for player in [p0, p1] {
        for _ in 0..3 {
            put(&mut game, PLAINS, player, ZoneType::Library);
        }
    }
    game.turn.active_player = p0;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(ActivateOnce {
            seen: Rc::clone(&seen),
            card,
        }),
        Box::new(PassAgent),
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    GameLoop::new(2).run_turn(&mut game, &mut agents, &mut rng);
    drop(agents);
    let seen = Rc::try_unwrap(seen)
        .ok()
        .expect("agent dropped")
        .into_inner();
    assert!(seen.activated);
    Turn { game, card, seen }
}

fn unearth_effects(game: &GameState) -> usize {
    game.cards
        .iter()
        .filter(|card| card.zone == ZoneType::Command && card.card_name == "Unearth Effect")
        .count()
}

#[test]
fn outlast_puts_a_counter_on_the_creature() {
    let turn = play_a_turn(
        OUTLASTER,
        ZoneType::Battlefield,
        &[(PLAINS, ZoneType::Battlefield)],
    );
    let creature = turn.game.card(turn.card);
    assert_eq!(creature.counter_count(&CounterType::P1P1), 1);
}

#[test]
fn an_unearthed_creature_has_haste_and_is_exiled_at_the_end_step() {
    let turn = play_a_turn(
        UNEARTHER,
        ZoneType::Graveyard,
        &[
            (SWAMP, ZoneType::Battlefield),
            (SWAMP, ZoneType::Battlefield),
        ],
    );
    assert!(turn.seen.haste_on_battlefield);
    assert_eq!(turn.game.card(turn.card).zone, ZoneType::Exile);
    assert_eq!(unearth_effects(&turn.game), 0);
}

#[test]
fn an_unearthed_creature_that_would_die_is_exiled_instead() {
    let turn = play_a_turn(
        UNEARTHER,
        ZoneType::Graveyard,
        &[
            (SWAMP, ZoneType::Battlefield),
            (WITHERING, ZoneType::Battlefield),
        ],
    );
    assert_eq!(turn.game.card(turn.card).zone, ZoneType::Exile);
    assert_eq!(unearth_effects(&turn.game), 0);
}
