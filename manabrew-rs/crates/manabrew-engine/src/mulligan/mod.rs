//! London Mulligan implementation.
//!
//! Mirrors the Java mulligan package at
//! `forge/forge-game/src/main/java/forge/game/mulligan/`.
//!
//! After libraries are shuffled and opening hands are drawn, each player
//! (starting from the player who goes first) decides whether to keep or
//! mulligan.  On a mulligan the hand is shuffled back and a fresh 7 cards
//! are drawn.  When a player finally keeps, they put N cards from hand on
//! the bottom of their library, where N is the number of mulligans taken.
//!
//! ## Java parity divergence
//!
//! Java's `MulliganService.runPlayerMulligans()` prompts each player
//! sequentially (one blocking call per player per round). This Rust port
//! fans out all non-kept players' prompts per round and collects the
//! responses in parallel, then applies keep/mulligan decisions in turn
//! order. The put-back prompts after all players have kept are also
//! fanned out.
//!
//! Observable game state is identical (decisions still apply in turn
//! order; mulligans don't interact). The divergence is purely in prompt
//! timing, so networked multiplayer clients can respond simultaneously
//! instead of waiting in queue.

use crate::agent::DecisionContext;
use crate::agent::PlayerAgent;
use crate::game::GameState;
use crate::game_log::GameLog;
use crate::game_log_entry_type::GameLogEntryType;
use crate::ids::{CardId, PlayerId};
use crate::mana::ManaPool;
use forge_foundation::ZoneType;
use serde::{Deserialize, Serialize};

const STARTING_HAND_SIZE: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FilteredHands {
    pub candidates: u8,
    pub mulligans: bool,
    pub choice: FilteredHandsChoice,
}

