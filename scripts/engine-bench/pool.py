#!/usr/bin/env python3
"""Reads a `stress.mjs` run as one population and says what the engine cost.

Pools the same-turn decisions of every game in the run by seat count and prompt
type, and reports the percentiles a client would feel. Games are never compared
one to one: at a fixed seed they still diverge, so the unit is the decision and
the population is the run.

    python3 scripts/engine-bench/pool.py runs/pr
    python3 scripts/engine-bench/pool.py runs/pr --baseline runs/main --fail-over 25

With `--baseline` the same table is read from another run and each cell is
shown as a ratio. `--fail-over N` exits 1 when any p50 or p90 with at least
`--min-n` decisions on both sides is more than N percent slower. `--json` writes
the pooled table for a later baseline.
"""
import argparse
import glob
import json
import os
import re
import sys
from collections import defaultdict

GC_LINE = re.compile(r"^\[\d+:0x[0-9a-f]+\]\s+(\d+) ms: (\w+).*?([\d.]+) / [\d.]+ ms")


def quantile(values, percent):
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(percent / 100 * len(ordered)))]


def read_run(path):
    games = []
    for jsonl in sorted(glob.glob(os.path.join(path, "*.jsonl"))):
        rows = [json.loads(line) for line in open(jsonl) if line.strip()]
        start = next((r for r in rows if r["ev"] == "start"), {})
        pauses = []
        log = jsonl[:-6] + ".log"
        if os.path.exists(log):
            for line in open(log, errors="replace"):
                found = GC_LINE.match(line)
                if found:
                    pauses.append(float(found.group(3)))
        games.append({"name": os.path.basename(jsonl)[:-6], "seats": start.get("seats"), "rows": rows, "pauses": pauses})
    return games


def pool(games):
    """{(seats, type): [same-turn ms]} plus per-game facts."""
    cells = defaultdict(list)
    facts = []
    for g in games:
        ends = [r for r in g["rows"] if r["ev"] == "end"]
        for r in g["rows"]:
            if r["ev"] == "decision" and not r.get("turns"):
                cells[(g["seats"], r["type"])].append(r["ms"])
        for r in g["rows"]:
            if r["ev"] == "decision" and not r.get("turns"):
                cells[(g["seats"], "*")].append(r["ms"])
        plays = sum(1 for r in g["rows"] if r["ev"] == "play" and r["output"] == "act")
        loops = sum(1 for r in g["rows"] if r["ev"] == "loop")
        heaps = [e["memory"]["rss"] for e in ends if "memory" in e]
        facts.append(
            {
                "name": g["name"],
                "seats": g["seats"],
                "games": len(ends),
                "clean": all(e["why"] == "game:over" for e in ends) and bool(ends),
                "why": [e["why"] for e in ends],
                "turns": [e.get("turn") for e in ends],
                "seconds": [round(e.get("duration_ms", 0) / 1000) for e in ends],
                "acts": plays,
                "loops": loops,
                "rss_mb": [round(h / 1e6) for h in heaps],
                "gc_max_ms": max(g["pauses"]) if g["pauses"] else None,
                "gc_total_ms": round(sum(g["pauses"])) if g["pauses"] else None,
            }
        )
    return cells, facts


def summarise(cells):
    return {
        f"{seats}|{kind}": {
            "n": len(v),
            "p50": quantile(v, 50),
            "p90": quantile(v, 90),
            "p99": quantile(v, 99),
            "max": quantile(v, 100),
        }
        for (seats, kind), v in cells.items()
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--baseline")
    ap.add_argument("--fail-over", type=float, default=None, help="percent slower on p50/p90 that fails")
    ap.add_argument("--min-n", type=int, default=100)
    ap.add_argument("--json", help="write the pooled table here")
    args = ap.parse_args()

    games = read_run(args.run)
    if not games:
        print(f"{args.run}: no games")
        return 2
    cells, facts = pool(games)
    table = summarise(cells)

    base = None
    if args.baseline:
        if os.path.isdir(args.baseline):
            base = summarise(pool(read_run(args.baseline))[0])
        else:
            base = json.load(open(args.baseline))

    print(f"run {args.run}: {len(games)} processes, {sum(f['games'] for f in facts)} games, "
          f"{sum(f['games'] for f in facts if f['clean'])} clean")
    for seats in sorted({f["seats"] for f in facts}):
        fs = [f for f in facts if f["seats"] == seats]
        secs = [s for f in fs for s in f["seconds"]]
        turns = [t for f in fs for t in f["turns"] if t is not None]
        print(f"  {seats} seats: {len(fs)} processes, game {quantile(secs, 50)}s median / {max(secs) if secs else 0}s max, "
              f"turn {quantile(turns, 50)} median, {sum(f['acts'] for f in fs)} human acts, "
              f"{sum(f['loops'] for f in fs)} loop flips")
    dirty = [f for f in facts if not f["clean"]]
    if dirty:
        print("  not clean: " + ", ".join(f"{f['name']} {f['why']}" for f in dirty))

    print()
    head = f"{'seats':>5} {'type':<28}{'n':>7}{'p50':>8}{'p90':>8}{'p99':>8}{'max':>9}"
    if base:
        head += f"{'p50 vs':>9}{'p90 vs':>9}"
    print(head)
    worst = []
    for key in sorted(table, key=lambda k: (int(k.split('|')[0]), -table[k]["n"])):
        seats, kind = key.split("|")
        c = table[key]
        line = f"{seats:>5} {kind:<28}{c['n']:>7}{c['p50']:>8}{c['p90']:>8}{c['p99']:>8}{c['max']:>9}"
        if base:
            b = base.get(key)
            if b and b["n"] >= args.min_n and c["n"] >= args.min_n and b["p50"] and b["p90"]:
                r50 = c["p50"] / b["p50"]
                r90 = c["p90"] / b["p90"]
                line += f"{r50:>8.2f}x{r90:>8.2f}x"
                worst.append((max(r50, r90), key))
            else:
                line += f"{'':>9}{'':>9}"
        print(line)

    heapy = [f for f in facts if len(f["rss_mb"]) > 1]
    if heapy:
        print("\nrss per game in one engine (MB):")
        for f in heapy:
            print(f"  {f['name']}: {' '.join(str(m) for m in f['rss_mb'])}")
    gc = [f for f in facts if f["gc_max_ms"] is not None]
    if gc:
        print(f"\ngc: largest pause {max(f['gc_max_ms'] for f in gc):.0f}ms, "
              f"total per process median {quantile([f['gc_total_ms'] for f in gc], 50)}ms")

    if args.json:
        json.dump(table, open(args.json, "w"), indent=1)
        print(f"\nwrote {args.json}")

    if base and args.fail_over is not None:
        limit = 1 + args.fail_over / 100
        bad = [(r, k) for r, k in worst if r > limit]
        if bad:
            print(f"\nFAIL: {len(bad)} cells more than {args.fail_over:.0f}% slower than baseline:")
            for r, k in sorted(bad, reverse=True):
                print(f"  {k}: {r:.2f}x")
            return 1
        print(f"\nok: no cell more than {args.fail_over:.0f}% slower than baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main())
