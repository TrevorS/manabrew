#!/usr/bin/env python3
"""List IR fields that are filled from a card-script parameter and never read.

A parameter can be parsed into an `*Ir` struct and then ignored by every
effect: the script says `Tapped$ True`, the IR holds `tapped: true`, and nothing
asks. No failing game points at that, so this looks for it in the source.

For every `pub struct <Name>Ir` in the engine it takes each field, counts reads
of the form `ir.<field>` (also `<x>_ir.<field>` and `.ir.<field>`) across the
engine, and reports the fields with none. The script parameter behind a field is
taken from its constructor line (`field: ... keys::X ...` or a string literal),
and the report is weighted by how many card scripts carry that parameter.

    python3 scripts/parity-ir-audit.py [--cards-file names.txt] [--top 40]

Run from the repo root. `--cards-file` limits the weighting to the named cards
(one name per line), e.g. a Standard card list.

A field read only through another path (destructuring, a getter that maps a key
to the field) shows up here as unread; check the `any` column, which counts
`.<field>` anywhere, before treating a row as a defect.
"""

import argparse
import collections
import os
import re
import sys

ENGINE = "manabrew-rs/crates/manabrew-engine/src"
CARDS = "forge/forge-gui/res/cardsfolder"
CONTROL_PARAM = "ValidTgts"


def read_sources():
    sources = {}
    for root, _dirs, files in os.walk(ENGINE):
        for name in files:
            if name.endswith(".rs"):
                path = os.path.join(root, name)
                with open(path, encoding="utf-8") as handle:
                    sources[path] = handle.read()
    return sources


def key_constants(sources):
    text = sources.get(os.path.join(ENGINE, "parsing", "keys.rs"), "")
    return dict(re.findall(r'pub const (\w+): &str = "([^"]+)";', text))


def ir_structs(sources):
    pattern = re.compile(r"pub struct (\w*Ir)\s*\{(.*?)\n\}", re.S)
    for path, text in sources.items():
        for match in pattern.finditer(text):
            fields = re.findall(r"^\s*pub (\w+):", match.group(2), re.M)
            if fields:
                yield path, match.group(1), fields


def param_for_field(text, field, constants):
    match = re.search(r"^\s*" + re.escape(field) + r":(.*?)(?:,\s*$|\n\s*\w+:)", text, re.M | re.S)
    if not match:
        return None
    body = match.group(1)
    constant = re.search(r"keys::(\w+)", body)
    if constant and constant.group(1) in constants:
        return constants[constant.group(1)]
    literal = re.search(r'"([A-Za-z][A-Za-z0-9_]*)"', body)
    return literal.group(1) if literal else None


def script_param_counts(card_names):
    counts = collections.Counter()
    seen_control = False
    wanted = {name.lower() for name in card_names} if card_names else None
    for root, _dirs, files in os.walk(CARDS):
        for name in files:
            if not name.endswith(".txt"):
                continue
            with open(os.path.join(root, name), encoding="utf-8", errors="replace") as handle:
                text = handle.read()
            if wanted is not None:
                names = re.findall(r"^Name:(.+)$", text, re.M)
                if not any(n.strip().lower() in wanted for n in names):
                    continue
            params = set(re.findall(r"(?:^|[|:\s])([A-Za-z][A-Za-z0-9_]*)\$", text))
            seen_control = seen_control or CONTROL_PARAM in params
            counts.update(params)
    if not seen_control:
        sys.exit(f"ir-audit: no script under {CARDS} has {CONTROL_PARAM}$; is the forge submodule checked out?")
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cards-file")
    parser.add_argument("--top", type=int, default=40)
    args = parser.parse_args()

    if not os.path.isdir(ENGINE):
        sys.exit(f"ir-audit: {ENGINE} not found; run from the repo root")
    card_names = None
    if args.cards_file:
        with open(args.cards_file, encoding="utf-8") as handle:
            card_names = [line.strip() for line in handle if line.strip() and not line.startswith("#")]

    sources = read_sources()
    constants = key_constants(sources)
    everything = "\n".join(sources.values())
    weights = script_param_counts(card_names)

    rows = []
    total_fields = 0
    for path, struct, fields in ir_structs(sources):
        for field in fields:
            total_fields += 1
            ir_reads = len(re.findall(r"(?:\bir|_ir|\.ir)\." + re.escape(field) + r"\b", everything))
            if ir_reads:
                continue
            any_reads = len(re.findall(r"\." + re.escape(field) + r"\b", everything))
            param = param_for_field(sources[path], field, constants)
            rows.append((weights.get(param, 0) if param else 0, struct, field, param or "?", any_reads))

    rows.sort(key=lambda row: (-row[0], row[1], row[2]))
    scope = f"{len(card_names)} named cards" if card_names else "all card scripts"
    print(f"{total_fields} IR fields checked; {len(rows)} have no `ir.<field>` read. Weighted by {scope}.")
    print(f"{'cards':>6}  {'any':>4}  struct.field  <-  parameter")
    for weight, struct, field, param, any_reads in rows[: args.top]:
        print(f"{weight:>6}  {any_reads:>4}  {struct}.{field}  <-  {param}$")


if __name__ == "__main__":
    main()
