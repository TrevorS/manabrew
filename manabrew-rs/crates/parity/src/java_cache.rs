//! File-system cache for Java harness output.
//!
//! Avoids running the Java harness for matchups whose output cannot have changed.
//! Cache is keyed on:
//! - A **source hash** covering the Java sources, the card and token scripts,
//!   and the harness jar. When it changes the entire cache is wiped.
//! - Per-matchup parameters (deck1, deck2, seed, max_turns, prefer_actions,
//!   deep, variant, commanders) plus the contents of the two decks, so editing
//!   one deck only invalidates the matchups that use it.
//!
//! Individual entries are stored as JSON files so they are portable between
//! local dev, CI artefacts and Docker volumes.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::java_bridge::JavaMatchupData;
use crate::protocol::ParityLogEntry;
use crate::runner::{deck_search_dirs, RunConfig};

/// Lightweight wrapper that manages a directory of cached Java matchup outputs.
pub struct JavaCache {
    cache_dir: PathBuf,
    source_hash: String,
    carried: Vec<String>,
    java_turns: Option<u32>,
    prefix_hits: AtomicUsize,
}

#[derive(Hash, Clone, Copy)]
struct MatchupKey<'a> {
    deck1: &'a str,
    deck1_contents: u64,
    deck2: &'a str,
    deck2_contents: u64,
    seed: u64,
    max_turns: u32,
    prefer_actions: bool,
    deep: bool,
    variant: &'a str,
    commanders: &'a [String],
}

// Minimal serde wrappers so we can store JavaMatchupData as JSON without
// requiring Serialize/Deserialize on the original struct.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedMatchup {
    log: Vec<ParityLogEntry>,
}

impl From<&JavaMatchupData> for CachedMatchup {
    fn from(d: &JavaMatchupData) -> Self {
        Self { log: d.log.clone() }
    }
}

impl From<CachedMatchup> for JavaMatchupData {
    fn from(c: CachedMatchup) -> Self {
        Self { log: c.log }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    source_hash: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    carried: Vec<String>,
}

const MANIFEST_FILE: &str = "manifest.json";
const INCOMPLETE_HASH_PREFIX: &str = "incomplete-";
pub const CACHE_VERSION: u32 = 9;
pub const MAX_PREFIX_TURNS: u32 = 100;

impl JavaCache {
    /// Open (or create) a cache directory.
    ///
    /// `source_hash` is an opaque string that identifies the current Java
    /// sources, card scripts and jar. When it changes the entire cache is wiped.
    pub fn open(
        cache_dir: &Path,
        source_hash: String,
        java_turns: Option<u32>,
    ) -> std::io::Result<Self> {
        fs::create_dir_all(cache_dir)?;

        let manifest_path = cache_dir.join(MANIFEST_FILE);
        let existing = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Manifest>(&s).ok());
        let needs_wipe = manifest_path.exists()
            && !existing
                .as_ref()
                .is_some_and(|m| m.source_hash == source_hash && m.version == CACHE_VERSION);
        let carried = existing
            .filter(|_| !needs_wipe)
            .map(|m| m.carried)
            .unwrap_or_default();

        if needs_wipe && source_hash.starts_with(INCOMPLETE_HASH_PREFIX) {
            eprintln!(
                "[java-cache] Not wiping {}: the source hash is missing inputs (no forge sources under the current directory, as in a git worktree); running without the cache",
                cache_dir.display()
            );
            return Err(std::io::Error::other("incomplete source hash"));
        }

