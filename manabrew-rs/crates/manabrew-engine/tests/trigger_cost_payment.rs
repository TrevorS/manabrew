use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{ManaAtom, PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, GameEntity, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::{ActionSpaceManaProbe, Mana, ManaPool};
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const DAMAGE_SOURCE: &str = "Name:Damage Source\nManaCost:no cost\nTypes:Artifact\nOracle:";

fn paying_warden(cost: &str) -> String {
    format!("Name:Paying Warden\nManaCost:1 G\nTypes:Creature Elf\nPT:2/2\nT:Mode$ DamageDoneOnce | ValidTarget$ Creature.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever a creature you control is dealt damage, you may pay {cost}. If you do, you gain 1 life.\nSVar:TrigGain:AB$ GainLife | Cost$ {cost} | LifeAmount$ 1 | Defined$ You\nSVar:X:Count$xPaid\nOracle:")
}

struct PoolWatcher(Rc<RefCell<Vec<usize>>>);

impl PlayerAgent for PoolWatcher {
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
        c: DecisionContext<'_>,
        p: PlayerId,
        h: &[CardId],
        n: u32,
    ) -> bool {
        PassAgent.mulligan_decision(c, p, h, n)
    }
    fn choose_action(
        &mut self,
        p: PlayerId,
        s: Option<&PriorityActionSpace>,
        pr: &mut dyn PriorityContext,
    ) -> PlayerAction {
        PassAgent.choose_action(p, s, pr)
    }
    fn choose_attackers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        Vec::new()
    }
    fn choose_blockers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[CardId],
        _: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        Vec::new()
    }
    fn choose_target_player(
        &mut self,
        _: DecisionContext<'_>,
        p: PlayerId,
        valid: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        valid.iter().copied().find(|&v| v != p)
    }
    fn choose_target_card(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> Option<CardId> {
        None
    }
    fn choose_target_any(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        TargetChoice::None
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
    #[allow(clippy::too_many_arguments)]
    fn choose_number(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: Option<CardId>,
        _: &str,
        _: Option<&str>,
        _: i32,
        _: i32,
    ) -> Option<i32> {
        Some(3)
    }
    fn choose_mana_from_pool(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        mana_choices: &[Mana],
    ) -> usize {
        self.0.borrow_mut().push(mana_choices.len());
        0
    }
    #[allow(clippy::too_many_arguments)]
    fn pay_mana_cost(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: CardId,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: bool,
        _: &[CardId],
        _: &[ManaAbilityOption],
        _: &[CardId],
        _: &[CardId],
        _: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
    #[allow(clippy::too_many_arguments)]
    fn pay_cost_to_prevent_effect(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &str,
        _: &str,
        _: Option<CardId>,
        _: Option<manabrew_engine::ability::api_type::ApiType>,
        _: bool,
        _: &[GameEntity],
        _: &str,
    ) -> bool {
        false
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

struct Setup {
    game: GameState,
    game_loop: GameLoop,
    asked: Rc<RefCell<Vec<usize>>>,
}

fn damage_the_warden(cost: &str) -> Setup {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p1 = PlayerId(1);
    game.turn.active_player = p1;
    game.new_turn_for_player(p1);
    game.turn.phase = PhaseType::Main1;
    put(
        &mut game,
        &paying_warden(cost),
        PlayerId(0),
        ZoneType::Battlefield,
    );
    let source = put(&mut game, DAMAGE_SOURCE, p1, ZoneType::Command);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(
            Some(source),
            p1,
            "DB$ DamageAll | ValidCards$ Creature | NumDmg$ 1",
        ),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    Setup {
        game,
        game_loop: GameLoop::new(2),
        asked: Rc::default(),
    }
}

fn resolve(setup: &mut Setup) {
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(PoolWatcher(Rc::clone(&setup.asked))),
        Box::new(PassAgent),
    ];
    setup
        .game_loop
        .step_with_priority(&mut setup.game, &mut agents, false);
}

#[test]
fn a_trigger_cost_paid_from_a_tied_pool_asks_which_mana_to_spend() {
    let mut setup = damage_the_warden("1");
    setup.game_loop.mana_pools[0].add(ManaAtom::RED, 1);
    setup.game_loop.mana_pools[0].add(ManaAtom::GREEN, 1);
    resolve(&mut setup);
    assert_eq!(setup.game.player(PlayerId(0)).life, 21);
    assert_eq!(*setup.asked.borrow(), vec![2]);
}

fn failed_payment_leaves_forest_tapped(mirror_forge_bugs: bool) -> bool {
    let mut setup = damage_the_warden("X");
    setup.game.mirror_forge_bugs = mirror_forge_bugs;
    setup.game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    let forest = put(&mut setup.game, FOREST, PlayerId(0), ZoneType::Battlefield);
    resolve(&mut setup);
    assert_eq!(setup.game.player(PlayerId(0)).life, 20);
    setup.game.card(forest).tapped
}

#[test]
fn a_failed_trigger_payment_refunds_and_keeps_its_taps_only_under_the_forge_mirror() {
    assert!(!failed_payment_leaves_forest_tapped(false));
    assert!(failed_payment_leaves_forest_tapped(true));
}
