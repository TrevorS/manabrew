use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::{GameState, TypeRegistry};
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::SpellAbility;

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const STONE: &str = "Name:Glass Stone\nManaCost:2\nTypes:Artifact\nOracle:";
const SHOAL_KEEPER: &str = "Name:Shoal Keeper\nManaCost:1 U\nTypes:Creature Merfolk\nPT:*/4\nS:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | Description$ CARDNAME's power is equal to the number of Merfolk you control.\nSVar:X:Count$Valid Merfolk.YouCtrl\nOracle:";
const GREAT_MIMIC: &str = "Name:Great Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:4/4\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | SetPower$ 4 | SetToughness$ 4 | SpellDescription$ You may have CARDNAME enter as a copy of a creature, except it's 4/4.\nOracle:";
const BIRD_MIMIC: &str = "Name:Bird Mimic\nManaCost:U\nTypes:Creature Bird\nPT:1/1\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Artifact.Other | AddTypes$ Bird | SpellDescription$ You may have CARDNAME enter as a copy of an artifact, except it's a Bird in addition to its other types.\nOracle:";

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
fn a_creature_type_added_to_a_noncreature_copy_is_dropped() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, BIRD_MIMIC, p0, ZoneType::Hand);
    put(&mut game, STONE, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let copy = game.card(spell);
    assert_eq!(copy.zone, ZoneType::Battlefield);
    assert_eq!(copy.card_name, "Glass Stone");
    assert!(copy.type_line.subtypes.is_empty(), "{:?}", copy.type_line);
}

#[test]
fn a_copy_set_to_a_power_drops_the_copied_power_defining_ability() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, GREAT_MIMIC, p0, ZoneType::Hand);
    let keeper = put(&mut game, SHOAL_KEEPER, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    let copy = game.card(spell);
    assert_eq!(copy.zone, ZoneType::Battlefield);
    assert_eq!(copy.card_name, "Shoal Keeper");
    assert_eq!((copy.power(), copy.toughness()), (4, 4));
    assert_eq!(game.card(keeper).power(), 2);
}
