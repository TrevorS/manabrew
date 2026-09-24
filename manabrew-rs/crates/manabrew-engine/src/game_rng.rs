//! Pluggable RNG for game effects (shuffles, coin flips, dice rolls).
//!
//! By default, effects use the system's thread-local RNG. For parity testing,
//! a deterministic RNG (e.g. JavaRandom) can be injected to match Java Forge's
//! `MyRandom` consumption order exactly.
//!
//! # WASM Compatibility
//!
//! The `rand` crate 0.8+ supports WASM via `getrandom`. For browser WASM,
//! ensure the WASM entry point crate (wasm) includes:
//! ```toml
//! getrandom = { version = "0.2", features = ["js"] }
//! ```
//! This enables `thread_rng()` to work in browser environments.

use crate::ids::CardId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameRngState {
    pub seed: i64,
    pub call_count: u64,
    pub api_call_count: u64,
}

/// Trait for game-level randomness, used by effect resolvers.
///
/// This abstraction lets parity tests inject a Java-compatible RNG
/// that matches `java.util.Random` and `Collections.shuffle()` exactly,
/// while normal gameplay uses the default thread-local RNG.
pub trait GameRng {
    /// Shuffle a slice of CardIds in-place.
    /// Must match `java.util.Collections.shuffle(list, rng)` for parity, so the slice has to be in
    /// the order Java's own list is in. Fisher-Yates walks from the end, so reversing the slice
    /// first draws the same numbers onto different positions. A caller holding a library, which
    /// Rust stores last-element-is-top against Java's index-0-is-top, converts around this call;
    /// `Zone::shuffle` is the only one that does.
    fn shuffle_cards(&mut self, cards: &mut [CardId]);

    /// Return a random integer in `[0, bound)`.
    /// Must match `java.util.Random.nextInt(bound)` for parity.
    fn next_int(&mut self, bound: i32) -> i32;

    fn next_boolean(&mut self) -> bool {
        self.next_int(2) == 1
    }

    /// Debug: return the total number of RNG calls made so far (if tracked).
    fn call_count(&self) -> u64 {
        0
    }

    fn save_state(&self) -> Option<GameRngState> {
        None
    }

    fn restore_state(&mut self, _state: GameRngState) {}
}

/// Default RNG using `rand::thread_rng()` — non-deterministic, for normal gameplay.
pub struct ThreadRngAdapter {
    #[cfg(debug_assertions)]
    site: &'static std::panic::Location<'static>,
}

impl Default for ThreadRngAdapter {
    #[cfg_attr(debug_assertions, track_caller)]
    fn default() -> Self {
        ThreadRngAdapter {
            #[cfg(debug_assertions)]
            site: std::panic::Location::caller(),
        }
    }
}

impl ThreadRngAdapter {
    #[cfg(debug_assertions)]
    fn report_fallback_draw(&self) {
        let site = format!("{}:{}", self.site.file(), self.site.line());
        if std::env::var_os("FORGE_RNG_STRICT").is_some_and(|v| v == "1") {
            panic!("[rng-fallback] {site} drew outside the game RNG");
        }
        eprintln!("[rng-fallback] {site}");
    }
}

#[allow(clippy::disallowed_methods)]
impl GameRng for ThreadRngAdapter {
    fn shuffle_cards(&mut self, cards: &mut [CardId]) {
        use rand::seq::SliceRandom;
        #[cfg(debug_assertions)]
        self.report_fallback_draw();
        cards.shuffle(&mut rand::thread_rng());
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        use rand::Rng;
        #[cfg(debug_assertions)]
        self.report_fallback_draw();
        rand::thread_rng().gen_range(0..bound)
    }
}
