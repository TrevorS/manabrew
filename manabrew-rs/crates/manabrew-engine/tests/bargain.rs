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

const TORCH: &str = "Name:Torch the Tower\nManaCost:R\nTypes:Instant\nK:Bargain\nA:SP$ DealDamage | ValidTgts$ Creature,Planeswalker | TgtPrompt$ Select target creature or planeswalker | NumDmg$ X | RememberDamaged$ True | ReplaceDyingDefined$ Remembered | SubAbility$ DBScry | SpellDescription$ CARDNAME deals 2 damage to target creature or planeswalker. If this spell was bargained, instead it deals 3 damage to that permanent and you scry 1. If a permanent dealt damage by CARDNAME would die this turn, exile it instead.\nSVar:DBScry:DB$ Scry | ScryNum$ 1 | Condition$ Bargain | SubAbility$ DBCleanup\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nSVar:X:Count$Bargain.3.2\nOracle:";
const MOUNTAIN: &str = "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:";
const ORNITHOPTER: &str =
    "Name:Ornithopter\nManaCost:0\nTypes:Artifact Creature Thopter\nPT:0/2\nK:Flying\nOracle:";
const GIANT: &str = "Name:Hill Giant\nManaCost:3 R\nTypes:Creature Giant\nPT:3/3\nOracle:";

#[derive(Default)]
struct Seen {
    cast: bool,
    offers: Vec<String>,
    sacrificed: Vec<CardId>,
    scries: usize,
}

struct Bargainer {
    accept: bool,
    seen: Rc<RefCell<Seen>>,
}

impl PlayerAgent for Bargainer {
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
        player: PlayerId,
        space: Option<&PriorityActionSpace>,
        priority: &mut dyn PriorityContext,
    ) -> PlayerAction {
        if self.seen.borrow().cast {
            return PassAgent.choose_action(player, space, priority);
        }
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let Some(&play) = space.playable.first() else {
            return PlayerAction::PassPriority;
        };
        self.seen.borrow_mut().cast = true;
        PlayerAction::CastSpell(play)
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
        _context: DecisionContext<'_>,
        _player: PlayerId,
        valid: &[PlayerId],
        _sa: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        valid.first().copied()
    }
    fn choose_target_card(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        valid: &[CardId],
        _sa: Option<&SpellAbility>,
    ) -> Option<CardId> {
        valid
            .iter()
            .copied()
            .find(|&cid| context.game.card(cid).controller != player)
    }
    fn choose_target_any(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
        _players: &[PlayerId],
        cards: &[CardId],
        sa: Option<&SpellAbility>,
    ) -> TargetChoice {
        match self.choose_target_card(context, player, cards, sa) {
            Some(card) => TargetChoice::Card(card),
            None => TargetChoice::None,
        }
    }
    fn choose_land_or_spell(
        &mut self,
        context: DecisionContext<'_>,
        player: PlayerId,
    ) -> Option<bool> {
        PassAgent.choose_land_or_spell(context, player)
    }
    fn choose_kicker(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        kicker_cost: &str,
        _source: Option<CardId>,
    ) -> bool {
        self.seen.borrow_mut().offers.push(kicker_cost.to_string());
        self.accept
    }
    fn choose_permanents_to_sacrifice(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        min: usize,
        _max: usize,
        valid: &[CardId],
        _source: Option<CardId>,
    ) -> Vec<CardId> {
        let picks = valid[..min].to_vec();
        self.seen.borrow_mut().sacrificed.extend(&picks);
        picks
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
    fn choose_scry(
        &mut self,
        _game: &GameState,
        _player: PlayerId,
        _source: Option<CardId>,
        cards: &[CardId],
    ) -> Vec<Vec<CardId>> {
        self.seen.borrow_mut().scries += 1;
        vec![cards.to_vec(), vec![]]
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

struct Board {
    game: GameState,
    giant: CardId,
    thopter: Option<CardId>,
}

fn board(with_thopter: bool) -> Board {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, TORCH, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, MOUNTAIN, p0, ZoneType::Library);
    let thopter = with_thopter.then(|| put(&mut game, ORNITHOPTER, p0, ZoneType::Battlefield));
    let giant = put(&mut game, GIANT, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    Board {
        game,
        giant,
        thopter,
    }
}

fn cast(board: &mut Board, accept: bool) -> Rc<RefCell<Seen>> {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Bargainer {
            accept,
            seen: Rc::clone(&seen),
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut board.game, &mut agents, true);
    seen
}

#[test]
fn a_bargained_torch_sacrifices_the_artifact_deals_three_and_scries() {
    let mut board = board(true);
    let seen = cast(&mut board, true);
    let seen = seen.borrow();

    assert_eq!(seen.offers, ["Bargain"]);
    assert_eq!(seen.sacrificed, [board.thopter.expect("thopter")]);
    assert_eq!(
        board.game.card(board.thopter.unwrap()).zone,
        ZoneType::Graveyard
    );
    assert_eq!(board.game.card(board.giant).zone, ZoneType::Exile);
    assert_eq!(seen.scries, 1);
}

#[test]
fn a_declined_bargain_deals_two_and_does_not_scry() {
    let mut board = board(true);
    let seen = cast(&mut board, false);
    let seen = seen.borrow();

    assert_eq!(seen.offers, ["Bargain"]);
    assert!(seen.sacrificed.is_empty());
    assert_eq!(
        board.game.card(board.thopter.unwrap()).zone,
        ZoneType::Battlefield
    );
    assert_eq!(board.game.card(board.giant).zone, ZoneType::Battlefield);
    assert_eq!(board.game.card(board.giant).damage, 2);
    assert_eq!(seen.scries, 0);
}

#[test]
fn bargain_is_not_offered_without_an_artifact_enchantment_or_token() {
    let mut board = board(false);
    let seen = cast(&mut board, true);
    let seen = seen.borrow();

    assert!(seen.offers.is_empty());
    assert_eq!(board.game.card(board.giant).damage, 2);
}
