#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

BIN="${PARITY_BIN:-manabrew-rs/target/parity/parity}"
JAR="${PARITY_JAR:-forge-harness/target/forge-harness-jar-with-dependencies.jar}"
MATCHUPS=manabrew-rs/crates/parity/survey_matchups.tsv
BASELINE=manabrew-rs/crates/parity/survey_baseline.txt
OUT="${SURVEY_OUT:-$(mktemp)}"

"$BIN" --java-jar "$JAR" --java-heap "${JAVA_HEAP:-2g}" \
  --java-workers "${JAVA_WORKERS:-4}" --matrix --seeds 42 --max-turns 20 \
  --matchups "$MATCHUPS" >"$OUT" 2>&1 || true

sed 's/\x1b\[[0-9;]*m//g' "$OUT" |
  awk '$1 ~ /^survey_g/ {
    line = $1 " " $2 " " $4
    if (match($0, /FAILED AT TURN [0-9]+/)) line = line " " substr($0, RSTART, RLENGTH)
    print line
  }' |
  sort >"$OUT.observed"

if diff -u "$BASELINE" "$OUT.observed"; then
  echo SURVEY_SAME
else
  echo SURVEY_CHANGED
  exit 1
fi
