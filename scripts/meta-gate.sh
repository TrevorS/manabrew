#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# The first-matchup gate: tournament decks, a seed sweep, 30 turns. Same checks and
# output as survey-gate.sh, against its own baseline.
export SURVEY_MATCHUPS="${SURVEY_MATCHUPS:-manabrew-rs/crates/parity/meta_matchups.tsv}"
export SURVEY_BASELINE="${SURVEY_BASELINE:-manabrew-rs/crates/parity/meta_baseline.jsonl}"
export SURVEY_SEEDS="${SURVEY_SEEDS:-$(seq 1 360 | paste -sd, -)}"
export SURVEY_MAX_TURNS="${SURVEY_MAX_TURNS:-30}"

exec bash scripts/survey-gate.sh
