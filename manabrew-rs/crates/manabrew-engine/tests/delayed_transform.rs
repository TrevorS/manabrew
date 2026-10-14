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
use manabrew_engine::ids::CardId;
use manabrew_engine::ids::PlayerId;
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const SHIFTER: &str = "Name:Day Shifter\nManaCost:1 G\nTypes:Creature Human\nPT:2/2\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Shifter\nManaCost:no cost\nTypes:Creature Wolf\nPT:4/4\nOracle:";

#[test]
fn a_delayed_transform_made_before_its_host_transformed_does_nothing() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let rules = parse_card_script(SHIFTER).expect("script");
    let shifter = game.create_card(CardInstance::from_rules(&rules, p0));
    game.move_card(shifter, ZoneType::Battlefield, p0);
    game.card_mut(shifter).transform_count = 1;

    let mut sa = SpellAbility::new_simple(
        Some(shifter),
        p0,
        "DB$ SetState | Defined$ Self | Mode$ Transform | StoredTransform$ 0",
    );
    sa.is_trigger = true;
    let mut entry = StackEntry {
        id: 0,
        spell_ability: sa,
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    };
    entry.store_transform(&game);
    game.stack.push(entry);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    let card = game.card(shifter);
    assert!(!card.is_transformed);
    assert_eq!(card.transform_count, 1);
}

#[test]
fn a_transform_ability_does_nothing_once_its_host_has_left_and_returned() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let rules = parse_card_script(SHIFTER).expect("script");
    let shifter = game.create_card(CardInstance::from_rules(&rules, p0));
    game.move_card(shifter, ZoneType::Battlefield, p0);

    let mut sa = SpellAbility::new_simple(
        Some(shifter),
        p0,
        "AB$ SetState | Cost$ 0 | Defined$ Self | Mode$ Transform",
    );
    sa.source_zone_timestamp = Some(game.card(shifter).zone_timestamp);
    game.move_card(shifter, ZoneType::Exile, p0);
    game.move_card(shifter, ZoneType::Battlefield, p0);
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
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    assert!(!game.card(shifter).is_transformed);
}

const TWO_FACED_SACRIFICER: &str = "Name:Front Sacrificer\nManaCost:2 B\nTypes:Creature Human\nPT:3/3\nSVar:X:Count$ResolvedThisTurn\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Back Sacrificer\nManaCost:no cost\nTypes:Creature Angel\nPT:5/5\nSVar:X:Count$xPaid\nOracle:";
const FODDER: &str = "Name:Fodder\nManaCost:B\nTypes:Creature Rat\nPT:1/1\nOracle:";
const SWAMP: &str = "Name:Swamp\nManaCost:no cost\nTypes:Basic Land Swamp\nOracle:";

#[test]
fn a_back_face_trigger_resolving_after_its_host_turned_back_pays_its_own_x() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let put = |game: &mut GameState, script: &str, zone: ZoneType| {
        let rules = parse_card_script(script).expect("script");
        let card = game.create_card(CardInstance::from_rules(&rules, p0));
        game.move_card(card, zone, p0);
        card
    };
    let host = put(&mut game, TWO_FACED_SACRIFICER, ZoneType::Graveyard);
    let fodder = put(&mut game, FODDER, ZoneType::Battlefield);
    put(&mut game, SWAMP, ZoneType::Library);
    put(&mut game, SWAMP, ZoneType::Library);
    let mut sa = SpellAbility::new_simple(
        Some(host),
        p0,
        "AB$ Draw | Cost$ Sac<X/Creature.Other/another creature> | NumCards$ X",
    );
    sa.is_trigger = true;
    sa.card_state = game
        .card(host)
        .other_part
        .as_ref()
        .map(|other| other.state_name);
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
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(MaxNumber), Box::new(PassAgent)];
    GameLoop::new(2).resolve_stack(&mut game, &mut agents);

    assert_eq!(game.card(fodder).zone, ZoneType::Graveyard);
    assert_eq!(game.cards_in_zone(ZoneType::Hand, p0).len(), 1);
}

struct MaxNumber;

impl PlayerAgent for MaxNumber {
    fn choose_number(
        &mut self,
        _context: DecisionContext<'_>,
        _player: PlayerId,
        _source: Option<CardId>,
        _title: &str,
        _description: Option<&str>,
        _min: i32,
        max: i32,
    ) -> Option<i32> {
        Some(max)
    }
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
        context: DecisionContext<'_>,
        player: PlayerId,
        available: &[CardId],
        defenders: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        PassAgent.choose_attackers(context, player, available, defenders)
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