        if needs_wipe {
            eprintln!(
                "[java-cache] Source hash changed — wiping cache at {}",
                cache_dir.display()
            );
            for entry in fs::read_dir(cache_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        let manifest = Manifest {
            source_hash: source_hash.clone(),
            version: CACHE_VERSION,
            carried: carried.clone(),
        };
        let tmp = manifest_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string(&manifest)?)?;
        fs::rename(&tmp, &manifest_path)?;

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            source_hash,
            carried,
            java_turns,
            prefix_hits: AtomicUsize::new(0),
        })
    }

    /// Look up a cached matchup.  Returns `None` on miss or corruption.
    pub fn get(&self, config: &RunConfig) -> Option<JavaMatchupData> {
        let key = matchup_key(config);
        if let Some(data) = self.find(&key) {
            return Some(data);
        }
        if config.deep {
            return None;
        }
        let stored = |max_turns| self.find(&MatchupKey { max_turns, ..key });
        let log = (config.max_turns + 1..=MAX_PREFIX_TURNS)
            .find_map(|turns| stored(turns).map(|data| truncate_log(data.log, config.max_turns)))
            .or_else(|| {
                (1..config.max_turns).rev().find_map(|turns| {
                    stored(turns)
                        .filter(|data| data.log.iter().all(|entry| entry_turn(entry) < turns))
                        .map(|data| data.log)
                })
            })?;
        self.prefix_hits.fetch_add(1, Ordering::Relaxed);
        Some(JavaMatchupData { log })
    }

    pub fn java_turns(&self, config: &RunConfig) -> u32 {
        match self.java_turns {
            Some(turns) if !config.deep => turns.max(config.max_turns),
            _ => config.max_turns,
        }
    }

    pub fn prefix_hits(&self) -> usize {
        self.prefix_hits.load(Ordering::Relaxed)
    }

    /// An entry stored under a carried hash moves to the current hash's path on its first hit.
    fn find(&self, key: &MatchupKey) -> Option<JavaMatchupData> {
        let path = self.key_path(key);
        if let Some(data) = self.read(&path) {
            return Some(data);
        }
        self.carried.iter().find_map(|hash| {
            let old = self.key_path_for(hash, key);
            let data = self.read(&old)?;
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&old, &path);
            Some(data)
        })
    }

    fn read(&self, path: &Path) -> Option<JavaMatchupData> {
        let bytes = fs::read(path).ok()?;
        let cached: CachedMatchup = match serde_json::from_slice(&bytes) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[java-cache] Corrupt entry {}, removing: {}",
                    path.display(),
                    e
                );
                let _ = fs::remove_file(path);
                return None;
            }
        };
        Some(cached.into())
    }

    /// Store a matchup result.  Uses atomic write (temp + rename) to be safe
    /// under concurrent access from rayon threads.
    pub fn put(&self, config: &RunConfig, data: &JavaMatchupData) -> std::io::Result<()> {
        let path = self.entry_path(config);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let cached = CachedMatchup::from(data);
        let json = serde_json::to_vec(&cached)?;

        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &json)?;
        fs::rename(&tmp, &path)?;

        Ok(())
    }

    /// Whether the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of cached entries (for stats logging).
    pub fn len(&self) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(&self.cache_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().is_dir() && entry.file_name() != "." {
                for sub in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                    if sub.path().extension().map(|e| e == "json").unwrap_or(false)
                        && sub.file_name() != MANIFEST_FILE
                    {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    pub fn source_hash(&self) -> &str {
        &self.source_hash
    }

    fn entry_path(&self, config: &RunConfig) -> PathBuf {
        self.key_path(&matchup_key(config))
    }

    fn key_path(&self, key: &MatchupKey) -> PathBuf {
        self.key_path_for(&self.source_hash, key)
    }

    fn key_path_for(&self, source_hash: &str, key: &MatchupKey) -> PathBuf {
        let hash = {
            let mut h = DefaultHasher::new();
            source_hash.hash(&mut h);
            key.hash(&mut h);
            format!("{:016x}", h.finish())
        };
        let shard = &hash[..2];
        self.cache_dir.join(shard).join(format!("{hash}.json"))
    }
}

fn matchup_key(config: &RunConfig) -> MatchupKey<'_> {
    let decks_dirs = deck_search_dirs(config.decks_dir.as_deref());
    MatchupKey {
        deck1: &config.deck1,
        deck1_contents: deck_contents_hash(&config.deck1, &decks_dirs),
        deck2: &config.deck2,
        deck2_contents: deck_contents_hash(&config.deck2, &decks_dirs),
        seed: config.seed,
        max_turns: config.max_turns,
        prefer_actions: config.prefer_actions,
        deep: config.deep,
        variant: &config.variant,
        commanders: &config.commanders,
    }
}

fn entry_turn(entry: &ParityLogEntry) -> u32 {
    match entry {
        ParityLogEntry::Snapshot(snapshot) => snapshot.turn,
        ParityLogEntry::Callback(callback) => callback.turn,
        ParityLogEntry::Decision(decision) => decision.turn,
        ParityLogEntry::Event(event) => event.turn,
    }
}

fn entry_phase(entry: &ParityLogEntry) -> &str {
    match entry {
        ParityLogEntry::Snapshot(snapshot) => &snapshot.phase,
        ParityLogEntry::Callback(callback) => &callback.phase,
        ParityLogEntry::Decision(decision) => &decision.phase,
        ParityLogEntry::Event(event) => &event.phase,
    }
}

// Main.runGame ends the game at the first phase event past max_turns without a snapshot,
// and Forge still runs that phase's turn-based actions, so their rows stay.
pub fn truncate_log(log: Vec<ParityLogEntry>, max_turns: u32) -> Vec<ParityLogEntry> {
    let mut limit_phase: Option<String> = None;
    let mut snapshots = 0;
    let mut kept = Vec::with_capacity(log.len());
    for mut entry in log {
        if entry_turn(&entry) > max_turns {
            let phase = limit_phase.get_or_insert_with(|| entry_phase(&entry).to_string());
            if entry_turn(&entry) > max_turns + 1 || entry_phase(&entry) != phase.as_str() {
                break;
            }
            if entry.as_snapshot().is_some() {
                continue;
            }
        }
        match &mut entry {
            ParityLogEntry::Snapshot(_) => snapshots += 1,
            ParityLogEntry::Callback(callback) => callback.snapshot_index = snapshots,
            _ => {}
        }
        kept.push(entry);
    }
    kept
}

fn deck_contents_hash(spec: &str, decks_dirs: &[&str]) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Some(path) = spec.strip_prefix("file:") {
        fs::read(path).ok().hash(&mut hasher);
    } else if !spec.starts_with("inline:") {
        let found = decks_dirs
            .iter()
            .find_map(|dir| fs::read(Path::new(dir).join(format!("{spec}.json"))).ok());
        found.hash(&mut hasher);
    }
    hasher.finish()
}

