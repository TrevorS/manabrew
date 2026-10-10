use std::cell::Cell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, GameEntity, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::{GameState, TypeRegistry};
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::{ActionSpaceManaProbe, ManaPool};
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;

const BANNER_SAGE: &str = "Name:Banner Sage\nManaCost:1 U\nTypes:Creature Human Scientist\nPT:1/1\nA:AB$ Draw | Cost$ X X T | NumCards$ X | SpellDescription$ Draw X cards.\nSVar:X:Count$xPaid\nOracle:";
const CAVERN: &str = "Name:Cavern of Souls\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ C | SpellDescription$ Add {C}.\nA:AB$ Mana | Cost$ T | Produced$ Any | RestrictValid$ Spell.Creature+ChosenType | AddsNoCounter$ True | SpellDescription$ Add one mana of any color.\nOracle:";

struct Activator {
    name: &'static str,
    max_seen: Rc<Cell<Option<u32>>>,
    done: bool,
}

impl PlayerAgent for Activator {
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
        if !self.done {
            let game = pr.context().game;
            if let Some(action) = space
                .activatable
                .iter()
                .find(|action| game.card(action.card_id).card_name == self.name)
            {
                self.done = true;
                return PlayerAction::ActivateAbility(AbilityRef {
                    card_id: action.card_id,
                    ability_index: action.ability_index,
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
    fn announce_requirements_x(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: Option<CardId>,
        _: u32,
        max: u32,
    ) -> u32 {
        self.max_seen.set(Some(max));
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
    let card = game.create_card(manabrew_engine::card::CardInstance::from_rules(
        &rules, owner,
    ));
    game.move_card(card, zone, owner);
    card
}

fn max_x_offered(probe: ActionSpaceManaProbe) -> Option<u32> {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sage = put(&mut game, BANNER_SAGE, p0, ZoneType::Battlefield);
    game.card_mut(sage).summoning_sick = false;
    for chosen in ["Human", "Time Lord"] {
        let cavern = put(&mut game, CAVERN, p0, ZoneType::Battlefield);
        game.card_mut(cavern).chosen_type = Some(chosen.to_string());
    }
    for _ in 0..20 {
        put(
            &mut game,
            "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:",
            p0,
            ZoneType::Library,
        );
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.action_space_mana_probe = probe;
    let max_seen = Rc::new(Cell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Activator {
            name: "Banner Sage",
            max_seen: Rc::clone(&max_seen),
            done: false,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    max_seen.get()
}

#[test]
fn the_harness_x_for_an_ability_follows_the_cavern_of_souls_probe() {
    assert_eq!(
        max_x_offered(ActionSpaceManaProbe::ComputerUtilMana),
        Some(0)
    );
    assert_eq!(max_x_offered(ActionSpaceManaProbe::AutoPay), Some(1));
}

const SHAPELESS_SANCTUARY: &str = "Name:Shapeless Sanctuary\nManaCost:no cost\nTypes:Land Creature\nPT:3/3\nS:Mode$ Continuous | Affected$ Card.Self | CharacteristicDefining$ True | AddAllCreatureTypes$ True | EffectZone$ All\nA:AB$ GainLife | Cost$ 2 | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";

fn load_types() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
}

fn life_after_activating_an_all_creature_types_land(probe: ActionSpaceManaProbe) -> i32 {
    load_types();
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sanctuary = put(&mut game, SHAPELESS_SANCTUARY, p0, ZoneType::Battlefield);
    game.card_mut(sanctuary).summoning_sick = false;
    let cavern = put(&mut game, CAVERN, p0, ZoneType::Battlefield);
    game.card_mut(cavern).chosen_type = Some("Human".to_string());
    put(
        &mut game,
        "Name:Plains\nManaCost:no cost\nTypes:Basic Land Plains\nOracle:",
        p0,
        ZoneType::Battlefield,
    );
    for _ in 0..20 {
        put(
            &mut game,
            "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:",
            p0,
            ZoneType::Library,
        );
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.action_space_mana_probe = probe;
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Activator {
            name: "Shapeless Sanctuary",
            max_seen: Rc::new(Cell::new(None)),
            done: false,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, false);
    game.players[0].life
}

#[test]
fn the_cavern_of_souls_probe_counts_a_host_with_all_creature_types_as_its_chosen_type() {
    assert_eq!(
        life_after_activating_an_all_creature_types_land(ActionSpaceManaProbe::ComputerUtilMana),
        20
    );
    assert_eq!(
        life_after_activating_an_all_creature_types_land(ActionSpaceManaProbe::AutoPay),
        21
    );
}
