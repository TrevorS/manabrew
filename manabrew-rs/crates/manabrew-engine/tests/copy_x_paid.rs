use forge_carddb::parse_card_script;
use forge_foundation::{PhaseType, ZoneType};
use manabrew_engine::agent::{
    DecisionContext, GameEntity, ManaAbilityOption, ManaCostAction, PassAgent, PlayerAgent,
    PriorityActionSpace, PriorityContext, TargetChoice,
};
use manabrew_engine::combat::DefenderId;
use manabrew_engine::game::GameState;
use manabrew_engine::game_loop::GameLoop;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::mana::ManaPool;
use manabrew_engine::player::actions::PlayerAction;
use manabrew_engine::spellability::{SpellAbility, StackEntry};

const X_HYDRA: &str = "Name:X Hydra\nManaCost:X G\nTypes:Creature Hydra\nPT:0/0\nK:Flash\nK:etbCounter:P1P1:X\nSVar:X:Count$xPaid\nOracle:";
const X_BLAST: &str = "Name:X Blast\nManaCost:X R\nTypes:Instant\nA:SP$ DealDamage | ValidTgts$ Player | NumDmg$ X | SpellDescription$ CARDNAME deals X damage to target player.\nSVar:X:Count$xPaid\nOracle:";
const TWIN_ECHO: &str = "Name:Twin Echo\nManaCost:R\nTypes:Instant\nA:SP$ CopySpellAbility | ValidTgts$ Instant,Sorcery | TargetType$ Spell | SpellDescription$ Copy target instant or sorcery spell.\nOracle:";
const FOREST: &str = "Name:Forest\nManaCost:no cost\nTypes:Basic Land Forest\nOracle:";
const MOUNTAIN: &str = "Name:Mountain\nManaCost:no cost\nTypes:Basic Land Mountain\nOracle:";
const KILL_SOURCE: &str = "Name:Kill Source\nManaCost:no cost\nTypes:Artifact\nOracle:";
const MIND_COPIER: &str = "Name:Mind Copier\nManaCost:G\nTypes:Creature Shapeshifter\nPT:4/4\nK:Flash\nK:ETBReplacement:Copy:DBCopy:Optional\nSVar:DBCopy:DB$ Clone | Choices$ Creature.Other | ChoiceZone$ Graveyard | SpellDescription$ You may have CARDNAME enter as a copy of a creature card in a graveyard.\nOracle:";

struct Caster {
    casts: Vec<&'static str>,
    x: u32,
}

impl PlayerAgent for Caster {
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
        if let Some(&name) = self.casts.first() {
            let game = pr.context().game;
            if let Some(&play) = space
                .playable
                .iter()
                .find(|play| game.card(play.card_id).card_name == name)
            {
                self.casts.remove(0);
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
        p: PlayerId,
        valid: &[PlayerId],
        _: Option<&SpellAbility>,
    ) -> Option<PlayerId> {
        valid.iter().copied().find(|&v| v != p)
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
        p: PlayerId,
        players: &[PlayerId],
        _: &[CardId],
        _: Option<&SpellAbility>,
    ) -> TargetChoice {
        players
            .iter()
            .copied()
            .find(|&v| v != p)
            .map_or(TargetChoice::None, TargetChoice::Player)
    }
    fn choose_land_or_spell(&mut self, c: DecisionContext<'_>, p: PlayerId) -> Option<bool> {
        PassAgent.choose_land_or_spell(c, p)
    }
    fn announce_requirements_x(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: Option<CardId>,
        _: u32,
        max: u32,
    ) -> u32 {
        max.min(self.x)
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
    #[allow(clippy::too_many_arguments)]
    fn pay_cost_to_prevent_effect(
        &mut self,
        _: DecisionContext<'_>,
        _: PlayerId,
        _: &str,
        _: &str,
        _: Option<CardId>,
        _: Option<manabrew_engine::ability::api_type::ApiType>,
        _: bool,
        _: &[GameEntity],
        _: &str,
    ) -> bool {
        false
    }
}

fn put(game: &mut GameState, script: &str, owner: PlayerId, zone: ZoneType) -> CardId {
    let rules = parse_card_script(script).expect("script");
    let card = game.create_card(manabrew_engine::card::CardInstance::from_rules(
        &rules, owner,
    ));
    game.move_card(card, zone, owner);
    card
}

fn new_game() -> GameState {
    let mut game = GameState::new(&["Alice", "Bob"], 20);
    game.turn.active_player = PlayerId(0);
    game.new_turn_for_player(PlayerId(0));
    game.turn.phase = PhaseType::Main1;
    game
}

fn cast(game: &mut GameState, casts: Vec<&'static str>, x: u32) {
    let mut agents: Vec<Box<dyn PlayerAgent>> =
        vec![Box::new(Caster { casts, x }), Box::new(PassAgent)];
    GameLoop::new(2).step_with_priority(game, &mut agents, false);
}

#[test]
fn a_copy_of_a_dead_creature_cast_with_x_has_x_zero() {
    let mut game = new_game();
    let p0 = PlayerId(0);
    let hydra = put(&mut game, X_HYDRA, p0, ZoneType::Hand);
    let copier = put(&mut game, MIND_COPIER, p0, ZoneType::Hand);
    for _ in 0..4 {
        put(&mut game, FOREST, p0, ZoneType::Battlefield);
    }
    cast(&mut game, vec!["X Hydra"], 2);
    assert_eq!(game.card(hydra).power(), 2);
    let source = put(&mut game, KILL_SOURCE, p0, ZoneType::Command);
    game.card_mut(source).add_remembered_card(hydra);
    game.stack.push(StackEntry {
        id: 0,
        spell_ability: SpellAbility::new_simple(
            Some(source),
            p0,
            "DB$ Destroy | Defined$ Remembered",
        ),
        is_creature_spell: false,
        is_permanent_spell: false,
        is_pending_cast: false,
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    });
    cast(&mut game, vec![], 0);
    assert_eq!(game.card(hydra).zone, ZoneType::Graveyard);
    cast(&mut game, vec!["Mind Copier"], 0);
    assert_eq!(game.card(copier).zone, ZoneType::Graveyard);
}

#[test]
fn a_copy_of_a_spell_cast_with_x_keeps_its_x() {
    let mut game = new_game();
    let (p0, p1) = (PlayerId(0), PlayerId(1));
    put(&mut game, X_BLAST, p0, ZoneType::Hand);
    put(&mut game, TWIN_ECHO, p0, ZoneType::Hand);
    for _ in 0..4 {
        put(&mut game, MOUNTAIN, p0, ZoneType::Battlefield);
    }
    cast(&mut game, vec!["X Blast", "Twin Echo"], 2);
    assert!(game.stack.is_empty());
    assert_eq!(game.player(p1).life, 16);
}
