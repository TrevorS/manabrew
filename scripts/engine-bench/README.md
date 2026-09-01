# Engine bench

Plays whole Forge games headlessly and records what each decision cost. Built to
answer #817, the four-seat Commander stall.

Two drivers, same game and same policy. `forge-wasm-game.mjs` runs the browser
engine on Node, `forge-jvm-game.py` runs the JVM harness. Having both is the
point: they differ in threading and in whether a profiler can name anything.

```sh
npm install --no-save @manabrew/forge-wasm@latest
node --trace-gc scripts/engine-bench/forge-wasm-game.mjs --seats 4 --out g4.jsonl > g4.log
python3 scripts/engine-bench/summarise.py 'g*.jsonl'
```

`--seats` 2 or 4, `--decks` a comma-separated list of `public/preset_decks`
basenames or deck files, `--out` the JSONL, `--timeout` seconds. By default the
human seat passes on every priority, so a reading is the AI's cost and not a
scripted line of play. `--policy greedy` plays a land a turn, casts what
auto-pay can cover and attacks with everything, which puts the human's
permanents on the board. `--engine <dir>` loads a checkout's
`packages/forge-wasm` (after `yarn build:forge-wasm`) instead of the npm
package. `--games N` plays N games in one engine and samples the host heap
between them. `--seed` pins the shuffle.

## A run, not a game

```sh
node scripts/engine-bench/stress.mjs --tag main --seats 2,4 --games 12
node scripts/engine-bench/stress.mjs --tag pr --engine packages/forge-wasm --seats 2,4 --games 12
python3 scripts/engine-bench/pool.py scripts/engine-bench/runs/pr \
  --baseline scripts/engine-bench/runs/main --fail-over 25
```

`stress.mjs` plays a matrix, a game per process, `--jobs` at a time (half the
cores by default). Decks rotate through a pool: the ten Commander presets, or
`--decks` as a list or a directory of deck files. `--per-engine N` plays N
games back to back in each process, which is what a tab does. `pool.py` reads
the run as one population: same-turn percentiles by seat count and prompt
type, game outcomes, human acts, loop flips, rss per game and GC pauses under
`--trace-gc`. With `--baseline` every cell is a ratio and `--fail-over` turns
it into an exit code. `--json` keeps a run's table for a later baseline.

Two hundred decisions per cell is where the p90 stops moving between runs.
Twelve 4-seat games is about two thousand `chooseAction` decisions.

## A/B

```sh
node scripts/engine-bench/fetch-engine.mjs --into target/engines/prod
node scripts/engine-bench/stress.mjs --tag ab --engines prod=target/engines/prod,pr=packages/forge-wasm \
  --seats 2,4 --games 30 --jobs 3
python3 scripts/engine-bench/pool.py --ab scripts/engine-bench/runs/ab --fail-over 20
```

`fetch-engine.mjs` assembles an engine directory from a deployed site: the
launcher and wasm players are running, under this checkout's facade. No Web
Image toolchain and no npm release needed to bench prod or staging
(`--from https://staging.manabrew.app`).

`--engines` plays the one plan under every arm, same seeds and decks, arms
interleaved in the queue so machine load lands on both. The first arm is the
control. `pool.py --ab` gives each cell the ratio of p50 and p90, a 95%
bootstrap interval and the share of resamples in which the arm is slower.

The bootstrap resamples games, not decisions. A game's decisions rise and fall
with its board, so an A/A test over decisions reported a 25% difference that
was not there; over games the same run reads 0.7-1.2. That is also why the
interval narrows with games rather than decisions: eight per arm is a smoke
test, thirty is where a 20% change separates from noise. `--fail-over` fails
only when the whole p50 interval sits above the threshold.

Read the same-turn half. The cross-turn half contains whole opponent turns.
`docs/agents/LATENCY_ANALYSIS.md` has the rest of the traps.

## What it is for

The browser tests measure the client. This measures the engine: same Forge, same
worker, no render loop in front of it. A four-seat Commander game takes about
two minutes and reproduces the production stall signature.

Run it under `--trace-gc` and `summarise.py` sums the collector's pauses, which
is what tells a GC pause from a slow AI search. The engine's Java heap is the
host's, because the Web Image build targets WasmGC and declares no linear
memory, so there is no engine-side heap cap to raise.

