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
use manabrew_engine::staticability::static_ability_continuous::{can_play, may_play_grants};

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

const SUPERIOR_SPIDER_MAN: &str = "Name:Superior Spider-Man\nManaCost:2 U B\nTypes:Legendary Creature Spider Human Hero\nPT:4/4\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | NewName$ Superior Spider-Man | AddTypes$ Spider & Human & Hero | SetPower$ 4 | SetToughness$ 4 | RememberCloneOrigin$ True | SubAbility$ DBImmediateTrig | SpellDescription$ Mind Swap — You may have CARDNAME enter as a copy of any creature card in a graveyard, except his name is Superior Spider-Man and he's a 4/4 Spider Human Hero in addition to his other types. When you do, exile that card.\nSVar:DBImmediateTrig:DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ GE1 | Execute$ TrigExile | RememberObjects$ RememberedCard | SubAbility$ DBCleanup | TriggerDescription$ When you do, exile that card.\nSVar:TrigExile:DB$ ChangeZone | Defined$ DelayTriggerRememberedLKI | Origin$ Graveyard | Destination$ Exile\nSVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\nOracle:Mind Swap — You may have Superior Spider-Man enter as a copy of any creature card in a graveyard, except his name is Superior Spider-Man and he's a 4/4 Spider Human Hero in addition to his other types. When you do, exile that card.";
const VALGAVOTH: &str = "Name:Valgavoth, Terror Eater\nManaCost:6 B B B\nTypes:Legendary Creature Elder Demon\nPT:9/9\nK:Flying\nK:Lifelink\nK:Ward:Sac<3/Permanent.nonLand/nonland permanent>\nR:Event$ Moved | ActiveZones$ Battlefield | Destination$ Graveyard | ValidLKI$ Card.!token+YouDontCtrl+OppOwn | ReplaceWith$ Exile | Description$ If a card you didn't control would be put into an opponent's graveyard from anywhere, exile it instead.\nSVar:Exile:DB$ ChangeZone | Hidden$ True | Origin$ All | Destination$ Exile | Defined$ ReplacedCard\nS:Mode$ Continuous | Condition$ PlayerTurn | MayPlay$ True | Affected$ Card.ExiledWithSource | AffectedZone$ Exile | MayPlayAltManaCost$ PayLife<ConvertedManaCost> | Description$ During your turn, you may play cards exiled with NICKNAME. If you cast a spell this way, pay life equal to its mana value rather than pay its mana cost.\nOracle:Flying, lifelink\\nWard—Sacrifice three nonland permanents.\\nIf a card you didn't control would be put into an opponent's graveyard from anywhere, exile it instead.\\nDuring your turn, you may play cards exiled with Valgavoth. If you cast a spell this way, pay life equal to its mana value rather than pay its mana cost.";
const SWAMP: &str = "Name:Swamp\nManaCost:no cost\nTypes:Basic Land Swamp\nOracle:";

#[test]
fn a_copy_may_play_only_the_cards_its_copied_abilities_exiled() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let spell = put(&mut game, SUPERIOR_SPIDER_MAN, p0, ZoneType::Hand);
    let valgavoth = put(&mut game, VALGAVOTH, p1, ZoneType::Graveyard);
    for land in [ISLAND, ISLAND, SWAMP, SWAMP] {
        put(&mut game, land, p0, ZoneType::Battlefield);
    }
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    for _ in 0..4 {
        if game.card(valgavoth).zone == ZoneType::Exile {
            break;
        }
        game_loop.step_with_priority(&mut game, &mut agents, true);
    }
    assert_eq!(game.card(valgavoth).zone, ZoneType::Exile);
    let shock = put(&mut game, SHOCK, p1, ZoneType::Hand);
    game.move_card_with_agents(shock, ZoneType::Graveyard, p1, &mut agents);
    assert_eq!(game.card(shock).zone, ZoneType::Exile);

    let may_play = |game: &GameState, card: CardId| {
        may_play_grants(game, p0, game.card(card))
            .any(|(source, st_ab)| can_play(st_ab, source, game.card(card), game))
    };
    assert!(!may_play(&game, valgavoth));
    assert!(may_play(&game, shock));
}

