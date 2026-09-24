# manabrew-gym

Reinforcement-learning environment over the Rust engine. The trainer is meant to be Python (PyTorch) through a PyO3 binding that does not exist yet; everything the trainer needs to see (decisions, legal options, observation buffers) is produced here, in Rust, so a Rust trainer stays possible.

## Layout

| File                    | Role                                                                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/game_env.rs`       | `GameEnv` (one game), `EnvConfig`, `GameSpec`, `Step`, `Outcome`, `EndReason`, the game thread (`Worker`, `play`) and `state_checksum`.                                              |
| `src/vec_env.rs`        | `VecEnv`: N game threads behind one event channel. `reset`/`send`/`recv(min)` is the asynchronous shape; `reset_all`/`step_all` is lockstep. `GameEnv` is a `VecEnv` of one.         |
| `src/learner_agent.rs`  | `LearnerAgent`, the channel-backed `PlayerAgent` for learner seats, and `Link`, the per-game channel ends and caps it shares between seats.                                          |
| `src/decision.rs`       | `Decision`, `DecisionKind` (typed legal options), `Action`, `ActionError`, validation.                                                                                               |
| `src/random_agent.rs`   | `Opponent` (non-learner seats), `RandomAgent` (a copy of the selfplay example's agent), `RandomPolicy` (the same distributions over `DecisionKind`, used by the bench and the test). |
| `src/encode/`           | Observation encoder (`mod.rs`), candidate flattening and picks-to-`Action` (`action.rs`), card vocabulary (`vocab.rs`).                                                              |
| `card_vocab.txt`        | Card-name vocabulary, one name per line; id = line index + 3 (0 pad, 1 unknown, 2 hidden). Generated, append-only.                                                                   |
| `examples/bench.rs`     | VecEnv throughput on the survey decks; prints a selfplay-compatible checksum.                                                                                                        |
| `examples/gen_vocab.rs` | Appends new names to `card_vocab.txt`.                                                                                                                                               |

## How a game runs

`VecEnv::reset` spawns one OS thread per game (`PARITY_THREAD_STACK_SIZE` stack), built exactly like the selfplay example: `build_deck_from_spec`, lazy priority action space, `SeededGameRng(seed ^ 0x9e37_79b9_7f4a_7c15)`, `GameRuntime::run(StdRng(seed), max_turns)`. With no learner seats the bench reproduces the selfplay checksum for the same deck pair and seeds; keep it that way, it is the check that game setup has not drifted.

A learner seat's agent sends a `Decision` and blocks on the action channel. The action is validated on the game thread; an invalid one comes back as `Err(ActionError)` and the decision stays open. Dropping the env (or `reset` on a running env) sets the game loop's abort signal and closes the channel; the agent then answers every call with its default and the loop exits at its next abort check. A panic inside the engine ends the game with `EndReason::EnginePanic`.

Caps: `max_turns` (the engine's own turn limit, `EndReason::TurnCap`), `Limits::max_decisions` and `Limits::max_turn_decisions` (learner decisions per game and per turn). A cap ends the game with no winner.

## PlayerAgent methods

Learner decisions: `choose_action` (options: pass, every playable `PlayOption`, every non-mana activatable; pass-only windows are answered without a decision), `choose_land_or_spell`, `get_ability_to_play`, `choose_attackers` / `choose_blockers` (per-creature legal defenders or attackers), every target method (`choose_targets_for` goes through `choose_targets_by_kind`; one candidate is answered without a decision), `choose_target_cards`, `choose_cards_for_effect` (and the zone-change methods that default to it), `choose_dig`, `choose_sacrifice`, `choose_permanents_to_sacrifice`, `choose_discard`, `choose_discard_any_number`, `choose_mode`, `choose_optional_trigger`, `confirm_action` (and `choose_binary`, `choose_sa_to_activate_from_opening_hand`, which default to it), `confirm_replacement_effect`.

Fixed answers: `mulligan_decision` keeps, `pay_mana_cost` returns `Pay { auto: true }` (the trait default cancels every spell), `choose_random_discard` draws from a seeded RNG, `choose_tap_type_for_cost` taps the strongest creatures until the power floor is met, and `confirm_replacement_effect` declines a prompt that repeats its source and question since the last priority decision (the selfplay guard against Superior Spider-Man's loop). Everything else is the trait default.

## Observation encoding

The engine hands the agent a `&GameState` only in `snapshot_state` and a few methods; `choose_action`, `choose_attackers` and `choose_blockers` get none. So `snapshot_state` encodes the state (and, in the declare steps, the per-creature combat legality), and the decision methods add the candidate table from the last snapshot. `snapshot_state` runs about 20 times per learner decision because most priority windows are passed without one; `Outcome::observes` and `Outcome::encoding` measure it.

Buffers (row-major, shapes from `EncoderConfig::shapes`, column names from the `*_INTS` / `*_FEATURES` constants in `encode/mod.rs`):

- cards: `card_ints [N,3]` (vocab id, attached-to row, blocking row), `card_floats [N,50]`, `card_mask [N]`. Rows: viewer battlefield, opponent battlefield, viewer hand, stacks, graveyards, exiles, command zones; overflow is counted in `global.cards_dropped`. The opponent's hand and both libraries are counts in `global` only; a face-down card the viewer does not control has vocab id 2 and, off the battlefield, only zone and owner features.
- `global [53]`: turn, phase, priority, day/night, and per player life, poison, zone counts, land drop, speed, mana pool.
- stack: `stack_ints [S,4]` (vocab id, source row, target card row, target stack row), `stack_floats [S,8]`, `stack_mask [S]`, top of stack first.
- candidates: `candidate_ints [K,5]` (card row, other row, stack row, index, vocab id), `candidate_floats [K,22]`, `candidate_mask [K]`; `decision [4]` = kind id, min picks, max picks, untruncated candidate count.

Defaults: N 160, S 16, K 128. `encode::action::candidates` flattens a `DecisionKind` into the candidate list (attack and block pairs included), and `encode::action::action_from_picks` turns picked candidate indices back into an `Action`.

## Vocabulary

`cargo run -p manabrew-gym --example gen_vocab` reads the Standard sets from the archive's `formats/Sanctioned/Standard.txt`, the card and token lines of those editions, the survey decks, and the tokens their scripts make, and appends names it has not seen. Never reorder or delete lines: a trained embedding table is indexed by line.

## Running

Tests, examples and benches need `CARDSET_ARCHIVE` (or the archive at `src-tauri/resources/cardset.rkyv` relative to the working directory). Deck and matchup paths resolve from `data::repo_root()`, so they work from any working directory.

```bash
cargo test -p manabrew-gym
cargo build --profile selfplay -p manabrew-gym --example bench
target/selfplay/examples/bench 8 400 2 1        # 8 envs, 400 games, both seats learners, play weight 1
target/selfplay/examples/bench 8 400 0 1 0 0 0  # no learner seats; matches `selfplay survey_g000 survey_g001 400 40 1`
```
