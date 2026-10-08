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
const VULTURES: &str = "Name:Circling Vultures\nManaCost:B\nTypes:Creature Bird\nPT:3/2\nK:Flying\nA:ST$ Discard | Cost$ 0 | Mode$ Defined | DefinedCards$ Self | Optional$ True | DiscardMessage$ Do you want discard this card? | ActivationZone$ Hand | InstantSpeed$ True | SpellDescription$ You may discard CARDNAME any time you could cast an instant.\nOracle:";
const SUDDEN_SHOCK: &str = "Name:Sudden Shock\nManaCost:1 R\nTypes:Instant\nK:Split second\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to any target.\nOracle:";

struct Recorder {
    seen: Rc<RefCell<Option<Vec<CardId>>>>,
    turn_up: CardId,
    confirm: bool,
    cast_pay: Option<ManaCostAction>,
}

impl PlayerAgent for Recorder {
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
        if self.cast_pay.is_some() {
            self.seen.borrow_mut().get_or_insert_with(Vec::new);
            return match space
                .playable
                .iter()
                .find(|play| play.card_id == self.turn_up)
            {
                Some(play) if first => PlayerAction::CastSpell(*play),
                _ => PlayerAction::PassPriority,
            };
        }
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
        self.cast_pay.unwrap_or(ManaCostAction::Pay { auto: true })
    }
    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        PassAgent.choose_land_or_spell(context, player)
    }
    fn confirm_action(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _mode: Option<&str>,
        _message: &str,
        _options: &[String],
        _source: Option<CardId>,
        _api: Option<manabrew_engine::ability::api_type::ApiType>,
    ) -> bool {
        self.confirm
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
            confirm: false,
            cast_pay: None,
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

fn put_split_second_shock(game: &mut GameState, caster: PlayerId) {
    let shock = put(game, SUDDEN_SHOCK, caster, ZoneType::Stack);
    let mut sa = SpellAbility::new_simple(
        Some(shock),
        caster,
        "SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2",
    );
    sa.is_spell = true;
    sa.target_chosen.target_player = Some(caster);
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

#[test]
fn a_scripted_static_ability_is_offered_and_resolves_under_split_second() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let vultures = put(&mut game, VULTURES, p0, ZoneType::Hand);
    put_split_second_shock(&mut game, p1);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: vultures,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let seen = seen.borrow().clone().expect("a priority decision");

    assert!(seen.contains(&vultures));
    assert_eq!(game.card(vultures).zone, ZoneType::Graveyard);
}

const LOYAL_WALKER: &str = "Name:Loyal Walker\nManaCost:2 U\nTypes:Legendary Planeswalker Jace\nLoyalty:3\nA:AB$ Draw | Cost$ SubCounter<1/LOYALTY> | Planeswalker$ True | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";
const INSTANT_LOYALTY: &str = "Name:Instant Loyalty Effect\nManaCost:no cost\nTypes:Effect\nS:Mode$ CastWithFlash | EffectZone$ Command | ValidCard$ Planeswalker.Jace+YouCtrl | ValidSA$ Activated.Loyalty | Caster$ You | Description$ You may activate loyalty abilities of Jace planeswalkers you control any time you could cast an instant.\nOracle:";

fn loyalty_offered_in_combat(with_effect: bool) -> bool {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let walker = put(&mut game, LOYAL_WALKER, p0, ZoneType::Battlefield);
    game.card_mut(walker).add_counter(
        &manabrew_engine::card::counter_type::parse_counter_type("LOYALTY"),
        3,
    );
    if with_effect {
        put(&mut game, INSTANT_LOYALTY, p0, ZoneType::Command);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::CombatBegin;
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: CardId(u32::MAX),
            confirm: false,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    let seen = seen.borrow().clone().unwrap_or_default();
    seen.contains(&walker)
}

#[test]
fn a_flash_static_for_loyalty_abilities_lets_them_be_activated_in_combat() {
    assert!(!loyalty_offered_in_combat(false));
    assert!(loyalty_offered_in_combat(true));
}

const HEAVY_WALKER: &str = "Name:Heavy Walker\nManaCost:2 U\nTypes:Legendary Planeswalker Jace\nLoyalty:3\nA:AB$ GainLife | Cost$ SubCounter<2/LOYALTY> | Planeswalker$ True | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";
const SCULPTOR_WATCH: &str = "Name:Sculptor Watch\nManaCost:4 U\nTypes:Enchantment\nT:Mode$ AbilityCast | ValidActivatingPlayer$ You | ValidSA$ Activated.Loyalty+CountersRemovedToPayGE2 | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Whenever you activate a loyalty ability, if you removed two or more loyalty counters to activate it, draw a card.\nSVar:TrigDraw:DB$ Draw | NumCards$ 1\nOracle:";

#[test]
fn a_loyalty_ability_records_the_counters_its_cost_removed() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let walker = put(&mut game, HEAVY_WALKER, p0, ZoneType::Battlefield);
    game.card_mut(walker).add_counter(
        &manabrew_engine::card::counter_type::parse_counter_type("LOYALTY"),
        3,
    );
    put(&mut game, SCULPTOR_WATCH, p0, ZoneType::Battlefield);
    for _ in 0..5 {
        put(&mut game, ISLAND, p0, ZoneType::Library);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let library = game.zone(ZoneType::Library, p0).len();
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: walker,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);

    assert_eq!(game.player(p0).life, 21);
    assert_eq!(game.zone(ZoneType::Library, p0).len(), library - 1);
}

const NINJA_WALKER: &str = "Name:Ninja Walker\nManaCost:2 U B\nTypes:Legendary Planeswalker Kaito\nLoyalty:2\nS:Mode$ Continuous | AffectedDefined$ Self | Affected$ Permanent.counters_GE1_LOYALTY | Condition$ PlayerTurn | AddType$ Creature & Ninja | RemoveCardTypes$ True | SetPower$ 3 | SetToughness$ 4 | Description$ During your turn, as long as it has a loyalty counter, it's a 3/4 Ninja creature.\nA:AB$ GainLife | Cost$ SubCounter<2/LOYALTY> | Planeswalker$ True | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";
const DEATH_WATCH: &str = "Name:Death Watch\nManaCost:1 R\nTypes:Artifact\nT:Mode$ ChangesZoneAll | TriggerZones$ Battlefield | ValidCards$ Creature | Origin$ Battlefield | Destination$ Graveyard | Execute$ TrigGain | TriggerDescription$ Whenever one or more creatures die, you gain 5 life.\nSVar:TrigGain:DB$ GainLife | LifeAmount$ 5\nOracle:";

#[test]
fn a_creature_walker_paid_down_to_no_loyalty_dies_as_a_planeswalker() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let walker = put(&mut game, NINJA_WALKER, p0, ZoneType::Battlefield);
    put(&mut game, DEATH_WATCH, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: walker,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);

    assert_eq!(game.card(walker).zone, ZoneType::Graveyard);
    assert_eq!(game.player(p0).life, 21);
    assert_eq!(game.player(p1).life, 20);
}

