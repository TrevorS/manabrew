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
    fn choose_legend_keep(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        duplicates: &[CardId],
    ) -> CardId {
        *duplicates.last().expect("duplicates")
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

#[test]
fn a_card_exiled_until_a_token_leaves_returns_when_the_token_leaves() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let rules = parse_card_script(CELL).expect("script");
    let mut token = CardInstance::from_rules(&rules, p0);
    token.set_is_token(true);
    let token = game.create_card(token);
    game.move_card(token, ZoneType::Battlefield, p0);
    let bears = put(&mut game, BEARS, p1, ZoneType::Exile);
    game.card_mut(bears).exiled_by = Some(token);
    game.card_mut(bears).until_host_leaves_origin = Some(ZoneType::Battlefield);

    game.move_card(token, ZoneType::Graveyard, p0);

    assert_eq!(game.card(token).zone, ZoneType::None);
    assert_eq!(game.card(bears).zone, ZoneType::Battlefield);
}

#[test]
fn a_sorcery_card_exiled_until_its_exiler_leaves_stays_in_exile() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let cell = put(&mut game, CELL, p0, ZoneType::Battlefield);
    let sorcery = put(&mut game, SHATTER, p1, ZoneType::Exile);
    game.card_mut(sorcery).exiled_by = Some(cell);
    game.card_mut(sorcery).until_host_leaves_origin = Some(ZoneType::Battlefield);

    game.move_card(cell, ZoneType::Graveyard, p0);

    assert_eq!(game.card(sorcery).zone, ZoneType::Exile);
}

const LEAVE_WATCHER: &str = "Name:Leave Watcher\nManaCost:1 B\nTypes:Creature Human\nPT:1/1\nT:Mode$ ChangesZone | ValidCard$ Permanent.Other | Origin$ Battlefield | Destination$ Any | Execute$ TrigGain | TriggerZones$ Battlefield | TriggerDescription$ Whenever another permanent leaves the battlefield, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";

#[test]
fn a_card_returned_when_its_aura_exiler_leaves_does_not_see_the_aura_leave() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let crush = put(&mut game, SHATTER_CREATURE, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let shelter = put(&mut game, SHELTER, p0, ZoneType::Battlefield);
    game.card_mut(shelter).attached_to = Some(host);
    game.card_mut(host).attachments.push(shelter);
    let watcher = put(&mut game, LEAVE_WATCHER, p1, ZoneType::Exile);
    game.card_mut(watcher).exiled_by = Some(shelter);
    game.card_mut(watcher).until_host_leaves_origin = Some(ZoneType::Battlefield);
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
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(shelter).zone, ZoneType::Graveyard);
    assert_eq!(game.card(watcher).zone, ZoneType::Battlefield);
    assert!(game.stack.is_empty());
    assert_eq!(game.player(p1).life, 20);
}

const DEATH_WATCHER: &str = "Name:Death Watcher\nManaCost:1 B\nTypes:Legendary Creature Human\nPT:2/2\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.Other | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever another creature dies, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";

#[test]
fn a_legend_returned_during_state_checks_does_not_see_its_twin_die_to_the_legend_rule() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let p1 = PlayerId(1);
    let crush = put(&mut game, SHATTER_CREATURE, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let shelter = put(&mut game, SHELTER, p0, ZoneType::Battlefield);
    game.card_mut(shelter).attached_to = Some(host);
    game.card_mut(host).attachments.push(shelter);
    let watcher = put(&mut game, DEATH_WATCHER, p1, ZoneType::Battlefield);
    let twin = put(&mut game, DEATH_WATCHER, p1, ZoneType::Exile);
    game.card_mut(twin).exiled_by = Some(shelter);
    game.card_mut(twin).until_host_leaves_origin = Some(ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: crush,
            cast: false,
        }),
        Box::new(CastOnce {
            spell: crush,
            cast: true,
        }),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(twin).zone, ZoneType::Battlefield);
    assert_eq!(game.card(watcher).zone, ZoneType::Graveyard);
    assert!(game.stack.is_empty());
    assert_eq!(game.player(p1).life, 21);
}

