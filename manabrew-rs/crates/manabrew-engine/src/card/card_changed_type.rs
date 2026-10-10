use forge_foundation::{CardTypeLine, CoreType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardChangedType {
    pub add_type: Vec<String>,
    #[serde(default)]
    pub remove_type: Vec<String>,
    pub add_all_creature_types: bool,
    pub remove_super_types: bool,
    pub remove_card_types: bool,
    pub remove_sub_types: bool,
    #[serde(default)]
    pub remove_land_types: bool,
    pub remove_creature_types: bool,
    #[serde(default)]
    pub remove_artifact_types: bool,
    #[serde(default)]
    pub remove_enchantment_types: bool,
}

impl CardChangedType {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply_changes(&self, new_type: &mut CardTypeLine) {
        use crate::game::TypeRegistry;
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
        } else if !new_type.subtypes.is_empty() {
            if self.remove_land_types {
                new_type.subtypes.retain(|s| !TypeRegistry::is_land_type(s));
            }
            if self.remove_creature_types {
                new_type
                    .subtypes
                    .retain(|s| !TypeRegistry::is_creature_type(s));
                new_type.all_creature_types = false;
            }
            if self.remove_artifact_types {
                new_type
                    .subtypes
                    .retain(|s| !TypeRegistry::is_subtype_in("ArtifactTypes", s));
            }
            if self.remove_enchantment_types {
                new_type
                    .subtypes
                    .retain(|s| !TypeRegistry::is_subtype_in("EnchantmentTypes", s));
            }
        }
        for t in &self.remove_type {
            new_type
                .supertypes
                .retain(|st| !st.name().eq_ignore_ascii_case(t));
            new_type
                .core_types
                .retain(|ct| !ct.name().eq_ignore_ascii_case(t));
            new_type.subtypes.retain(|s| !s.eq_ignore_ascii_case(t));
        }
        for t in &self.add_type {
            new_type.add_type(t);
        }
        if self.add_all_creature_types {
            new_type.all_creature_types = true;
        }
    }
}
