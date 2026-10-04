use std::cell::RefCell;
use std::rc::Rc;

use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayCardMode, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::{ActionSpaceManaProbe, ManaPool};
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::SpellAbility;

const ROOM: &str = "Name:Quiet Annex\nManaCost:B\nTypes:Enchantment Room\nAlternateMode:Split\nOracle:\n\nALTERNATE\n\nName:Loud Chamber\nManaCost:1 B\nTypes:Enchantment Room\nOracle:";
const SWAMP: &str = "Name:Swamp\nManaCost:no cost\nTypes:Basic Land Swamp\nOracle:";
const SPELL_TABLET: &str = "Name:Spell Tablet\nManaCost:2\nTypes:Artifact\nA:AB$ Mana | Cost$ T | Produced$ B | Amount$ 2 | RestrictValid$ Spell.Instant,Spell.Sorcery | SpellDescription$ Add {B}{B}. Spend this mana only to cast instant and sorcery spells.\nOracle:";

struct Watch {
    unlocks: Rc<RefCell<Option<usize>>>,
}

impl PlayerAgent for Watch {
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
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
        let mut unlocks = self.unlocks.borrow_mut();
        if unlocks.is_none() {
            *unlocks = Some(
                space
                    .playable
                    .iter()
                    .filter(|play| play.mode == PlayCardMode::UnlockDoor)
                    .count(),
            );
        }
        PassAgent.choose_action(player, Some(space), priority)
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
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn unlocks_offered(sources: &[&str], probe: ActionSpaceManaProbe) -> usize {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, ROOM, p0, ZoneType::Battlefield);
    for script in sources {
        put(&mut game, script, p0, ZoneType::Battlefield);
    }
    game.action_space_mana_probe = probe;
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let unlocks = Rc::new(RefCell::new(None));
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(Watch {
            unlocks: Rc::clone(&unlocks),
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let offered = unlocks.borrow().expect("p0 got priority");
    offered
}

#[test]
fn mana_for_instants_and_sorceries_does_not_pay_to_unlock_a_door() {
    for probe in [
        ActionSpaceManaProbe::AutoPay,
        ActionSpaceManaProbe::ComputerUtilMana,
    ] {
        assert_eq!(unlocks_offered(&[SWAMP, SWAMP], probe), 2);
        assert_eq!(unlocks_offered(&[SPELL_TABLET], probe), 0);
    }
}
