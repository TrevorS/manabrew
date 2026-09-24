use forge_foundation::{CardTypeLine, CoreType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardChangedType {
    pub add_type: Vec<String>,
    pub add_all_creature_types: bool,
    pub remove_super_types: bool,
    pub remove_card_types: bool,
    pub remove_sub_types: bool,
    pub remove_creature_types: bool,
}

impl CardChangedType {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply_changes(&self, new_type: &mut CardTypeLine) {
        if self.remove_card_types {
            new_type
                .core_types
                .retain(|t| matches!(t, CoreType::Instant | CoreType::Sorcery));
        }
        if self.remove_super_types {
            new_type.supertypes.clear();
        }
        if self.remove_sub_types {
            new_type.subtypes.clear();
        } else if !new_type.subtypes.is_empty() && self.remove_creature_types {
            new_type
                .subtypes
                .retain(|s| !crate::game::TypeRegistry::is_creature_type(s));
            new_type.all_creature_types = false;
        }
        for t in &self.add_type {
            new_type.add_type(t);
        }
        if self.add_all_creature_types {
            new_type.all_creature_types = true;
        }
    }
}
