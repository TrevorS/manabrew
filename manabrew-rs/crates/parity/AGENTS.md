# Parity harness

Differential testing: runs the Rust engine and the Java Forge engine on the same deck/seed and compares decision-by-decision. The home base for parity bug investigation.

Read first: `/AGENTS.md`, `docs/agents/ENGINE_BUGFIX_WORKFLOW.md`, `docs/PARITY_TESTING.md`.

## Layout

| File / folder                                                       | Role                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `regression.json`                                                   | The canonical regression suite. Each entry is `{name, args}`, where `args` is the parity CLI string. `yarn parity <name>` looks up entries here.                                                                                                       |
| `parity_ignore.json`                                                | Known-divergent matchups to skip, with a written reason.                                                                                                                                                                                               |
| `survey_matchups.tsv`, `survey_baseline.jsonl`                      | The Standard survey sample: 70 `deck1<TAB>deck2` matchups over `parity_decks/survey_g*.json`, and the gate file the baseline run produced (header line, then one JSON record per matchup). `scripts/survey-gate.sh` classifies a fresh run against it. |
| `src/runner.rs`, `src/scheduler.rs`                                 | Top-level orchestration.                                                                                                                                                                                                                               |
| `src/deterministic_agent.rs`                                        | The reproducible agent both engines drive. Same logic, same RNG, same decisions.                                                                                                                                                                       |
| `src/java_bridge.rs`, `src/java_cache.rs`, `src/java_random.rs`     | Java harness FFI — calls into `forge-harness/`.                                                                                                                                                                                                        |
| `src/runtime.rs`                                                    | Shared Rust/Java matchup runtime. CLI, CI/server mode, and debugger tooling should use this instead of growing separate engine scheduling logic.                                                                                                       |
| `src/comparator.rs`, `src/snapshot.rs`, `src/gate.rs`               | Snapshot comparison, snapshot extraction, and gate files (`--gate-out`, `parity gate-diff`).                                                                                                                                                           |
| `src/parity_log.rs`, `src/log_buffer.rs`, `src/callback_fmt.rs`     | Divergence reporting.                                                                                                                                                                                                                                  |
| `src/choice_space.rs`, `src/combat_choice_space.rs`                 | Legal-action enumeration mirrored against Java.                                                                                                                                                                                                        |
| `src/parity_card_map.rs`, `src/parity_id.rs`, `src/parity_order.rs` | Cross-engine identity bridging (card name ↔ id).                                                                                                                                                                                                       |
| `src/deck_generator.rs`, `src/card_pool.rs`                         | Deck construction for matrix runs.                                                                                                                                                                                                                     |
| `src/probe.rs`                                                      | Per-card probes against Java (`--probe`, `--probe-file`) and failing-deck shrinking (`--shrink`).                                                                                                                                                      |
| `src/decision_diff.rs`                                              | First differing agent decision, and the per-engine state delta and decisions between two snapshots.                                                                                                                                                    |
| `src/sweep.rs`                                                      | `parity sweep` / `sweep-deck`: process-isolated random-play sweep with loop detection, budgets, stall sampling and replay lines.                                                                                                                       |
| `src/script_index.rs`, `src/census_report.rs`                       | Card scripts as parsed `Key$ Value` lines, and the report that joins them with an engine census (`--census-out`, `parity census-report`).                                                                                                              |
| `src/bin/`, `src/tools/`, `src/utils/`, `src/infra/`                | CLI binaries, debugging tools, shared utilities.                                                                                                                                                                                                       |

`parity` is the single built binary for parity infrastructure. CI queue client commands live under `parity ci-client <health|submit|poll>` rather than a separate `parity-ci` executable.

## Deck directories

Deck names passed via `--deck1` / `--deck2` resolve from two folders in order:

1. `parity_decks/` — canonical home for decks referenced by `regression.json`. Land new regression decks here.
2. `public/preset_decks/` — wider preset library shared with the web build (UI, `yarn import-deck` landing zone). Decks here are still resolvable by name for ad-hoc parity runs.

