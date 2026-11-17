use crate::Key;
/// Renderer-neutral bounds for storage and index queries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRange {
    pub lower: Option<Key>,
    pub upper: Option<Key>,
    pub lower_open: bool,
    pub upper_open: bool,
}

impl KeyRange {
    pub fn validate(&self) -> Result<(), crate::IndexedDbError> {
        for key in self.lower.iter().chain(self.upper.iter()) {
            key.validate_depth()
                .map_err(|message| crate::IndexedDbError::Data(message.to_owned()))?;
        }
        if let (Some(lower), Some(upper)) = (&self.lower, &self.upper)
            && (lower > upper || (lower == upper && (self.lower_open || self.upper_open)))
        {
            return Err(crate::IndexedDbError::Data(
                "invalid IndexedDB key range".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn contains(&self, key: &Key) -> bool {
        let above_lower = self
            .lower
            .as_ref()
            .is_none_or(|lower| key > lower || (key == lower && !self.lower_open));
        let below_upper = self
            .upper
            .as_ref()
            .is_none_or(|upper| key < upper || (key == upper && !self.upper_open));
        above_lower && below_upper
    }
}
