#!/usr/bin/env node
// Build the parity binary at named commits, run them over the same matchups
// against the same cached Java results, and classify what changed.
//
//   node scripts/parity-ab.mjs build <rev>
//   node scripts/parity-ab.mjs ab <revA> <revB> [--matchups FILE] [--seeds 42] [--max-turns 20]
//   node scripts/parity-ab.mjs bisect <good> <bad> [same options]
//   node scripts/parity-ab.mjs baseline
//
// <rev> is anything `git rev-parse` accepts, or WORKTREE for the working tree
// as it stands (built in place, labelled <sha>+dirty when it has changes).
//
// Binaries are cached by full commit SHA under target/parity-bins/<sha>/, so a
// commit is built once. They are built in a detached checkout under
// target/parity-bins/_src: the binary for a SHA is always that SHA's source,
// never a dirty tree. Java results are
// shared between binaries that use the same Java cache format, so Java runs
// once per matchup however many commits are compared.
//
// `baseline` rewrites survey_baseline.jsonl from a clean build of HEAD and
// refuses when the working tree has uncommitted engine or harness changes.
//
// Exit codes: 0 = no change, 1 = changes found, 2 = harness error.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const BINS = join(ROOT, "target", "parity-bins");
const DEFAULT_MATCHUPS = "manabrew-rs/crates/parity/survey_matchups.tsv";
const BASELINE = "manabrew-rs/crates/parity/survey_baseline.jsonl";
const PROFILE_CONFIG = [
  'profile.parity-dev.inherits="release"',
  "profile.parity-dev.opt-level=2",
  "profile.parity-dev.lto=false",
  "profile.parity-dev.codegen-units=256",
  "profile.parity-dev.incremental=true",
  "profile.parity-dev.debug-assertions=true",
];
const TRACKED_FOR_DIRTY = ["manabrew-rs", "forge-harness", "Cargo.toml", "Cargo.lock"];

function die(message) {
  console.error(`parity-ab: ${message}`);
  process.exit(2);
}

function run(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", ...options });
  if (result.error) die(`${cmd}: ${result.error.message}`);
  return result;
}

