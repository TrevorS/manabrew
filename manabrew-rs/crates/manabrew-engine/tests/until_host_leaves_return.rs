use forge_carddb::parse_card_script;
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
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::SpellAbility;

const MOUNTAIN: &str = "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:";
const CELL: &str = "Name:Holding Cell\nManaCost:1 W\nTypes:Enchantment\nOracle:";
const SHATTER: &str = "Name:Cell Break\nManaCost:R\nTypes:Sorcery\nA:SP$ Destroy | ValidTgts$ Enchantment | TgtPrompt$ Select target enchantment | SpellDescription$ Destroy target enchantment.\nOracle:";
const SHELTER: &str = "Name:Shelter Aura\nManaCost:W\nTypes:Enchantment Aura\nK:Enchant:Creature\nA:SP$ Attach | Cost$ W | ValidTgts$ Creature | AILogic$ Pump\nOracle:";
const SHATTER_CREATURE: &str = "Name:Crush\nManaCost:R\nTypes:Sorcery\nA:SP$ Destroy | ValidTgts$ Creature | TgtPrompt$ Select target creature | SpellDescription$ Destroy target creature.\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const MIND_COPIER: &str = "Name:Mind Copier\nManaCost:2 U\nTypes:Creature Human\nPT:2/2\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | RememberCloneOrigin$ True | SubAbility$ DBImmediateTrig | SpellDescription$ You may have CARDNAME enter as a copy of a creature card in a graveyard. When you do, exile that card.\nSVar:DBImmediateTrig:DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ GE1 | Execute$ TrigExile | RememberObjects$ RememberedCard | SubAbility$ DBCleanup | TriggerDescription$ When you do, exile that card.\nSVar:TrigExile:DB$ ChangeZone | Defined$ DelayTriggerRememberedLKI | Origin$ Graveyard | Destination$ Exile\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nOracle:";

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
        let requested;
        let space = match space {
            Some(space) => space,
            None => {
                requested = priority.action_space();
                &requested
            }
        };
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
fn a_card_returned_when_its_exiler_leaves_runs_its_enters_replacement_with_the_game() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let shatter = put(&mut game, SHATTER, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let cell = put(&mut game, CELL, p0, ZoneType::Battlefield);
    let bears = put(&mut game, BEARS, p1, ZoneType::Graveyard);
    let copier = put(&mut game, MIND_COPIER, p1, ZoneType::Exile);
    game.card_mut(copier).exiled_by = Some(cell);
    game.card_mut(copier).until_host_leaves_origin = Some(ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: shatter,
            cast: false,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(cell).zone, ZoneType::Graveyard);
    assert_eq!(game.card(copier).zone, ZoneType::Battlefield);
    assert_eq!(game.card(copier).card_name, "Grizzly Bears");
    assert_eq!(game.card(bears).zone, ZoneType::Exile);
}

#[test]
fn a_card_returned_when_an_aura_exiler_dies_to_the_state_check_runs_its_enters_replacement_with_the_game(
) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let crush = put(&mut game, SHATTER_CREATURE, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let shelter = put(&mut game, SHELTER, p0, ZoneType::Battlefield);
    game.card_mut(shelter).attached_to = Some(host);
    game.card_mut(host).attachments.push(shelter);
    let copier = put(&mut game, MIND_COPIER, p1, ZoneType::Exile);
    game.card_mut(copier).exiled_by = Some(shelter);
    game.card_mut(copier).until_host_leaves_origin = Some(ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: crush,
            cast: false,
        }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(shelter).zone, ZoneType::Graveyard);
    assert_eq!(game.card(copier).zone, ZoneType::Battlefield);
    assert_eq!(game.card(copier).card_name, "Grizzly Bears");
    assert_eq!(game.card(host).zone, ZoneType::Exile);
}
