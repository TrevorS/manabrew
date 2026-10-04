use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::card::card_trait_changes::CardTraitChanges;
use manabrew_engine::card::perpetual::perpetual_abilities::PerpetualAbilities;
use manabrew_engine::card::perpetual::perpetual_interface::PerpetualInterface;
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::staticability::layer::apply_continuous_effects;
use manabrew_engine::trigger::{parse_trigger, Trigger, TriggerType};

const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const WEREBEAR: &str = "Name:Day Werebear\nManaCost:1 G\nTypes:Creature Human Werewolf\nPT:1/1\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Night Werebear\nManaCost:no cost\nColors:green\nTypes:Creature Werewolf\nPT:3/3\nOracle:";
const WARD_ANTHEM: &str = "Name:Ward Anthem\nManaCost:2 U\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddKeyword$ Ward:1 | Description$ Creatures you control have ward {1}.\nOracle:";
const HUMBLING: &str = "Name:Humbling Field\nManaCost:2 W W\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature | RemoveAllAbilities$ True | Description$ All creatures lose all abilities.\nOracle:";
const SCOUT: &str = "Name:Herald Scout\nManaCost:1 W\nTypes:Creature Human Scout\nPT:2/2\nT:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigGain | TriggerDescription$ When this attacks, gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nOracle:";
const DOMINANT: &str = "Name:Test Dominant\nManaCost:1 R W\nTypes:Legendary Creature Human\nPT:3/4\nT:Mode$ ChangesZone | ValidCard$ Card.Self | Origin$ Any | Destination$ Battlefield | Execute$ TrigGain | TriggerDescription$ When this enters, gain 1 life.\nSVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\nAlternateMode:DoubleFaced\nOracle:\n\nALTERNATE\n\nName:Test Warden\nManaCost:no cost\nColors:red,white\nTypes:Legendary Enchantment Creature Saga Phoenix\nPT:4/4\nK:Chapter:2:DBOne,DBTwo\nSVar:DBOne:DB$ GainLife | Defined$ You | LifeAmount$ 1\nSVar:DBTwo:DB$ GainLife | Defined$ You | LifeAmount$ 2\nOracle:";
const PROWESS_ANTHEM: &str = "Name:Prowess Anthem\nManaCost:2 U\nTypes:Enchantment\nS:Mode$ Continuous | Affected$ Creature.YouCtrl | AddKeyword$ Prowess | Description$ Creatures you control have prowess.\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

fn trigger(mode: &str) -> Trigger {
    let mut id = 0;
    parse_trigger(
        &format!(
            "Mode$ {mode} | ValidCard$ Card.Self | Execute$ TrigGain | TriggerDescription$ {mode}"
        ),
        &mut id,
    )
    .expect("trigger")
}

fn perpetual_trigger(game: &mut GameState, card: CardId, timestamp: i64, mode: &str) {
    PerpetualAbilities {
        timestamp,
        changes: CardTraitChanges {
            triggers: vec![trigger(mode)],
            ..Default::default()
        },
    }
    .apply_effect(game.card_mut(card));
}

fn kinds(game: &GameState, card: CardId) -> Vec<TriggerType> {
    game.card(card).triggers.iter().map(|t| t.kind).collect()
}

fn count(game: &GameState, card: CardId, kind: TriggerType) -> usize {
    kinds(game, card).into_iter().filter(|&k| k == kind).count()
}

#[test]
fn the_trait_baseline_holds_only_the_cards_own_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let anthem = put(&mut game, PROWESS_ANTHEM, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert_eq!(count(&game, bears, TriggerType::SpellCast), 1);
    perpetual_trigger(&mut game, bears, 500, "Attacks");
    game.move_card(anthem, ZoneType::Graveyard, p0);
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    assert_eq!(count(&game, bears, TriggerType::SpellCast), 0);
    assert_eq!(count(&game, bears, TriggerType::Attacks), 1);
}

#[test]
fn a_perpetual_trigger_survives_layer_passes_ahead_of_pump_and_granted_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    put(&mut game, PROWESS_ANTHEM, p0, ZoneType::Battlefield);
    perpetual_trigger(&mut game, bears, 500, "Attacks");
    game.card_mut(bears).add_pump_trigger(trigger("Blocks"));
    apply_continuous_effects(&mut game);
    let first: Vec<(TriggerType, u32)> = game
        .card(bears)
        .triggers
        .iter()
        .map(|t| (t.kind, t.id))
        .collect();
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    let last: Vec<(TriggerType, u32)> = game
        .card(bears)
        .triggers
        .iter()
        .map(|t| (t.kind, t.id))
        .collect();
    assert_eq!(
        kinds(&game, bears),
        vec![
            TriggerType::Attacks,
            TriggerType::Blocks,
            TriggerType::SpellCast
        ]
    );
    assert_eq!(first, last);
}

