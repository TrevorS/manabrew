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

const POWERLESS_DRUID: &str = "Name:Powerless Druid\nManaCost:1 G\nTypes:Creature Elf Druid\nPT:0/2\nA:AB$ Mana | Cost$ T | Produced$ Any | Amount$ X | SpellDescription$ Add X mana of any one color, where X is this creature's power.\nSVar:X:Count$CardPower\nOracle:";
const ANY_DRUID: &str = "Name:Any Druid\nManaCost:1 G\nTypes:Creature Elf Druid\nPT:1/1\nA:AB$ Mana | Cost$ T | Produced$ Any | SpellDescription$ Add one mana of any color.\nOracle:";
const SPROUT: &str = "Name:Sprout\nManaCost:G\nTypes:Sorcery\nA:SP$ GainLife | LifeAmount$ 1 | Defined$ You | SpellDescription$ You gain 1 life.\nOracle:";

struct Caster {
    cast: bool,
}

impl PlayerAgent for Caster {
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
        let requested;
        let space = match s {
            Some(space) => space,
            None => {
                requested = pr.action_space();
                &requested
            }
        };
        if !self.cast {
            if let Some(&play) = space.playable.first() {
                self.cast = true;
                return PlayerAction::CastSpell(play);
            }
        }
        PassAgent.choose_action(p, Some(space), pr)
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
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(CardInstance::from_rules(&rules, owner));
    game.move_card(card, zone, owner);
    card
}

#[test]
fn a_mana_ability_whose_amount_is_zero_adds_no_mana_to_an_auto_payment() {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    let p0 = PlayerId(0);
    game.turn.active_player = p0;
    game.new_turn_for_player(p0);
    game.turn.phase = PhaseType::Main1;
    let powerless = put(&mut game, POWERLESS_DRUID, p0, ZoneType::Battlefield);
    let any = put(&mut game, ANY_DRUID, p0, ZoneType::Battlefield);
    for card in [powerless, any] {
        game.card_mut(card).set_summoning_sick(false);
    }
    put(&mut game, SPROUT, p0, ZoneType::Hand);
    let mut agents: Vec<Box<dyn PlayerAgent>> =
        vec![Box::new(Caster { cast: false }), Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(&mut game, &mut agents, true);
    assert_eq!(game.player(p0).life, 21);
    assert!(game.card(powerless).tapped);
    assert!(game.card(any).tapped);
}
