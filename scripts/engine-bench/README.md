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

## The JVM driver

```sh
node scripts/harness.mjs build
python3 scripts/engine-bench/forge-jvm-game.py --seats 4 --out g4.jsonl --jfr g4.jfr
```

Wasm frames in a released build carry no names, so a profile there stops at
`wasm-function[51278]`. The JVM gives Java stacks for the same AI on the same
board, which is how #817 was found.

Do not A/B whole games. They diverge run to run even at a fixed seed, so game
length swamps the change under test. Compare profiles, or pool decisions across
several games and read the percentiles.