const DARK_IMPOSTOR: &str = "Name:Dark Impostor\nManaCost:2 B\nTypes:Creature Vampire Assassin\nPT:2/2\nS:Mode$ Continuous | AffectedDefined$ Self | EffectZone$ Battlefield | GainsAbilitiesOf$ Creature.ExiledWithSource | GainsAbilitiesOfZones$ Exile | Description$ CARDNAME has all activated abilities of all creature cards exiled with it.\nA:AB$ ChangeZone | Cost$ 4 B B | ValidTgts$ Creature | Origin$ Battlefield | Destination$ Exile | SubAbility$ DBCounter | SpellDescription$ Exile target creature and put a +1/+1 counter on CARDNAME.\nSVar:DBCounter:DB$ PutCounter | CounterType$ P1P1 | CounterNum$ 1 | Defined$ Self\nOracle:{4}{B}{B}: Exile target creature and put a +1/+1 counter on Dark Impostor.\\nDark Impostor has all activated abilities of all creature cards exiled with it.";
const SORCERER: &str = "Name:Prodigal Sorcerer\nManaCost:2 U\nTypes:Creature Human Wizard\nPT:1/1\nA:AB$ DealDamage | Cost$ T | ValidTgts$ Any | NumDmg$ 1 | SpellDescription$ CARDNAME deals 1 damage to any target.\nOracle:";

#[test]
fn a_copy_gains_the_abilities_of_the_cards_its_copied_ability_exiled() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let spell = put(&mut game, GRAVE_MIMIC, p0, ZoneType::Hand);
    let impostor = put(&mut game, DARK_IMPOSTOR, p0, ZoneType::Graveyard);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    let sorcerer = put(&mut game, SORCERER, p1, ZoneType::Battlefield);
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
    assert_eq!(ability.original_host, Some(impostor));
    let mut sa = SpellAbility::new_simple(Some(spell), p0, &ability.ability_text);
    sa.set_original_host(impostor);
    sa.target_chosen.target_card = Some(sorcerer);
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
    assert_eq!(game.card(sorcerer).zone, ZoneType::Exile);
    assert!(game
        .card(spell)
        .activated_abilities
        .iter()
        .any(|ability| ability.ability_text.contains("DealDamage")));
}

const FLYING_MIMIC: &str = "Name:Flying Mimic\nManaCost:U\nTypes:Creature Bird Bard\nPT:1/1\nK:Flying\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | AddTypes$ Bird | AddKeywords$ Flying | SpellDescription$ You may have CARDNAME enter as a copy of a creature on the battlefield, except it's a Bird in addition to its other types and it has flying.\nOracle:";
const BEAR: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";

#[test]
fn a_token_copy_of_a_copy_keeps_the_copy_s_exceptions() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, FLYING_MIMIC, p0, ZoneType::Hand);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    put(&mut game, BEAR, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(spell).card_name, "Grizzly Bears");
    assert!(game.card(spell).has_keyword("Flying"));
    let mut sa = SpellAbility::new_simple(Some(spell), p0, "DB$ CopyPermanent | Defined$ Self");
    sa.activating_player = p0;
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
    let token = game
        .zone(ZoneType::Battlefield, p0)
        .cards
        .iter()
        .copied()
        .find(|&id| game.card(id).is_token)
        .expect("token copy");
    assert_eq!(game.card(token).card_name, "Grizzly Bears");
    assert!(game.card(token).has_keyword("Flying"));
    assert!(game.card(token).type_line.has_subtype("Bird"));
}

struct CountingRng(std::rc::Rc<std::cell::Cell<usize>>);

impl manabrew_engine::game_rng::GameRng for CountingRng {
    fn shuffle_cards(&mut self, _cards: &mut [CardId]) {}

    fn next_int(&mut self, _bound: i32) -> i32 {
        self.0.set(self.0.get() + 1);
        0
    }
}

fn copy_permanent_draws(copy_the_clone: bool) -> usize {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, FLYING_MIMIC, p0, ZoneType::Hand);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    let token = put(&mut game, BEAR, p0, ZoneType::Battlefield);
    game.card_mut(token).is_token = true;
    game.card_mut(token)
        .svars
        .insert("TokenScript".to_string(), "g_2_2_bear".to_string());
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(spell).card_name, "Grizzly Bears");
    let draws = std::rc::Rc::new(std::cell::Cell::new(0));
    game_loop.game_rng = Box::new(CountingRng(draws.clone()));
    let source = if copy_the_clone { spell } else { token };
    let mut sa = SpellAbility::new_simple(Some(source), p0, "DB$ CopyPermanent | Defined$ Self");
    sa.activating_player = p0;
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
    draws.get()
}

