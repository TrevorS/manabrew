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
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const SOUL_CAVE: &str = "Name:Soul Cave\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ G | AddsNoCounter$ True | SpellDescription$ Add {G}. That spell can't be countered.\nOracle:";
const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const LEAF_PUP: &str = "Name:Leaf Pup\nManaCost:G\nTypes:Creature Dog\nPT:1/1\nOracle:";
const SENSE_COUNTER: &str = "Name:Sense Counter\nManaCost:1 U\nTypes:Instant\nA:SP$ Counter | TargetType$ Instant,Sorcery,Triggered | ValidTgts$ Card,Emblem | TgtPrompt$ Select target spell or ability | SpellDescription$ Counter target instant spell, sorcery spell, or triggered ability.\nOracle:";
const QUICK_DENIAL: &str = "Name:Quick Denial\nManaCost:U\nTypes:Instant\nA:SP$ Counter | TargetType$ Spell | ValidTgts$ Card | TgtPrompt$ Select target spell | SpellDescription$ Counter target spell.\nOracle:";

struct CastOnce {
    spell: CardId,
    cast: bool,
}

impl PlayerAgent for CastOnce {
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
        let play = space
            .playable
            .iter()
            .find(|play| play.card_id == self.spell)
            .copied();
        match play {
            Some(play) if !self.cast => {
                self.cast = true;
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

#[test]
fn a_spell_paid_with_no_counter_mana_tapped_during_the_payment_cannot_be_countered() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let pup = put(&mut game, LEAF_PUP, p0, ZoneType::Hand);
    put(&mut game, SOUL_CAVE, p0, ZoneType::Battlefield);
    let denial = put(&mut game, QUICK_DENIAL, p1, ZoneType::Hand);
    put(&mut game, ISLAND, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: pup,
            cast: false,
        }),
        Box::new(CastOnce {
            spell: denial,
            cast: false,
        }),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(denial).zone, ZoneType::Graveyard);
    assert_eq!(game.card(pup).zone, ZoneType::Battlefield);
    assert!(game.cards_in_zone(ZoneType::Command, p0).is_empty());
}

fn stack_entry(spell_ability: SpellAbility) -> StackEntry {
    StackEntry {
        id: 0,
        spell_ability,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    }
}

#[test]
fn a_triggered_ability_of_a_permanent_cast_with_no_counter_mana_can_be_countered() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let pup = put(&mut game, LEAF_PUP, p0, ZoneType::Hand);
    put(&mut game, SOUL_CAVE, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: pup,
            cast: false,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(pup).zone, ZoneType::Battlefield);

    let mut trigger = SpellAbility::new_simple(Some(pup), p0, "DB$ GainLife | LifeAmount$ 5");
    trigger.is_trigger = true;
    game.stack.push(stack_entry(trigger));
    let trigger_id = game.stack.peek().unwrap().id;
    let sense = put(&mut game, SENSE_COUNTER, p1, ZoneType::Stack);
    let mut counter = manabrew_engine::spellability::build_spell_ability(
        &game,
        sense,
        "SP$ Counter | TargetType$ Instant,Sorcery,Triggered | ValidTgts$ Card,Emblem",
        p1,
    );
    counter.is_spell = true;
    counter.target_chosen.target_stack_entry = Some(trigger_id);
    game.stack.push(stack_entry(counter));
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(game.stack.is_empty());
}