const DIES_ALL_WATCHER: &str = "Name:Dies All Watcher\nManaCost:2\nTypes:Artifact\nT:Mode$ ChangesZoneAll | TriggerZones$ Battlefield | ValidCards$ Creature | Origin$ Battlefield | Destination$ Graveyard | Execute$ TrigGain | TriggerDescription$ Whenever one or more creatures die, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const LONE_LEGEND: &str =
    "Name:Lone Legend\nManaCost:1 W\nTypes:Legendary Creature Human\nPT:1/2\nOracle:";

fn life_after_a_returned_legend_dies_to_the_legend_rule(mirror_forge_bugs: bool) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    game.mirror_forge_bugs = mirror_forge_bugs;
    put(&mut game, DIES_ALL_WATCHER, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let shelter = put(&mut game, SHELTER, p1, ZoneType::Battlefield);
    game.card_mut(shelter).attached_to = Some(host);
    game.card_mut(host).attachments.push(shelter);
    let legend = put(&mut game, LONE_LEGEND, p0, ZoneType::Battlefield);
    let twin = put(&mut game, LONE_LEGEND, p0, ZoneType::Exile);
    game.card_mut(twin).exiled_by = Some(shelter);
    game.card_mut(twin).until_host_leaves_origin = Some(ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.card_mut(host).damage = 2;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(host).zone, ZoneType::Graveyard);
    assert_eq!(game.card(legend).zone, ZoneType::Battlefield);
    assert_eq!(game.card(twin).zone, ZoneType::Graveyard);
    assert!(game.stack.is_empty());
    game.player(p0).life
}

#[test]
fn a_creature_that_entered_during_the_state_check_is_seen_dying_in_it() {
    assert_eq!(
        life_after_a_returned_legend_dies_to_the_legend_rule(false),
        22
    );
}

#[test]
fn forge_misses_a_creature_dying_in_the_state_check_it_entered_during() {
    assert_eq!(
        life_after_a_returned_legend_dies_to_the_legend_rule(true),
        21
    );
}

const FRAIL_SHEPHERD: &str = "Name:Frail Shepherd\nManaCost:1 W\nTypes:Creature Human\nPT:1/1\nS:Mode$ Continuous | Affected$ Creature.Other+YouCtrl | AddToughness$ 1 | Description$ Other creatures you control get +0/+1.\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.Other+YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever another creature you control dies, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const HOLLOW_SPRITE: &str =
    "Name:Hollow Sprite\nManaCost:1\nTypes:Creature Spirit\nPT:0/0\nOracle:";

#[test]
fn a_creature_that_died_in_one_state_check_does_not_see_a_later_one() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let shepherd = put(&mut game, FRAIL_SHEPHERD, p0, ZoneType::Battlefield);
    let sprite = put(&mut game, HOLLOW_SPRITE, p0, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    game.card_mut(shepherd).damage = 1;
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(shepherd).zone, ZoneType::Graveyard);
    assert_eq!(game.card(sprite).zone, ZoneType::Graveyard);
    assert!(game.stack.is_empty());
    assert_eq!(game.player(p0).life, 20);
}

const GIADA: &str = "Name:Giada, Font of Hope\nManaCost:1 W\nTypes:Legendary Creature Angel\nPT:2/2\nK:Flying\nK:Vigilance\nK:ETBReplacement:Other:AddExtraCounter:Mandatory:Battlefield:Creature.Angel+YouCtrl+Other\nSVar:AddExtraCounter:DB$ PutCounter | ETB$ True | Defined$ ReplacedCard | CounterType$ P1P1 | CounterNum$ X | SpellDescription$ Each other Angel you control enters with an additional +1/+1 counter on it for each Angel you already control.\nSVar:X:Count$Valid Angel.YouCtrl\nA:AB$ Mana | Cost$ T | Produced$ W | RestrictValid$ Spell.Angel | SpellDescription$ Add {W}. Spend this mana only to cast an Angel spell.\nDeckHas:Ability$Counters\nDeckNeeds:Type$Angel\nOracle:Flying, vigilance\\nEach other Angel you control enters with an additional +1/+1 counter on it for each Angel you already control.\\n{T}: Add {W}. Spend this mana only to cast an Angel spell.";
const SKY_ANGEL: &str = "Name:Sky Angel\nManaCost:2 W\nTypes:Creature Angel\nPT:3/3\nOracle:";
const CRUSH_OPPONENT: &str = "Name:Crush Theirs\nManaCost:R\nTypes:Sorcery\nA:SP$ Destroy | ValidTgts$ Creature.OppCtrl | TgtPrompt$ Select target creature an opponent controls | SpellDescription$ Destroy target creature an opponent controls.\nOracle:";