#[test]
fn a_token_copy_draws_for_its_art_only_when_the_original_is_a_token() {
    assert_eq!(copy_permanent_draws(false), 1);
    assert_eq!(copy_permanent_draws(true), 0);
}

const TWO_FACED_SYGG: &str = "Name:Front Sygg\nManaCost:1 U\nTypes:Creature Merfolk Wizard\nPT:2/2\nT:Mode$ Phase | Phase$ Main1 | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigTransform | TriggerDescription$ At the beginning of your first main phase, you may pay {W}. If you do, transform NICKNAME.\nSVar:TrigTransform:AB$ SetState | Cost$ W | Defined$ Self | Mode$ Transform\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Back Sygg\nManaCost:no cost\nColors:white\nTypes:Creature Merfolk Rogue\nPT:2/2\nT:Mode$ Phase | Phase$ Main1 | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigTransform | TriggerDescription$ At the beginning of your first main phase, you may pay {U}. If you do, transform NICKNAME.\nSVar:TrigTransform:AB$ SetState | Cost$ U | Defined$ Self | Mode$ Transform\nOracle:";

#[test]
fn a_copy_of_a_transformed_card_keeps_the_face_it_copied_after_the_original_turns_back() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let sygg = put(&mut game, TWO_FACED_SYGG, p0, ZoneType::Battlefield);
    game.card_mut(sygg).transform();
    let spell = put(&mut game, GREAT_MIMIC, p0, ZoneType::Hand);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(spell).card_name, "Back Sygg");
    game.card_mut(sygg).transform();
    assert_eq!(game.card(sygg).card_name, "Front Sygg");
    let trigger = game.card(spell).triggers[0].clone();
    let sa = trigger.build_triggered_spell_ability(
        &game,
        spell,
        p0,
        0,
        &manabrew_engine::event::RunParams::default(),
    );
    assert!(sa.ability_text.contains("Cost$ U"), "{}", sa.ability_text);
}

const DAY_SHIFTER: &str = "Name:Day Shifter\nManaCost:1 G\nTypes:Creature Human\nPT:2/2\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Shifter\nManaCost:no cost\nTypes:Creature Wolf\nPT:4/4\nOracle:";

#[test]
fn a_token_copy_of_a_transforming_card_keeps_its_added_type_on_the_back_face() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let shifter = put(&mut game, DAY_SHIFTER, p0, ZoneType::Graveyard);
    let sa = SpellAbility::new_simple(
        None,
        p0,
        "DB$ CopyPermanent | Defined$ Remembered | AddTypes$ Nightmare",
    );
    let mut copy = manabrew_engine::ability::effects::copy_permanent_effect::get_proto_type(
        &sa,
        game.card(shifter),
        p0,
    );
    assert!(copy.type_line.has_subtype("Nightmare"));
    copy.transform();
    assert_eq!(copy.card_name, "Night Shifter");
    assert!(copy.type_line.has_subtype("Nightmare"));
}

const PROWESS_MIMIC: &str = "Name:Prowess Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:1/1\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | AddKeywords$ Prowess | SpellDescription$ You may have CARDNAME enter as a copy of a creature, except it has prowess.\nOracle:";
const SPELL_WATCHER: &str = "Name:Spell Watcher\nManaCost:1 U\nTypes:Creature Human Wizard\nPT:1/3\nT:Mode$ SpellCast | ValidCard$ Card.nonCreature | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever you cast a noncreature spell, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const MIRROR_MIMIC: &str = "Name:Mirror Mimic\nManaCost:U\nTypes:Creature Shapeshifter\nPT:1/1\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | NonLegendary$ True | SpellDescription$ You may have CARDNAME enter as a copy of a creature, except it isn't legendary.\nOracle:";
const LEGEND: &str =
    "Name:Lone Legend\nManaCost:1 W\nTypes:Legendary Creature Human\nPT:2/2\nOracle:";

