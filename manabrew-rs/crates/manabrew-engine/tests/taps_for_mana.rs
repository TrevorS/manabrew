use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, PassAgent, PlayerAgent, PriorityActionSpace, PriorityContext,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ActionSpaceManaProbe;
use manabrew_engine::player::actions::PlayerAction;

const MOUNTAIN: &str =
    "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:({T}: Add {R}.)";
const MOLTEN_TIDE_EFFECT: &str = "Name:Molten Tide Effect\nManaCost:no cost\nTypes:Effect\nT:Mode$ TapsForMana | ValidCard$ Mountain | Activator$ You | Execute$ TrigMana | Static$ True | TriggerZones$ Command | TriggerDescription$ Whenever you tap a Mountain for mana, add an additional {R}.\nSVar:TrigMana:DB$ Mana | Produced$ R\nOracle:";
const THREE_DROP: &str = "Name:Three Drop\nManaCost:2 R\nTypes:Sorcery\nA:SP$ GainLife | LifeAmount$ 1 | SpellDescription$ You gain 1 life.\nOracle:";

struct Recorder {
    seen: Rc<RefCell<Option<Vec<CardId>>>>,
}

impl PlayerAgent for Recorder {
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
        _: PlayerId,
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
        self.seen
            .borrow_mut()
            .get_or_insert_with(|| space.playable.iter().map(|p| p.card_id).collect());
        PlayerAction::PassPriority
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
    fn choose_targets_for(
        &mut self,
        sa: &mut manabrew_engine::spellability::SpellAbility,
        game: &GameState,
        pools: &[manabrew_engine::mana::ManaPool],
    ) -> bool {
        manabrew_engine::spellability::choose_targets_by_kind(self, sa, game, pools)
    }
    fn choose_target_player(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        valid: &[PlayerId],
        _: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<PlayerId> {
        valid.first().copied()
    }
    fn choose_target_card(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        valid: &[CardId],
        _: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> Option<CardId> {
        valid.first().copied()
    }
    fn choose_target_any(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: &[CardId],
        _: Option<&manabrew_engine::spellability::SpellAbility>,
    ) -> manabrew_engine::agent::TargetChoice {
        manabrew_engine::agent::TargetChoice::None
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn offered(probe: ActionSpaceManaProbe) -> bool {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.action_space_mana_probe = probe;
    let p0 = PlayerId(0);
    let spell = put(&mut game, THREE_DROP, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, MOLTEN_TIDE_EFFECT, p0, ZoneType::Command);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let seen = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Recorder {
            seen: Rc::clone(&seen),
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let playable = seen.borrow().clone().expect("a priority decision");
    playable.contains(&spell)
}

#[test]
fn a_command_zone_taps_for_mana_trigger_counts_toward_a_spell() {
    assert!(offered(ActionSpaceManaProbe::ComputerUtilMana));
    assert!(offered(ActionSpaceManaProbe::default()));
}
