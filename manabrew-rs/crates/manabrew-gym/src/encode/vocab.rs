use std::sync::OnceLock;

use rustc_hash::FxHashMap;

pub const PAD: i32 = 0;
pub const UNKNOWN: i32 = 1;
pub const HIDDEN: i32 = 2;
pub const RESERVED: usize = 3;

pub const VOCAB_FILE: &str = "card_vocab.txt";

pub struct CardVocab {
    names: Vec<&'static str>,
    ids: FxHashMap<&'static str, i32>,
}

impl CardVocab {
    pub fn get() -> &'static CardVocab {
        static VOCAB: OnceLock<CardVocab> = OnceLock::new();
        VOCAB.get_or_init(|| CardVocab::parse(include_str!("../../card_vocab.txt")))
    }

    pub fn parse(text: &'static str) -> CardVocab {
        let names: Vec<&'static str> = text.lines().filter(|l| !l.is_empty()).collect();
        let ids = names
            .iter()
            .enumerate()
            .map(|(i, &name)| (name, (i + RESERVED) as i32))
            .collect();
        CardVocab { names, ids }
    }

    pub fn id(&self, name: &str) -> i32 {
        self.ids.get(name).copied().unwrap_or(UNKNOWN)
    }

    pub fn len(&self) -> usize {
        self.names.len() + RESERVED
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> &[&'static str] {
        &self.names
    }
}