fn clone_onto_battlefield(mimic: &str, original: &str) -> (GameState, CardId, CardId) {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let spell = put(&mut game, mimic, p0, ZoneType::Hand);
    let original = put(&mut game, original, p0, ZoneType::Battlefield);
    put(&mut game, ISLAND, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce { spell, cast: false }),
        Box::new(PassAgent),
    ];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    (game, spell, original)
}

#[test]
fn a_clone_that_adds_a_triggered_keyword_orders_its_trigger_after_the_copied_ones() {
    let (game, copy, _) = clone_onto_battlefield(PROWESS_MIMIC, SPELL_WATCHER);
    let copy = game.card(copy);
    assert_eq!(copy.card_name, "Spell Watcher");
    let ids = |keyword: bool| -> Vec<u32> {
        copy.triggers
            .iter()
            .filter(|trigger| trigger.base.card_trait_base.get_keyword().is_some() == keyword)
            .map(|trigger| trigger.id)
            .collect()
    };
    let (printed, prowess) = (ids(false), ids(true));
    assert_eq!((printed.len(), prowess.len()), (1, 1));
    assert!(printed[0] < prowess[0], "{printed:?} {prowess:?}");
    let other_ids: Vec<u32> = game
        .cards
        .iter()
        .filter(|card| card.id != copy.id)
        .flat_map(|card| card.triggers.iter().map(|trigger| trigger.id))
        .collect();
    assert!(!other_ids.contains(&prowess[0]));
}

#[test]
fn a_clone_that_is_not_legendary_keeps_the_legend_it_copied() {
    let (game, copy, original) = clone_onto_battlefield(MIRROR_MIMIC, LEGEND);
    assert_eq!(game.card(copy).card_name, "Lone Legend");
    assert!(!game.card(copy).type_line.is_legendary());
    assert_eq!(game.card(copy).zone, ZoneType::Battlefield);
    assert_eq!(game.card(original).zone, ZoneType::Battlefield);
    let clone_state = game.card(copy).clone_state.as_ref().expect("clone state");
    assert_eq!(clone_state.origin, Some(original));
    assert!(clone_state
        .source
        .as_ref()
        .is_some_and(|source| source.ability_text.contains("NonLegendary")));
}

#[test]
fn a_token_copy_sets_power_only_on_something_with_power() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let keeper = put(&mut game, SHOAL_KEEPER, p0, ZoneType::Battlefield);
    let stone = put(&mut game, STONE, p0, ZoneType::Battlefield);
    let sa = SpellAbility::new_simple(
        None,
        p0,
        "DB$ CopyPermanent | Defined$ Remembered | SetPower$ 4 | SetToughness$ 4",
    );
    let get_proto_type = manabrew_engine::ability::effects::copy_permanent_effect::get_proto_type;
    let creature = get_proto_type(&sa, game.card(keeper), p0);
    assert_eq!(
        (creature.base_power, creature.base_toughness),
        (Some(4), Some(4))
    );
    assert!(creature
        .static_abilities
        .iter()
        .all(|sta| !sta.ir.characteristic_defining));
    let artifact = get_proto_type(&sa, game.card(stone), p0);
    assert_eq!((artifact.base_power, artifact.base_toughness), (None, None));
    assert_eq!(
        creature
            .token_spawning_ability
            .as_ref()
            .map(|sa| sa.ability_text.as_str()),
        Some(sa.ability_text.as_str())
    );
}

#[test]
fn a_token_copy_of_a_transforming_card_takes_its_new_name_on_both_faces() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let shifter = put(&mut game, DAY_SHIFTER, p0, ZoneType::Graveyard);
    let sa = SpellAbility::new_simple(
        None,
        p0,
        "DB$ CopyPermanent | Defined$ Remembered | NewName$ Shifter Echo | SetPower$ 1 | SetToughness$ 1",
    );
    let mut copy = manabrew_engine::ability::effects::copy_permanent_effect::get_proto_type(
        &sa,
        game.card(shifter),
        p0,
    );
    assert_eq!(copy.card_name, "Shifter Echo");
    copy.transform();
    assert_eq!(copy.card_name, "Shifter Echo");
    assert_eq!((copy.base_power, copy.base_toughness), (Some(1), Some(1)));
}
