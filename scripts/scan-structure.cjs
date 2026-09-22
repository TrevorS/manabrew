#!/usr/bin/env node
// scan-structure.cjs — Compare Java (forge/game) files against Rust (manabrew-engine/src) ports.
// Prints a colorful tree and per-module coverage report.

const fs = require('fs');
const path = require('path');

const JAVA_ROOT = path.join(__dirname, '..', 'forge/forge-game/src/main/java/forge/game');
const RUST_ROOT = path.join(__dirname, '..', 'manabrew-rs/crates/manabrew-engine/src');

// Parse flags
let filterModule = null;
const modIdx = process.argv.indexOf('--module');
if (modIdx !== -1) {
  filterModule = process.argv[modIdx + 1];
  if (!filterModule) {
    console.error('Usage: node scripts/scan-structure.cjs [--module <module_name>] [--symbols]');
    process.exit(1);
  }
}
const showSymbols = process.argv.includes('--symbols');
const showCore = process.argv.includes('--core');
const showExtra = process.argv.includes('--extra');

// --extra: every `pub fn` in the engine crate whose name matches no method anywhere in
// forge-game, forge-core or forge-ai. These are the names the port invented.
if (showExtra) {
  const FORGE = path.join(__dirname, '..', 'forge');
  const javaRoots = ['forge-game', 'forge-core', 'forge-ai'].map(m => path.join(FORGE, m, 'src/main/java'));
  const javaNames = new Set();
  const methodRe = /^\s*(?:(?:public|protected|private|static|final|synchronized|abstract|native|default)\s+)*(?:<[^>]+>\s+)?[\w<>\[\],.?]+(?:\s*<[^>]*>)?\s+([a-zA-Z_]\w*)\s*\(/gm;
  const toSnake = n => n.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/([A-Z])([A-Z][a-z])/g, '$1_$2').toLowerCase();
  const walk = (dir, visit) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p, visit);
      else visit(p);
    }
  };
  for (const root of javaRoots) {
    if (!fs.existsSync(root)) continue;
    walk(root, p => {
      if (!p.endsWith('.java')) return;
      const text = fs.readFileSync(p, 'utf-8');
      let m;
      while ((m = methodRe.exec(text)) !== null) {
        const snake = toSnake(m[1]);
        javaNames.add(snake);
        const stripped = snake.replace(/^(get|is)_/, '');
        if (stripped !== snake) javaNames.add(stripped);
      }
    });
  }
  const RUST_IDIOMS = new Set(['new', 'default', 'len', 'is_empty', 'iter', 'iter_mut', 'from_str', 'fmt']);
  const modFilter = filterModule ? path.join(RUST_ROOT, filterModule) : RUST_ROOT;
  const rows = [];
  let scanned = 0;
  walk(modFilter, p => {
    if (!p.endsWith('.rs')) return;
    const rel = path.relative(RUST_ROOT, p);
    for (const f of extractRustFns(p)) {
      scanned++;
      if (!javaNames.has(f) && !RUST_IDIOMS.has(f)) rows.push(`${rel}\t${f}`);
    }
  });
  rows.sort();
  for (const r of rows) console.log(r);
  console.error(`${rows.length} of ${scanned} pub fns match no forge-game/forge-core/forge-ai method (${javaNames.size} Java names indexed)`);
  process.exit(0);
}

// ── Scan map: file & symbol overrides ──
const SCAN_MAP_PATH = path.join(__dirname, 'scan-map.pmap');
const fileRemaps = {};    // "cost/Cost.java" -> "cost/mod.rs"
const symbolRemaps = {};  // "cost/CostPayment.java" -> { "getMana": "mana" }

