use forge_carddb::parse_card_script;
use forge_foundation::ZoneType;
use manabrew_engine::agent::{
    DecisionContext, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::card::CardInstance;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::SpellAbility;

const AUTHORITY: &str = "Name:Authority of the Consuls\nManaCost:W\nTypes:Enchantment\nR:Event$ Moved | ValidCard$ Creature.OppCtrl | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Creatures your opponents control enter tapped.\nSVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard\nOracle:";
const BARE_WARDEN: &str = "Name:Bare Warden\nManaCost:W\nTypes:Enchantment\nR:Event$ Moved | ValidCard$ Creature.OppCtrl+counters_EQ0_P1P1 | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Creatures your opponents control with no +1/+1 counters enter tapped.\nSVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard\nOracle:";
const GROWN_BEAR: &str =
    "Name:Grown Bear\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nK:etbCounter:P1P1:1\nOracle:";
const BEARS: &str = "Name:Grizzly Bears\nManaCost:1 G\nTypes:Creature Bear\nPT:2/2\nOracle:";
const TAPLAND: &str = "Name:Slow Grove\nManaCost:no cost\nTypes:Land Forest\nR:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | Description$ CARDNAME enters tapped.\nSVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ Self\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";

struct OwnReplacementFirst(CardId);

impl PlayerAgent for OwnReplacementFirst {
    fn choose_targets_for(&mut self, _: &mut SpellAbility, _: &GameState, _: &[ManaPool]) -> bool {
        true
    }
    fn mulligan_decision(
        &mut self,
        c: DecisionContext<'_>,
        p: PlayerId,
        h: &[CardId],
        n: u32,
    ) -> bool {
        PassAgent.mulligan_decision(c, p, h, n)
    }
    fn choose_action(
        &mut self,
        p: PlayerId,
        s: Option<&PriorityActionSpace>,
        pr: &mut dyn PriorityContext,
    ) -> PlayerAction {
        PassAgent.choose_action(p, s, pr)
    }
    fn choose_attackers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[DefenderId],
    ) -> Vec<(CardId, DefenderId)> {
        Vec::new()
    }
    fn choose_blockers(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: &[CardId],
        _: Option<usize>,
    ) -> Vec<(CardId, CardId)> {
        Vec::new()
    }
    fn choose_target_player(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        None
    }
    fn choose_target_card(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> Option<CardId> {
        None
    }
    fn choose_target_any(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        TargetChoice::None
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
    #[allow(clippy::too_many_arguments)]
    fn pay_mana_cost(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: CardId,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: bool,
        _: &[CardId],
        _: &[ManaAbilityOption],
        _: &[CardId],
        _: &[CardId],
        _: &ManaPool,
    ) -> ManaCostAction {
        ManaCostAction::Pay { auto: true }
    }
    fn choose_single_replacement_effect(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &[String],
        hosts: &[CardId],
    ) -> usize {
        hosts.iter().position(|&host| host == self.0).unwrap_or(0)
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn an_enters_tapped_replacement_the_entering_object_no_longer_matches_does_not_tap_it() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    put(&mut game, BARE_WARDEN, PlayerId(0), ZoneType::Battlefield);
    let bear = put(&mut game, GROWN_BEAR, PlayerId(1), ZoneType::Hand);
    let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
        Box::new(OwnReplacementFirst(bear)),
        Box::new(OwnReplacementFirst(bear)),
    ];
    game.move_card_with_agents(bear, ZoneType::Battlefield, PlayerId(1), &mut agents);
    assert_eq!(game.card(bear).zone, ZoneType::Battlefield);
    assert!(!game.card(bear).tapped);
}

#[test]
fn enters_tapped_replacements_still_tap_what_they_apply_to() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    put(&mut game, AUTHORITY, PlayerId(0), ZoneType::Battlefield);
    let bears = put(&mut game, BEARS, PlayerId(1), ZoneType::Hand);
    game.move_card(bears, ZoneType::Battlefield, PlayerId(1));
    assert!(game.card(bears).tapped);
    let grove = put(&mut game, TAPLAND, PlayerId(0), ZoneType::Hand);
    game.move_card(grove, ZoneType::Battlefield, PlayerId(0));
    assert!(game.card(grove).tapped);
}

#[test]
fn a_tapped_permanent_comes_back_untapped() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let forest = put(&mut game, FOREST, PlayerId(0), ZoneType::Battlefield);
    game.card_mut(forest).set_tapped(true);
    game.move_card(forest, ZoneType::Graveyard, PlayerId(0));
    game.move_card(forest, ZoneType::Battlefield, PlayerId(0));
    assert!(!game.card(forest).tapped);
    game.card_mut(forest).set_tapped(true);
    game.put_on_bottom_of_library(forest, PlayerId(0));
    assert!(!game.card(forest).tapped);
    game.move_card(forest, ZoneType::Battlefield, PlayerId(0));
    assert!(!game.card(forest).tapped);
}