Both engines share this lookup: the Rust list lives in `runner::DEFAULT_DECKS_DIRS`; the Java harness reads it via `-Dpreset.decks.dir=parity_decks,preset_decks` (set automatically by `java_bridge::decks_dir_property`). `--decks-dir <path>` still overrides with a single explicit folder for tests/debugging.

## Common workflows

### Reproduce a divergence

```bash
yarn parity <test-name>
# verbose:
yarn parity:test -- --deck1 <d1> --deck2 <d2> --seed <N> --max-turns 30 -v
```

Trace flags: `FORGE_RNG_TRACE=1` (both engines), `FORGE_TRIGGER_TRACE=1`, `FORGE_STACK_TRACE=1`, `FORGE_ZONE_TRACE=1` (Rust only, free text on stderr). See `docs/PARITY_TESTING.md` for the full env-var list.

The comparator stops at the first divergent snapshot and records every differing field of it: the headline is `first_divergence`, the full list is `divergences`, and the text report prints the rest as `also ...` lines. Battlefield cards are paired by name, so one extra permanent shows up once as `battlefield.cards` and per-card fields carry the card name. Nothing after that snapshot is compared, so a field that is absent from the list agreed only up to that point. A failed run carries a verdict: `FAIL` for a state divergence, `ABORTED` or `TIMEOUT` when a parity guard stopped the Rust game before Java finished, `OOM` or `ERROR` for harness failures. Every compared matchup also carries `decision`: the first agent callback the engines disagree on (`src/decision_diff.rs`), with the last callback they agreed on as context. It is usually a turn earlier than the state divergence and names the cause: a prompt Java raises and Rust never does, or an action space that offers different options. A matchup can pass on state and still have one; `gate-diff` prints how many matchups agree on both. The Java controller writes two rows under one name for `confirm_action`, `confirm_payment` and `choose_binary` (the pick, then the description) where Rust writes one; `is_java_pick_row` drops the Java pick row before either callback comparison, and a new boolean prompt in `DeterministicController` must be added there. `$ACTION_SPACE` and `choose_action` are compared by the sequence of `name@id` cards on offer, not by their text: Java prints any ability that is not an activated ability (Plot's special action) as `CastSpell`, and its `ability_index` counts the abilities currently possible while Rust's counts the card's ability list, so the kind and the index never line up. A different card sequence is a different option set, a different order, or a different pick. The Rust log names a callback after the agent method, except where Java reaches the same decision through a general prompt: the legend rule's `choose_legend_keep` is logged as `choose_single_entity_for_effect`, which is what `GameAction.handleLegendRule` asks (`runner.rs`, the logging-agent method table). One pair of names is compared as equal (`decision_diff::canonical_name`): a discard paid as a cost is `choose_discard` in Rust and `choose_cards_for_effect` in Java, because `HarnessCostPlumbing` routes `CostDiscard` through `chooseCardsForEffect`; both agents make the same draw. A `choose_single_replacement_effect` or `choose_counter_type` row with exactly one option is dropped on both sides (`is_forced_choice`): Forge asks even when one replacement applies or one counter type is listed, no RNG is drawn, and Rust applies some of those rules inline (`K:etbCounter`, stun counters), so the row said nothing about a decision and hid every later one. A pick among two or more is still compared. `pay_cost_to_prevent_effect` is not a compared callback: Java logs it after the nested payment prompts (and logs `false` when the cost cannot be paid), Rust logs it before paying and skips it when the cost cannot be paid, and neither draws RNG for it; the prompts inside the payment are compared as usual. A face-down card is printed as `@id` with no name in every Rust callback row (`FmtCtx::card`), as Java's `getName()` is empty for it. `--localize` (matrix mode and `--deck1/--deck2 --seeds`) reruns each failing matchup with `--deep`, reports where that run first diverges, and prints what each engine did since the last matching snapshot (zone moves, life, taps, counters, stack), the decisions each took there, and the Forge game events the Java harness recorded in that window (`EventRecorder`, deep runs only; Rust has no such stream, so events are shown and never compared). `PARITY_ALL_DIVERGENCES=1` still prints the list as `[all-div]` lines on stderr. `--repeat-check` runs the Rust game twice on the same seed and reports the first differing log entry, which separates nondeterminism from a real divergence. `--callback-compare` also compares the agent callback sequence (acting player and prompt name, not the answer) inside each snapshot window. Game-state fields are compared before the RNG call counts, because the counts diverge whenever the engines make different decisions.

### Gate a change on the Standard survey

```bash
yarn parity:survey       # scripts/survey-gate.sh; needs target/parity/parity and the harness jar
```

Runs `--matrix --matchups survey_matchups.tsv --seeds 42 --max-turns 20 --gate-out <file>` and hands the result to `parity gate-diff survey_baseline.jsonl <file>`, which prints `GATE_SAME`, or one line per changed matchup and `GATE_CHANGED`; the script then prints `SURVEY_SAME` or `SURVEY_CHANGED` and exits 0 or 1. The classes are `FIXED`, `REGRESSED`, `MOVED_LATER` and `MOVED_EARLIER` (same status, the divergence turn moved), `CHANGED` (same turn, different field or verdict) and `NEWLY_VISIBLE`: the headline field is one the baseline's comparator did not check, so the divergence became observable rather than being introduced. The gate file header records the compared-field list (`comparator::COMPARED_FIELDS`, keep it in sync when adding a snapshot field) and the build that produced it. A fix that moves a divergence later shows as `MOVED_LATER`; only closing the last gap shows as `FIXED`. When a change is meant to alter the baseline, copy the observed file the script names over `survey_baseline.jsonl` and commit both together. Every run is copied to `.parity-history/<utc>-<sha>.jsonl`, so two past runs can be compared with `gate-diff` as well. `PARITY_PROFILE` (default `parity`, binary at `target/<profile>/parity`), `PARITY_BIN`, `PARITY_JAR`, `JAVA_WORKERS`, and `JAVA_HEAP` override the defaults. The script exits 2 when the binary, the jar, or the cardset archive is missing or the run produced no matchup lines; in a git worktree it falls back to the main checkout's `src-tauri/resources/cardset.rkyv`. It prints the binary, commit and dirty flag it ran, then the Java cache hit count and the per-stage totals.

The parity binary mmaps `src-tauri/resources/cardset.rkyv` at startup. `yarn parity` ensures it's present, but direct invocations (`cargo run -p parity …`, manual `./target/parity/parity …`, custom CI jobs) need to materialise it first — see `manabrew-engine/AGENTS.md` § "Cardset archive". A bare `cargo build` of this crate doesn't build it.

Parity workflows build with `--profile parity` (release + `debug-assertions`), output dir `target/parity/`, so the dual-evaluation drift guards in the engine stay active during parity runs. For the edit-build-gate loop use `--profile parity-dev` (`opt-level = 2`, no LTO, incremental, `debug-assertions`), output dir `target/parity-dev/`: a one-file engine edit rebuilds without the fat-LTO link, and the survey gate gives the same result under both profiles. Compiled-vs-legacy selector drift is reported as `[selector-drift]` lines (once per selector); set `FORGE_SELECTOR_ASSERT=1` to make it panic at the divergence site instead.

### Probe one card, or shrink a failing matchup

```bash
parity --java-jar <jar> --probe "Flock Impostor" --seeds 42,43,44 --max-turns 14
parity --java-jar <jar> --probe-file names.txt --probe-out card_parity.tsv
parity --java-jar <jar> --deck1 survey_g390 --deck2 survey_g391 --seed 42 --max-turns 8 --shrink
```

`--probe` (`src/probe.rs`) builds a deck of 12 copies of the card, `--probe-partners` (two cheap colourless creatures by default, so auras, equipment and pump have something to land on) and 24 basic lands split across the card's colour identity, and plays it against `--probe-opponent` on each seed. A card is `PASS` only if every seed passed, the card was cast or played in at least one of them, and the engines took the same decisions throughout; `PASS_STATE` means the snapshots agreed while the decision sequences did not (the `decision` column says where), so the agreement may not survive another seed; `NEVER_USED` means the run could not have observed it, `DIVERGES` names the seed, turn, field and card, `UNBUILDABLE` means the name is not in the card database. Results come from the same Java cache as every other mode, so reprobing after an engine change reruns only Rust.

`--shrink` rebuilds both decks as inline specs and removes non-basic cards (replacing them with the deck's most common basic land, so deck sizes hold) while the matchup still has verdict `FAIL`, then prints the smallest pair as a ready-to-run command. The surviving divergence can be a different one from the original: read the headline it prints.

### Ask the card scripts a question

```bash
parity query --api Dig --param Tapped --cards-file standard_names.txt
parity query --mode ChangesZone --param ValidCard --value Creature.Other --show 5
parity query --keyword Web-slinging
parity coverage --matchups manabrew-rs/crates/parity/survey_matchups.tsv --cards-file standard_names.txt
parity coverage --decks red_burn,green_stompy --cards-file standard_names.txt
```

`query` (`src/script_query.rs`) filters the parsed script lines of every card (or of `--cards-file`) by API, trigger or static `--mode`, replacement `--event`, `--param`, `--value` substring, `--keyword` and `--kind`, prints matching cards with their lines, and says which engine files contain the parameter or API as a string literal. Use it instead of grepping `cardsfolder`: `$` and `|` in a pattern make a text search match nothing without saying so. Each answer carries its own control: the number of cards scanned, how many carry `ValidTgts$` (zero aborts with an error), and how many match the API filter alone, so a zero for the full query means the parameter is absent and not that the search failed.

`coverage` lists the APIs, trigger modes, replacement events and keywords of the pool that no deck in a gate carries. A gate result says nothing about those, whatever it prints.

### Sweep a card list for panics, loops and hangs

```bash
parity sweep --cards-file standard_names.txt --out-dir sweep-out --games 10 --max-turns 30 --census-out census.json
parity sweep-deck --deck "<inline spec>" --seeds 3 --max-turns 30     # replay one game; repro.txt has the exact line
```

`sweep` (`src/sweep.rs`) is Rust-only random play: cards grouped by colour identity into decks of `--chunk` cards, `--games` seeds each. Every deck runs in its own child process, so a game that never returns is sampled (`sample <pid>`, macOS) and killed after `--game-secs` + `--kill-after-secs` without stopping the run, and a panic cannot leak thread-local state into later games. Game statuses: `OK`, `PANIC` (message and location), `LOOP` (the same state fingerprint came back 200 times within a turn at a decision point; the detail lists the last actions), `BUDGET` (`--decision-budget` decisions with no repeated state), `TIMEOUT`, `KILLED`. Agent seeds come from the deck contents and the seed, not from the deck's position, so each line of `repro.txt` replays its game alone. `games.tsv` and `cards.tsv` are written in deck order and are the same for the same inputs. With `--census-out` the per-deck censuses are merged, which gives the census a much wider reach than the survey.

### Find what the engine ignores

```bash
parity --java-jar <jar> --matrix --matchups <file> --census-out census.json ...   # any mode that plays games
parity census-report census.json --cards-file standard_names.txt
python3 scripts/parity-ir-audit.py --cards-file standard_names.txt
```

`--census-out` turns on `manabrew_engine::census` for the run. `census-report` joins it with the parsed card scripts (`src/script_index.rs`) and prints, weighted by how many of the named cards are affected:

- script parameters with no matching string anywhere in the engine sources. These are certain gaps. The report refuses to run if it cannot find `"ValidTgts"` in the engine sources, so an empty list never means "looked in the wrong place".
- parameters the engine knows, carried by abilities the run consulted, and never read through a tracked accessor. Candidates only: a builder that reads `Params::inner()` directly is not tracked.
- APIs, trigger modes and replacement events the run never exercised, so it says nothing about them.
- permissive fallbacks that fired (`property-as-subtype`, `condition-assumed-true`, `valid-player-matches-all`, `count-expression-as-zero`, `alter-attribute-ignored`, ...) with the offending string, real subtypes filtered out against `TypeLists.txt` and core types and supertypes filtered out by name (`Card::has_string_type` answers those, as Forge's `CardType.hasStringType` does).

`scripts/parity-ir-audit.py` covers the case the census cannot see: a parameter parsed into an `*Ir` field that no effect ever reads.

### Compare two commits, or bisect

```bash
yarn parity:ab ab <revA> <revB>          # WORKTREE stands for the working tree as it is
yarn parity:ab bisect <good> <bad>
yarn parity:ab baseline                   # refuses a dirty tree
```

`scripts/parity-ab.mjs` builds each commit once into `target/parity-bins/<sha>/parity` from a detached checkout, so a binary labelled with a SHA is that SHA's source. Both sides run the survey matchups (`--matchups`, `--seeds`, `--max-turns` override) and the two gate files go through `gate-diff`. A binary that predates `--gate-out` is run with `--format json` and converted. Use this instead of reverting a change and rerunning by hand: a matchup that differs between a commit and its parent is attributable to that commit, and one that does not is not.

### Java result cache

`src/java_cache.rs` stores each matchup's full Java log under `.parity-cache/`, so a hit runs only the Rust game. The whole cache is wiped when the source hash changes: the harness and Forge Java sources, the card and token scripts, and the harness jar. Each entry is keyed on the matchup parameters plus the contents of its two decks, so editing or adding a deck invalidates only the matchups that use it. Java workers start on the first cache miss (`JavaServerPool::lazy`), up to `--java-workers`; a fully cached run starts none.

### Add a regression entry

After fixing a bug, lock the fix in. Add to `regression.json`:

```json
{
  "name": "descriptive_snake_case_name",
  "args": "--deck1 <deck> --deck2 <deck> --seed 42 --max-turns 20 --games 1"
}
```

Pick the smallest seed/turn budget that reliably triggers the bug. The matrix runs 3 seeds × 7 decks = 126 matchups, so one entry per regression is enough.

### Skip a known-divergent matchup

Edit `parity_ignore.json`. Every entry needs a written reason. Don't ignore a divergence to make CI green — investigate first.

## Conventions

- **Both engines share an RNG seed.** Anything that consumes randomness must be threaded through `game_rng` (Rust) and the matching `MyRandom` path (Java). New RNG callsites that drift cause every downstream divergence.
- **Card identity is by name, not id.** Internal IDs differ between engines. The comparator sorts by name.
- **Default mode takes one snapshot per turn, at untap.** A divergence reported at "T9 Untap" happened somewhere in turn 8. `--deep` snapshots every phase change, every priority change and every decision on both sides; `--localize` does that rerun for you.
- **The Java harness API surface is stable.** `forge-harness/` is ours, but its API is consumed cross-language; changing a method signature breaks every parity test. Add new methods, don't rename existing ones.

## When the harness itself is broken

If divergence reports look wrong (e.g. spurious differences in unrelated fields), suspect:

- A new field in `snapshot.rs` / `comparator.rs` that's non-deterministic across engines.
- A change to the Java harness that didn't propagate to the JAR (`yarn build:harness`). The Java cache key includes the jar, so a rebuilt jar reruns Java once and an unchanged one does not.
- A `FORGE_*_TRACE` env var leaking ordering information into the snapshot.

Rebuild the harness and rerun before assuming the engine is wrong.