if (fs.existsSync(SCAN_MAP_PATH)) {
  const lines = fs.readFileSync(SCAN_MAP_PATH, 'utf-8').split('\n');
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const parts = line.split('->').map(s => s.trim());
    if (parts.length !== 2) continue;
    const [left, right] = parts;

    // Count slashes to distinguish file vs symbol remaps
    const leftParts = left.split('/');
    const lastLeft = leftParts[leftParts.length - 1];

    if (!lastLeft.includes('.')) {
      // Symbol remap: cost/CostPayment.java/getMana -> cost/cost_payment.rs/mana
      const javaFile = leftParts.slice(0, -1).join('/');
      const javaSymbol = lastLeft;
      const rightParts = right.split('/');
      const rustSymbol = rightParts[rightParts.length - 1];
      const rustFile = rightParts.slice(0, -1).join('/');
      if (!symbolRemaps[javaFile]) symbolRemaps[javaFile] = {};
      symbolRemaps[javaFile][javaSymbol] = { rustFile, rustSymbol };
    } else {
      // File remap: cost/Cost.java -> cost/mod.rs
      fileRemaps[left] = right;
    }
  }
}

const COLOR = process.stdout.isTTY && !process.env.NO_COLOR;
const ansi = (code) => (COLOR ? code : '');
const BLUE = ansi('\x1b[1;34m');
const GREEN = ansi('\x1b[0;32m');
const RED = ansi('\x1b[0;31m');
const YELLOW = ansi('\x1b[0;33m');
const CYAN = ansi('\x1b[1;36m');
const WHITE = ansi('\x1b[0;37m');
const BOLD = ansi('\x1b[1m');
const DIM = ansi('\x1b[2m');
const RESET = ansi('\x1b[0m');
const MAGENTA = ansi('\x1b[0;35m');
const LIGHT_BLUE = ansi('\x1b[0;96m');

// Detect if a Java file declares an abstract class or interface from its source
// Returns { kind: 'abstract'|'interface', name: string } or null
// For interfaces, strips the I prefix (IFoo -> Foo) since Rust traits don't use it
function detectTraitSource(filePath) {
  const content = fs.readFileSync(filePath, 'utf-8');
  // interface Foo
  const ifaceRe = /^\s*public\s+interface\s+(\w+)/m;
  const ifaceMatch = ifaceRe.exec(content);
  if (ifaceMatch) {
    let name = ifaceMatch[1];
    // Strip I prefix: ISpellAbility -> SpellAbility
    if (/^I[A-Z]/.test(name)) name = name.slice(1);
    return { kind: 'interface', name };
  }
  // abstract class Foo
  const abstractRe = /^\s*public\s+abstract\s+class\s+(\w+)/m;
  const abstractMatch = abstractRe.exec(content);
  if (abstractMatch) return { kind: 'abstract', name: abstractMatch[1] };
  return null;
}

// Extract the body of a block starting after an opening `{`, handling nested braces
function extractBlockBody(content, startIndex) {
  let depth = 1;
  let i = startIndex;
  const body = [];
  while (i < content.length && depth > 0) {
    if (content[i] === '{') depth++;
    else if (content[i] === '}') depth--;
    if (depth > 0) body.push(content[i]);
    i++;
  }
  return body.join('');
}

