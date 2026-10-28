//! Version 1 schema key paths: a tagged UTF-8 string or string sequence.

use crate::{IndexedDbError, KeyPath, read_budget::ReadBudget};

impl KeyPath {
    /// Encode a validated schema key path. Absence is represented by SQL NULL.
    pub fn encode_value(&self) -> Result<Vec<u8>, IndexedDbError> {
        if !self.is_valid() {
            return Err(IndexedDbError::Data(
                "invalid IndexedDB key path".to_owned(),
            ));
        }
        let mut output = Vec::new();
        match self {
            KeyPath::String(path) => {
                output.push(0);
                write_string(&mut output, path)?;
            }
            KeyPath::Sequence(paths) => {
                output.push(1);
                let count = u32::try_from(paths.len())
                    .map_err(|_| IndexedDbError::Data("key path sequence too long".to_owned()))?;
                output.extend_from_slice(&count.to_be_bytes());
                for path in paths {
                    write_string(&mut output, path)?;
                }
            }
        }
        Ok(output)
    }

    /// Decode one complete key path, checking lengths before allocation.
    pub fn decode_value(bytes: &[u8]) -> Result<Self, IndexedDbError> {
        let mut budget = u64::MAX;
        Self::decode_value_budgeted(bytes, &mut budget)
    }

    pub(crate) fn decode_value_budgeted(
        bytes: &[u8],
        budget: &mut dyn ReadBudget,
    ) -> Result<Self, IndexedDbError> {
        let mut input = Input { bytes, offset: 0 };
        let tag = input.take(1)?[0];
        let path = match tag {
            0 => Self::String(input.string(budget)?),
            1 => {
                let count = usize::try_from(input.count()?).map_err(|_| invalid())?;
                // Every string needs a four-byte length, including empty ones.
                if count > input.bytes.len().saturating_sub(input.offset) / 4 {
                    return Err(invalid());
                }
                let mut paths = Vec::new();
                budget.charge(
                    (count as u64)
                        .checked_mul(std::mem::size_of::<String>() as u64)
                        .ok_or_else(invalid)?,
                )?;
                paths.try_reserve_exact(count).map_err(|_| {
                    IndexedDbError::Resource("Unable to allocate IndexedDB key paths".into())
                })?;
                for _ in 0..count {
                    paths.push(input.string(budget)?);
                }
                Self::Sequence(paths)
            }
            _ => return Err(invalid()),
        };
        if input.offset != bytes.len() || !path.is_valid() {
            return Err(invalid());
        }
        Ok(path)
    }
}

fn write_string(output: &mut Vec<u8>, string: &str) -> Result<(), IndexedDbError> {
    let length = u32::try_from(string.len())
        .map_err(|_| IndexedDbError::Data("key path string too long".to_owned()))?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(string.as_bytes());
    Ok(())
}

fn invalid() -> IndexedDbError {
    IndexedDbError::Corruption("invalid IndexedDB binary key path".to_owned())
}

struct Input<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Input<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], IndexedDbError> {
        let end = self.offset.checked_add(length).ok_or_else(invalid)?;
        let bytes = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(bytes)
    }

    fn count(&mut self) -> Result<u32, IndexedDbError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four-byte count"),
        ))
    }

    fn string(&mut self, budget: &mut dyn ReadBudget) -> Result<String, IndexedDbError> {
        let length = usize::try_from(self.count()?).map_err(|_| invalid())?;
        let bytes = self.take(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| invalid())?;
        budget.charge(length as u64)?;
        let mut string = String::new();
        string.try_reserve_exact(length).map_err(|_| {
            IndexedDbError::Resource("Unable to allocate an IndexedDB key path".into())
        })?;
        string.push_str(value);
        Ok(string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_key_paths_roundtrip_without_collapsing_string_and_sequence() {
        for path in [
            KeyPath::String(String::new()),
            KeyPath::String("id".to_owned()),
            KeyPath::String("用户.编号".to_owned()),
            KeyPath::Sequence(vec!["id".to_owned()]),
            KeyPath::Sequence(vec!["用户".to_owned(), "id".to_owned()]),
        ] {
            let bytes = path.encode_value().unwrap();
            assert_eq!(KeyPath::decode_value(&bytes).unwrap(), path);
            for end in 0..bytes.len() {
                assert!(KeyPath::decode_value(&bytes[..end]).is_err());
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert!(KeyPath::decode_value(&trailing).is_err());
        }
        assert_ne!(
            KeyPath::String("id".to_owned()).encode_value().unwrap(),
            KeyPath::Sequence(vec!["id".to_owned()])
                .encode_value()
                .unwrap()
        );
    }

    #[test]
    fn corrupt_schema_paths_cannot_allocate_from_untrusted_counts() {
        for bytes in [
            vec![],
            vec![2],
            vec![0, 0xff, 0xff, 0xff, 0xff],
            vec![1, 0xff, 0xff, 0xff, 0xff],
            vec![1, 0, 0, 0, 0],
            vec![0, 0, 0, 0, 1, 0xff],
            vec![0, 0, 0, 0, 1, b'-'],
        ] {
            assert!(KeyPath::decode_value(&bytes).is_err());
        }
        assert!(KeyPath::Sequence(vec![]).encode_value().is_err());
    }

    #[test]
    fn schema_key_path_budget_covers_sequence_capacity_and_utf8_before_allocation() {
        let path = KeyPath::Sequence(vec!["id".into(), "用户.编号".into()]);
        let bytes = path.encode_value().unwrap();
        let needed = (2 * std::mem::size_of::<String>() + "id".len() + "用户.编号".len()) as u64;
        let mut exact = needed;
        assert_eq!(
            KeyPath::decode_value_budgeted(&bytes, &mut exact).unwrap(),
            path
        );
        assert_eq!(exact, 0);
        assert!(KeyPath::decode_value_budgeted(&bytes, &mut (needed - 1)).is_err());
        let mut tiny = 1;
        assert!(KeyPath::decode_value_budgeted(&[1, 255, 255, 255, 255], &mut tiny).is_err());
        assert_eq!(tiny, 1);
    }
}