impl Default for FilteredHands {
    fn default() -> Self {
        FilteredHands {
            candidates: 2,
            mulligans: false,
            choice: FilteredHandsChoice::Deterministic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FilteredHandsChoice {
    Deterministic,
    Weighted { temperature: f32 },
}

impl FilteredHandsChoice {
    pub fn choose(self, scores: &[f32], rng: &mut impl rand::Rng) -> usize {
        let best = scores.iter().copied().fold(f32::INFINITY, f32::min);
        match self {
            FilteredHandsChoice::Deterministic => scores
                .iter()
                .position(|&score| score == best)
                .expect("a candidate hand"),
            FilteredHandsChoice::Weighted { temperature } => {
                assert!(
                    temperature.is_finite() && temperature > 0.0,
                    "filtered hands need a positive temperature, not {temperature}"
                );
                let weights: Vec<f64> = scores
                    .iter()
                    .map(|&score| (f64::from(best - score) / f64::from(temperature)).exp())
                    .collect();
                let mut pick = rng.gen_range(0.0..weights.iter().sum::<f64>());
                for (index, weight) in weights.iter().enumerate() {
                    if pick < *weight {
                        return index;
                    }
                    pick -= weight;
                }
                weights.len() - 1
            }
        }
    }
}

/// Run the London Mulligan procedure for every player in the game.
///
/// Players act in turn order beginning with `first_player`.  Each round,
/// every player who has not yet kept is asked whether to keep or mulligan.
/// The loop ends once all players have kept.
pub fn run_london_mulligans(
    game: &mut GameState,
    agents: &mut [Box<dyn PlayerAgent>],
    rng: &mut impl rand::Rng,
    first_player: PlayerId,
    mana_pools: &[ManaPool],
    game_log: &GameLog,
) {
    let ordered = mulligan_order(&game.player_order, first_player);
    let player_count = ordered.len();
    let mut mulligan_count = vec![0u32; player_count];
    let mut has_kept = vec![false; player_count];

    // Per round: fan out prompts to every non-kept player, collect all
    // responses, then apply decisions in turn order. See module docs for
    // the Java parity divergence this introduces.
    loop {
        if has_kept.iter().all(|&k| k) {
            break;
        }

        let active: Vec<(usize, PlayerId, Vec<CardId>)> = (0..player_count)
            .filter(|i| !has_kept[*i])
            .map(|i| {
                let pid = ordered[i];
                let hand = game.cards_in_zone(ZoneType::Hand, pid).to_vec();
                (i, pid, hand)
            })
            .collect();

        // Phase 1: snapshot + fire prompts for every active player.
        for (i, pid, hand) in &active {
            agents[pid.index()].snapshot_state(game, mana_pools);
            if !hand.is_empty() {
                agents[pid.index()].mulligan_decision_send(
                    DecisionContext::new(game, mana_pools),
                    *pid,
                    hand,
                    mulligan_count[*i],
                );
            }
        }

        // Phase 2: collect responses.  Prompts are already in flight so
        // remote clients can respond concurrently.
        let decisions: Vec<bool> = active
            .iter()
            .map(|(i, pid, hand)| {
                if hand.is_empty() {
                    true
                } else {
                    agents[pid.index()].mulligan_decision_recv(
                        DecisionContext::new(game, mana_pools),
                        *pid,
                        hand,
                        mulligan_count[*i],
                    )
                }
            })
            .collect();

        // Phase 3: apply in turn order.
        for ((i, pid, _), keep) in active.iter().zip(decisions.iter()) {
            if *keep {
                has_kept[*i] = true;
            } else {
                perform_mulligan(game, *pid, rng, game_log);
                mulligan_count[*i] += 1;
            }
        }
    }

    run_put_back_phase(
        game,
        agents,
        &ordered,
        &mulligan_count,
        mana_pools,
        game_log,
    );
}

/// After all players have kept, fan out put-back prompts in parallel and
/// apply the results in turn order.
fn run_put_back_phase(
    game: &mut GameState,
    agents: &mut [Box<dyn PlayerAgent>],
    ordered: &[PlayerId],
    mulligan_count: &[u32],
    mana_pools: &[ManaPool],
    game_log: &GameLog,
) {
    struct PutBackJob {
        player: PlayerId,
        hand: Vec<CardId>,
        count: usize,
    }

    let jobs: Vec<PutBackJob> = ordered
        .iter()
        .enumerate()
        .filter_map(|(i, &pid)| {
            let count = mulligan_count[i] as usize;
            if count == 0 {
                return None;
            }
            agents[pid.index()].snapshot_state(game, mana_pools);
            let hand = game.cards_in_zone(ZoneType::Hand, pid).to_vec();
            agents[pid.index()].choose_cards_to_bottom_send(
                DecisionContext::new(game, mana_pools),
                pid,
                &hand,
                count,
            );
            Some(PutBackJob {
                player: pid,
                hand,
                count,
            })
        })
        .collect();

    for job in jobs {
        let picks = agents[job.player.index()].choose_cards_to_bottom_recv(
            DecisionContext::new(game, mana_pools),
            job.player,
            &job.hand,
            job.count,
        );
        for &card_id in &picks {
            game.put_on_bottom_of_library(card_id, job.player);
        }
    }

    for &pid in ordered {
        let final_size = game.cards_in_zone(ZoneType::Hand, pid).len();
        game_log.log(
            GameLogEntryType::Mulligan,
            1,
            format!(
                "{} keeps hand ({} card{})",
                game.player(pid).name,
                final_size,
                if final_size == 1 { "" } else { "s" },
            ),
        );
    }
}

/// Shuffle the player's hand back into their library, then draw a fresh 7.
fn perform_mulligan(
    game: &mut GameState,
    player: PlayerId,
    rng: &mut impl rand::Rng,
    game_log: &GameLog,
) {
    let hand: Vec<_> = game.cards_in_zone(ZoneType::Hand, player).to_vec();
    for card_id in hand {
        game.move_card(card_id, ZoneType::Library, player);
    }
    game.shuffle_library(player, rng);
    match game.filtered_hands {
        Some(filtered) if filtered.mulligans => crate::game_loop::GameLoop::draw_starting_hand(
            game,
            player,
            rng,
            filtered,
            STARTING_HAND_SIZE,
        ),
        _ => {
            game.draw_cards(player, STARTING_HAND_SIZE);
        }
    }

    game_log.log(
        GameLogEntryType::Mulligan,
        1,
        format!("{} mulligans", game.player(player).name),
    );
}

/// Rotate `player_order` so that `first_player` is at the front.
fn mulligan_order(player_order: &[PlayerId], first_player: PlayerId) -> Vec<PlayerId> {
    let offset = player_order
        .iter()
        .position(|&p| p == first_player)
        .unwrap_or(0);
    let len = player_order.len();
    (0..len).map(|i| player_order[(offset + i) % len]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::PriorityContext;
    use crate::agent::{PlayerAgent, TargetChoice};
    use crate::card::Card;
    use crate::combat::DefenderId;
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::mana::ManaPool;
    use crate::player::actions::PlayerAction;
    use crate::spellability::SpellAbility;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};
    use rand::SeedableRng;

    struct TestAgent {
        mulligans_to_take: u32,
        bottom_picks: Option<Vec<CardId>>,
    }

    impl TestAgent {
        fn keep() -> Self {
            TestAgent {
                mulligans_to_take: 0,
                bottom_picks: None,
            }
        }

        fn mulligan(times: u32) -> Self {
            TestAgent {
                mulligans_to_take: times,
                bottom_picks: None,
            }
        }
    }

    impl PlayerAgent for TestAgent {
        fn mulligan_decision(
            &mut self,
            _context: DecisionContext<'_>,
            _player: PlayerId,
            _hand: &[CardId],
            mulligan_count: u32,
        ) -> bool {
            mulligan_count >= self.mulligans_to_take
        }

        fn choose_cards_to_bottom(
            &mut self,
            _context: DecisionContext<'_>,
            _player: PlayerId,
            hand: &[CardId],
            count: usize,
        ) -> Vec<CardId> {
            if let Some(ref picks) = self.bottom_picks {
                picks.clone()
            } else {
                hand.iter().copied().take(count).collect()
            }
        }

        fn choose_action(
            &mut self,
            _player: PlayerId,
            _action_space: Option<&crate::agent::PriorityActionSpace>,
            _priority: &mut dyn PriorityContext,
        ) -> PlayerAction {
            PlayerAction::PassPriority
        }

        fn choose_attackers(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
            _: &[CardId],
            _: &[DefenderId],
        ) -> Vec<(CardId, DefenderId)> {
            vec![]
        }

        fn choose_blockers(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
            _: &[CardId],
            _: &[CardId],
            _: Option<usize>,
        ) -> Vec<(CardId, CardId)> {
            vec![]
        }

        fn choose_target_player(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
            v: &[PlayerId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> Option<PlayerId> {
            v.first().copied()
        }

        fn choose_target_card(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
            v: &[CardId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> Option<CardId> {
            v.first().copied()
        }

        fn choose_target_any(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
            _: &[PlayerId],
            _: &[CardId],
            _sa: Option<&crate::spellability::SpellAbility>,
        ) -> TargetChoice {
            TargetChoice::None
        }

        fn choose_land_or_spell(
            &mut self,
            _context: DecisionContext<'_>,
            _: PlayerId,
        ) -> Option<bool> {
            None
        }

        fn choose_targets_for(
            &mut self,
            _sa: &mut SpellAbility,
            _game: &GameState,
            _mana_pools: &[ManaPool],
        ) -> bool {
            false
        }
    }

    fn filler_card(owner: PlayerId) -> Card {
        Card::new(
            CardId(0),
            "Filler".to_string(),
            owner,
            CardTypeLine::parse("Creature"),
            ManaCost::no_cost(),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        )
    }

    fn setup_game_with_libraries(deck_size: usize) -> (GameState, rand::rngs::StdRng) {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        for _ in 0..deck_size {
            let c0 = game.create_card(filler_card(p0));
            game.add_card_to_zone(ZoneType::Library, p0, c0);
            game.card_mut(c0).zone = ZoneType::Library;

            let c1 = game.create_card(filler_card(p1));
            game.add_card_to_zone(ZoneType::Library, p1, c1);
            game.card_mut(c1).zone = ZoneType::Library;
        }
        let rng = rand::rngs::StdRng::seed_from_u64(42);
        (game, rng)
    }

    #[test]
    fn keep_immediately_preserves_seven_card_hand() {
        let (mut game, mut rng) = setup_game_with_libraries(40);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        game.shuffle_library(p0, &mut rng);
        game.shuffle_library(p1, &mut rng);
        game.draw_cards(p0, 7);
        game.draw_cards(p1, 7);

        let mut agents: Vec<Box<dyn PlayerAgent>> =
            vec![Box::new(TestAgent::keep()), Box::new(TestAgent::keep())];
        let pools = vec![ManaPool::new(), ManaPool::new()];
        let log = GameLog::new();

        run_london_mulligans(&mut game, &mut agents, &mut rng, p0, &pools, &log);

        assert_eq!(game.cards_in_zone(ZoneType::Hand, p0).len(), 7);
        assert_eq!(game.cards_in_zone(ZoneType::Hand, p1).len(), 7);
    }

    #[test]
    fn one_mulligan_leaves_six_cards() {
        let (mut game, mut rng) = setup_game_with_libraries(40);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        game.shuffle_library(p0, &mut rng);
        game.shuffle_library(p1, &mut rng);
        game.draw_cards(p0, 7);
        game.draw_cards(p1, 7);

        let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
            Box::new(TestAgent::mulligan(1)),
            Box::new(TestAgent::keep()),
        ];
        let pools = vec![ManaPool::new(), ManaPool::new()];
        let log = GameLog::new();

        run_london_mulligans(&mut game, &mut agents, &mut rng, p0, &pools, &log);

        assert_eq!(game.cards_in_zone(ZoneType::Hand, p0).len(), 6);
        assert_eq!(game.cards_in_zone(ZoneType::Hand, p1).len(), 7);
    }

    #[test]
    fn two_mulligans_leaves_five_cards() {
        let (mut game, mut rng) = setup_game_with_libraries(40);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        game.shuffle_library(p0, &mut rng);
        game.shuffle_library(p1, &mut rng);
        game.draw_cards(p0, 7);
        game.draw_cards(p1, 7);

        let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
            Box::new(TestAgent::mulligan(2)),
            Box::new(TestAgent::keep()),
        ];
        let pools = vec![ManaPool::new(), ManaPool::new()];
        let log = GameLog::new();

        run_london_mulligans(&mut game, &mut agents, &mut rng, p0, &pools, &log);

        assert_eq!(game.cards_in_zone(ZoneType::Hand, p0).len(), 5);
        assert_eq!(game.cards_in_zone(ZoneType::Hand, p1).len(), 7);
    }

    #[test]
    fn both_players_can_mulligan_independently() {
        let (mut game, mut rng) = setup_game_with_libraries(40);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        game.shuffle_library(p0, &mut rng);
        game.shuffle_library(p1, &mut rng);
        game.draw_cards(p0, 7);
        game.draw_cards(p1, 7);

        let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
            Box::new(TestAgent::mulligan(1)),
            Box::new(TestAgent::mulligan(2)),
        ];
        let pools = vec![ManaPool::new(), ManaPool::new()];
        let log = GameLog::new();

        run_london_mulligans(&mut game, &mut agents, &mut rng, p0, &pools, &log);

        assert_eq!(game.cards_in_zone(ZoneType::Hand, p0).len(), 6);
        assert_eq!(game.cards_in_zone(ZoneType::Hand, p1).len(), 5);
    }

    #[test]
    fn a_mulligan_after_a_filtered_hand_redraws_from_a_single_shuffle() {
        use rand::RngCore;

        let p0 = PlayerId(0);
        for seed in 0..8 {
            let (mut kept, _) = setup_game_with_libraries(40);
            kept.filtered_hands = Some(FilteredHands::default());
            let mut mulliganed = kept.clone();
            let mut kept_rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut mulliganed_rng = kept_rng.clone();

            let mut keepers: Vec<Box<dyn PlayerAgent>> =
                vec![Box::new(TestAgent::keep()), Box::new(TestAgent::keep())];
            crate::game_loop::GameLoop::new(2).setup(&mut kept, &mut keepers, &mut kept_rng);
            let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
                Box::new(TestAgent::mulligan(1)),
                Box::new(TestAgent::keep()),
            ];
            crate::game_loop::GameLoop::new(2).setup(
                &mut mulliganed,
                &mut agents,
                &mut mulliganed_rng,
            );

            for card in kept.cards_in_zone(ZoneType::Hand, p0).to_vec() {
                kept.move_card(card, ZoneType::Library, p0);
            }
            kept.shuffle_library(p0, &mut kept_rng);
            let redrawn = kept.draw_cards(p0, 7);

            let hand = mulliganed.cards_in_zone(ZoneType::Hand, p0);
            assert_eq!(hand.len(), 6);
            assert!(hand.iter().all(|card| redrawn.contains(card)));
            assert_eq!(mulliganed_rng.next_u64(), kept_rng.next_u64());
        }
    }

    fn land_card(owner: PlayerId) -> Card {
        Card::new(
            CardId(0),
            "Plains".to_string(),
            owner,
            CardTypeLine::parse("Land"),
            ManaCost::no_cost(),
            ColorSet::COLORLESS,
            None,
            None,
            vec![],
            vec![],
        )
    }

    #[test]
    fn a_smoothed_mulligan_redraws_the_earliest_closest_of_three_candidates() {
        use rand::seq::SliceRandom;
        use rand::RngCore;

        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        let mut swapped = 0;
        for seed in 0..32 {
            let mut game = GameState::new(&["Alice", "Bob"], 20);
            for owner in [p0, p1] {
                for i in 0..40 {
                    let card = if i % 2 == 0 {
                        land_card(owner)
                    } else {
                        filler_card(owner)
                    };
                    let card = game.create_card(card);
                    game.add_card_to_zone(ZoneType::Library, owner, card);
                    game.card_mut(card).zone = ZoneType::Library;
                }
            }
            game.filtered_hands = Some(FilteredHands {
                candidates: 3,
                mulligans: true,
                choice: FilteredHandsChoice::Deterministic,
            });
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            game.shuffle_library(p0, &mut rng);
            game.shuffle_library(p1, &mut rng);
            game.draw_cards(p0, 7);
            game.draw_cards(p1, 7);

            let mut twin = game.clone();
            let mut probe = rng.clone();
            for card in twin.cards_in_zone(ZoneType::Hand, p0).to_vec() {
                twin.move_card(card, ZoneType::Library, p0);
            }
            twin.shuffle_library(p0, &mut probe);
            let library = twin.cards_in_zone(ZoneType::Library, p0).to_vec();
            let mut candidates = vec![library.clone()];
            for _ in 0..2 {
                let mut shuffled = library.clone();
                shuffled.shuffle(&mut probe);
                candidates.push(shuffled);
            }
            let distances: Vec<i32> = candidates
                .iter()
                .map(|cards| {
                    let lands = cards[cards.len() - 7..]
                        .iter()
                        .filter(|&&c| twin.card(c).is_land())
                        .count() as i32;
                    (2 * lands - 7).abs()
                })
                .collect();
            let best = *distances.iter().min().expect("a candidate");
            let chosen = distances
                .iter()
                .position(|&d| d == best)
                .expect("a candidate");
            let redrawn = &candidates[chosen][candidates[chosen].len() - 7..];
            swapped += usize::from(chosen > 0);

            let mut agents: Vec<Box<dyn PlayerAgent>> = vec![
                Box::new(TestAgent::mulligan(1)),
                Box::new(TestAgent::keep()),
            ];
            let pools = vec![ManaPool::new(), ManaPool::new()];
            let log = GameLog::new();
            run_london_mulligans(&mut game, &mut agents, &mut rng, p0, &pools, &log);

            let hand = game.cards_in_zone(ZoneType::Hand, p0);
            assert_eq!(hand.len(), 6);
            assert!(hand.iter().all(|card| redrawn.contains(card)));
            assert_eq!(rng.next_u64(), probe.next_u64());
        }
        assert!(swapped > 0);
    }

    #[test]
    fn mulligan_order_rotates_correctly() {
        let order = vec![PlayerId(0), PlayerId(1), PlayerId(2)];
        assert_eq!(
            mulligan_order(&order, PlayerId(1)),
            vec![PlayerId(1), PlayerId(2), PlayerId(0)]
        );
        assert_eq!(
            mulligan_order(&order, PlayerId(0)),
            vec![PlayerId(0), PlayerId(1), PlayerId(2)]
        );
    }
}