#[test]
fn an_angel_returned_after_giada_in_the_same_state_check_gets_her_counter() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let crush = put(&mut game, CRUSH_OPPONENT, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let exile_under_aura = |game: &mut GameState, script: &str| {
        let aura = put(game, SHELTER, p1, ZoneType::Battlefield);
        game.card_mut(aura).attached_to = Some(host);
        game.card_mut(host).attachments.push(aura);
        let exiled = put(game, script, p0, ZoneType::Exile);
        game.card_mut(exiled).exiled_by = Some(aura);
        game.card_mut(exiled).until_host_leaves_origin = Some(ZoneType::Battlefield);
        exiled
    };
    let giada = exile_under_aura(&mut game, GIADA);
    let angel = exile_under_aura(&mut game, SKY_ANGEL);
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
    assert_eq!(game.card(giada).zone, ZoneType::Battlefield);
    assert_eq!(game.card(angel).zone, ZoneType::Battlefield);
    assert_eq!(
        game.card(angel)
            .counter_count(&manabrew_engine::card::CounterType::P1P1),
        1
    );
}

const SMALL_WATCHER: &str = "Name:Small Watcher\nManaCost:1 B\nTypes:Creature Dog\nPT:2/2\nT:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.powerLE2+YouCtrl+Other | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever another creature you control with power 2 or less enters, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const BIG_ANTHEM: &str = "Name:Big Anthem\nManaCost:2 W\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddPower$ 3 | Description$ Creatures you control get +3/+0.\nOracle:";
const SMALL_SOLDIER: &str =
    "Name:Small Soldier\nManaCost:W\nTypes:Creature Human Soldier\nPT:1/1\nOracle:";

#[test]
fn a_card_returned_by_an_aura_s_state_check_enters_before_the_statics_apply_again() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let crush = put(&mut game, CRUSH_OPPONENT, p0, ZoneType::Hand);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    put(&mut game, SMALL_WATCHER, p0, ZoneType::Battlefield);
    put(&mut game, BIG_ANTHEM, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    let shelter = put(&mut game, SHELTER, p1, ZoneType::Battlefield);
    game.card_mut(shelter).attached_to = Some(host);
    game.card_mut(host).attachments.push(shelter);
    let soldier = put(&mut game, SMALL_SOLDIER, p0, ZoneType::Exile);
    game.card_mut(soldier).exiled_by = Some(shelter);
    game.card_mut(soldier).until_host_leaves_origin = Some(ZoneType::Battlefield);
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
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(soldier).zone, ZoneType::Battlefield);
    assert_eq!(game.player(p0).life, 21);
}

const VOID_LEYLINE: &str = "Name:Void Leyline\nManaCost:2 B B\nTypes:Enchantment\nR:Event$ Moved | ActiveZones$ Battlefield | Destination$ Graveyard | ValidCard$ Card.!token+OppOwn | ReplaceWith$ Exile | Description$ If a card would be put into an opponent's graveyard from anywhere, exile it instead.\nSVar:Exile:DB$ ChangeZone | Hidden$ True | Origin$ All | Destination$ Exile | Defined$ ReplacedCard\nOracle:";