const STATION_HULL: &str =
    "Name:Station Hull\nManaCost:2\nTypes:Artifact Spacecraft\nK:Station:12\nOracle:";
const SKITTISH_CREW: &str = "Name:Skittish Crew\nManaCost:1 U\nTypes:Creature Human Pilot\nPT:2/2\nT:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigBounce | TriggerDescription$ Whenever CARDNAME becomes tapped, return it to its owner's hand.\nSVar:TrigBounce:DB$ ChangeZone | Defined$ Self | Origin$ Battlefield | Destination$ Hand\nOracle:";

#[test]
fn station_counts_the_power_of_a_creature_that_left_before_it_resolved() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let hull = put(&mut game, STATION_HULL, p0, ZoneType::Battlefield);
    let crew = put(&mut game, SKITTISH_CREW, p0, ZoneType::Battlefield);
    game.card_mut(crew).add_counter(
        &manabrew_engine::card::counter_type::parse_counter_type("P1P1"),
        1,
    );
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: hull,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);

    assert_eq!(game.card(crew).zone, ZoneType::Hand);
    let charge = manabrew_engine::card::counter_type::parse_counter_type("CHARGE");
    assert_eq!(game.card(hull).counter_count(&charge), 3);
}

const TOKEN_PORT: &str = "Name:Token Port\nManaCost:no cost\nTypes:Land\nA:AB$ Draw | Cost$ 2 T Sac<1/Permanent.token/token> | SpellDescription$ Draw a card.\nOracle:";
const TREASURE: &str = "Name:Treasure Token\nManaCost:no cost\nTypes:Artifact Treasure\nA:AB$ Mana | Cost$ T Sac<1/CARDNAME/this token> | Produced$ Any | Amount$ 1 | SpellDescription$ Add one mana of any color.\nOracle:";
const SACK_WATCH: &str = "Name:Sack Watch\nManaCost:B\nTypes:Creature Halfling\nPT:1/1\nT:Mode$ Sacrificed | ValidPlayer$ You | ValidCard$ Permanent.token+YouCtrl | TriggerZones$ Battlefield | Execute$ TrigLoseLife | TriggerDescription$ Whenever you sacrifice a token, each opponent loses 1 life.\nSVar:TrigLoseLife:DB$ LoseLife | LifeAmount$ 1 | Defined$ Player.Opponent\nOracle:";

