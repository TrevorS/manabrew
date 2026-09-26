//! Static utility methods related to combat.
use forge_foundation::ZoneType;

use super::attack_constraints::AttackConstraints;
use super::{CombatState, DefenderId, LureType};
use crate::card::valid_filter;
use crate::card::Card;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::staticability::{
    static_ability_block_restrict, static_ability_cant_attack_block, static_ability_must_block,
};

pub fn get_available_attackers(game: &GameState, player: PlayerId) -> Vec<CardId> {
    let defending = game.opponent_of(player);
    let defenders = get_possible_defenders(game, player);
    game.creatures_on_battlefield(player)
        .into_iter()
        .filter(|&cid| {
            let card = game.card(cid);
            let ready = card.can_attack()
                || (card.is_creature()
                    && !card.tapped
                    && !card.detained
                    && (card.has_haste() || !card.summoning_sick)
                    && card.zone == ZoneType::Battlefield
                    && card.has_defender()
                    && crate::staticability::static_ability_cant_attack_block::can_attack_defender(
                        game,
                        &game.cards,
                        card,
                        defending,
                    ));
            ready
                && defenders.iter().any(|&defender| {
                    !crate::staticability::static_ability_cant_attack_block::cant_attack(
                        game,
                        &game.cards,
                        card,
                        defender,
                    )
                })
        })
        .collect()
}

pub fn can_attack_player(game: &GameState, player: PlayerId) -> bool {
    !get_available_attackers(game, player).is_empty()
}

pub fn can_attack_defender(game: &GameState, attacker_id: CardId, defender: DefenderId) -> bool {
    let card = game.card(attacker_id);

    // Basic creature checks
    if !card.is_creature() || card.tapped || card.phased_out {
        return false;
    }
    // Summoning sickness (unless haste)
    if card.summoning_sick && !card.has_haste() {
        return false;
    }

    if crate::staticability::static_ability_cant_attack_block::cant_attack(
        game,
        &game.cards,
        card,
        defender,
    ) {
        return false;
    }

    true
}

pub fn validate_attackers(
    game: &GameState,
    constraints: &AttackConstraints,
    current_attackers: &[(CardId, DefenderId)],
) -> bool {
    let my_violations = constraints.count_violations(current_attackers, &game.cards);
    if my_violations == -1 {
        return false;
    }
    let (_, best_violations) = constraints.get_legal_attackers(&game.cards);
    my_violations <= best_violations
}

pub fn get_possible_defenders(game: &GameState, attacking_player: PlayerId) -> Vec<DefenderId> {
    let mut defenders = Vec::new();
    for pid in game.alive_players() {
        if pid == attacking_player {
            continue;
        }
        defenders.push(DefenderId::Player(pid));
        // Planeswalkers controlled by this opponent
        for &cid in game.cards_in_zone(ZoneType::Battlefield, pid) {
            let card = game.card(cid);
            if card.type_line.is_planeswalker() {
                defenders.push(DefenderId::Permanent(cid));
            }
        }
    }
    defenders
}

pub fn get_available_blockers(game: &GameState, player: PlayerId) -> Vec<CardId> {
    game.creatures_on_battlefield(player)
        .into_iter()
        .filter(|&cid| can_block(game, cid))
        .collect()
}

pub fn can_creature_block(game: &GameState, blocker_id: CardId, attacker_id: CardId) -> bool {
    let attacker = game.card(attacker_id);
    let blocker = game.card(blocker_id);

    if !blocker.can_block() {
        return false;
    }

    // Flying: only blocked by flying or reach
    if attacker.has_flying() && !blocker.has_flying() && !blocker.has_reach() {
        return false;
    }
    // Fear: only blocked by artifact or black creatures
    if attacker.has_fear() && !blocker.type_line.is_artifact() && !blocker.color.has_black() {
        return false;
    }
    // Intimidate: only blocked by artifact or creatures sharing a color
    if attacker.has_intimidate()
        && !blocker.type_line.is_artifact()
        && !blocker.color.shares_color_with(attacker.color)
    {
        return false;
    }
    // Shadow: shadow only blocked by shadow, non-shadow not blocked by shadow
    if attacker.has_shadow() != blocker.has_shadow() {
        return false;
    }
    // Horsemanship: only blocked by horsemanship
    if attacker.has_horsemanship() && !blocker.has_horsemanship() {
        return false;
    }
    // Skulk: can't be blocked by creatures with greater power
    if attacker.has_skulk() && blocker.power() > attacker.power() {
        return false;
    }
    // Protection: can't be blocked by matching creatures
    if attacker.is_protected_from(blocker) {
        return false;
    }
    // CantBlockBy static abilities
    if static_ability_cant_attack_block::cant_block_by(game, &game.cards, attacker, Some(blocker)) {
        return false;
    }
    true
}

