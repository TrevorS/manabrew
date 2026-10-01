#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# The decision-level gate: a deck pool you supply, played in lockstep with Java checking every Rust
# agent draw. Same output and exit codes as survey-gate.sh, against the pool's own baseline.
PROFILE="${PARITY_PROFILE:-parity}"
BIN="${LOCKSTEP_BIN:-target/$PROFILE/examples/lockstep}"
JAR="${PARITY_JAR:-forge-harness/target/forge-harness-jar-with-dependencies.jar}"
DECKS="${LOCKSTEP_DECKS_DIR:-}"
OUT="${LOCKSTEP_OUT:-$(mktemp -d)}"
HISTORY="${SURVEY_HISTORY:-.parity-history}"
UPDATE="${LOCKSTEP_UPDATE:-0}"

if [ -z "$DECKS" ] || [ ! -d "$DECKS" ]; then
  echo "lockstep-gate: set LOCKSTEP_DECKS_DIR to the deck pool directory (its decks, lockstep_matchups.tsv and lockstep_baseline.jsonl)" >&2
  exit 2
fi
MATCHUPS="${LOCKSTEP_MATCHUPS:-$DECKS/lockstep_matchups.tsv}"
BASELINE="${LOCKSTEP_BASELINE:-$DECKS/lockstep_baseline.jsonl}"

if [ ! -f "$MATCHUPS" ]; then
  echo "lockstep-gate: no matchup list at $MATCHUPS (deck1<TAB>deck2 per line, or set LOCKSTEP_MATCHUPS)" >&2
  exit 2
fi
if [ ! -f "$BASELINE" ] && [ "$UPDATE" != "1" ]; then
  echo "lockstep-gate: no baseline at $BASELINE (LOCKSTEP_UPDATE=1 writes one from this run)" >&2
  exit 2
fi
if [ ! -x "$BIN" ]; then
  echo "lockstep-gate: no lockstep binary at $BIN (cargo build --profile $PROFILE -p parity --example lockstep)" >&2
  exit 2
fi
if [ ! -f "$JAR" ]; then
  echo "lockstep-gate: no harness jar at $JAR (yarn build:harness, or set PARITY_JAR)" >&2
  exit 2
fi
if [ -z "${CARDSET_ARCHIVE:-}" ] && [ ! -f src-tauri/resources/cardset.rkyv ]; then
  main_checkout="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
  if [ -f "$main_checkout/src-tauri/resources/cardset.rkyv" ]; then
    export CARDSET_ARCHIVE="$main_checkout/src-tauri/resources/cardset.rkyv"
  else
    echo "lockstep-gate: no cardset archive; build it or set CARDSET_ARCHIVE" >&2
    exit 2
  fi
fi

SHA="$(git rev-parse --short HEAD)"
git diff --quiet HEAD -- manabrew-rs forge-harness || SHA="$SHA+dirty"
echo "lockstep-gate: $BIN ($SHA), games in $OUT" >&2

LOCKSTEP_JOBS="${LOCKSTEP_JOBS:-2}" LOCKSTEP_DECKS_DIR="$DECKS" \
  "$BIN" "$MATCHUPS" "${LOCKSTEP_SEEDS:-1}" "${LOCKSTEP_MAX_TURNS:-40}" "$JAR" "$OUT" \
  >"$OUT/run.txt" 2>"$OUT/run.err" || true

if [ ! -s "$OUT/lockstep.jsonl" ]; then
  echo "lockstep-gate: the run wrote no lockstep.jsonl; last output:" >&2
  tail -20 "$OUT/run.err" >&2
  exit 2
fi

grep -E '^lockstep: ' "$OUT/run.err" >&2 || true

mkdir -p "$HISTORY"
cp "$OUT/lockstep.jsonl" "$HISTORY/$(date -u +%Y%m%dT%H%M%SZ)-$SHA-lockstep.jsonl"

if [ "$UPDATE" = "1" ]; then
  cp "$OUT/lockstep.jsonl" "$BASELINE"
  echo "LOCKSTEP_UPDATED (baseline: $BASELINE)"
  exit 0
fi

if "$BIN" diff "$BASELINE" "$OUT/lockstep.jsonl"; then
  echo LOCKSTEP_SAME
else
  echo "LOCKSTEP_CHANGED (observed: $OUT/lockstep.jsonl, failing games' logs: $OUT; LOCKSTEP_UPDATE=1 rewrites the baseline)"
  exit 1
fi
