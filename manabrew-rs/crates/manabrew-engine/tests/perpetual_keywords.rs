use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::ability::api_type::ApiType;
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayCardMode, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::card_trait_changes::CardTraitChanges;
use manabrew_engine::card::perpetual::perpetual_abilities::PerpetualAbilities;
use manabrew_engine::card::perpetual::perpetual_interface::PerpetualInterface;
use manabrew_engine::card::perpetual::perpetual_keywords::PerpetualKeywords;
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::keyword::Keyword;
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;
use manabrew_engine::staticability::layer::apply_continuous_effects;
use manabrew_engine::trigger::{parse_trigger, TriggerType};
use rand::SeedableRng;

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const SWAMP: &str = "Name:Swamp\nManaCost:no cost\nTypes:Basic Land Swamp\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const SCHOLAR: &str = "Name:Exploiting Scholar\nManaCost:3 U\nTypes:Creature Human Wizard\nPT:2/2\nK:Exploit\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigGainLife | TriggerDescription$ When this creature enters, you gain 1 life.\nSVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\nA:AB$ GainLife | Cost$ T | Defined$ You | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";
const GRAVE_GIFT: &str = "Name:Grave Gift\nManaCost:B\nTypes:Sorcery\nA:SP$ Pump | Defined$ ValidGraveyard Creature.YouOwn | PumpZone$ Graveyard | KW$ Unearth:B | Duration$ Perpetual | SpellDescription$ Creature cards in your graveyard perpetually gain unearth {B}.\nOracle:";
const TAP_DOWN: &str = "Name:Tap Down\nManaCost:U\nTypes:Instant\nA:SP$ Tap | ValidTgts$ Creature | TgtPrompt$ Select target creature | SpellDescription$ Tap target creature.\nOracle:";
const WEREBEAR: &str = "Name:Day Werebear\nManaCost:1 G\nTypes:Creature Human Werewolf\nPT:1/1\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Werebear\nManaCost:no cost\nColors:green\nTypes:Creature Werewolf\nPT:3/3\nOracle:";
const OPT: &str = "Name:Quick Study\nManaCost:U\nTypes:Sorcery\nA:SP$ Draw | Defined$ You | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";
const MIMIC_FORM: &str = "Name:Mimic Form\nManaCost:1 U\nTypes:Sorcery\nA:SP$ Clone | ValidTgts$ Creature.OppCtrl | TgtPrompt$ Select target creature an opponent controls | CloneTarget$ Valid Creature.YouCtrl | Duration$ UntilEndOfTurn | SpellDescription$ Until end of turn, each creature you control becomes a copy of target creature an opponent controls.\nOracle:";
const CLONE: &str = "Name:Clone\nManaCost:3 U\nTypes:Creature Shapeshifter\nPT:0/0\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | SpellDescription$ You may have CARDNAME enter as a copy of any creature on the battlefield.\nOracle:";

#[derive(Clone, Copy)]
enum Step {
    Cast(CardId),
    Activate(CardId),
}

struct Scripted {
    steps: Vec<Step>,
    done: Rc<RefCell<usize>>,
}