pub fn compute_source_hash(project_root: &Path, jar_path: Option<&Path>) -> String {
    let dirs_to_hash = [
        "forge-harness/src",
        "forge/forge-game/src",
        "forge/forge-core/src",
        "forge/forge-ai/src",
        "forge/forge-gui/res/cardsfolder",
        "forge/forge-gui/res/tokenscripts",
    ];

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut missing = false;
    for dir in &dirs_to_hash {
        let full = project_root.join(dir);
        if !full.exists() {
            missing = true;
            continue;
        }
        collect_files(&full, &full, &mut files);
    }
    missing |= !project_root
        .join("forge-harness/src/main/java/forge/harness/protocol")
        .exists();
    files.sort();

    let digests: Vec<u64> = files
        .par_iter()
        .map(|(rel_path, path)| {
            let mut hasher = DefaultHasher::new();
            rel_path.hash(&mut hasher);
            fs::read(path).ok().hash(&mut hasher);
            hasher.finish()
        })
        .collect();

    let mut hasher = DefaultHasher::new();
    digests.hash(&mut hasher);
    if let Some(jar) = jar_path {
        compute_jar_hash(jar).ok().hash(&mut hasher);
    }
    let prefix = if missing { INCOMPLETE_HASH_PREFIX } else { "" };
    format!("{prefix}{:016x}", hasher.finish())
}

pub fn compute_jar_hash(jar_path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(jar_path)?;
    let mut hasher = DefaultHasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        buf[..n].hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "java" | "json" | "xml" | "properties" | "txt") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                out.push((rel, path));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(seed: u64) -> RunConfig {
        RunConfig {
            deck1: "inline:Memnite*40".to_string(),
            deck2: "inline:Memnite*40".to_string(),
            seed,
            max_turns: 10,
            cards_dir: None,
            decks_dir: None,
            verbose: crate::deterministic_agent::VerboseMode::Off,
            prefer_actions: false,
            deep: false,
            loose_parity: false,
            log_snapshots: false,
            java_heap: "2g".to_string(),
            variant: "Constructed".to_string(),
            commanders: Vec::new(),
            full_log: false,
            live_log: None,
            callback_compare: false,
            localize: false,
            mana_probe: Default::default(),
        }
    }

    fn write_manifest(dir: &Path, hash: &str, carried: &[&str]) {
        let manifest = Manifest {
            source_hash: hash.to_string(),
            version: CACHE_VERSION,
            carried: carried.iter().map(|h| h.to_string()).collect(),
        };
        fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn put_under(dir: &Path, hash: &str, seed: u64) -> PathBuf {
        let cache = JavaCache::open(dir, hash.to_string(), None).unwrap();
        cache
            .put(&config(seed), &JavaMatchupData { log: Vec::new() })
            .unwrap();
        cache.entry_path(&config(seed))
    }

    #[test]
    fn carried_hash_entries_move_to_the_current_hash() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = put_under(dir.path(), "a", 1);
        write_manifest(dir.path(), "b", &["a"]);

        let cache = JavaCache::open(dir.path(), "b".to_string(), None).unwrap();
        assert!(cache.get(&config(1)).is_some());
        assert!(!old_path.exists());
        assert!(cache.entry_path(&config(1)).exists());
        let manifest = fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap();
        assert!(manifest.contains(r#""carried":["a"]"#));
    }

    #[test]
    fn quarantined_entries_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let quarantine = tempfile::tempdir().unwrap();
        let old_path = put_under(dir.path(), "a", 1);
        fs::rename(&old_path, quarantine.path().join("entry.json")).unwrap();
        write_manifest(dir.path(), "b", &["a"]);

        let cache = JavaCache::open(dir.path(), "b".to_string(), None).unwrap();
        assert!(cache.get(&config(1)).is_none());
    }

    #[test]
    fn entries_under_a_hash_that_is_not_carried_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let other_path = put_under(dir.path(), "x", 1);
        write_manifest(dir.path(), "b", &["a"]);

        let cache = JavaCache::open(dir.path(), "b".to_string(), None).unwrap();
        assert!(cache.get(&config(1)).is_none());
        assert!(other_path.exists());
    }

    #[test]
    fn carried_hashes_survive_a_second_migration() {
        let dir = tempfile::tempdir().unwrap();
        put_under(dir.path(), "a", 1);
        write_manifest(dir.path(), "b", &["a"]);
        put_under(dir.path(), "b", 2);
        write_manifest(dir.path(), "c", &["b", "a"]);

        let cache = JavaCache::open(dir.path(), "c".to_string(), None).unwrap();
        assert!(cache.get(&config(1)).is_some());
        assert!(cache.get(&config(2)).is_some());
        assert!(cache.entry_path(&config(1)).exists());
        assert!(cache.entry_path(&config(2)).exists());
    }
}
