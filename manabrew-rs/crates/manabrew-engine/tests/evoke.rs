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
const ADAMANT_SPRITE: &str = "Name:Adamant Sprite\nManaCost:3 U\nTypes:Creature Elemental\nPT:1/1\nK:Evoke:U U\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | CheckSVar$ CastSA>Count$Adamant_2.Blue.2.0 | ValidCard$ Card.Self | Execute$ TrigGainLife | TriggerDescription$ When this creature enters, if {U}{U} was spent to cast it, you gain 3 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 3\nOracle:";
const BIG_SPELL_SPRING: &str = "Name:Big Spell Spring\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ U | Amount$ 2 | RestrictValid$ Spell.cmcGE4 | SpellDescription$ Add {U}{U}. Spend this mana only to cast spells with mana value 4 or greater.\nOracle:";
const EXILE_CASTER: &str = "Name:Exile Caster\nManaCost:1\nTypes:Artifact\nS:Mode$ Continuous | Affected$ Card.YouOwn+nonLand | AffectedZone$ Exile | MayPlay$ True | Description$ You may cast spells you own from exile.\nOracle:";
const IMPENDING_AVATAR: &str = "Name:Impending Avatar\nManaCost:3 U U\nTypes:Enchantment Creature Avatar\nPT:5/5\nK:Impending:5:1 U\nOracle:";
const HARMONIZE_STORY: &str = "Name:Harmonize Story\nManaCost:2 U\nTypes:Sorcery\nA:SP$ GainLife | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nK:Harmonize:4 U\nOracle:";
const GRAVE_TAX: &str = "Name:Grave Tax\nManaCost:1 W W\nTypes:Creature Bird\nPT:2/2\nS:Mode$ RaiseCost | Activator$ Opponent | ValidCard$ Card.wasCastFromGraveyard,Card.wasCastFromExile | Type$ Spell | Amount$ 2 | Description$ Spells your opponents cast from graveyards or from exile cost {2} more to cast.\nOracle:";
const ZERO_WALL: &str =
    "Name:Zero Wall\nManaCost:1 W\nTypes:Creature Wall\nPT:0/3\nK:Defender\nOracle:";
const BIG_BEAST: &str = "Name:Big Beast\nManaCost:4 G\nTypes:Creature Beast\nPT:5/5\nOracle:";
const EVOKE_GRANTER: &str = "Name:Evoke Granter\nManaCost:2 R\nTypes:Creature Elemental\nPT:2/2\nS:Mode$ Continuous | Affected$ Permanent.Elemental+YouOwn | AffectedZone$ Hand | AddKeyword$ Evoke:2 | Description$ Elemental permanent spells you cast from your hand have evoke {2}.\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    sacrifice_triggers: usize,
    sacrifice_on_top: bool,
}

struct EvokeOnce {
    seen: Rc<RefCell<Seen>>,
    alt_cost: AlternativeCost,
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
                play.mode == PlayCardMode::Alternative(self.alt_cost)
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
    fn choose_number(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _source: Option<CardId>,
        _title: &str,
        _description: Option<&str>,
        _min: i32,
        max: i32,
    ) -> Option<i32> {
        Some(max)
    }
    fn choose_number_for_keyword_cost(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        max: i32,
        _prompt: &str,
        _source: Option<CardId>,
    ) -> i32 {
        max
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
            alt_cost: AlternativeCost::Evoke,
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

fn evoke_offered(lands: &[&str], mirror_forge_bugs: bool, zone: ZoneType) -> bool {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = mirror_forge_bugs;
    let p0 = PlayerId(0);
    put(&mut game, SPRITE, p0, zone);
    if zone == ZoneType::Exile {
        put(&mut game, EXILE_CASTER, p0, ZoneType::Battlefield);
    }
    for script in lands {
        put(&mut game, script, p0, ZoneType::Battlefield);
    }
    game.action_space_mana_probe = manabrew_engine::mana::ActionSpaceManaProbe::ComputerUtilMana;
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost: AlternativeCost::Evoke,
            alt_cost_index: 0,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let cast = seen.borrow().cast;
    cast
}

#[test]
fn mana_for_big_spells_pays_an_evoke_of_a_big_spell() {
    for zone in [ZoneType::Hand, ZoneType::Exile] {
        assert!(evoke_offered(&[ISLAND], false, zone));
        assert!(evoke_offered(&[BIG_SPELL_SPRING], false, zone));
    }
}

#[test]
fn mana_for_big_spells_does_not_pay_an_evoke_as_forge_reads_it() {
    for zone in [ZoneType::Hand, ZoneType::Exile] {
        assert!(evoke_offered(&[ISLAND], true, zone));
        assert!(!evoke_offered(&[BIG_SPELL_SPRING], true, zone), "{zone:?}");
    }
}

#[test]
fn an_evoked_creature_keeps_the_mana_spent_for_its_enters_condition_after_the_sacrifice() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sprite = put(&mut game, ADAMANT_SPRITE, p0, ZoneType::Hand);
    for _ in 0..2 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost: AlternativeCost::Evoke,
            alt_cost_index: 0,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert!(seen.borrow().cast);
    assert_eq!(game.card(sprite).zone, ZoneType::Graveyard);
    assert_eq!(game.players[0].life, 23);
}

#[test]
fn an_exiled_card_that_may_be_cast_offers_its_impending_cost() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let avatar = put(&mut game, IMPENDING_AVATAR, p0, ZoneType::Exile);
    put(&mut game, EXILE_CASTER, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost: AlternativeCost::Impending,
            alt_cost_index: 0,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(seen.borrow().cast);
    assert_eq!(game.card(avatar).zone, ZoneType::Battlefield);
    assert!(!game.card(avatar).is_creature());
}

#[test]
fn a_harmonize_reduction_applies_after_a_cost_increase() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, HARMONIZE_STORY, p0, ZoneType::Graveyard);
    put(&mut game, GRAVE_TAX, p1, ZoneType::Battlefield);
    let beast = put(&mut game, BIG_BEAST, p0, ZoneType::Battlefield);
    game.card_mut(beast).summoning_sick = false;
    let islands: Vec<CardId> = (0..7)
        .map(|_| put(&mut game, ISLAND, p0, ZoneType::Battlefield))
        .collect();
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost: AlternativeCost::Harmonize,
            alt_cost_index: 0,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(seen.borrow().cast);
    assert!(game.card(beast).tapped);
    assert_eq!(game.player(p0).life, 21);
    let tapped = islands
        .iter()
        .filter(|&&island| game.card(island).tapped)
        .count();
    assert_eq!(tapped, 2);
}

#[test]
fn a_harmonize_cast_offers_to_tap_a_creature_with_no_power() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, HARMONIZE_STORY, p0, ZoneType::Graveyard);
    let wall = put(&mut game, ZERO_WALL, p0, ZoneType::Battlefield);
    game.card_mut(wall).summoning_sick = false;
    for _ in 0..5 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(EvokeOnce {
            seen: Rc::clone(&seen),
            alt_cost: AlternativeCost::Harmonize,
            alt_cost_index: 0,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(seen.borrow().cast);
    assert!(game.card(wall).tapped);
    assert_eq!(game.player(p0).life, 21);
}