#[test]
fn a_failed_activation_drops_the_triggers_its_mana_payment_collected() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let port = put(&mut game, TOKEN_PORT, p0, ZoneType::Battlefield);
    let treasures = [
        put(&mut game, TREASURE, p0, ZoneType::Battlefield),
        put(&mut game, TREASURE, p0, ZoneType::Battlefield),
    ];
    for token in treasures {
        game.card_mut(token).is_token = true;
    }
    put(&mut game, SACK_WATCH, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: port,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);

    let left = treasures
        .iter()
        .filter(|&&token| game.card(token).zone == ZoneType::Battlefield)
        .count();
    assert_eq!(left, 1);
    assert_eq!(game.player(p1).life, 20);
}

const TARGET_WATCH: &str = "Name:Target Watch\nManaCost:1 U\nTypes:Creature God\nPT:2/1\nT:Mode$ BecomesTarget | ValidTarget$ Player,Permanent | ValidSource$ Ability.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Whenever a player or permanent becomes the target of an ability you control, draw a card.\nSVar:TrigDraw:DB$ Draw\nOracle:";

#[test]
fn a_becomes_target_trigger_matches_its_source_ability_by_kind() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sorcerer = put(&mut game, PRODIGAL, p0, ZoneType::Battlefield);
    put(&mut game, TARGET_WATCH, p0, ZoneType::Battlefield);
    for _ in 0..3 {
        put(&mut game, ISLAND, p0, ZoneType::Library);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let library = game.zone(ZoneType::Library, p0).len();
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
            turn_up: sorcerer,
            confirm: true,
            cast_pay: None,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);

    assert!(game.card(sorcerer).tapped);
    assert_eq!(game.zone(ZoneType::Library, p0).len(), library - 1);
}

const BRAIDED_NET: &str = "Name:Braided Net\nManaCost:2 U\nTypes:Artifact\nK:etbCounter:NET:3\nA:AB$ Tap | Cost$ T SubCounter<1/NET> | ValidTgts$ Permanent.Other+nonLand | TgtPrompt$ Select another target nonland permanent | SubAbility$ DBEffect | SpellDescription$ Tap another target nonland permanent.\nSVar:DBEffect:DB$ Effect | RememberObjects$ Targeted | StaticAbilities$ CantActivate | ForgetOnMoved$ Battlefield | Duration$ UntilTargetedUntaps | SpellDescription$ Its activated abilities can't be activated for as long as it remains tapped.\nSVar:CantActivate:Mode$ CantBeActivated | ValidCard$ Card.IsRemembered | ValidSA$ Activated | Description$ Its activated abilities can't be activated for as long as it remains tapped.\nOracle:";

