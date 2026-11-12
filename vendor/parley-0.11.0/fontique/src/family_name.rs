// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Model for font family names.

use super::family::FamilyId;
use alloc::sync::Arc;
use hashbrown::HashMap;
use smallvec::SmallVec;

/// Handle for a font family that includes both the name and a unique
/// identifier.
#[derive(Clone, Debug)]
pub struct FamilyName {
    id: FamilyId,
    name: Arc<str>,
}

impl FamilyName {
    /// Returns the unique identifier for the font family.
    pub fn id(&self) -> FamilyId {
        self.id
    }

    /// Returns the name of the font family.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl From<&FamilyName> for FamilyId {
    fn from(value: &FamilyName) -> Self {
        value.id
    }
}

/// Bidirectional map that associates font family names with unique
/// identifiers.
#[derive(Clone, Default)]
pub struct FamilyNameMap {
    name_map: HashMap<Arc<[u8]>, FamilyName>,
    id_map: HashMap<FamilyId, FamilyName>,
}

impl FamilyNameMap {
    /// Returns the family name object for the given name.
    pub fn get(&self, name: &str) -> Option<&FamilyName> {
        let key = NameKey::from_str(name);
        self.name_map.get(key.as_bytes())
    }

    /// Returns the family name object for the given identifier.
    pub fn get_by_id(&self, id: FamilyId) -> Option<&FamilyName> {
        self.id_map.get(&id)
    }

    /// Returns the family name object with the given name or creates
    /// a new one if it doesn't exist.
    pub fn get_or_insert(&mut self, name: &str) -> FamilyName {
        let key = NameKey::from_str(name);
        if let Some(name) = self.name_map.get(key.as_bytes()) {
            name.clone()
        } else {
            let new_name = FamilyName {
                name: name.into(),
                id: FamilyId::new(),
            };
            self.name_map
                .insert(key.as_bytes().into(), new_name.clone());
            self.id_map.insert(new_name.id, new_name.clone());
            new_name
        }
    }

    /// Adds `name` as an alias for the given family identifier.
    #[allow(unused)]
    pub fn add_alias(&mut self, id: FamilyId, name: &str) {
        if self.id_map.contains_key(&id) {
            let key = NameKey::from_str(name);
            if self.name_map.contains_key(key.as_bytes()) {
                return;
            }
            let new_name = FamilyName {
                name: name.into(),
                id,
            };
            self.name_map.insert(key.as_bytes().into(), new_name);
        }
    }

    /// Returns an iterator over all of the font family names, including aliases.
    pub fn iter(&self) -> impl Iterator<Item = &FamilyName> + Clone {
        self.name_map.values()
    }

    /// Returns an iterator over all of the font family identifiers.
    pub fn ids(&self) -> impl Iterator<Item = FamilyId> + Clone + '_ {
        self.id_map.keys().copied()
    }
}

/// Key for case-insensitive lookup of family names.
#[derive(Default)]
struct NameKey {
    data: SmallVec<[u8; 128]>,
}

impl NameKey {
    fn from_str(s: &str) -> Self {
        let mut res = Self::default();
        let mut buf = [0_u8; 4];
        for ch in s.chars() {
            for ch in ch.to_lowercase() {
                res.data
                    .extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
        res
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::FamilyNameMap;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn iterate_names_and_ids() {
        let mut map = FamilyNameMap::default();
        let name = map.get_or_insert("Family");
        map.add_alias(name.id(), "Alias");
        let other_name = map.get_or_insert("Other family");

        let mut names: Vec<_> = map.iter().map(|f| f.name()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            &["Alias", "Family", "Other family"],
            "family names and aliases should both show up"
        );

        let mut ids: Vec<_> = map.ids().collect();
        ids.sort_unstable();
        let mut expected = vec![name.id(), other_name.id()];
        expected.sort_unstable();
        assert_eq!(ids, expected, "family identifiers should only show up once");
    }
}