// Extract fn names from a block body
function extractFnsFromBody(body, requirePub) {
  const methods = [];
  const re = requirePub
    ? /pub\s+(?:async\s+)?fn\s+([a-z_]\w*)\s*[(<]/g
    : /fn\s+([a-z_]\w*)\s*[(<]/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    methods.push(m[1]);
  }
  return [...new Set(methods)];
}

// Search all .rs files in a directory for `pub trait Name` or `impl Name`
// Returns { kind: 'trait'|'impl', relFile, methods } or null
// Prefers trait over impl if both exist
function findTypeInModule(rustDir, typeName) {
  if (!fs.existsSync(rustDir) || !fs.statSync(rustDir).isDirectory()) return null;
  const files = fs.readdirSync(rustDir).filter(f => f.endsWith('.rs'));

  let implResult = null;

  for (const file of files) {
    const filePath = path.join(rustDir, file);
    const content = fs.readFileSync(filePath, 'utf-8');

    // Check for pub trait (preferred)
    const traitRe = new RegExp(`pub\\s+trait\\s+${typeName}\\b[^{]*\\{`);
    const traitMatch = traitRe.exec(content);
    if (traitMatch) {
      const body = extractBlockBody(content, traitMatch.index + traitMatch[0].length);
      // Trait methods don't need pub — they're public by default
      const methods = extractFnsFromBody(body, false);
      return { kind: 'trait', relFile: file, methods };
    }

    // Check for impl Name (not impl Trait for Name)
    const implRe = new RegExp(`impl\\s+${typeName}\\s*\\{`, 'g');
    let implMatch;
    const implMethods = [];
    while ((implMatch = implRe.exec(content)) !== null) {
      // Make sure it's not `impl SomeTrait for Name`
      const before = content.slice(Math.max(0, implMatch.index - 50), implMatch.index);
      if (/for\s*$/.test(before)) continue;
      const body = extractBlockBody(content, implMatch.index + implMatch[0].length);
      implMethods.push(...extractFnsFromBody(body, true));
    }
    if (implMethods.length > 0 && !implResult) {
      implResult = { kind: 'impl', relFile: file, methods: [...new Set(implMethods)] };
    }
  }

  return implResult;
}

// Convenience: search a single file for trait or impl
function findTypeInFile(filePath, typeName) {
  if (!fs.existsSync(filePath)) return null;
  const content = fs.readFileSync(filePath, 'utf-8');
  const dir = path.dirname(filePath);
  const file = path.basename(filePath);

  // Check trait
  const traitRe = new RegExp(`pub\\s+trait\\s+${typeName}\\b[^{]*\\{`);
  const traitMatch = traitRe.exec(content);
  if (traitMatch) {
    const body = extractBlockBody(content, traitMatch.index + traitMatch[0].length);
    return { kind: 'trait', relFile: file, methods: extractFnsFromBody(body, false) };
  }

  // Check impl
  const implRe = new RegExp(`impl\\s+${typeName}\\s*\\{`, 'g');
  let implMatch;
  const implMethods = [];
  while ((implMatch = implRe.exec(content)) !== null) {
    const before = content.slice(Math.max(0, implMatch.index - 50), implMatch.index);
    if (/for\s*$/.test(before)) continue;
    const body = extractBlockBody(content, implMatch.index + implMatch[0].length);
    implMethods.push(...extractFnsFromBody(body, true));
  }
  if (implMethods.length > 0) {
    return { kind: 'impl', relFile: file, methods: [...new Set(implMethods)] };
  }

  return null;
}

function camelToSnake(name) {
  return name
    .replace(/\.java$/, '')
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase() + '.rs';
}

// IFoo.java -> trait_foo.rs, FooBase.java -> trait_foo.rs
function traitFileName(javaName) {
  let name = javaName.replace(/\.java$/, '');
  // Strip I prefix for interfaces
  if (/^I[A-Z]/.test(name)) name = name.slice(1);
  // Strip Base suffix for abstract base classes
  if (/Base$/.test(name)) name = name.replace(/Base$/, '');
  // Convert to snake_case and add trait_ prefix
  const snake = name
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase();
  return 'trait_' + snake + '.rs';
}

// Check from filename if this is an interface (IFoo pattern)
function isJavaInterface(filename) {
  const name = filename.replace(/\.java$/, '');
  return /^I[A-Z]/.test(name);
}

// Check from source if this is an abstract class
function isAbstractClass(filePath) {
  if (!fs.existsSync(filePath)) return false;
  const content = fs.readFileSync(filePath, 'utf-8');
  return /^\s*public\s+abstract\s+class\s+/m.test(content);
}

// Convert a camelCase method name to snake_case
function methodToSnake(name) {
  return name
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase();
}

// Java methods that don't translate 1:1 to Rust free functions
const SKIP_METHODS = new Set([
  // toString/hashCode/equals/clone/compareTo -> traits/derives in Rust
  'toString', 'hashCode', 'equals', 'clone', 'compareTo',
  // visitor pattern -> enums + match in Rust
  'accept',
  // serialization
  'readObject', 'writeObject', 'readResolve',
]);

// Bean accessor pattern: getFoo(), isFoo(), setFoo(x)
// Do not treat utility methods like getValidCardsToTarget(...) as accessors.
function isGetterSetter(name, returnType, paramsRaw) {
  if (!/^(get|set|is)[A-Z]/.test(name)) return false;

  const params = paramsRaw.trim();
  const paramCount = params === ''
    ? 0
    : params.split(',').map(p => p.trim()).filter(Boolean).length;

  if (name.startsWith('get')) {
    return paramCount === 0 && returnType !== 'void';
  }
  if (name.startsWith('is')) {
    return paramCount === 0 && returnType === 'boolean';
  }
  if (name.startsWith('set')) {
    return paramCount === 1 && returnType === 'void';
  }
  return false;
}

// Extract public method names from a Java file (skip constructors, boilerplate)
function extractJavaMethods(filePath) {
  const content = fs.readFileSync(filePath, 'utf-8');
  const className = path.basename(filePath, '.java');
  const methods = [];
  const re = /^\s*public\s+(?:static\s+)?(?:final\s+)?(?:synchronized\s+)?(?:<[^>]+>\s+)?(\S+)\s+([a-zA-Z_]\w*)\s*\(([^)]*)\)/gm;
  let m;
  while ((m = re.exec(content)) !== null) {
    const returnType = m[1];
    const methodName = m[2];
    const paramsRaw = m[3];
    if (methodName === className) continue;
    if (SKIP_METHODS.has(methodName)) continue;
    if (isGetterSetter(methodName, returnType, paramsRaw)) continue;
    methods.push(methodName);
  }
  // Deduplicate (overloaded methods)
  return [...new Set(methods)];
}

