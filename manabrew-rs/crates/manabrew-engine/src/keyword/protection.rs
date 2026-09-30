//! Protection keyword implementation.
//!
//! Ported from Java's `Protection.java` in `forge/game/keyword/`.

use super::keyword_instance::{Keyword, KeywordInstanceData};

/// Protection keyword data.
/// This creature can't be blocked, targeted, dealt damage, or
/// equipped/enchanted by the specified quality.
#[derive(Debug, Clone)]
pub struct Protection {
    pub base: KeywordInstanceData,
    /// What this creature has protection from (e.g. "red", "creatures").
    pub from_what: String,
}

impl Protection {
    /// Create a new Protection keyword.
    pub fn new(original: String) -> Self {
        // Extract "from what" from the original string if possible.
        // E.g. "Protection from red" -> "red"
        let from_what = if let Some(rest) = original.strip_prefix("Protection from ") {
            rest.to_string()
        } else {
            String::new()
        };
        Self {
            base: KeywordInstanceData::new(Keyword::Protection, original),
            from_what,
        }
    }

    /// Parse the details string.
    pub fn parse(&mut self, _details: &str) {
        // In Java, parse is a no-op. The from_what is set from the original string.
    }

    /// Get the display title.
    pub fn get_title(&self) -> String {
        format!("Protection from {}", self.from_what)
    }

    /// Format reminder text.
    pub fn format_reminder_text(&self, reminder_text: &str) -> String {
        reminder_text.replace("%s", &self.from_what)
    }
}

pub fn get_protection_valid(kw: &str, damage: bool) -> Option<String> {
    let valid_source = if let Some(rest) = kw.strip_prefix("Protection:") {
        let characteristic = rest.split(':').next()?;
        if characteristic.starts_with("Player") {
            format!("ControlledBy {characteristic}")
        } else {
            let is_color = [
                "White",
                "Blue",
                "Black",
                "Red",
                "Green",
                "Colorless",
                "MonoColor",
                "MultiColor",
                "EnemyColor",
            ]
            .iter()
            .any(|color| characteristic.ends_with(color));
            return Some(if damage && is_color {
                format!("{characteristic}Source")
            } else {
                characteristic.to_string()
            });
        }
    } else {
        let protect_type = kw.strip_prefix("Protection from ")?.trim();
        let color = match protect_type {
            "white" => "White",
            "blue" => "Blue",
            "black" => "Black",
            "red" => "Red",
            "green" => "Green",
            "colorless" => "Colorless",
            "each color" => "nonColorless",
            "everything" => return Some(String::new()),
            _ => return None,
        };
        format!("{color}{}", if damage { "Source" } else { "" })
    };
    Some(format!("Card.{valid_source},Emblem.{valid_source}"))
}