## HotSpot does not compile the hot method

`CardProperty.cardHasProperty` is 14,112 bytes of bytecode. HotSpot refuses to
compile a method over 8000 (`DontCompileHugeMethods`, `HugeMethodLimit`), so on
a stock JVM it runs **interpreted for the whole game**: three to eight times its
compiled per-call cost, 10-28% of a four-seat game, and the top self frame in
any profile taken here.

Nothing we ship is HotSpot. The desktop engine is a GraalVM native image built
by `forge-harness/build-native.sh`, the browser one is Web Image, and both
compile every reachable method ahead of time. So this is a property of the
measuring rig, not of the product, and a profile taken with the limit in place
ranks the engine wrongly: with the method compiled, `cardHasProperty` leaves the
top of the profile entirely and `FCollection` allocation and the static-ability
rebuild take its place.

`forge-jvm-game.py` therefore passes `-XX:-DontCompileHugeMethods` by default.
`--no-compile-huge` puts the limit back, which is only worth doing to reproduce
an old measurement. Any JVM profile of this engine taken before 2026-09-01 was
taken with the limit on.

## The JVM driver

```sh
node scripts/harness.mjs build
python3 scripts/engine-bench/forge-jvm-game.py --seats 4 --out g4.jsonl --jfr g4.jfr
```

Wasm frames in a released build carry no names, so a profile there stops at
`wasm-function[51278]`. The JVM gives Java stacks for the same AI on the same
board, which is how #817 was found.

A seed replays the same game on both runtimes when the human seat answers the
same way: `--policy greedy` here mirrors the wasm driver's, and the seat names
match. So a stall seen in a `stress.mjs` run can be replayed on the JVM by seed
with `--sysprop forge.synchronous=true --jfr`, and the fix measured on the
identical game (same decision count, same turn count) rather than on a
population. That is how the alternative-cost and `canGainKeyword` reorders in
witchesofthehill/forge#13 were found and checked.

`jfr-top.py` ranks the frames in a recording by self and inclusive samples, and
splits the samples by thread so the AI's search can be told from the rules
engine.

```sh
python3 scripts/engine-bench/jfr-top.py g4.jfr
python3 scripts/engine-bench/jfr-top.py g4.jfr --thread 'Game AI Eval'
```

Do not A/B whole games. They diverge run to run even at a fixed seed, so game
length swamps the change under test. Compare profiles, or pool decisions across
several games and read the percentiles.

## Counting instead of timing

`--counters` records what the engine did for each decision rather than only how
long it took: static-ability passes, calls to `getValidCards`, cards examined by
them, `Card.isValid` and `CardProperty.cardHasProperty` entries, and how many
times an AI seat took priority. A count is deterministic where a duration is
not, so it survives the divergence that makes whole-game A/B useless.

It needs the engine counters, which live on the fork's `khaliostr/decision-counters`
branch and are not in a release build. Check that branch out in `forge/`, rebuild,
and pass the flag:

```sh
git -C forge fetch origin khaliostr/decision-counters && git -C forge checkout khaliostr/decision-counters
node scripts/harness.mjs build
python3 scripts/engine-bench/forge-jvm-game.py --seats 4 --counters --out c4.jsonl
python3 scripts/engine-bench/counters.py --type chooseAction 'c4*.jsonl'
python3 scripts/engine-bench/property-chain.py 'c4*.jsonl'
```

`counters.py` bins by battlefield size rather than by turn, and reports per AI
priority as well as per decision. Both matter. A decision here is the gap
between two prompts to the human seat, and at four seats that gap holds three
AI seats taking priority where two seats holds one, so per-decision counts are
not comparable across seat counts. It also reports the share of each decision
spent inside `isValid` and `cardHasProperty`, measured with nanoTime and
corrected for what nanoTime itself costs, which is a number the profile does
not give you.

`property-chain.py` joins the property histogram to `CardProperty`'s if/else
chain and says how many string comparisons the engine walked to answer them.

Everything is behind `-Dforge.engineCounters=true`, a static final read at class
init, so it folds away when it is off. An earlier round of this used atomics and
two `nanoTime` calls on every `checkStaticAbilities` and had to be reverted
(forge `9d14d1511bf`).
