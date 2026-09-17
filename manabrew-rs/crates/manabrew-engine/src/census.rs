//! Opt-in census of what the engine reads and what it silently gives up on.
//!
//! Two questions it answers for a run:
//! - for each script parameter of each API, trigger mode or replacement event,
//!   how often an ability carrying it was consulted and how often the engine
//!   actually asked for it (a parameter that is present and never read is ignored);
//! - which permissive fallbacks fired, and on what input.
//!
//! Off unless `enable()` is called. Counts collect per thread and reach the
//! shared table on `flush()`, which the caller runs at the end of each game.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ENABLED: AtomicBool = AtomicBool::new(false);

const PRESENCE_SAMPLE: u32 = 8;

#[derive(Default, Clone, Copy)]
pub struct ParamUse {
    pub present: u64,
    pub read: u64,
}

#[derive(Default)]
struct Tables {
    params: BTreeMap<(String, String), ParamUse>,
    unhandled: BTreeMap<(&'static str, String), u64>,
}

#[derive(Default)]
struct Local {
    tables: Tables,
    lookups: u32,
}

static SHARED: Mutex<Option<Tables>> = Mutex::new(None);

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local::default());
}

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    forge_card_script::set_param_read_hook(parsed_param_read);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

const OWNER_KEYS: [&str; 4] = ["SP", "AB", "DB", "ST"];

fn owner_of<'a>(get: impl Fn(&str) -> Option<&'a str>) -> String {
    for api_key in OWNER_KEYS {
        if let Some(api) = get(api_key) {
            return api.to_string();
        }
    }
    if let Some(mode) = get("Mode") {
        return format!("Mode:{mode}");
    }
    if let Some(event) = get("Event") {
        return format!("Event:{event}");
    }
    "?".to_string()
}

#[inline]
pub fn param_read(map: &BTreeMap<String, String>, key: &str) {
    if !enabled() {
        return;
    }
    record_param_read(
        owner_of(|k| map.get(k).map(String::as_str)),
        map.keys().map(String::as_str),
        key,
    );
}

fn parsed_param_read(params: &forge_card_script::ParsedParams<'_>, key: &str) {
    if !enabled() {
        return;
    }
    let entries = params.entries();
    record_param_read(
        owner_of(|k| entries.iter().rfind(|e| e.key == k).map(|e| e.value)),
        entries.iter().map(|e| e.key),
        key,
    );
}

#[cold]
fn record_param_read<'a>(owner: String, present_keys: impl Iterator<Item = &'a str>, key: &str) {
    LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        local.lookups = local.lookups.wrapping_add(1);
        if local.lookups % PRESENCE_SAMPLE == 0 {
            for present_key in present_keys {
                local
                    .tables
                    .params
                    .entry((owner.clone(), present_key.to_string()))
                    .or_default()
                    .present += 1;
            }
        }
        local
            .tables
            .params
            .entry((owner, key.to_string()))
            .or_default()
            .read += 1;
    });
}

/// A permissive fallback fired: an unknown property, filter, condition or
/// expression that the engine answered with a default instead of handling.
#[inline]
pub fn unhandled(kind: &'static str, detail: &str) {
    if !enabled() {
        return;
    }
    record_unhandled(kind, detail);
}

#[cold]
fn record_unhandled(kind: &'static str, detail: &str) {
    LOCAL.with(|local| {
        *local
            .borrow_mut()
            .tables
            .unhandled
            .entry((kind, detail.to_string()))
            .or_default() += 1;
    });
}

pub fn flush() {
    if !enabled() {
        return;
    }
    let tables = LOCAL.with(|local| std::mem::take(&mut local.borrow_mut().tables));
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let shared = shared.get_or_insert_with(Tables::default);
    for (key, usage) in tables.params {
        let entry = shared.params.entry(key).or_default();
        entry.present += usage.present;
        entry.read += usage.read;
    }
    for (key, count) in tables.unhandled {
        *shared.unhandled.entry(key).or_default() += count;
    }
}

pub struct Report {
    pub params: Vec<(String, String, ParamUse)>,
    pub unhandled: Vec<(&'static str, String, u64)>,
}

pub fn report() -> Report {
    flush();
    let shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(tables) = shared.as_ref() else {
        return Report {
            params: vec![],
            unhandled: vec![],
        };
    };
    Report {
        params: tables
            .params
            .iter()
            .map(|((owner, key), usage)| (owner.clone(), key.clone(), *usage))
            .collect(),
        unhandled: tables
            .unhandled
            .iter()
            .map(|((kind, detail), count)| (*kind, detail.clone(), *count))
            .collect(),
    }
}