fn netted_sorcerer_effects(untap_step: bool) -> usize {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let net = put(&mut game, BRAIDED_NET, p0, ZoneType::Battlefield);
    let sorcerer = put(&mut game, PRODIGAL, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    let mut sa = manabrew_engine::spellability::build_spell_ability(
        &game,
        net,
        "AB$ Tap | ValidTgts$ Permanent.Other+nonLand | SubAbility$ DBEffect",
        p0,
    );
    sa.target_chosen.target_card = Some(sorcerer);
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
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    game_loop.resolve_stack(&mut game, &mut agents);
    assert!(game.card(sorcerer).tapped);
    game_loop.step_cleanup(&mut game, &mut agents);
    if untap_step {
        game.turn.active_player = p1;
        game.new_turn_for_player(p1);
        game_loop.step_untap(&mut game, &mut agents);
        assert!(!game.card(sorcerer).tapped);
    }
    game.cards_in_zone(ZoneType::Command, p0).len()
}

#[test]
fn a_braided_net_lock_lasts_past_the_turn_while_its_target_stays_tapped() {
    assert_eq!(netted_sorcerer_effects(false), 1);
}

#[test]
fn a_braided_net_lock_ends_when_its_target_untaps() {
    assert_eq!(netted_sorcerer_effects(true), 0);
}

const EXILED_BOLT: &str = "Name:Exiled Bolt\nManaCost:R\nTypes:Instant\nA:SP$ DealDamage | Defined$ Opponent | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to each opponent.\nOracle:";
const SHAPE_EFFECT: &str = "Name:Shape Effect\nManaCost:no cost\nTypes:Effect\nS:Mode$ Continuous | MayPlay$ True | Affected$ Card.IsRemembered | AffectedZone$ Exile | EffectZone$ Command | Description$ You may play the exiled cards.\nOracle:";
const RED_WELL: &str = "Name:Red Well\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ R | SpellDescription$ Add {R}.\nOracle:";

const EXILED_WASTES: &str = "Name:Exiled Wastes\nManaCost:no cost\nTypes:Land\nOracle:";

fn cast_a_face_down_exiled_card(
    mirror_forge_bugs: bool,
    pay: ManaCostAction,
) -> (GameState, CardId, CardId) {
    play_a_face_down_exiled_card(EXILED_BOLT, mirror_forge_bugs, pay)
}

fn play_a_face_down_exiled_card(
    script: &str,
    mirror_forge_bugs: bool,
    pay: ManaCostAction,
) -> (GameState, CardId, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.mirror_forge_bugs = mirror_forge_bugs;
    let p0 = PlayerId(0);
    let bolt = put(&mut game, script, p0, ZoneType::Exile);
    let effect = put(&mut game, SHAPE_EFFECT, p0, ZoneType::Command);
    put(&mut game, RED_WELL, p0, ZoneType::Battlefield);
    game.card_mut(bolt).set_face_down(true);
    game.card_mut(effect).add_remembered_card(bolt);
    game.card_mut(effect)
        .set_forget_on_moved_origin(Some(ZoneType::Exile));
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = forge_foundation::PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::new(RefCell::new(None)),
            turn_up: bolt,
            confirm: true,
            cast_pay: Some(pay),
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.resolve_stack(&mut game, &mut agents);
    (game, bolt, effect)
}

#[test]
fn a_face_down_exiled_card_cast_through_a_may_play_resolves_face_up() {
    let (game, bolt, _) = cast_a_face_down_exiled_card(false, ManaCostAction::Pay { auto: true });
    assert_eq!(game.player(PlayerId(1)).life, 18);
    assert_eq!(game.card(bolt).zone, ZoneType::Graveyard);
    assert!(!game.card(bolt).face_down);
    assert!(game.card(bolt).type_line.is_instant());
}

#[test]
fn a_land_face_down_in_exile_played_through_a_may_play_enters_as_a_land() {
    let (game, wastes, _) =
        play_a_face_down_exiled_card(EXILED_WASTES, false, ManaCostAction::Pay { auto: true });
    assert_eq!(game.card(wastes).zone, ZoneType::Battlefield);
    assert!(!game.card(wastes).face_down);
    assert!(game.card(wastes).is_land());
    assert_eq!(game.player(PlayerId(0)).lands_played_this_turn, 1);
    assert!(game.stack.get_spells_cast_this_turn().is_empty());
}

const EXILED_TRIUMPH: &str = "Name:Exiled Triumph\nManaCost:R\nTypes:Instant\nK:AlternateAdditionalCost:PayLife<3>:Discard<1/Card>\nA:SP$ DealDamage | Defined$ Opponent | NumDmg$ 2 | SpellDescription$ CARDNAME deals 2 damage to each opponent.\nOracle:";

#[test]
fn a_spell_cast_from_face_down_exile_pays_its_additional_cost() {
    let (game, _, _) =
        play_a_face_down_exiled_card(EXILED_TRIUMPH, false, ManaCostAction::Pay { auto: true });
    assert_eq!(game.player(PlayerId(1)).life, 18);
    assert_eq!(game.player(PlayerId(0)).life, 17);
}

#[test]
fn forge_skips_the_additional_cost_of_a_spell_cast_from_face_down_exile() {
    let (game, _, _) =
        play_a_face_down_exiled_card(EXILED_TRIUMPH, true, ManaCostAction::Pay { auto: true });
    assert_eq!(game.player(PlayerId(1)).life, 18);
    assert_eq!(game.player(PlayerId(0)).life, 20);
}
