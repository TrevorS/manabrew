use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::ability::ability_factory::build_spell_ability;
use manabrew_engine::ability::AbilityKey;
use manabrew_engine::card::valid_filter::matches_valid;
use manabrew_engine::card::CardInstance;
use manabrew_engine::event::RunParams;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::replacement::replace_damage;
use manabrew_engine::replacement::replacement_handler::ReplacementEvent;
use manabrew_engine::spellability::SpellAbility;
use manabrew_engine::staticability::layer::apply_continuous_effects;
use manabrew_engine::staticability::static_ability_cant_be_cast::cant_be_cast_ability;
use manabrew_engine::staticability::static_ability_panharmonicon::extra_triggers;

const BEAR: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const AURA: &str =
    "Name:Pacifism\nManaCost:1 W\nTypes:Enchantment Aura\nK:Enchant:Creature\nOracle:";

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn an_opponents_aura_does_not_make_a_permanent_modified() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let bear = put(&mut game, BEAR, p0, ZoneType::Battlefield);
    let host = put(&mut game, BEAR, p0, ZoneType::Battlefield);
    let opponents_aura = put(&mut game, AURA, p1, ZoneType::Battlefield);
    game.attach_to(opponents_aura, bear);
    let modified = |game: &GameState| {
        matches_valid(
            "Permanent.modified+YouCtrl",
            Some(game.card(bear)),
            None,
            game.card(host),
            p0,
            game,
        )
    };

    assert!(!modified(&game));
    let own_aura = put(&mut game, AURA, p0, ZoneType::Battlefield);
    game.attach_to(own_aura, bear);
    assert!(modified(&game));
}

const ENCHANTED_BEING: &str = "Name:Enchanted Being\nManaCost:1 W W\nTypes:Creature Human\nPT:2/2\nR:Event$ DamageDone | ActiveZones$ Battlefield | Prevent$ True | ValidTarget$ Card.Self | ValidSource$ Creature.enchanted | IsCombat$ True | Description$ Prevent all combat damage that would be dealt to CARDNAME by enchanted creatures.\nOracle:";

#[test]
fn enchanted_being_prevents_combat_damage_from_enchanted_creatures_only() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let being = put(&mut game, ENCHANTED_BEING, p0, ZoneType::Battlefield);
    let attacker = put(&mut game, BEAR, p1, ZoneType::Battlefield);
    let event = ReplacementEvent::DamageToCard {
        target: being,
        amount: 2,
        source: Some(attacker),
        is_combat: true,
    };
    let prevents = |game: &GameState| {
        replace_damage::can_replace(
            &game.card(being).replacement_effects[0],
            &event,
            game,
            game.card(being),
        )
    };

    assert!(!prevents(&game));
    let aura = put(&mut game, AURA, p1, ZoneType::Battlefield);
    game.attach_to(aura, attacker);
    assert!(prevents(&game));
}

const EXCLUSION_RITUAL: &str = "Name:Exclusion Ritual\nManaCost:4 W W\nTypes:Enchantment\nS:Mode$ CantBeCast | ValidCard$ Card.sharesNameWith Imprinted | Description$ Players can't cast spells with the same name as the exiled card.\nOracle:";

#[test]
fn exclusion_ritual_stops_spells_named_like_the_exiled_card() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    let ritual = put(&mut game, EXCLUSION_RITUAL, p0, ZoneType::Battlefield);
    let exiled = put(&mut game, BEAR, p1, ZoneType::Exile);
    game.card_mut(ritual).imprinted_cards.push(exiled);
    let bear = put(&mut game, BEAR, p1, ZoneType::Hand);
    let aura = put(&mut game, AURA, p1, ZoneType::Hand);
    let cant_cast = |game: &GameState, card: CardId| {
        let spell = SpellAbility::new_simple(Some(card), p1, "");
        cant_be_cast_ability(game, &spell, game.card(card), p1)
    };

    assert!(cant_cast(&game, bear));
    assert!(!cant_cast(&game, aura));
}

const CLOUD: &str = "Name:Cloud, Midgar Mercenary\nManaCost:W W\nTypes:Legendary Creature Human Soldier Mercenary\nPT:2/1\nT:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Whenever CARDNAME attacks, draw a card.\nSVar:TrigDraw:DB$ Draw\nS:Mode$ Panharmonicon | ValidCard$ Card.Self+equipped,Equipment.Attached | Description$ As long as NICKNAME is equipped, if an ability of NICKNAME or an Equipment attached to it triggers, that ability triggers an additional time.\nOracle:";
const SWORD: &str = "Name:Short Sword\nManaCost:1\nTypes:Artifact Equipment\nK:Equip:1\nOracle:";

#[test]
fn cloud_doubles_its_own_triggers_while_equipped() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let cloud = put(&mut game, CLOUD, p0, ZoneType::Battlefield);
    let trigger = game.card(cloud).triggers[0].clone();
    let extra = |game: &GameState| extra_triggers(game, cloud, &trigger, &RunParams::default());

    assert_eq!(extra(&game), 0);
    let sword = put(&mut game, SWORD, p0, ZoneType::Battlefield);
    game.attach_to(sword, cloud);
    assert_eq!(extra(&game), 1);
}

const ILLUMINATOR: &str = "Name:Chittering Illuminator\nManaCost:2 U\nTypes:Creature Rat\nPT:2/2\nS:Mode$ Continuous | Affected$ Card.Self+TopLibrary | AffectedZone$ Library | EffectZone$ All | MayPlay$ True | MayLookAt$ You | Description$ As long as CARDNAME is at the top of your library, you may look at it any time and you may cast it.\nOracle:";

#[test]
fn a_top_of_library_card_lets_its_owner_look_at_it() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let illuminator = put(&mut game, ILLUMINATOR, p0, ZoneType::Library);

    apply_continuous_effects(&mut game);
    assert!(game.card(illuminator).may_player_look(p0));

    put(&mut game, BEAR, p0, ZoneType::Library);
    apply_continuous_effects(&mut game);
    assert!(!game.card(illuminator).may_player_look(p0));
}

const PANDA: &str = "Name:Fiendish Panda\nManaCost:2 W B\nTypes:Creature Bear Demon\nPT:3/2\nSVar:X:TriggeredCard$CardPower\nOracle:";
const OGRE: &str = "Name:Gray Ogre\nManaCost:2 R\nTypes:Creature Ogre\nPT:2/2\nOracle:";
const GIANT: &str = "Name:Hill Giant\nManaCost:3 R\nTypes:Creature Giant\nPT:3/3\nOracle:";

#[test]
fn a_return_trigger_has_candidates_only_within_the_dead_creatures_power() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    let panda = put(&mut game, PANDA, p0, ZoneType::Graveyard);
    game.card_mut(panda).lki_power = Some(3);
    put(&mut game, GIANT, p0, ZoneType::Graveyard);
    let mut sa = build_spell_ability(
        &game,
        panda,
        "DB$ ChangeZone | ValidTgts$ Creature.cmcLEX+YouOwn+nonBear+Other | Origin$ Graveyard | Destination$ Battlefield",
        p0,
    );
    sa.set_triggering_value(AbilityKey::Card, panda);
    let has_candidates = |game: &GameState| {
        sa.target_restrictions
            .as_ref()
            .expect("targets")
            .has_candidates(game, p0, sa.source, Some(&sa))
    };

    assert!(!has_candidates(&game));
    put(&mut game, OGRE, p0, ZoneType::Graveyard);
    assert!(has_candidates(&game));
}
