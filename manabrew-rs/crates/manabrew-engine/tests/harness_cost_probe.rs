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
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const X_BURST: &str = "Name:X Burst\nManaCost:X\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Player | NumDmg$ X | SpellDescription$ CARDNAME deals X damage to target player.\nSVar:X:Count$xPaid\nOracle:";
const X_FONT: &str = "Name:X Font\nManaCost:2\nTypes:Artifact\nA:AB$ GainLife | Cost$ X | LifeAmount$ X | Defined$ You | SpellDescription$ You gain X life.\nSVar:X:Count$xPaid\nOracle:";
const EFFECT_SOURCE: &str = "Name:Effect Source\nManaCost:no cost\nTypes:Artifact\nOracle:";

struct Prober {
    asked: Rc<RefCell<Vec<usize>>>,
    acted: bool,
}

impl PlayerAgent for Prober {
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
        let requested;
        let space = match s {
            Some(space) => space,
            None => {
                requested = pr.action_space();
                &requested
            }
        };
        if !self.acted {
            if let Some(&play) = space.playable.first() {
                self.acted = true;
                return PlayerAction::CastSpell(play);
            }
            if let Some(ability) = space.activatable.first() {
                self.acted = true;
                return PlayerAction::ActivateAbility(AbilityRef {
                    card_id: ability.card_id,
                    ability_index: ability.ability_index,
                });
            }
        }
        PassAgent.choose_action(p, Some(space), pr)
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
        p: PlayerId,
        players: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        players
            .iter()
            .copied()
            .find(|&v| v != p)
            .map_or(TargetChoice::None, TargetChoice::Player)
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
    fn choose_mana_from_pool(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        mana_choices: &[Mana],
    ) -> usize {
        self.asked.borrow_mut().push(mana_choices.len());
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
        true
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn run(mirror_forge_bugs: bool, setup: impl FnOnce(&mut GameState)) -> (GameState, Vec<usize>) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.turn.active_player = PlayerId(0);
    game.new_turn_for_player(PlayerId(0));
    game.turn.phase = PhaseType::Main1;
    game.mirror_forge_bugs = mirror_forge_bugs;
    game.action_space_mana_probe = ActionSpaceManaProbe::ComputerUtilMana;
    setup(&mut game);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let mut game_loop = GameLoop::new(2);
    game_loop.mana_pools[0].add(ManaAtom::RED, 1);
    game_loop.mana_pools[0].add(ManaAtom::GREEN, 1);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Prober {
            asked: Rc::clone(&asked),
            acted: false,
        }),
        Box::new(PassAgent),
    ];
    game_loop.step_with_priority(&mut game, &mut agents, false);
    let asked = asked.borrow().clone();
    (game, asked)
}

fn unless_cost(mirror_forge_bugs: bool) -> Vec<usize> {
    let (game, asked) = run(mirror_forge_bugs, |game| {
        let source = put(game, EFFECT_SOURCE, PlayerId(0), ZoneType::Command);
        game.stack.push(StackEntry {
            id: 0,
            spell_ability: SpellAbility::new_simple(
                Some(source),
                PlayerId(0),
                "DB$ LoseLife | LifeAmount$ 3 | Defined$ You | UnlessCost$ 1 | UnlessPayer$ You",
            ),
            is_creature_spell: false,
            is_permanent_spell: false,
            is_pending_cast: false,
            cast_from_zone: None,
            optional_trigger_decider: None,
            optional_trigger_description: None,
            optional_trigger_source_name: None,
        });
    });
    assert_eq!(game.player(PlayerId(0)).life, 20);
    asked
}

#[test]
fn an_unless_cost_runs_the_harness_probe_only_under_the_forge_mirror() {
    assert_eq!(unless_cost(false), vec![2]);
    assert_eq!(unless_cost(true), vec![2, 2]);
}

fn cast_x(mirror_forge_bugs: bool) -> Vec<usize> {
    let (game, asked) = run(mirror_forge_bugs, |game| {
        put(game, X_BURST, PlayerId(0), ZoneType::Hand);
    });
    assert_eq!(game.player(PlayerId(1)).life, 18);
    asked
}

#[test]
fn an_x_spell_runs_the_harness_probe_per_x_only_under_the_forge_mirror() {
    assert_eq!(cast_x(false), vec![2]);
    assert_eq!(cast_x(true), vec![2, 2, 2, 2]);
}

fn activate_x(mirror_forge_bugs: bool) -> Vec<usize> {
    let (game, asked) = run(mirror_forge_bugs, |game| {
        put(game, X_FONT, PlayerId(0), ZoneType::Battlefield);
    });
    assert_eq!(game.player(PlayerId(0)).life, 22);
    asked
}

#[test]
fn an_x_ability_runs_the_harness_probe_per_x_only_under_the_forge_mirror() {
    assert_eq!(activate_x(false), vec![2]);
    assert_eq!(activate_x(true), vec![2, 2, 2, 2]);
}