/// Filter blockers to only those that can legally block at least one attacker.
pub fn filter_legal_blockers(
    game: &GameState,
    attackers: &[CardId],
    blockers: &[CardId],
) -> Vec<CardId> {
    blockers
        .iter()
        .filter(|&&blocker_id| {
            attackers
                .iter()
                .any(|&attacker_id| can_creature_block(game, blocker_id, attacker_id))
        })
        .copied()
        .collect()
}

pub fn blockers_missing_group_block(
    game: &GameState,
    combat: &CombatState,
    defender: PlayerId,
) -> Vec<CardId> {
    let mut removed: Vec<CardId> = Vec::new();
    loop {
        let mut remaining_blockers: Vec<CardId> = Vec::new();
        for &(blocker_id, _) in &combat.blockers {
            if !removed.contains(&blocker_id)
                && !remaining_blockers.contains(&blocker_id)
                && game.card(blocker_id).controller == defender
            {
                remaining_blockers.push(blocker_id);
            }
        }
        let mut reached_steady_state = true;
        for &blocker_id in &remaining_blockers {
            let blocker = game.card(blocker_id);
            let cant_block_alone = blocker.has_keyword("CARDNAME can't attack or block alone.")
                || blocker.has_keyword("CARDNAME can't block alone.");
            let remove_blocker = if remaining_blockers.len() < 2 && cant_block_alone {
                true
            } else if remaining_blockers.len() < 3
                && blocker
                    .has_keyword("CARDNAME can't block unless at least two other creatures block.")
            {
                true
            } else if blocker.has_keyword(
                "CARDNAME can't block unless a creature with greater power also blocks.",
            ) {
                let power = blocker.power();
                !remaining_blockers
                    .iter()
                    .any(|&other| game.card(other).power() > power)
            } else {
                false
            };
            if remove_blocker {
                removed.push(blocker_id);
                reached_steady_state = false;
            }
        }
        if reached_steady_state {
            return removed;
        }
    }
}

