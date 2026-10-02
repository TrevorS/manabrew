use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::card_factory_util::turn_face_down_with_state;
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const SPY: &str = "Name:Mistway Spy\nManaCost:U\nTypes:Creature Merfolk Detective\nPT:1/1\nK:Flying\nK:Disguise:1 U\nOracle:";
const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const PRODIGAL: &str = "Name:Prodigal Sorcerer\nManaCost:2 U\nTypes:Creature Human Wizard\nPT:1/1\nA:AB$ DealDamage | Cost$ T | ValidTgts$ Any | NumDmg$ 1 | SpellDescription$ CARDNAME deals 1 damage to any target.\nOracle:";
const SUDDEN_SHOCK: &str = "Name:Sudden Shock\nManaCost:1 R\nTypes:Instant\nK:Split second\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";

struct Recorder {
    seen: Rc<RefCell<Option<Vec<CardId>>>>,
    turn_up: CardId,
}

impl PlayerAgent for Recorder {
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
        _player: PlayerId,
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
        let first = self.seen.borrow().is_none();
        self.seen
            .borrow_mut()
            .get_or_insert_with(|| space.activatable.iter().map(|a| a.card_id).collect());
        match space.activatable.iter().find(|a| a.card_id == self.turn_up) {
            Some(action) if first => PlayerAction::ActivateAbility(AbilityRef {
                card_id: action.card_id,
                ability_index: action.ability_index,
            }),
            _ => PlayerAction::PassPriority,
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
    game.card_mut(card).summoning_sick = false;
    card
}

fn offered(split_second: bool) -> (Vec<CardId>, CardId, CardId, bool) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let spy = put(&mut game, SPY, p0, ZoneType::Battlefield);
    turn_face_down_with_state(game.card_mut(spy));
    let sorcerer = put(&mut game, PRODIGAL, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    if split_second {
        let shock = put(&mut game, SUDDEN_SHOCK, p1, ZoneType::Stack);
        let mut sa = SpellAbility::new_simple(
            Some(shock),
            p1,
            "SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2",
        );
        sa.is_spell = true;
        sa.target_chosen.target_player = Some(p1);
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
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: spy,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let seen = seen.borrow().clone().expect("a priority decision");
    (seen, spy, sorcerer, game.card(spy).face_down)
}

#[test]
fn a_disguised_creature_turns_face_up_under_split_second() {
    let (seen, spy, sorcerer, face_down) = offered(true);

    assert!(seen.contains(&spy));
    assert!(!seen.contains(&sorcerer));
    assert!(!face_down);
}

#[test]
fn without_split_second_both_abilities_are_offered() {
    let (seen, spy, sorcerer, _) = offered(false);

    assert!(seen.contains(&spy));
    assert!(seen.contains(&sorcerer));
}
