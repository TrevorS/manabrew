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
use manabrew_engine::spellability::{AlternativeCost, SpellAbility};

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const SPRITE: &str = "Name:Evoke Sprite\nManaCost:3 U\nTypes:Creature Elemental\nPT:1/1\nK:Evoke:U\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigGainLife | TriggerDescription$ When this creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const TORPOR_ORB: &str = "Name:Torpor Orb\nManaCost:2\nTypes:Artifact\nS:Mode$ DisableTriggers | ValidCause$ Creature | ValidMode$ ChangesZone,ChangesZoneAll | Destination$ Battlefield | Description$ Creatures entering don't cause abilities to trigger.\nOracle:";
const EVOKE_GRANTER: &str = "Name:Evoke Granter\nManaCost:2 R\nTypes:Creature Elemental\nPT:2/2\nS:Mode$ Continuous | Affected$ Permanent.Elemental+YouOwn | AffectedZone$ Hand | AddKeyword$ Evoke:2 | Description$ Elemental permanent spells you cast from your hand have evoke {2}.\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    sacrifice_triggers: usize,
    sacrifice_on_top: bool,
}

struct EvokeOnce {
    seen: Rc<RefCell<Seen>>,
    alt_cost_index: u8,
}

impl PlayerAgent for EvokeOnce {
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
            let sacrifices = game
                .stack
                .iter()
                .filter(|entry| entry.spell_ability.api == Some(ApiType::Sacrifice))
                .count();
            let mut seen = self.seen.borrow_mut();
            if sacrifices > seen.sacrifice_triggers {
                seen.sacrifice_triggers = sacrifices;
                seen.sacrifice_on_top = game
                    .stack
                    .peek()
                    .is_some_and(|entry| entry.spell_ability.api == Some(ApiType::Sacrifice));
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
            .find(|play| {
                play.mode == PlayCardMode::Alternative(AlternativeCost::Evoke)
                    && play.alt_cost_index == self.alt_cost_index
            })
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

struct Evoked {
    sacrifice_triggers: usize,
    sacrifice_on_top: bool,
    zone: ZoneType,
    life: i32,
}

fn evoke_a_sprite(others: &[&str], alt_cost_index: u8) -> Evoked {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sprite = put(&mut game, SPRITE, p0, ZoneType::Hand);
    for _ in 0..3 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    for script in others {
        put(&mut game, script, p0, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost_index,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert!(seen.borrow().cast);
    let seen = seen.borrow();
    Evoked {
        sacrifice_triggers: seen.sacrifice_triggers,
        sacrifice_on_top: seen.sacrifice_on_top,
        zone: game.card(sprite).zone,
        life: game.players[0].life,
    }
}

#[test]
fn an_evoked_creature_is_sacrificed_after_its_enters_trigger_is_put_under_it() {
    let evoked = evoke_a_sprite(&[], 0);
    assert_eq!(evoked.sacrifice_triggers, 1);
    assert!(evoked.sacrifice_on_top);
    assert_eq!(evoked.zone, ZoneType::Graveyard);
    assert_eq!(evoked.life, 21);
}

#[test]
fn torpor_orb_stops_the_evoke_sacrifice() {
    let evoked = evoke_a_sprite(&[TORPOR_ORB], 0);
    assert_eq!(evoked.sacrifice_triggers, 0);
    assert_eq!(evoked.zone, ZoneType::Battlefield);
    assert_eq!(evoked.life, 20);
}

#[test]
fn a_granted_evoke_cast_keeps_both_sacrifice_triggers() {
    let evoked = evoke_a_sprite(&[EVOKE_GRANTER], 1);
    assert_eq!(evoked.sacrifice_triggers, 2);
    assert_eq!(evoked.zone, ZoneType::Graveyard);
}

#[test]
fn the_printed_evoke_cast_beside_a_granted_one_has_one_sacrifice_trigger() {
    let evoked = evoke_a_sprite(&[EVOKE_GRANTER], 0);
    assert_eq!(evoked.sacrifice_triggers, 1);
    assert_eq!(evoked.zone, ZoneType::Graveyard);
}