pub fn validate_blocks(
    game: &GameState,
    combat: &CombatState,
    defending: PlayerId,
) -> Option<String> {
    let defenders_army = game.creatures_on_battlefield(defending);
    let blockers: Vec<CardId> = combat
        .get_all_blockers()
        .into_iter()
        .filter(|&b| game.card(b).controller == defending)
        .collect();
    let mut free_blockers = find_free_blockers(game, &defenders_army, combat);
    let has_block_cost = |blocker: &Card, attacker_id: CardId| {
        super::block_cost::get_block_cost(game, blocker, game.card(attacker_id)) > 0
    };

    for &blocker_id in &defenders_army {
        let blocker = game.card(blocker_id);
        let blocked_so_far = combat.get_attackers_for(blocker_id);
        for &to_be_blocked in &blocker.must_block_cards {
            if has_block_cost(blocker, to_be_blocked) {
                continue;
            }
            let additional_blockers =
                get_min_num_blockers_for_attacker(game, to_be_blocked, defending) - 1;
            let mut potential_blockers = 0;
            for _ in 0..additional_blockers {
                for free_blocker in free_blockers.clone() {
                    if free_blocker != blocker_id
                        && can_creature_block(game, free_blocker, to_be_blocked)
                    {
                        free_blockers.retain(|&b| b != free_blocker);
                        potential_blockers += 1;
                    }
                }
            }
            if potential_blockers >= additional_blockers
                && !blocked_so_far.contains(&to_be_blocked)
                && (can_block_more_creatures(game, blocker_id, &blocked_so_far)
                    || free_blockers.contains(&blocker_id))
                && combat.is_attacking(to_be_blocked)
                && can_creature_block(game, blocker_id, to_be_blocked)
            {
                return Some(format!(
                    "{} must still block {}.",
                    blocker.log_name(),
                    game.card(to_be_blocked).log_name()
                ));
            }
        }
        if must_block_an_attacker(game, combat, blocker_id, Some(&free_blockers)) {
            let which = if blockers.contains(&blocker_id) {
                "the right ones."
            } else {
                "any."
            };
            return Some(format!(
                "{} must block an attacker, but has not been assigned to block {which}",
                blocker.log_name()
            ));
        }
        if !blockers.contains(&blocker_id)
            && static_ability_must_block::blocks_each_combat_if_able(&game.cards, blocker)
        {
            for &(attacker_id, _) in &combat.attackers {
                if has_block_cost(blocker, attacker_id)
                    || !can_creature_block_in_combat(game, combat, blocker_id, attacker_id)
                {
                    continue;
                }
                let must = get_min_num_blockers_for_attacker(game, attacker_id, defending) <= 1
                    || {
                        let mut possible_blockers = free_blockers.clone();
                        possible_blockers.extend(combat.get_blockers_for(attacker_id));
                        can_be_blocked_with(game, combat, attacker_id, &possible_blockers)
                    };
                if must {
                    return Some(format!(
                        "{} must block each combat but was not assigned to block any attacker now.",
                        blocker.log_name()
                    ));
                }
            }
        }
    }

    for &blocker_id in &blockers {
        let blocker = game.card(blocker_id);
        let cant_block_alone = blocker.has_keyword("CARDNAME can't attack or block alone.")
            || blocker.has_keyword("CARDNAME can't block alone.");
        if blockers.len() < 2 && cant_block_alone {
            return Some(format!("{} can't block alone.", blocker.log_name()));
        } else if blockers.len() < 3
            && blocker
                .has_keyword("CARDNAME can't block unless at least two other creatures block.")
        {
            return Some(format!(
                "{} can't block unless at least two other creatures block.",
                blocker.log_name()
            ));
        } else if blocker
            .has_keyword("CARDNAME can't block unless a creature with greater power also blocks.")
        {
            let power = blocker.power();
            if !blockers
                .iter()
                .any(|&other| game.card(other).power() > power)
            {
                return Some(format!(
                    "{} can't block unless a creature with greater power also blocks.",
                    blocker.log_name()
                ));
            }
        }
    }

    for &(attacker_id, _) in &combat.attackers {
        let blocker_count = combat.get_blockers_for(attacker_id).len();
        if blocker_count > 0
            && !can_attacker_be_blocked_with_amount(game, combat, attacker_id, blocker_count)
        {
            return Some(format!(
                "{} cannot be blocked with {blocker_count} creatures you've assigned",
                game.card(attacker_id).log_name()
            ));
        }
    }

    None
}

pub fn must_block_an_attacker(
    game: &GameState,
    combat: &CombatState,
    blocker_id: CardId,
    free_blockers: Option<&[CardId]>,
) -> bool {
    let blocker = game.card(blocker_id);
    let defender = blocker.controller;
    let has_block_cost = |attacker_id: CardId| {
        super::block_cost::get_block_cost(game, blocker, game.card(attacker_id)) > 0
    };
    let mut requirement_cards = Vec::new();
    for &(attacker_id, _) in &combat.attackers {
        if has_block_cost(attacker_id)
            || attacker_lure_satisfied(
                game,
                attacker_id,
                blocker_id,
                &combat.get_blockers_for(attacker_id),
            )
        {
            continue;
        }
        if can_be_blocked(game, combat, attacker_id, defender)
            && can_creature_block(game, blocker_id, attacker_id)
            && can_be_blocked_without(game, combat, attacker_id, blocker_id, None)
        {
            requirement_cards.push(attacker_id);
        }
    }
    for &attacker_id in &blocker.must_block_cards {
        if !has_block_cost(attacker_id)
            && can_be_blocked(game, combat, attacker_id, defender)
            && can_creature_block(game, blocker_id, attacker_id)
            && combat.is_attacking(attacker_id)
            && can_be_blocked_without(game, combat, attacker_id, blocker_id, free_blockers)
            && !requirement_cards.contains(&attacker_id)
        {
            requirement_cards.push(attacker_id);
        }
    }
    if requirement_cards.is_empty() {
        return false;
    }
    let blocking = combat.get_attackers_for(blocker_id);
    if requirement_cards.iter().all(|a| blocking.contains(a)) {
        return false;
    }
    if !can_block_in_combat(game, combat, blocker_id) {
        for &(attacker_id, _) in &combat.attackers {
            let mut blockers = combat.get_blockers_for(attacker_id);
            if blockers.contains(&blocker_id)
                && attacker_lure_satisfied(game, attacker_id, blocker_id, &blockers)
            {
                blockers.retain(|&b| b != blocker_id);
                if !attacker_lure_satisfied(game, attacker_id, blocker_id, &blockers) {
                    return false;
                }
            }
        }
    }
    requirement_cards.iter().all(|a| !blocking.contains(a))
}

