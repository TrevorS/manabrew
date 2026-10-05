//! Mirrors the parts of Java's `forge.util.Lang` that cost descriptions use.

use crate::CoreType;

const NUMBERS0: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const NUMBERS20: [&str; 8] = [
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

fn numeric(cnt: &str) -> Option<i32> {
    (!cnt.is_empty() && cnt.chars().all(|c| c.is_ascii_digit()))
        .then(|| cnt.parse().ok())
        .flatten()
}

pub fn join_homogenous(objects: &[String], last_union: &str) -> String {
    let mut remaining = objects.len();
    let mut out = String::new();
    for object in objects {
        remaining -= 1;
        out.push_str(object);
        if remaining > 1 {
            out.push_str(", ");
        } else if remaining == 1 {
            out.push_str(&format!(" {last_union} "));
        }
    }
    out
}

pub fn get_plural(noun: &str) -> String {
    let suffix = if noun.ends_with('s') && !noun.ends_with("ds")
        || noun.ends_with('x')
        || noun.ends_with("ch")
    {
        "es"
    } else if noun.ends_with("ds") {
        ""
    } else {
        "s"
    };
    format!("{noun}{suffix}")
}

pub fn noun_with_numeral(cnt: &str, noun: &str) -> String {
    match numeric(cnt) {
        Some(1) => format!("{} {noun}", get_numeral(1)),
        Some(n) => format!("{} {}", get_numeral(n), get_plural(noun)),
        None => format!("{cnt} {}", get_plural(noun)),
    }
}

pub fn noun_with_numeral_except_one(cnt: &str, noun: &str) -> String {
    match numeric(cnt) {
        Some(1) => format!(
            "{} {noun}",
            if starts_with_vowel(noun) { "an" } else { "a" }
        ),
        Some(n) => format!("{} {}", get_numeral(n), get_plural(noun)),
        None => format!("{cnt} {}", get_plural(noun)),
    }
}

pub fn starts_with_vowel(word: &str) -> bool {
    word.trim()
        .chars()
        .next()
        .is_some_and(|c| "aeiouAEIOU".contains(c))
}

pub fn get_numeral(n: i32) -> String {
    let prefix = if n < 0 { "minus " } else { "" };
    let n = n.unsigned_abs() as usize;
    if n < 20 {
        return format!("{prefix}{}", NUMBERS0[n]);
    }
    if n < 100 {
        let ones = match n % 10 {
            0 => "",
            n1 => NUMBERS0[n1],
        };
        return format!("{prefix}{} {ones}", NUMBERS20[n / 10 - 2]);
    }
    n.to_string()
}

pub fn get_nick_name(name: &str) -> &str {
    [',', ':', ' ']
        .into_iter()
        .find(|&c| name.contains(c))
        .and_then(|c| name.split(c).next())
        .unwrap_or(name)
}

pub fn build_valid_desc(valid: &[&str], multiple: bool) -> String {
    let formatted: Vec<String> = valid.iter().map(|v| format_valid_desc(v)).collect();
    join_homogenous(&formatted, if multiple { "and/or" } else { "or" })
}

pub fn format_valid_desc(valid: &str) -> String {
    let common = ["Player", "Opponent", "Card", "Spell", "Permanent"];
    if common.contains(&valid) || CoreType::from_name(valid).is_some() {
        valid.to_lowercase()
    } else {
        valid.to_string()
    }
}