// Extract pub fn names from a Rust file
function extractRustFns(filePath) {
  const content = fs.readFileSync(filePath, 'utf-8');
  const fns = [];
  const re = /pub\s+(?:async\s+)?fn\s+([a-z_]\w*)\s*[(<]/gm;
  let m;
  while ((m = re.exec(content)) !== null) {
    fns.push(m[1]);
  }
  return new Set(fns);
}

function walkJava(dir, rel = '') {
  const results = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    const relPath = rel ? `${rel}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      results.push(...walkJava(fullPath, relPath));
    } else if (entry.name.endsWith('.java') && entry.name !== 'package-info.java') {
      results.push(relPath);
    }
  }
  return results;
}

const javaFiles = walkJava(JAVA_ROOT).sort();

let totalFiles = 0;
let totalPorted = 0;
let totalSymbols = 0;
let totalSymbolsPorted = 0;
const moduleStats = {}; // module -> { ported, total, symbols, symbolsPorted }

let currentDir = null;

for (const jfile of javaFiles) {
  const dir = path.dirname(jfile);
  const base = path.basename(jfile);
  const baseName = base.replace(/\.java$/, '').toLowerCase();
  const javaFullPath = dir === '.' ? path.join(JAVA_ROOT, base) : path.join(JAVA_ROOT, dir, base);

  // Determine if this file maps to a trait_ file
  // IFoo -> reliable from filename, FooBase -> check source for abstract
  let isTrait = false;
  let expectedRs;
  if (isJavaInterface(base)) {
    isTrait = true;
    expectedRs = traitFileName(base);
  } else if (/Base\.java$/.test(base) && isAbstractClass(javaFullPath)) {
    isTrait = true;
    expectedRs = traitFileName(base);
  } else {
    expectedRs = camelToSnake(base);
  }

  let rustPath, module;
  let fileRemapped = false;

  // Check file remap first
  if (fileRemaps[jfile]) {
    rustPath = path.join(RUST_ROOT, fileRemaps[jfile]);
    module = dir === '.' ? '(root)' : dir.split('/')[0];
    fileRemapped = true;
  } else if (dir === '.') {
    rustPath = path.join(RUST_ROOT, expectedRs);
    module = '(root)';
  } else {
    rustPath = path.join(RUST_ROOT, dir, expectedRs);
    module = dir.split('/')[0];
    // If class name matches folder name (case insensitive), it maps to mod.rs
    if (baseName === path.basename(dir).toLowerCase()) {
      rustPath = path.join(RUST_ROOT, dir, 'mod.rs');
      expectedRs = 'mod.rs';
    }
  }

  // Skip files not in the target module when filtering
  if (filterModule && module !== filterModule) continue;

  // Handle "-> skip" mapping: file is intentionally not ported
  const isSkipped = fileRemapped && fileRemaps[jfile] === 'skip';

  if (dir !== currentDir) {
    currentDir = dir;
    console.log('');
    process.stdout.write(`${CYAN}${BOLD}  📂 ${dir}/${RESET}\n`);
  }

  const exists = isSkipped || fs.existsSync(rustPath);
  const displayRs = isSkipped ? 'skip' : (fileRemapped ? fileRemaps[jfile] : (path.basename(rustPath) === 'mod.rs' ? 'mod.rs' : expectedRs));
  const overrideTag = isSkipped ? ` ${DIM}(intentionally skipped)${RESET}` : (fileRemapped ? ` ${BLUE}⬡ mapped${RESET}` : '');
  const fileColor = isTrait ? MAGENTA : GREEN;

  if (!moduleStats[module]) moduleStats[module] = { ported: 0, total: 0, symbols: 0, symbolsPorted: 0 };

  totalFiles++;
  moduleStats[module].total++;

  const padded = base.padEnd(45);
  if (isSkipped) {
    // Skipped file: count as ported, show green
    totalPorted++;
    moduleStats[module].ported++;
    process.stdout.write(`${GREEN}     ${padded} -> ${displayRs}${overrideTag}${RESET}\n`);
  } else if (exists) {
    totalPorted++;
    moduleStats[module].ported++;

    // Symbol matching
    if (showSymbols) {
      const javaMethods = extractJavaMethods(javaFullPath);

      // Detect abstract class / interface from source -> check trait/impl methods
      const traitSource = detectTraitSource(javaFullPath);
      let rustSymbols;
      let typeInfo = null; // { kind: 'trait'|'impl', name, file } or { name, file: null } (missing)

      if (traitSource) {
        const typeName = traitSource.name;
        // Always check the matched Rust file — trait/impl must be in the same file
        const result = findTypeInFile(rustPath, typeName);

        if (result) {
          rustSymbols = new Set(result.methods);
          typeInfo = { kind: result.kind, name: typeName, file: result.relFile };
        } else {
          // Not found in this file — fall back to pub fn but flag it
          rustSymbols = extractRustFns(rustPath);
          typeInfo = { name: typeName, file: null };
        }
      } else {
        rustSymbols = extractRustFns(rustPath);
      }

      // Also load symbols from remap target files
      const fileSymRemaps = symbolRemaps[jfile] || {};
      const extraRustFnSets = {};

      const matched = [];
      const missing = [];
      for (const m of javaMethods) {
        const snake = methodToSnake(m);

        if (fileSymRemaps[m]) {
          const remap = fileSymRemaps[m];
          const remapRustPath = path.join(RUST_ROOT, remap.rustFile);
          if (!extraRustFnSets[remap.rustFile]) {
            extraRustFnSets[remap.rustFile] = fs.existsSync(remapRustPath)
              ? extractRustFns(remapRustPath) : new Set();
          }
          const targetFns = extraRustFnSets[remap.rustFile];
          if (targetFns.has(remap.rustSymbol)) {
            matched.push({ java: m, rust: remap.rustSymbol, remapped: true, target: remap.rustFile });
          } else {
            missing.push({ java: m, rust: remap.rustSymbol, remapped: true, target: remap.rustFile });
          }
        } else if (rustSymbols.has(snake)) {
          matched.push({ java: m, rust: snake });
        } else {
          missing.push({ java: m, rust: snake });
        }
      }

      const symTotal = javaMethods.length;
      const symPorted = matched.length;
      totalSymbols += symTotal;
      totalSymbolsPorted += symPorted;
      moduleStats[module].symbols += symTotal;
      moduleStats[module].symbolsPorted += symPorted;

      if (symTotal > 0) {
        const symPct = Math.floor((symPorted / symTotal) * 100);
        let symColor;
        if (symPct >= 80) symColor = GREEN;
        else if (symPct >= 40) symColor = YELLOW;
        else symColor = RED;
        process.stdout.write(`${fileColor}     ${padded} -> ${displayRs}${overrideTag}  ${symColor}${symPorted}/${symTotal} symbols (${symPct}%)${RESET}\n`);
      } else {
        process.stdout.write(`${fileColor}     ${padded} -> ${displayRs}${overrideTag}${RESET}\n`);
      }

      // Type info on its own indented line
      if (typeInfo) {
        if (typeInfo.file && typeInfo.kind === 'trait') {
          process.stdout.write(`${GREEN}        ◆ ${typeInfo.name} -> ${MAGENTA}trait${GREEN} ${typeInfo.name}${RESET}\n`);
        } else if (typeInfo.file && typeInfo.kind === 'impl') {
          process.stdout.write(`${GREEN}        ◇ ${typeInfo.name} -> ${LIGHT_BLUE}impl${GREEN} ${typeInfo.name}${RESET}\n`);
        } else {
          process.stdout.write(`${RED}        ◇ ${typeInfo.name} -> missing${RESET}\n`);
        }
      }

      const symIndent = typeInfo ? '            ' : '        ';
      for (const s of matched) {
        if (s.remapped) {
          process.stdout.write(`${DIM}${symIndent}${BLUE}⬡ ${s.java} -> ${s.rust} (${s.target})${RESET}\n`);
        } else {
          process.stdout.write(`${DIM}${symIndent}${GREEN}✓ ${s.java} -> ${s.rust}${RESET}\n`);
        }
      }
      for (const s of missing) {
        if (s.remapped) {
          process.stdout.write(`${DIM}${symIndent}${BLUE}✗ ${s.java} -> ${s.rust} (${s.target})${RESET}\n`);
        } else {
          process.stdout.write(`${DIM}${symIndent}${RED}✗ ${s.java} -> ${s.rust} (missing)${RESET}\n`);
        }
      }
    } else {
      process.stdout.write(`${fileColor}     ${padded} -> ${displayRs}${overrideTag}${RESET}\n`);
    }
  } else {
    // Missing file
    if (fileRemapped) {
      process.stdout.write(`${RED}     ${padded} -> ${displayRs} (missing)${RESET} ${BLUE}⬡ mapped${RESET}\n`);
    } else {
      process.stdout.write(`${RED}     ${padded} -> ${displayRs} (missing)${RESET}\n`);
    }

  }
}

// ── Foundation dependencies (forge-core -> forge-foundation) ──
if (showCore && (!filterModule || filterModule === 'foundation')) {
  const FOUNDATION_RUST = path.join(__dirname, '..', 'manabrew-rs/crates/forge-foundation/src');

  // Scan forge-game imports to find which forge-core classes are actually used
  const usedClasses = new Set();
  const allJavaFiles = walkJava(JAVA_ROOT);
  for (const jfile of allJavaFiles) {
    const dir = path.dirname(jfile);
    const base = path.basename(jfile);
    const javaPath = dir === '.' ? path.join(JAVA_ROOT, base) : path.join(JAVA_ROOT, dir, base);
    const content = fs.readFileSync(javaPath, 'utf-8');
    const importRe = /^import\s+(forge\.[^;]+);/gm;
    let m;
    while ((m = importRe.exec(content)) !== null) {
      const imp = m[1];
      if (imp.startsWith('forge.game.')) continue;
      if (imp.endsWith('.*')) continue;
      const parts = imp.split('.');
      const classIdx = parts.findIndex((p, i) => i > 0 && /^[A-Z]/.test(p));
      if (classIdx === -1) continue;
      const className = parts[classIdx];
      const pkgPath = parts.slice(1, classIdx).join('/');
      usedClasses.add(pkgPath ? `${pkgPath}/${className}` : className);
    }
  }

  // Group by sub-package
  const coreByDir = {};
  for (const entry of usedClasses) {
    const parts = entry.split('/');
    const className = parts[parts.length - 1];
    const subDir = parts.slice(0, -1).join('/') || '.';

    if (!coreByDir[subDir]) coreByDir[subDir] = [];
    const expectedRs = camelToSnake(`${className}.java`);
    // Check flat in foundation/src/ first, then in subdirectory
    let rustPath = path.join(FOUNDATION_RUST, expectedRs);
    let displayRs = expectedRs;
    if (!fs.existsSync(rustPath) && subDir !== '.') {
      rustPath = path.join(FOUNDATION_RUST, subDir, expectedRs);
      displayRs = `${subDir}/${expectedRs}`;
    }
    const exists = fs.existsSync(rustPath);

    coreByDir[subDir].push({ className, base: `${className}.java`, displayRs, exists });
  }

  if (!moduleStats['foundation']) moduleStats['foundation'] = { ported: 0, total: 0, symbols: 0, symbolsPorted: 0 };

  const sortedCoreDirs = Object.keys(coreByDir).sort();
  let currentCoreDir = null;

  for (const subDir of sortedCoreDirs) {
    const deps = coreByDir[subDir].sort((a, b) => a.className.localeCompare(b.className));

    for (const dep of deps) {
      const displayDir = subDir === '.' ? 'foundation' : `foundation/${subDir}`;
      if (displayDir !== currentCoreDir) {
        currentCoreDir = displayDir;
        console.log('');
        process.stdout.write(`${CYAN}${BOLD}  📦 ${displayDir}/${RESET}\n`);
      }

      totalFiles++;
      moduleStats['foundation'].total++;

      const padded = dep.base.padEnd(45);
      if (dep.exists) {
        totalPorted++;
        moduleStats['foundation'].ported++;
        process.stdout.write(`${GREEN}     ${padded} -> ${dep.displayRs}${RESET}\n`);
      } else {
        process.stdout.write(`${RED}     ${padded} -> ${dep.displayRs} (missing)${RESET}\n`);
      }
    }
  }
}

// Summary
console.log('\n');
console.log(`${BOLD}═══════════════════════════════════════════════════════════════${RESET}`);
console.log(`${BOLD}  PORT COVERAGE REPORT${RESET}`);
console.log(`${BOLD}═══════════════════════════════════════════════════════════════${RESET}`);
console.log('');

const sortedModules = Object.keys(moduleStats).sort();

for (const mod of sortedModules) {
  const { ported, total } = moduleStats[mod];
  const pct = total > 0 ? Math.floor((ported / total) * 100) : 0;

  let color;
  if (pct >= 80) color = GREEN;
  else if (pct >= 40) color = YELLOW;
  else color = RED;

  const filled = Math.floor(pct / 5);
  const empty = 20 - filled;
  const bar = '█'.repeat(filled) + '░'.repeat(empty);

  const modPad = mod.padEnd(20);
  const pctStr = String(pct).padStart(3);
  if (showSymbols && moduleStats[mod].symbols > 0) {
    const symPct = Math.floor((moduleStats[mod].symbolsPorted / moduleStats[mod].symbols) * 100);
    let symColor;
    if (symPct >= 80) symColor = GREEN;
    else if (symPct >= 40) symColor = YELLOW;
    else symColor = RED;
    process.stdout.write(`  ${BOLD}${modPad}${RESET} ${color}${bar} ${pctStr}%${RESET}  ${DIM}(${ported}/${total})${RESET}  ${symColor}symbols: ${symPct}%${RESET} ${DIM}(${moduleStats[mod].symbolsPorted}/${moduleStats[mod].symbols})${RESET}\n`);
  } else {
    process.stdout.write(`  ${BOLD}${modPad}${RESET} ${color}${bar} ${pctStr}%${RESET}  ${DIM}(${ported}/${total})${RESET}\n`);
  }
}

console.log('');

const overallPct = totalFiles > 0 ? Math.floor((totalPorted / totalFiles) * 100) : 0;
let overallColor;
if (overallPct >= 80) overallColor = GREEN;
else if (overallPct >= 40) overallColor = YELLOW;
else overallColor = RED;

if (showSymbols && totalSymbols > 0) {
  const symOverallPct = Math.floor((totalSymbolsPorted / totalSymbols) * 100);
  let symOverallColor;
  if (symOverallPct >= 80) symOverallColor = GREEN;
  else if (symOverallPct >= 40) symOverallColor = YELLOW;
  else symOverallColor = RED;
  process.stdout.write(`${BOLD}  OVERALL:${RESET}            ${overallColor}${String(overallPct).padStart(3)}%${RESET}  ${DIM}(${totalPorted}/${totalFiles} files)${RESET}  ${symOverallColor}symbols: ${symOverallPct}%${RESET} ${DIM}(${totalSymbolsPorted}/${totalSymbols})${RESET}\n`);
} else {
  process.stdout.write(`${BOLD}  OVERALL:${RESET}            ${overallColor}${String(overallPct).padStart(3)}%${RESET}  ${DIM}(${totalPorted}/${totalFiles} files ported)${RESET}\n`);
}
console.log('');
console.log(`${BOLD}═══════════════════════════════════════════════════════════════${RESET}`);