fn can_be_blocked_without(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
    blocker_id: CardId,
    free_blockers: Option<&[CardId]>,
) -> bool {
    let defending = defending_player_of(game, combat, attacker_id);
    if get_min_num_blockers_for_attacker(game, attacker_id, defending) <= 1 {
        return true;
    }
    let mut blockers = free_blockers.map_or_else(
        || game.creatures_on_battlefield(defending),
        <[CardId]>::to_vec,
    );
    blockers.retain(|&b| b != blocker_id);
    can_be_blocked_with(game, combat, attacker_id, &blockers)
}

fn defending_player_of(game: &GameState, combat: &CombatState, attacker_id: CardId) -> PlayerId {
    combat
        .get_defender_player_by_attacker(attacker_id, game)
        .unwrap_or_else(|| game.opponent_of(game.card(attacker_id).controller))
}

fn attacker_lure_satisfied(
    game: &GameState,
    attacker_id: CardId,
    blocker_id: CardId,
    blockers: &[CardId],
) -> bool {
    let attacker = game.card(attacker_id);
    let keywords: Vec<&str> = attacker
        .keywords
        .iter_strings()
        .chain(attacker.granted_keywords.iter_strings())
        .chain(attacker.pump_keywords.iter_strings())
        .map(|kw| kw.strip_prefix("HIDDEN ").unwrap_or(kw))
        .collect();
    let has_start_of = |prefix: &str| keywords.iter().any(|kw| kw.starts_with(prefix));
    if has_start_of("All creatures able to block CARDNAME do so.")
        || (has_start_of("CARDNAME must be blocked if able.") && blockers.is_empty())
        || (has_start_of("CARDNAME must be blocked by exactly one creature if able.")
            && blockers.len() != 1)
        || (has_start_of("CARDNAME must be blocked by two or more creatures if able.")
            && blockers.len() < 2)
    {
        return false;
    }
    let blocker = game.card(blocker_id);
    let is_valid =
        |card: &Card, valid: &str| valid_filter::matches_valid_card(valid, card, attacker);
    for keyword in keywords {
        if let Some(valid) = keyword.strip_prefix("MustBeBlockedBy ") {
            if is_valid(blocker, valid) && !blockers.iter().any(|&b| is_valid(game.card(b), valid))
            {
                return false;
            }
        }
        if keyword.starts_with("MustBeBlockedByAll") {
            if let Some(valid) = keyword.split(':').nth(1) {
                if is_valid(blocker, valid) {
                    return false;
                }
            }
        }
    }
    true
}

pub fn lure_forbids_block(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
    blocker_id: CardId,
) -> bool {
    attacker_lure_satisfied(
        game,
        attacker_id,
        blocker_id,
        &combat.get_blockers_for(attacker_id),
    ) && !game
        .card(blocker_id)
        .must_block_cards
        .contains(&attacker_id)
        && must_block_an_attacker(game, combat, blocker_id, None)
}

/// Determine the lure type of an attacker.
pub fn get_lure_type(card: &Card) -> LureType {
    for kw in card
        .keywords
        .iter_strings()
        .chain(card.granted_keywords.iter_strings())
        .chain(card.pump_keywords.iter_strings())
    {
        let lower = kw.to_lowercase();
        if lower.contains("all creatures able to block") && lower.contains("do so") {
            return LureType::AllMustBlock;
        }
        if lower.contains("must be blocked if able") {
            return LureType::MustBeBlockedIfAble;
        }
    }
    LureType::None
}