fn cleanup_discard_zone(mirror_forge_bugs: bool, play_land_first: bool) -> ZoneType {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    game.mirror_forge_bugs = mirror_forge_bugs;
    put(&mut game, VOID_LEYLINE, p1, ZoneType::Battlefield);
    let land = put(&mut game, MOUNTAIN, p0, ZoneType::Hand);
    let hand: Vec<CardId> = (0..8)
        .map(|_| put(&mut game, BEARS, p0, ZoneType::Hand))
        .collect();
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: land,
            cast: !play_land_first,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    if play_land_first {
        game.turn.phase = PhaseType::Main1;
        game_loop.step_with_priority(&mut game, &mut agents, true);
        assert_eq!(game.card(land).zone, ZoneType::Battlefield);
    } else {
        game.move_card(land, ZoneType::Battlefield, p0);
    }
    game.turn.phase = PhaseType::Cleanup;
    game_loop.step_cleanup(&mut game, &mut agents);
    let discarded: Vec<ZoneType> = hand
        .iter()
        .map(|&card| game.card(card).zone)
        .filter(|&zone| zone != ZoneType::Hand)
        .collect();
    assert_eq!(discarded.len(), 1);
    discarded[0]
}

#[test]
fn a_leyline_on_the_battlefield_from_the_start_exiles_a_cleanup_discard() {
    assert_eq!(cleanup_discard_zone(false, false), ZoneType::Exile);
}

#[test]
fn forge_misses_a_leyline_its_last_state_never_copied_for_a_cleanup_discard() {
    assert_eq!(cleanup_discard_zone(true, false), ZoneType::Graveyard);
}

#[test]
fn forge_sees_the_leyline_once_a_land_play_copies_its_last_state() {
    assert_eq!(cleanup_discard_zone(true, true), ZoneType::Exile);
}

const WITCH: &str = "Name:Dying Witch\nManaCost:1 B\nTypes:Creature Vampire Warlock\nPT:1/1\nT:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self,Creature.Other+YouCtrl | TriggerZones$ Battlefield | Execute$ TrigGain | TriggerDescription$ Whenever this creature or another creature you control dies, you gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const WIPE: &str = "Name:Wipe\nManaCost:R\nTypes:Sorcery\nA:SP$ DestroyAll | ValidCards$ Creature | SpellDescription$ Destroy all creatures.\nOracle:";

fn witch_life_after_exiled_beside_a_dying_token(by_state_check: bool) -> i32 {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, VOID_LEYLINE, p0, ZoneType::Battlefield);
    put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    let wipe = put(&mut game, WIPE, p0, ZoneType::Hand);
    let witch = put(&mut game, WITCH, p1, ZoneType::Battlefield);
    let rules = parse_card_script(BEARS).expect("script");
    let mut token = CardInstance::from_rules(&rules, p1);
    token.set_is_token(true);
    let token = game.create_card(token);
    game.move_card(token, ZoneType::Battlefield, p1);
    let after = put(&mut game, BEARS, p1, ZoneType::Battlefield);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    if by_state_check {
        game.card_mut(witch).damage = 1;
        game.card_mut(token).damage = 2;
        game.card_mut(after).damage = 2;
    }
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(CastOnce {
            spell: wipe,
            cast: by_state_check,
        }),
        Box::new(PassAgent),
    ];
    let mut game_loop = GameLoop::new(2);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    game_loop.step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.card(witch).zone, ZoneType::Exile);
    assert_eq!(game.card(token).zone, ZoneType::None);
    assert_eq!(game.card(after).zone, ZoneType::Exile);
    assert!(game.stack.is_empty());
    game.player(p1).life
}

#[test]
fn a_creature_exiled_instead_of_dying_sees_a_token_die_beside_it_to_a_wipe() {
    assert_eq!(witch_life_after_exiled_beside_a_dying_token(false), 21);
}

#[test]
fn a_creature_exiled_instead_of_dying_sees_a_token_die_beside_it_to_the_state_check() {
    assert_eq!(witch_life_after_exiled_beside_a_dying_token(true), 21);
}
