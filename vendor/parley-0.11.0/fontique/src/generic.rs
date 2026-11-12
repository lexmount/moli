// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generic font families.

use super::FamilyId;
use crate::GenericFamily;
use smallvec::SmallVec;

type FamilyVec = SmallVec<[FamilyId; 2]>;
const COUNT: usize = GenericFamily::MAX_VALUE as usize + 1;

/// Maps generic families to family identifiers.
#[derive(Clone, Default, Debug)]
pub struct GenericFamilyMap {
    map: [FamilyVec; COUNT],
}

impl GenericFamilyMap {
    /// Returns the associated family identifiers for the given generic family.
    pub fn get(&self, generic: GenericFamily) -> &[FamilyId] {
        &self.map[generic as usize]
    }

    /// Sets the associated family identifiers for the given generic family.
    pub fn set(&mut self, generic: GenericFamily, families: impl Iterator<Item = FamilyId>) {
        let map = &mut self.map[generic as usize];
        map.clear();
        map.extend(families);
    }

    /// Appends the family identifiers to the list for the given generic family.
    pub fn append(&mut self, generic: GenericFamily, families: impl Iterator<Item = FamilyId>) {
        self.map[generic as usize].extend(families);
    }
}