pub fn compute_must_block_targets(
    game: &GameState,
    combat: &CombatState,
    blocker_id: CardId,
) -> Vec<CardId> {
    let required = must_block_an_attacker(game, combat, blocker_id, None)
        || (!combat.is_blocking(blocker_id)
            && static_ability_must_block::blocks_each_combat_if_able(
                &game.cards,
                game.card(blocker_id),
            ));
    if !required {
        return Vec::new();
    }
    combat
        .attackers
        .iter()
        .map(|&(attacker_id, _)| attacker_id)
        .filter(|&attacker_id| can_creature_block_in_combat(game, combat, blocker_id, attacker_id))
        .collect()
}

pub fn can_block_more_creatures(
    game: &GameState,
    blocker_id: CardId,
    blocked_by: &[CardId],
) -> bool {
    let blocker = game.card(blocker_id);
    blocked_by.is_empty()
        || blocker.can_block_any()
        || usize::try_from(blocker.can_block_additional()).unwrap_or(0) >= blocked_by.len()
}

pub fn get_min_num_blockers_for_attacker(
    game: &GameState,
    attacker_id: CardId,
    defender: PlayerId,
) -> i32 {
    static_ability_cant_attack_block::get_min_max_blocker(
        game,
        &game.cards,
        game.card(attacker_id),
        defender,
    )
    .0
}

pub fn can_attacker_be_blocked_with_amount(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
    amount: usize,
) -> bool {
    if amount == 0 {
        return false;
    }
    let (min, max) = static_ability_cant_attack_block::get_min_max_blocker(
        game,
        &game.cards,
        game.card(attacker_id),
        defending_player_of(game, combat, attacker_id),
    );
    let amount = i64::try_from(amount).unwrap_or(i64::MAX);
    i64::from(min) <= amount && amount <= i64::from(max)
}

/// Declared-attacker trigger helper.
pub fn check_declared_attacker(game: &mut GameState, attacker_id: CardId, defender: DefenderId) {
    let controlling = defender.controlling_player(game);
    game.card_mut(attacker_id).set_attacking_player(controlling);
}

/// Get attack constraints for a combat.
pub fn get_all_requirements(
    game: &GameState,
    attacking_player: PlayerId,
    possible_defenders: &[DefenderId],
) -> AttackConstraints {
    AttackConstraints::new(game, attacking_player, possible_defenders)
}

/// Check if a creature can attack any legal defender.
pub fn can_attack(game: &GameState, attacker_id: CardId) -> bool {
    let card = game.card(attacker_id);
    let possible_defenders = get_possible_defenders(game, card.controller);
    possible_defenders
        .iter()
        .any(|&defender| can_attack_defender(game, attacker_id, defender))
}

/// Check if a creature could attack next turn (ignores tap/summoning sickness).
pub fn can_attack_next_turn(game: &GameState, attacker_id: CardId, defender: DefenderId) -> bool {
    let card = game.card(attacker_id);

    // Skip tap/summoning sickness checks for next-turn evaluation
    if !card.is_creature() || card.phased_out {
        return false;
    }
    if crate::staticability::static_ability_cant_attack_block::cant_attack(
        game,
        &game.cards,
        card,
        defender,
    ) {
        return false;
    }
    true
}

/// Check if a creature could attack but is not currently attacking.
pub fn could_attack_but_not_attacking(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
) -> bool {
    if combat.is_attacking(attacker_id) {
        return false;
    }
    can_attack(game, attacker_id)
}

/// Check propaganda-style effects that require paying a cost to attack.
pub fn check_propaganda_effects(
    game: &GameState,
    attacker_id: CardId,
    defender: DefenderId,
) -> bool {
    let attacker = game.card(attacker_id);
    let cost = super::attack_cost::get_attack_cost(game, attacker, defender);
    cost == 0
}

/// Pay required block costs for a blocker.
pub fn pay_required_block_costs(game: &GameState, blocker_id: CardId, attacker_id: CardId) -> bool {
    let blocker = game.card(blocker_id);
    let attacker = game.card(attacker_id);
    let cost = super::block_cost::get_block_cost(game, blocker, attacker);
    cost == 0
}