impl PlayerAgent for Scripted {
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
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let next = *self.done.borrow();
        let action = match self.steps.get(next) {
            Some(&Step::Cast(card)) => space
                .playable
                .iter()
                .find(|play| play.card_id == card && play.mode == PlayCardMode::Normal)
                .map(|&play| PlayerAction::CastSpell(play)),
            Some(&Step::Activate(card)) => space
                .activatable
                .iter()
                .find(|action| action.card_id == card && !action.is_mana_ability)
                .map(|action| {
                    PlayerAction::ActivateAbility(AbilityRef {
                        card_id: action.card_id,
                        ability_index: action.ability_index,
                    })
                }),
            None => None,
        };
        match action {
            Some(action) => {
                *self.done.borrow_mut() += 1;
                action
            }
            None => PassAgent.choose_action(player, Some(space), priority),
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
        _context: DecisionContext<'_>,
        _player: PlayerId,
        valid: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        valid.first().copied()
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

fn scripted(steps: Vec<Step>) -> (Box<dyn PlayerAgent>, Rc<RefCell<usize>>) {
    let done = Rc::new(RefCell::new(0));
    (
        Box::new(Scripted {
            steps,
            done: Rc::clone(&done),
        }),
        done,
    )
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn grant_perpetual(game: &mut GameState, card: CardId, keywords: &[&str]) {
    PerpetualKeywords {
        timestamp: 1_000_000,
        add_keywords: keywords.iter().map(|kw| kw.to_string()).collect(),
        remove_keywords: Vec::new(),
        remove_all: false,
    }
    .apply_effect(game.card_mut(card));
}

fn has_ward_trigger(game: &GameState, card: CardId) -> bool {
    game.card(card)
        .triggers
        .iter()
        .any(|trigger| trigger.kind == TriggerType::BecomesTarget)
}

fn main_phase(game: &mut GameState, player: PlayerId) {
    game.turn.active_player = player;
    game.new_turn_for_player(player);
    game.turn.phase = PhaseType::Main1;
}

#[test]
fn a_perpetual_unearth_from_a_pump_can_be_activated() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let gift = put(&mut game, GRAVE_GIFT, p0, ZoneType::Hand);
    let bears = put(&mut game, BEARS, p0, ZoneType::Graveyard);
    for _ in 0..3 {
        put(&mut game, SWAMP, p0, ZoneType::Battlefield);
    }
    for player in [p0, p1] {
        for _ in 0..3 {
            put(&mut game, SWAMP, player, ZoneType::Library);
        }
    }
    game.turn.active_player = p0;
    let (agent, done) = scripted(vec![Step::Cast(gift), Step::Activate(bears)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![agent, Box::new(PassAgent)];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    GameLoop::new(2).run_turn(&mut game, &mut agents, &mut rng);
    assert_eq!(*done.borrow(), 2);
    assert_eq!(game.card(bears).zone, ZoneType::Exile);
}

#[test]
fn a_perpetual_keyword_granted_after_a_baseline_capture_survives_a_zone_round_trip() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    {
        let card = game.card_mut(bears);
        card.capture_changed_characteristics_baseline_if_needed();
        card.add_changed_card_keywords("Vigilance");
    }
    grant_perpetual(&mut game, bears, &["Ward:2"]);
    apply_continuous_effects(&mut game);
    assert!(game.card(bears).has_keyword_enum(Keyword::Ward));
    assert!(has_ward_trigger(&game, bears));

    game.move_card(bears, ZoneType::Graveyard, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    let card = game.card(bears);
    assert!(card.has_keyword_enum(Keyword::Ward));
    assert!(!card.has_keyword("Vigilance"));
    assert!(has_ward_trigger(&game, bears));
}

#[test]
fn a_perpetual_ward_counters_an_opponents_spell_after_a_zone_round_trip() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    grant_perpetual(&mut game, bears, &["Ward:2"]);
    game.move_card(bears, ZoneType::Graveyard, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    let tap_down = put(&mut game, TAP_DOWN, p1, ZoneType::Hand);
    put(&mut game, ISLAND, p1, ZoneType::Battlefield);
    main_phase(&mut game, p0);
    let (opponent, done) = scripted(vec![Step::Cast(tap_down)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), opponent];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(*done.borrow(), 1);
    assert_eq!(game.card(tap_down).zone, ZoneType::Graveyard);
    assert!(!game.card(bears).tapped);
}

#[test]
fn a_clone_does_not_copy_a_perpetual_keyword() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    grant_perpetual(&mut game, bears, &["Ward:2"]);
    let clone = put(&mut game, CLONE, p0, ZoneType::Hand);
    for _ in 0..6 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    main_phase(&mut game, p0);
    let (agent, done) = scripted(vec![Step::Cast(clone)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![agent, Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(*done.borrow(), 1);
    let copy = game.card(clone);
    assert_eq!(copy.zone, ZoneType::Battlefield);
    assert_eq!(copy.card_name, "Grizzly Bears");
    assert!(!copy.has_keyword_enum(Keyword::Ward));
    assert!(!has_ward_trigger(&game, clone));
    assert!(game.card(bears).has_keyword_enum(Keyword::Ward));
}

#[test]
fn perpetual_keyword_traits_come_after_printed_and_intrinsic_ones() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let scholar = put(&mut game, SCHOLAR, p0, ZoneType::Battlefield);
    let printed_trigger_ids: Vec<u32> = game.card(scholar).triggers.iter().map(|t| t.id).collect();
    let printed_abilities = game.card(scholar).activated_abilities.len();
    grant_perpetual(&mut game, scholar, &["Ward:2", "Unearth:B"]);
    apply_continuous_effects(&mut game);
    let card = game.card(scholar);
    let ward = card
        .triggers
        .iter()
        .find(|trigger| trigger.kind == TriggerType::BecomesTarget)
        .expect("ward trigger");
    assert!(printed_trigger_ids.iter().all(|&id| id < ward.id));
    assert_eq!(card.activated_abilities.len(), printed_abilities + 1);
    assert!(card.activated_abilities[printed_abilities]
        .ability_text
        .contains("Keyword$ Unearth"));
}

#[test]
fn perpetual_layer_triggers_fire_after_layer_passes_and_a_transform() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let werebear = put(&mut game, WEREBEAR, p0, ZoneType::Battlefield);
    let mut id = 0;
    let spell_cast = parse_trigger(
        "Mode$ SpellCast | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigGainLife | TriggerDescription$ Whenever you cast a spell, you gain 1 life.",
        &mut id,
    )
    .expect("trigger");
    game.card_mut(werebear).set_s_var(
        "TrigGainLife",
        "DB$ GainLife | Defined$ You | LifeAmount$ 1",
    );
    PerpetualAbilities {
        timestamp: 900_000,
        changes: CardTraitChanges {
            triggers: vec![spell_cast],
            ..Default::default()
        },
    }
    .apply_effect(game.card_mut(werebear));
    grant_perpetual(&mut game, werebear, &["Prowess"]);
    for _ in 0..3 {
        apply_continuous_effects(&mut game);
    }
    game.card_mut(werebear).transform();
    for _ in 0..3 {
        apply_continuous_effects(&mut game);
    }
    let opt = put(&mut game, OPT, p0, ZoneType::Hand);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    for player in [p0, p1] {
        put(&mut game, ISLAND, player, ZoneType::Library);
    }
    main_phase(&mut game, p0);
    let (agent, done) = scripted(vec![Step::Cast(opt)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![agent, Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(*done.borrow(), 1);
    assert_eq!(game.card(opt).zone, ZoneType::Graveyard);
    assert_eq!(game.players[0].life, 21);
    assert_eq!(game.card(werebear).power(), 4);
}

#[test]
fn a_clone_keeps_the_pump_triggers_of_the_creature_it_turns_into_a_copy() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let scholar = put(&mut game, SCHOLAR, p0, ZoneType::Battlefield);
    put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let reflection = put(&mut game, MIMIC_FORM, p0, ZoneType::Hand);
    for _ in 0..3 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    let mut id = 0;
    game.card_mut(scholar).add_pump_trigger(
        parse_trigger(
            "Mode$ Blocks | ValidCard$ Card.Self | Execute$ TrigGainLife | TriggerDescription$ Blocks",
            &mut id,
        )
        .expect("trigger"),
    );
    main_phase(&mut game, p0);
    let (agent, done) = scripted(vec![Step::Cast(reflection)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![agent, Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(*done.borrow(), 1);
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    let card = game.card(scholar);
    assert_eq!(card.card_name, "Grizzly Bears");
    let kinds: Vec<TriggerType> = card.triggers.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TriggerType::Blocks]);
    assert_eq!(card.pump_trigger_count, 1);
}

#[test]
fn a_perpetual_keyword_survives_becoming_a_copy_and_a_zone_change() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    grant_perpetual(&mut game, bears, &["Ward:2"]);
    put(&mut game, SCHOLAR, p1, ZoneType::Battlefield);
    let mimic = put(&mut game, MIMIC_FORM, p0, ZoneType::Hand);
    for _ in 0..2 {
        put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    }
    main_phase(&mut game, p0);
    let (agent, done) = scripted(vec![Step::Cast(mimic)]);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![agent, Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(*done.borrow(), 1);
    assert_eq!(game.card(bears).card_name, "Exploiting Scholar");
    assert!(game.card(bears).has_keyword_enum(Keyword::Ward));
    game.move_card(bears, ZoneType::Hand, p0);
    apply_continuous_effects(&mut game);
    assert_eq!(game.card(bears).card_name, "Grizzly Bears");
    assert!(game.card(bears).has_keyword_enum(Keyword::Ward));
}