function git(...args) {
  const result = run("git", args);
  if (result.status !== 0) die(`git ${args.join(" ")}: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

function parseOptions(argv) {
  const options = { positional: [] };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith("--")) options[arg.slice(2)] = argv[++i];
    else options.positional.push(arg);
  }
  return options;
}

function worktreeIsDirty() {
  return run("git", ["diff", "--quiet", "HEAD", "--", ...TRACKED_FOR_DIRTY]).status !== 0;
}

function cargoBuild(cwd, targetDir) {
  const args = ["build", "--profile", "parity-dev", "-p", "parity"];
  for (const entry of PROFILE_CONFIG) args.push("--config", entry);
  const result = spawnSync("cargo", args, {
    cwd,
    stdio: ["ignore", "inherit", "inherit"],
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
  });
  if (result.status !== 0) die(`cargo build failed in ${cwd}`);
}

// Returns { bin, label }.
function ensureBinary(rev) {
  if (rev === "WORKTREE") {
    const sha = git("rev-parse", "--short", "HEAD");
    cargoBuild(ROOT, join(ROOT, "target"));
    return {
      bin: join(ROOT, "target", "parity-dev", "parity"),
      label: worktreeIsDirty() ? `${sha}+dirty` : sha,
    };
  }
  const sha = git("rev-parse", "--verify", `${rev}^{commit}`);
  const dir = join(BINS, sha);
  const bin = join(dir, "parity");
  const label = sha.slice(0, 9);
  if (existsSync(bin)) return { bin, label };

  // One checkout reused for every commit, so cargo rebuilds only the crates
  // that differ between the last commit built here and this one.
  const src = join(BINS, "_src");
  console.error(`parity-ab: building ${label}`);
  if (existsSync(join(src, ".git"))) {
    const checkout = run("git", ["checkout", "--detach", "--force", sha], { cwd: src });
    if (checkout.status !== 0) die(`git checkout ${label}: ${checkout.stderr.trim()}`);
  } else {
    rmSync(src, { recursive: true, force: true });
    run("git", ["worktree", "prune"]);
    git("worktree", "add", "--detach", src, sha);
  }
  const targetDir = join(BINS, "_target");
  cargoBuild(src, targetDir);
  mkdirSync(dir, { recursive: true });
  copyFileSync(join(targetDir, "parity-dev", "parity"), bin);
  return { bin, label };
}

function buildInfo(bin) {
  const result = run(bin, ["build-info"]);
  if (result.status !== 0) return null;
  try {
    return JSON.parse(result.stdout);
  } catch {
    return null;
  }
}

function resolveEnv() {
  const env = { ...process.env };
  if (!env.CARDSET_ARCHIVE && !existsSync(join(ROOT, "src-tauri/resources/cardset.rkyv"))) {
    const mainCheckout = resolve(git("rev-parse", "--git-common-dir"), "..");
    const archive = join(mainCheckout, "src-tauri/resources/cardset.rkyv");
    if (!existsSync(archive)) die("no cardset archive; build it or set CARDSET_ARCHIVE");
    env.CARDSET_ARCHIVE = archive;
  }
  return env;
}

function normalizeField(field) {
  return field.replace(/\[\d+\]/g, "[i]");
}

// Gate records for a binary that predates --gate-out, from its matrix JSON.
function legacyGateFile(report, label, maxTurns) {
  const lines = [JSON.stringify({ compared_fields: [], build: label, max_turns: maxTurns })];
  const records = report.results.map((r) => {
    const headline = r.first_divergence;
    const aborted = (r.skip_reason ?? "").startsWith("ABORTED");
    let verdict = { pass: "PASS", skipped: "SKIPPED", error: "ERROR", fail: "FAIL" }[r.status];
    if (verdict === "FAIL" && aborted && headline?.field === "snapshot.exists") verdict = "ABORTED";
    const record = { deck1: r.deck1, deck2: r.deck2, seed: r.seed, verdict };
    if (headline) {
      record.turn = headline.turn;
      record.phase = headline.phase;
      record.field = normalizeField(headline.field);
      record.rust = headline.rust_value.slice(0, 240);
      record.java = headline.java_value.slice(0, 240);
    }
    return record;
  });
  records.sort((a, b) =>
    a.deck1 !== b.deck1 ? (a.deck1 < b.deck1 ? -1 : 1) : a.deck2 !== b.deck2 ? (a.deck2 < b.deck2 ? -1 : 1) : a.seed - b.seed,
  );
  for (const record of records) lines.push(JSON.stringify(record));
  return `${lines.join("\n")}\n`;
}

function runSurvey({ bin, label }, options) {
  const jar =
    options.jar ?? process.env.PARITY_JAR ?? "forge-harness/target/forge-harness-jar-with-dependencies.jar";
  if (!existsSync(resolve(ROOT, jar))) die(`no harness jar at ${jar} (set PARITY_JAR)`);
  const matchups = options.matchups ?? DEFAULT_MATCHUPS;
  const maxTurns = options["max-turns"] ?? "20";
  const info = buildInfo(bin);
  const cacheDir = join(BINS, `java-cache-v${info?.cache_version ?? "legacy"}`);
  const outDir = join(BINS, "_runs");
  mkdirSync(outDir, { recursive: true });
  const gateFile = join(outDir, `${label}.jsonl`);
  rmSync(gateFile, { force: true });

  const args = [
    "--java-jar", jar,
    "--java-heap", process.env.JAVA_HEAP ?? "2g",
    "--java-workers", process.env.JAVA_WORKERS ?? "4",
    "--matrix",
    "--seeds", options.seeds ?? "42",
    "--max-turns", maxTurns,
    "--matchups", matchups,
    "--cache-dir", cacheDir,
  ];
  const reportFile = join(outDir, `${label}.report`);
  if (info) args.push("--gate-out", gateFile, "--build-label", label, "-o", reportFile);
  else args.push("--format", "json", "-o", reportFile);

  console.error(`parity-ab: running ${label}`);
  const result = spawnSync(bin, args, {
    cwd: ROOT,
    env: resolveEnv(),
    stdio: ["ignore", "ignore", "pipe"],
    encoding: "utf8",
    maxBuffer: 1 << 28,
  });
  const stage = result.stderr.split("\n").filter((l) => /Java cache:.*hits|Stage totals/.test(l));
  for (const line of stage) console.error(`  ${line}`);
  if (!info) {
    if (!existsSync(reportFile)) die(`${label} wrote no report:\n${result.stderr.slice(-2000)}`);
    writeFileSync(gateFile, legacyGateFile(JSON.parse(readFileSync(reportFile, "utf8")), label, Number(maxTurns)));
  }
  if (!existsSync(gateFile)) die(`${label} wrote no gate file:\n${result.stderr.slice(-2000)}`);
  return gateFile;
}

function gateDiff(differ, before, after) {
  const result = run(differ, ["gate-diff", before, after]);
  if (result.status === 2) die(`gate-diff failed: ${result.stderr}`);
  return { same: result.status === 0, text: result.stdout };
}

function pickDiffer(...candidates) {
  for (const candidate of candidates) if (buildInfo(candidate.bin)) return candidate.bin;
  const local = join(ROOT, "target", "parity-dev", "parity");
  if (existsSync(local) && buildInfo(local)) return local;
  die("no binary with gate-diff is available; build the working tree with --profile parity-dev first");
}

function commandAb(options) {
  const [revA, revB] = options.positional;
  if (!revA || !revB) die("usage: ab <revA> <revB>");
  const a = ensureBinary(revA);
  const b = ensureBinary(revB);
  if (a.bin !== b.bin && readFileSync(a.bin).equals(readFileSync(b.bin))) {
    console.error(`parity-ab: ${a.label} and ${b.label} built byte-identical binaries; nothing to compare`);
  }
  const before = runSurvey(a, options);
  const after = runSurvey(b, options);
  const { same, text } = gateDiff(pickDiffer(b, a), before, after);
  process.stdout.write(text);
  process.exit(same ? 0 : 1);
}

function commandBisect(options) {
  const [good, bad] = options.positional;
  if (!good || !bad) die("usage: bisect <good> <bad>");
  const goodBinary = ensureBinary(good);
  const goodFile = runSurvey(goodBinary, options);
  const revs = git("rev-list", "--reverse", "--first-parent", `${good}..${bad}`).split("\n").filter(Boolean);
  if (revs.length === 0) die(`no commits in ${good}..${bad}`);

  const differs = (sha) => {
    const binary = ensureBinary(sha);
    const file = runSurvey(binary, options);
    return !gateDiff(pickDiffer(binary, goodBinary), goodFile, file).same;
  };
  if (!differs(revs[revs.length - 1])) {
    console.log(`${bad} gives the same results as ${good}; nothing to bisect`);
    process.exit(0);
  }
  let low = 0;
  let high = revs.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    console.error(`parity-ab: ${high - low + 1} candidates left, testing ${revs[mid].slice(0, 9)}`);
    if (differs(revs[mid])) high = mid;
    else low = mid + 1;
  }
  const culprit = revs[low];
  console.log(`first commit whose results differ from ${good}:`);
  console.log(git("log", "-1", "--format=%h %s", culprit));
  const binary = ensureBinary(culprit);
  process.stdout.write(gateDiff(pickDiffer(binary), goodFile, runSurvey(binary, options)).text);
  process.exit(1);
}

function commandBaseline(options) {
  if (worktreeIsDirty()) {
    die(`refusing to record a baseline: uncommitted changes under ${TRACKED_FOR_DIRTY.join(", ")}`);
  }
  const binary = ensureBinary("HEAD");
  const file = runSurvey(binary, options);
  copyFileSync(file, join(ROOT, BASELINE));
  console.log(`wrote ${BASELINE} from ${binary.label}`);
}

const [command, ...rest] = process.argv.slice(2);
const options = parseOptions(rest);
switch (command) {
  case "build":
    if (!options.positional[0]) die("usage: build <rev>");
    console.log(ensureBinary(options.positional[0]).bin);
    break;
  case "ab":
    commandAb(options);
    break;
  case "bisect":
    commandBisect(options);
    break;
  case "baseline":
    commandBaseline(options);
    break;
  default:
    die("usage: parity-ab.mjs <build|ab|bisect|baseline> ...");
}