/// Check if a creature can block (basic check: untapped creature).
pub fn can_block(game: &GameState, blocker_id: CardId) -> bool {
    let blocker = game.card(blocker_id);

    if !blocker.is_creature() || blocker.phased_out {
        return false;
    }

    if blocker.tapped
        && !static_ability_cant_attack_block::can_block_tapped(game, &game.cards, blocker)
    {
        return false;
    }

    if blocker.has_keyword("CARDNAME can't block.")
        || blocker.has_keyword("CARDNAME can't attack or block.")
    {
        return false;
    }

    if static_ability_cant_attack_block::cant_block(game, &game.cards, blocker) {
        return false;
    }

    blocker.zone == ZoneType::Battlefield
}

pub fn can_be_blocked(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
    defending: PlayerId,
) -> bool {
    let attacker = game.card(attacker_id);
    let (_, max) = static_ability_cant_attack_block::get_min_max_blocker(
        game,
        &game.cards,
        attacker,
        defending,
    );
    if usize::try_from(max).is_ok_and(|max| max == combat.get_blockers_for(attacker_id).len()) {
        return false;
    }
    if combat
        .get_defender_player_by_attacker(attacker_id, game)
        .is_some_and(|attacked| attacked != defending)
    {
        return false;
    }
    !static_ability_cant_attack_block::cant_block_by(game, &game.cards, attacker, None)
}

pub fn can_be_blocked_with(
    game: &GameState,
    combat: &CombatState,
    attacker_id: CardId,
    blockers: &[CardId],
) -> bool {
    let blocks = blockers
        .iter()
        .filter(|&&blocker_id| can_creature_block(game, blocker_id, attacker_id))
        .count();
    can_attacker_be_blocked_with_amount(game, combat, attacker_id, blocks)
}

pub fn can_block_in_combat(game: &GameState, combat: &CombatState, blocker_id: CardId) -> bool {
    if !can_block_more_creatures(game, blocker_id, &combat.get_attackers_for(blocker_id)) {
        return false;
    }
    let controller = game.card(blocker_id).controller;
    let other_blockers = combat
        .get_all_blockers()
        .into_iter()
        .filter(|&b| b != blocker_id && game.card(b).controller == controller)
        .count();
    if i64::try_from(other_blockers).unwrap_or(i64::MAX)
        >= i64::from(static_ability_block_restrict::block_restrict_num(
            &game.cards,
            controller,
        ))
    {
        return false;
    }
    can_block(game, blocker_id)
}

pub fn can_creature_block_in_combat(
    game: &GameState,
    combat: &CombatState,
    blocker_id: CardId,
    attacker_id: CardId,
) -> bool {
    can_block_in_combat(game, combat, blocker_id)
        && can_be_blocked(game, combat, attacker_id, game.card(blocker_id).controller)
        && !combat.is_blocking_attacker(blocker_id, attacker_id)
        && !lure_forbids_block(game, combat, attacker_id, blocker_id)
        && can_creature_block(game, blocker_id, attacker_id)
}

/// Check if a blocker can block at least one attacker from a list.
pub fn can_block_at_least_one(game: &GameState, blocker_id: CardId, attackers: &[CardId]) -> bool {
    attackers
        .iter()
        .any(|&attacker_id| can_creature_block(game, blocker_id, attacker_id))
}

pub fn find_free_blockers(
    game: &GameState,
    defenders_army: &[CardId],
    combat: &CombatState,
) -> Vec<CardId> {
    defenders_army
        .iter()
        .copied()
        .filter(|&blocker_id| {
            if !can_block(game, blocker_id)
                || must_block_an_attacker(game, combat, blocker_id, None)
            {
                return false;
            }
            let blocked_attackers = combat.get_attackers_for(blocker_id);
            blocked_attackers.is_empty()
                || blocked_attackers.iter().any(|&attacker_id| {
                    let mut blockers_reduced = combat.get_blockers_for(attacker_id);
                    blockers_reduced.retain(|&b| b != blocker_id);
                    can_block_more_creatures(game, blocker_id, &blocked_attackers)
                        || can_be_blocked_with(game, combat, attacker_id, &blockers_reduced)
                })
        })
        .collect()
}
