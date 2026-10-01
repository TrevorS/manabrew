use std::cell::Cell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{
    DecisionContext, PassAgent, PlayerAgent, PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::SpellAbility;

struct NoAttacks(Rc<Cell<u32>>);

impl PlayerAgent for NoAttacks {
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
        PassAgent.choose_action(player, space, priority)
    }
    fn choose_attackers(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _available: &[CardId],
        _defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        self.0.set(self.0.get() + 1);
        Vec::new()
    }
    fn choose_blockers(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        attackers: &[CardId],
        blockers: &[CardId],
        max: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        PassAgent.choose_blockers(context, player, attackers, blockers, max)
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
    fn choose_sacrifice(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        source: Option<CardId>,
    ) -> Option<CardId> {
        PassAgent.choose_sacrifice(context, player, valid, source)
    }
    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        PassAgent.choose_land_or_spell(context, player)
    }
}

const MUST_ATTACK: &str = "Name:Bloodrock Cyclops\nManaCost:2 R\nTypes:Creature Cyclops\nPT:3/3\nS:Mode$ MustAttack | ValidCreature$ Card.Self | Description$ CARDNAME attacks each combat if able.\nOracle:Bloodrock Cyclops attacks each combat if able.";
const PROPAGANDA: &str = "Name:Propaganda\nManaCost:2 U\nTypes:Enchantment\nS:Mode$ CantAttackUnless | ValidCard$ Creature | Target$ You | Cost$ 2 | Description$ Creatures can't attack you unless their controller pays {2} for each creature they control that's attacking you.\nOracle:Creatures can't attack you unless their controller pays {2} for each creature they control that's attacking you.";

#[test]
fn a_must_attack_creature_taxed_by_propaganda_may_stay_home() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let cyclops = parse_card_script(MUST_ATTACK).expect("script");
    let cyclops = game.create_card(CardInstance::from_rules(&cyclops, p0));
    game.move_card(cyclops, ZoneType::Battlefield, p0);
    game.card_mut(cyclops).summoning_sick = false;
    let propaganda = parse_card_script(PROPAGANDA).expect("script");
    let propaganda = game.create_card(CardInstance::from_rules(&propaganda, p1));
    game.move_card(propaganda, ZoneType::Battlefield, p1);

    let prompts = Rc::new(Cell::new(0));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(NoAttacks(Rc::clone(&prompts))),
        Box::new(PassAgent),
    ];
    game.turn.active_player = p0;
    GameLoop::new(2).step_combat(&mut game, &mut agents);

    assert_eq!(prompts.get(), 1);
    assert_eq!(game.player(p1).life, 20);
}