#[test]
fn clearing_every_layer_keeps_pump_and_granted_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    put(&mut game, PROWESS_ANTHEM, p0, ZoneType::Battlefield);
    game.card_mut(bears).add_changed_card_traits(
        CardTraitChanges {
            triggers: vec![trigger("Attacks")],
            ..Default::default()
        },
        500,
        0,
    );
    game.card_mut(bears).add_pump_trigger(trigger("Blocks"));
    apply_continuous_effects(&mut game);
    game.card_mut(bears).clear_changed_card_traits();
    assert_eq!(
        kinds(&game, bears),
        vec![TriggerType::Blocks, TriggerType::SpellCast]
    );
}

#[test]
fn a_zone_change_drops_lasting_triggers_and_keeps_perpetual_ones() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let bears = put(&mut game, BEARS, p0, ZoneType::Battlefield);
    game.card_mut(bears).add_lasting_trigger(trigger("Blocks"));
    perpetual_trigger(&mut game, bears, 500, "Attacks");
    game.card_mut(bears).add_lasting_trigger(trigger("Taps"));
    perpetual_trigger(&mut game, bears, 600, "Untaps");
    apply_continuous_effects(&mut game);
    assert_eq!(count(&game, bears, TriggerType::Blocks), 1);
    assert_eq!(count(&game, bears, TriggerType::Taps), 1);
    assert_eq!(count(&game, bears, TriggerType::Attacks), 1);
    assert_eq!(count(&game, bears, TriggerType::Untaps), 1);

    game.move_card(bears, ZoneType::Graveyard, p0);
    game.move_card(bears, ZoneType::Battlefield, p0);
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    assert_eq!(count(&game, bears, TriggerType::Blocks), 0);
    assert_eq!(count(&game, bears, TriggerType::Taps), 0);
    assert_eq!(count(&game, bears, TriggerType::Attacks), 1);
    assert_eq!(count(&game, bears, TriggerType::Untaps), 1);
}

#[test]
fn a_transform_keeps_one_copy_of_each_pump_and_granted_trigger() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let werebear = put(&mut game, WEREBEAR, p0, ZoneType::Battlefield);
    put(&mut game, PROWESS_ANTHEM, p0, ZoneType::Battlefield);
    perpetual_trigger(&mut game, werebear, 500, "Attacks");
    game.card_mut(werebear).add_pump_trigger(trigger("Blocks"));
    apply_continuous_effects(&mut game);
    game.card_mut(werebear).transform();
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    assert_eq!(
        kinds(&game, werebear),
        vec![
            TriggerType::Attacks,
            TriggerType::Blocks,
            TriggerType::SpellCast
        ]
    );
}

#[test]
fn a_newer_lose_all_abilities_static_removes_older_granted_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let scout = put(&mut game, SCOUT, p0, ZoneType::Battlefield);
    put(&mut game, WARD_ANTHEM, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert_eq!(
        kinds(&game, scout),
        vec![TriggerType::Attacks, TriggerType::BecomesTarget]
    );
    put(&mut game, HUMBLING, p0, ZoneType::Battlefield);
    for _ in 0..3 {
        apply_continuous_effects(&mut game);
        assert!(kinds(&game, scout).is_empty());
    }
}

#[test]
fn a_copy_of_a_creature_that_lost_all_abilities_copies_its_printed_triggers() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let scout = put(&mut game, SCOUT, p0, ZoneType::Battlefield);
    put(&mut game, HUMBLING, p0, ZoneType::Battlefield);
    apply_continuous_effects(&mut game);
    assert!(kinds(&game, scout).is_empty());
    let copiable: Vec<TriggerType> = game
        .card(scout)
        .copiable_triggers()
        .iter()
        .map(|t| t.kind)
        .collect();
    assert_eq!(copiable, vec![TriggerType::Attacks]);
}

#[test]
fn a_transform_after_a_lasting_change_keeps_one_copy_of_each_trigger() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let werebear = put(&mut game, WEREBEAR, p0, ZoneType::Battlefield);
    put(&mut game, PROWESS_ANTHEM, p0, ZoneType::Battlefield);
    game.card_mut(werebear).add_lasting_trigger(trigger("Taps"));
    perpetual_trigger(&mut game, werebear, 500, "Attacks");
    game.card_mut(werebear).add_pump_trigger(trigger("Blocks"));
    apply_continuous_effects(&mut game);
    game.card_mut(werebear).transform();
    apply_continuous_effects(&mut game);
    apply_continuous_effects(&mut game);
    for kind in [
        TriggerType::Taps,
        TriggerType::Attacks,
        TriggerType::Blocks,
        TriggerType::SpellCast,
    ] {
        assert_eq!(count(&game, werebear, kind), 1, "{kind:?}");
    }
}

#[test]
fn a_saga_back_faced_card_keeps_its_triggers_bound_to_itself_across_transforms() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    put(&mut game, BEARS, p0, ZoneType::Battlefield);
    let dominant = put(&mut game, DOMINANT, p0, ZoneType::Battlefield);
    assert_ne!(dominant, CardId(0));
    for _ in 0..2 {
        game.card_mut(dominant).transform();
        apply_continuous_effects(&mut game);
        assert!(game
            .card(dominant)
            .triggers
            .iter()
            .all(|trigger| trigger.host_card_id() == dominant));
    }
    assert_eq!(kinds(&game, dominant), vec![TriggerType::ChangesZone]);
}
