use std::collections::BTreeMap;

use forge_foundation::mana::ManaAtom;
use forge_foundation::ManaCostShard;

use crate::cost::{Cost, CostPart};
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::mana::mana_pool::mana_matches_context;
use crate::mana::{Mana, ManaPaymentContext, ManaPool};

pub struct CostPayment {
    /// The original, unadjusted cost.
    pub cost: Cost,
    /// The cost after adjustment (reductions, increases, etc.).
    pub adjusted_cost: Cost,
    /// The source card being paid for.
    pub source: CardId,
    /// The player paying the cost.
    pub player: PlayerId,
    /// Cost parts that have been successfully paid (for refund on cancel).
    pub paid_cost_parts: Vec<CostPart>,
    /// Whether we are paying as an effect (not a spell/ability activation).
    pub is_effect: bool,
}

impl CostPayment {
    /// Create a new `CostPayment` for the given cost and source.
    pub fn new(cost: Cost, source: CardId, player: PlayerId, is_effect: bool) -> Self {
        let adjusted_cost = cost.clone();
        CostPayment {
            cost,
            adjusted_cost,
            source,
            player,
            paid_cost_parts: Vec::new(),
            is_effect,
        }
    }

    /// Check if all cost parts have been paid.
    pub fn is_fully_paid(&self) -> bool {
        self.paid_cost_parts.len() == self.adjusted_cost.parts.len()
    }

    /// Refund all paid cost parts (on cancel/failure).
    pub fn refund_payment(&mut self, game: &mut GameState) {
        for part in &self.paid_cost_parts {
            refund_cost_part(game, self.source, self.player, part);
        }
        self.paid_cost_parts.clear();
    }

    /// Check if a cost can be paid as additional costs.
    pub fn can_pay_additional_costs(
        cost: &Cost,
        game: &GameState,
        source: CardId,
        player: PlayerId,
        _is_effect: bool,
    ) -> bool {
        // TODO: cost = CostAdjustment::adjust(cost, ability, effect);
        crate::cost::can_pay_ignoring_mana(cost, game, source, player)
    }

    /// Handle offering/emerge sacrifice after cost payment.
    pub fn handle_offerings(
        _game: &mut GameState,
        _source: CardId,
        _test: bool,
        _cost_is_paid: bool,
    ) -> bool {
        // TODO: Port Java's handleOfferings():
        // - If sa.isOffering(): sacrifice the offering card, fire zone triggers
        // - If sa.isEmerge(): sacrifice the emerge card, update LKI, fire zone triggers
        true
    }

    pub fn get_mana(
        pool: &mut ManaPool,
        shard: ManaCostShard,
        sa_being_paid_for: &ManaPaymentContext,
        any_color: bool,
        colors_paid: Option<u16>,
        x_mana_cost_paid_by_color: &BTreeMap<String, i32>,
        choose_mana_from_pool: &mut dyn FnMut(&[Mana]) -> usize,
    ) -> Option<Mana> {
        let weighted_options = Self::select_mana_to_pay_for(
            pool,
            shard,
            sa_being_paid_for,
            any_color,
            colors_paid,
            x_mana_cost_paid_by_color,
        );
        if weighted_options.is_empty() {
            return None;
        }

        let mut mana_choices: Vec<Mana> = Vec::new();
        let mut best_weight = i32::MIN;
        for (this_mana, this_weight) in weighted_options {
            if this_weight > best_weight {
                mana_choices.clear();
                best_weight = this_weight;
            }
            if this_weight == best_weight && !mana_choices.iter().any(|m| m.equals(&this_mana)) {
                mana_choices.push(this_mana);
            }
        }

        if mana_choices.len() == 1 {
            return mana_choices.pop();
        }
        let chosen = choose_mana_from_pool(&mana_choices);
        mana_choices.into_iter().nth(chosen)
    }

    fn select_mana_to_pay_for(
        manapool: &mut ManaPool,
        shard: ManaCostShard,
        sa_being_paid_for: &ManaPaymentContext,
        any_color: bool,
        colors_paid: Option<u16>,
        x_mana_cost_paid_by_color: &BTreeMap<String, i32>,
    ) -> Vec<(Mana, i32)> {
        let mut weighted_options = Vec::new();
        for this_mana in manapool.floating_mana() {
            if shard == ManaCostShard::ColoredX
                && x_mana_cost_paid_by_color.contains_key(ManaPool::atom_to_letter(this_mana.color))
            {
                continue;
            }
            if !manapool.can_pay_for_shard_with_color(shard, this_mana.color, any_color) {
                continue;
            }
            if shard.is_snow() && !this_mana.is_snow {
                continue;
            }
            if !mana_matches_context(&this_mana, sa_being_paid_for) {
                continue;
            }

            let mut weight = 0;
            match colors_paid {
                None => {
                    if this_mana.color == ManaAtom::COLORLESS {
                        weight += 5;
                    }
                }
                Some(colors_paid) => {
                    if (this_mana.color | colors_paid) != colors_paid {
                        weight += 5;
                    }
                }
            }
            if this_mana.restriction.is_some() {
                weight += 2;
            }
            if !this_mana.is_snow {
                weight += 1;
            }
            weighted_options.push((this_mana, weight));
        }
        weighted_options
    }
}

/// Refund a single cost part.
fn refund_cost_part(game: &mut GameState, source: CardId, player: PlayerId, part: &CostPart) {
    match part {
        CostPart::Tap => {
            crate::cost::cost_tap::refund(game, source);
        }
        CostPart::Untap => {
            crate::cost::cost_untap::refund(game, source);
        }
        CostPart::SubCounter {
            amount,
            counter_type,
            ..
        } => crate::cost::cost_remove_counter::refund(
            game,
            source,
            amount.resolve(game, source, player),
            counter_type,
        ),
        CostPart::AddCounter {
            amount,
            counter_type,
            ..
        } => {
            crate::cost::cost_put_counter::refund(
                game,
                source,
                amount.resolve(game, source, player),
                counter_type,
            );
        }
        CostPart::PayEnergy(amount) => {
            crate::cost::cost_pay_energy::refund(
                game,
                player,
                amount.resolve(game, source, player),
            );
        }
        CostPart::PayShards(amount) => {
            crate::cost::cost_pay_shards::refund(
                game,
                player,
                amount.resolve(game, source, player),
            );
        }
        CostPart::PayLife(amount) => {
            game.player_gain_life(player, amount.resolve(game, source, player));
        }
        CostPart::ChooseColor(_) => {
            crate::cost::cost_choose_color::refund(game, source);
        }
        CostPart::Blight(_) => {
            // Could refund by removing -1/-1 counters, but snapshot rollback handles it.
        }
        // Most cost parts (sacrifice, discard, exile, return, etc.) are not
        // individually refundable — zone changes are rolled back by GameSnapshot.
        _ => {
            eprintln!("[WARN] Unhandled cost part refund: {part:?}");
        }
    }
}
