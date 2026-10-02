use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
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

const SHOCK: &str = "Name:Shock\nManaCost:R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";
const MOUNTAIN: &str = "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const WARDING: &str = "Name:Warding Banner\nManaCost:1 W\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddKeyword$ Ward:1 | Description$ Creatures you control have ward {1}.\nOracle:";
const NOWHERE: &str = "Name:Nowhere to Run\nManaCost:1 B\nTypes:Enchantment\nS:Mode$ IgnoreHexproof | ValidEntity$ Creature.OppCtrl | Description$ Creatures your opponents control can be the targets of spells and abilities as though they didn't have hexproof. Ward abilities of those creatures don't trigger.\nS:Mode$ DisableTriggers | Secondary$ True | ValidTrigger$ Triggered.Ward | ValidCard$ Creature.OppCtrl+inZoneBattlefield | Description$ Ward abilities of those creatures don't trigger.\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    ward_prompts: usize,
}

struct Shocker(Rc<RefCell<Seen>>);

impl PlayerAgent for Shocker {
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
        if self.0.borrow().cast {
            return PassAgent.choose_action(player, space, priority);
        }
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let Some(&play) = space.playable.first() else {
            return PlayerAction::PassPriority;
        };
        self.0.borrow_mut().cast = true;
        PlayerAction::CastSpell(play)
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
    let mut agents: Vec<Box<dyn PlayerAgent>> =
        vec![Box::new(Shocker(Rc::clone(&seen))), Box::new(PassAgent)];
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
