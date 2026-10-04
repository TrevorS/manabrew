use forge_carddb::parse_card_script;
use forge_foundation::mana::ManaAtom;
use forge_foundation::{PhaseType, ZoneType};
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
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};
use manabrew_engine::spellability::SpellAbility;

const PAINFUL_SPRING: &str = "Name:Painful Spring\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T PayLife<1> | Produced$ R | SpellDescription$ Add {R}. Pay 1 life.\nOracle:";
const BLOOD_SPRING: &str = "Name:Blood Spring\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T PayLife<1> | Produced$ B | SpellDescription$ Add {B}.\nOracle:";
const DULL_STONE: &str = "Name:Dull Stone\nManaCost:no cost\nTypes:Land\nA:AB$ Mana | Cost$ T | Produced$ C | SpellDescription$ Add {C}.\nOracle:";
const COSTLY_ENGINE: &str = "Name:Costly Engine\nManaCost:1\nTypes:Artifact\nA:AB$ Draw | Cost$ 1 B PayLife<2> | NumCards$ 1 | SpellDescription$ Draw a card.\nOracle:";
const RENEWAL: &str = "Name:Small Renewal\nManaCost:R\nTypes:Sorcery\nA:SP$ GainLife | Defined$ You | LifeAmount$ 2 | SpellDescription$ You gain 2 life.\nOracle:";
const BLOOD_WATCHER: &str = "Name:Blood Watcher\nManaCost:1 B\nTypes:Creature Vampire\nPT:1/1\nT:Mode$ LifeLost | ValidPlayer$ Opponent | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever an opponent loses life, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";

struct CastOnce {
    spell: CardId,
    cast: bool,
}

impl PlayerAgent for CastOnce {
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
        if let Some(ability) = space
            .activatable
            .iter()
            .find(|ability| ability.card_id == self.spell && !self.cast)
        {
            self.cast = true;
            return PlayerAction::ActivateAbility(AbilityRef {
                card_id: ability.card_id,
                ability_index: ability.ability_index,
            });
        }
        let play = space
            .playable
            .iter()
            .find(|play| play.card_id == self.spell)
            .copied();
        match play {
            Some(play) if !self.cast => {
                self.cast = true;
                PlayerAction::CastSpell(play)
            }
            _ => PassAgent.choose_action(player, Some(space), priority),
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

#[test]
fn life_paid_for_mana_is_life_lost() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let spell = put(&mut game, RENEWAL, p0, ZoneType::Hand);
    put(&mut game, PAINFUL_SPRING, p0, ZoneType::Battlefield);
    put(&mut game, BLOOD_WATCHER, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(spell).zone, ZoneType::Graveyard);
    assert_eq!(game.players[0].life, 21);
    assert_eq!(game.players[1].life, 21);
}

#[test]
fn a_failed_activation_keeps_the_life_paid_for_its_mana_and_floats_it() {
    let mut game = GameState::new(&["Alice", "Bob"], 2);
    let p0 = PlayerId(0);
    let engine = put(&mut game, COSTLY_ENGINE, p0, ZoneType::Battlefield);
    let spring = put(&mut game, BLOOD_SPRING, p0, ZoneType::Battlefield);
    let stone = put(&mut game, DULL_STONE, p0, ZoneType::Battlefield);
    put(&mut game, DULL_STONE, p0, ZoneType::Library);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: engine,
            cast: false,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.players[0].life, 1);
    assert!(game.card(spring).tapped);
    assert!(!game.card(stone).tapped);
    assert_eq!(game.zone(ZoneType::Library, p0).len(), 1);
    assert_eq!(game_loop.pool(p0).total_mana(), 1);
    assert_eq!(game_loop.pool(p0).count_color(ManaAtom::BLACK), 1);
}
