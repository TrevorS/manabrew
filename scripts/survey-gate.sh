#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE="${PARITY_PROFILE:-parity}"
BIN="${PARITY_BIN:-target/$PROFILE/parity}"
JAR="${PARITY_JAR:-forge-harness/target/forge-harness-jar-with-dependencies.jar}"
MATCHUPS="${SURVEY_MATCHUPS:-manabrew-rs/crates/parity/survey_matchups.tsv}"
BASELINE="${SURVEY_BASELINE:-manabrew-rs/crates/parity/survey_baseline.jsonl}"
OUT="${SURVEY_OUT:-$(mktemp)}"
HISTORY="${SURVEY_HISTORY:-.parity-history}"

if [ ! -x "$BIN" ]; then
  echo "survey-gate: no parity binary at $BIN (cargo build --profile $PROFILE -p parity)" >&2
  exit 2
fi
if [ ! -f "$JAR" ]; then
  echo "survey-gate: no harness jar at $JAR (yarn build:harness, or set PARITY_JAR)" >&2
  exit 2
fi
if [ -z "${CARDSET_ARCHIVE:-}" ] && [ ! -f src-tauri/resources/cardset.rkyv ]; then
  main_checkout="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
  if [ -f "$main_checkout/src-tauri/resources/cardset.rkyv" ]; then
    export CARDSET_ARCHIVE="$main_checkout/src-tauri/resources/cardset.rkyv"
  else
    echo "survey-gate: no cardset archive; build it or set CARDSET_ARCHIVE" >&2
    exit 2
  fi
fi

SHA="$(git rev-parse --short HEAD)"
git diff --quiet HEAD -- manabrew-rs forge-harness || SHA="$SHA+dirty"
echo "survey-gate: $BIN ($SHA)" >&2

# shellcheck disable=SC2086
"$BIN" --java-jar "$JAR" --java-heap "${JAVA_HEAP:-2g}" \
  --java-workers "${JAVA_WORKERS:-4}" --matrix --seeds 42 --max-turns 20 \
  --matchups "$MATCHUPS" --gate-out "$OUT.jsonl" --build-label "$SHA" \
  ${SURVEY_ARGS:-} >"$OUT" 2>&1 || true

if [ ! -s "$OUT.jsonl" ]; then
  echo "survey-gate: the run wrote no gate file; last output:" >&2
  tail -20 "$OUT" >&2
  exit 2
fi

grep -E '^\[parity\] (Java cache|Stage totals)' "$OUT" >&2 || true

mkdir -p "$HISTORY"
cp "$OUT.jsonl" "$HISTORY/$(date -u +%Y%m%dT%H%M%SZ)-$SHA.jsonl"

if "$BIN" gate-diff "$BASELINE" "$OUT.jsonl"; then
  echo SURVEY_SAME
else
  echo "SURVEY_CHANGED (observed: $OUT.jsonl, full report: $OUT)"
  exit 1
fi
