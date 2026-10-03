use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::ability::api_type::ApiType;
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
use manabrew_engine::spellability::SpellAbility;

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const EXPLOITER: &str = "Name:Exploiting Scholar\nManaCost:3 U\nTypes:Creature Human Wizard\nPT:2/2\nK:Exploit\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigGainLife | TriggerDescription$ When this creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const CLONE: &str = "Name:Clone\nManaCost:3 U\nTypes:Creature Shapeshifter\nPT:0/0\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | SpellDescription$ You may have CARDNAME enter as a copy of any creature on the battlefield.\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    top_of_both: Option<Option<ApiType>>,
}

struct CastOnce {
    seen: Rc<RefCell<Seen>>,
    card: CardId,
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
    fn confirm_action(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _mode: Option<&str>,
        _message: &str,
        _options: &[String],
        _source: Option<CardId>,
        _api: Option<ApiType>,
    ) -> bool {
        true
    }
    fn choose_action(
        &mut self,
        player: PlayerId,
        space: Option<&PriorityActionSpace>,
        priority: &mut dyn PriorityContext,
    ) -> PlayerAction {
        {
            let game = priority.context().game;
            let apis: Vec<_> = game
                .stack
                .iter()
                .map(|entry| entry.spell_ability.api)
                .collect();
            let mut seen = self.seen.borrow_mut();
            if seen.top_of_both.is_none()
                && apis.contains(&Some(ApiType::Sacrifice))
                && apis.contains(&Some(ApiType::GainLife))
            {
                seen.top_of_both =
                    Some(game.stack.peek().and_then(|entry| entry.spell_ability.api));
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
            .find(|play| play.card_id == self.card && play.mode == PlayCardMode::Normal)
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

fn top_of_stack_when_both_trigger(cast: &str, on_battlefield: &[&str]) -> Option<ApiType> {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let card = put(&mut game, cast, p0, ZoneType::Hand);
    for _ in 0..6 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    for script in on_battlefield {
        put(&mut game, script, p0, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            seen: Rc::clone(&seen),
            card,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let seen = seen.borrow();
    assert!(seen.cast);
    seen.top_of_both.expect("both enters triggers on the stack")
}

#[test]
fn a_card_built_from_its_script_puts_its_keyword_trigger_on_top() {
    assert_eq!(
        top_of_stack_when_both_trigger(EXPLOITER, &[]),
        Some(ApiType::Sacrifice)
    );
}

#[test]
fn a_clone_puts_the_copied_printed_trigger_on_top() {
    assert_eq!(
        top_of_stack_when_both_trigger(CLONE, &[EXPLOITER]),
        Some(ApiType::GainLife)
    );
}
