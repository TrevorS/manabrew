use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
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
use manabrew_engine::spellability::SpellAbility;
use rand::SeedableRng;

const BEAR: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const WALL: &str = "Name:Wood Wall\nManaCost:1 G\nTypes:Creature Wall\nPT:1/4\nK:Defender\nOracle:";

struct Combatant {
    blocks: Vec<(CardId, CardId)>,
    assigned_by: Rc<RefCell<Vec<CardId>>>,
}

impl PlayerAgent for Combatant {
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
        available: &[CardId],
        defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        let Some(&defender) = defenders.first() else {
            return Vec::new();
        };
        available
            .iter()
            .map(|&attacker| (attacker, defender))
            .collect()
    }
    fn choose_blockers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[CardId],
        _: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        self.blocks.clone()
    }
    fn assign_combat_damage(
        &mut self,
        game: &GameState,
        player: PlayerId,
        source: CardId,
        targets: &[CardId],
        defender: Option<DefenderId>,
        damage: i32,
    ) -> Vec<(Option<CardId>, i32)> {
        self.assigned_by.borrow_mut().push(source);
        PassAgent.assign_combat_damage(game, player, source, targets, defender, damage)
    }
    fn choose_target_player(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        None
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
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn blockers_assign_damage_in_the_order_their_attackers_were_first_blocked() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let first_bear = put(&mut game, BEAR, p0, ZoneType::Battlefield);
    let second_bear = put(&mut game, BEAR, p0, ZoneType::Battlefield);
    let first_wall = put(&mut game, WALL, p1, ZoneType::Battlefield);
    let second_wall = put(&mut game, WALL, p1, ZoneType::Battlefield);
    for card in [first_bear, second_bear, first_wall, second_wall] {
        game.card_mut(card).set_summoning_sick(false);
    }
    for player in [p0, p1] {
        for _ in 0..3 {
            put(&mut game, BEAR, player, ZoneType::Library);
        }
    }
    game.turn.active_player = p0;
    let assigned_by = Rc::new(RefCell::new(Vec::new()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Combatant {
            blocks: Vec::new(),
            assigned_by: Rc::new(RefCell::new(Vec::new())),
        }),
        Box::new(Combatant {
            blocks: vec![(second_wall, second_bear), (first_wall, first_bear)],
            assigned_by: Rc::clone(&assigned_by),
        }),
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    GameLoop::new(2).run_turn(&mut game, &mut agents, &mut rng);
    assert_eq!(*assigned_by.borrow(), vec![second_wall, first_wall]);
}
