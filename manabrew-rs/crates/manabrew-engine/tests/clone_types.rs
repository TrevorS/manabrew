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
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const ISLAND: &str = "Name:Island\nManaCost:no cost\nTypes:Basic Land Island\nOracle:";
const STONE: &str = "Name:Glass Stone\nManaCost:2\nTypes:Artifact\nOracle:";
const SHOAL_KEEPER: &str = "Name:Shoal Keeper\nManaCost:1 U\nTypes:Creature Merfolk\nPT:*/4\nS:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | Description$ CARDNAME's power is equal to the number of Merfolk you control.\nSVar:X:Count$Valid Merfolk.YouCtrl\nOracle:";
const GREAT_MIMIC: &str = "Name:Great Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:4/4\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | SetPower$ 4 | SetToughness$ 4 | SpellDescription$ You may have CARDNAME enter as a copy of a creature, except it's 4/4.\nOracle:";
const WAR_SHIFTER: &str =
    "Name:War Shifter\nManaCost:1 G\nTypes:Creature Elf Warrior\nPT:2/2\nK:Changeling\nOracle:";
const DEMON_MIMIC: &str = "Name:Demon Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:1/1\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | SetCreatureTypes$ Demon | SpellDescription$ You may have CARDNAME enter as a copy of a creature, except it's a Demon.\nOracle:";
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

#[test]
fn a_copy_that_sets_creature_types_has_only_those() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, DEMON_MIMIC, p0, ZoneType::Hand);
    put(&mut game, WAR_SHIFTER, p0, ZoneType::Battlefield);
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
    assert_eq!(copy.card_name, "War Shifter");
    assert_eq!(copy.type_line.subtypes, vec!["Demon".to_string()]);
    assert!(!copy.has_keyword("Changeling"));
}

#[test]
fn a_token_copy_that_sets_creature_types_has_only_those() {
    let type_lists = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../forge/forge-gui/res/lists/TypeLists.txt"
    ))
    .expect("TypeLists.txt");
    TypeRegistry::load(&type_lists, []);
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let shifter = put(&mut game, WAR_SHIFTER, p0, ZoneType::Graveyard);
    let sa = SpellAbility::new_simple(
        None,
        p0,
        "DB$ CopyPermanent | Defined$ Remembered | SetPower$ 5 | SetToughness$ 5 | SetColor$ Black | SetCreatureTypes$ Demon",
    );
    let copy = manabrew_engine::ability::effects::copy_permanent_effect::get_proto_type(
        &sa,
        game.card(shifter),
        p0,
    );
    assert_eq!(copy.type_line.subtypes, vec!["Demon".to_string()]);
    assert!(!copy.has_keyword("Changeling"));
}

const GRAVE_MIMIC: &str = "Name:Grave Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:1/1\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | SpellDescription$ You may have CARDNAME enter as a copy of a creature card in a graveyard.\nOracle:";
const CURATOR: &str = "Name:Ledger Curator\nManaCost:G G\nTypes:Creature Raccoon Scout\nPT:3/3\nS:Mode$ Continuous | AffectedDefined$ Self | AddPower$ 4 | AddToughness$ 4 | CheckSVar$ X | SVarCompare$ GE4 | Description$ As long as there are four or more card types among cards exiled with CARDNAME, it gets +4/+4.\nA:AB$ ChangeZone | Cost$ 1 | Origin$ Graveyard | Destination$ Exile | ValidTgts$ Card | TgtPrompt$ Choose target card in a graveyard | SpellDescription$ Exile target card from a graveyard.\nSVar:X:Count$ValidExile Card.ExiledWithSource$CardTypes\nOracle:";
const SHOCK: &str =
    "Name:Shock\nManaCost:R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 2\nOracle:";
const DIVINATION: &str =
    "Name:Divination\nManaCost:2 U\nTypes:Sorcery\nA:SP$ Draw | NumCards$ 2\nOracle:";

#[test]
fn a_copy_counts_the_cards_its_copied_ability_exiled() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let spell = put(&mut game, GRAVE_MIMIC, p0, ZoneType::Hand);
    let curator = put(&mut game, CURATOR, p0, ZoneType::Graveyard);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    let exiled: Vec<CardId> = [STONE, ISLAND, SHOCK, DIVINATION]
        .into_iter()
        .map(|script| put(&mut game, script, p1, ZoneType::Graveyard))
        .collect();
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    let ability = game.card(spell).activated_abilities[0].clone();
    assert_eq!(ability.original_host, Some(curator));
    for card in exiled {
        let mut sa = SpellAbility::new_simple(Some(spell), p0, &ability.ability_text);
        sa.set_original_host(curator);
        sa.target_chosen.target_card = Some(card);
        game.stack.push(StackEntry {
            id: 0,
            spell_ability: sa,
            is_creature_spell: false,
            is_permanent_spell: false,
            is_pending_cast: false,
            cast_from_zone: None,
            optional_trigger_decider: None,
            optional_trigger_description: None,
            optional_trigger_source_name: None,
        });
        game_loop.step_with_priority(&mut game, &mut agents, true);
        assert_eq!(game.card(card).zone, ZoneType::Exile);
    }
    let copy = game.card(spell);
    assert_eq!(copy.card_name, "Ledger Curator");
    assert_eq!((copy.power(), copy.toughness()), (7, 7));
}

#[test]
fn a_curator_that_left_and_came_back_counts_none_of_its_earlier_exiles() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let curator = put(&mut game, CURATOR, p0, ZoneType::Battlefield);
    let exiled: Vec<CardId> = [STONE, ISLAND, SHOCK, DIVINATION]
        .into_iter()
        .map(|script| put(&mut game, script, p1, ZoneType::Graveyard))
        .collect();
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    let ability = game.card(curator).activated_abilities[0].clone();
    for card in exiled {
        let mut sa = SpellAbility::new_simple(Some(curator), p0, &ability.ability_text);
        sa.target_chosen.target_card = Some(card);
        game.stack.push(StackEntry {
            id: 0,
            spell_ability: sa,
            is_creature_spell: false,
            is_permanent_spell: false,
            is_pending_cast: false,
            cast_from_zone: None,
            optional_trigger_decider: None,
            optional_trigger_description: None,
            optional_trigger_source_name: None,
        });
        game_loop.step_with_priority(&mut game, &mut agents, true);
    }
    assert_eq!(game.card(curator).power(), 7);
    game.move_card(curator, ZoneType::Hand, p0);
    game.move_card(curator, ZoneType::Battlefield, p0);
    manabrew_engine::staticability::layer::apply_continuous_effects(&mut game);
    let curator = game.card(curator);
    assert_eq!((curator.power(), curator.toughness()), (3, 3));
}
