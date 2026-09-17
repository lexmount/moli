use std::str::FromStr;

use crate::{KeyPath, TransactionMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionModeParseError {
    Unsupported,
}

pub fn parse_regular_transaction_mode(
    label: Option<&str>,
) -> Result<TransactionMode, TransactionModeParseError> {
    let Some(label) = label else {
        return Ok(TransactionMode::ReadOnly);
    };
    match TransactionMode::from_str(label) {
        Ok(mode @ (TransactionMode::ReadOnly | TransactionMode::ReadWrite)) => Ok(mode),
        Ok(TransactionMode::VersionChange) | Err(_) => Err(TransactionModeParseError::Unsupported),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectStoreOptionsValidationError {
    AutoIncrementEmptyKeyPath,
    AutoIncrementSequenceKeyPath,
}

pub fn validate_object_store_options(
    key_path: Option<&KeyPath>,
    auto_increment: bool,
) -> Result<(), ObjectStoreOptionsValidationError> {
    if !auto_increment {
        return Ok(());
    }
    match key_path {
        Some(KeyPath::String(value)) if value.is_empty() => {
            Err(ObjectStoreOptionsValidationError::AutoIncrementEmptyKeyPath)
        }
        Some(KeyPath::Sequence(_)) => {
            Err(ObjectStoreOptionsValidationError::AutoIncrementSequenceKeyPath)
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexOptionsValidationError {
    MultiEntrySequenceKeyPath,
}

pub fn validate_index_options(
    key_path: &KeyPath,
    multi_entry: bool,
) -> Result<(), IndexOptionsValidationError> {
    if multi_entry && key_path.is_sequence() {
        Err(IndexOptionsValidationError::MultiEntrySequenceKeyPath)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetAllOptionsCandidate {
    pub is_object: bool,
    pub is_key_range: bool,
    pub is_date: bool,
    pub is_array: bool,
    pub is_buffer_source: bool,
}

pub fn should_parse_get_all_options(candidate: GetAllOptionsCandidate) -> bool {
    candidate.is_object
        && !candidate.is_key_range
        && !candidate.is_date
        && !candidate.is_array
        && !candidate.is_buffer_source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_transaction_mode_accepts_only_read_modes() {
        assert_eq!(
            parse_regular_transaction_mode(None),
            Ok(TransactionMode::ReadOnly)
        );
        assert_eq!(
            parse_regular_transaction_mode(Some("readonly")),
            Ok(TransactionMode::ReadOnly)
        );
        assert_eq!(
            parse_regular_transaction_mode(Some("readwrite")),
            Ok(TransactionMode::ReadWrite)
        );
        assert_eq!(
            parse_regular_transaction_mode(Some("versionchange")),
            Err(TransactionModeParseError::Unsupported)
        );
        assert_eq!(
            parse_regular_transaction_mode(Some("readWrite")),
            Err(TransactionModeParseError::Unsupported)
        );
    }

    #[test]
    fn object_store_options_reject_auto_increment_key_path_combinations() {
        assert_eq!(validate_object_store_options(None, true), Ok(()));
        assert_eq!(
            validate_object_store_options(Some(&KeyPath::String("id".to_owned())), true),
            Ok(())
        );
        assert_eq!(
            validate_object_store_options(Some(&KeyPath::String(String::new())), true),
            Err(ObjectStoreOptionsValidationError::AutoIncrementEmptyKeyPath)
        );
        assert_eq!(
            validate_object_store_options(Some(&KeyPath::Sequence(vec!["id".to_owned()])), true),
            Err(ObjectStoreOptionsValidationError::AutoIncrementSequenceKeyPath)
        );
        assert_eq!(
            validate_object_store_options(Some(&KeyPath::Sequence(vec![])), false),
            Ok(())
        );
    }

    #[test]
    fn index_options_reject_multi_entry_sequence_key_path() {
        assert_eq!(
            validate_index_options(&KeyPath::String("tags".to_owned()), true),
            Ok(())
        );
        assert_eq!(
            validate_index_options(
                &KeyPath::Sequence(vec!["a".to_owned(), "b".to_owned()]),
                true
            ),
            Err(IndexOptionsValidationError::MultiEntrySequenceKeyPath)
        );
        assert_eq!(
            validate_index_options(&KeyPath::Sequence(vec!["a".to_owned()]), false),
            Ok(())
        );
    }

    #[test]
    fn get_all_options_candidate_excludes_key_like_objects() {
        assert!(should_parse_get_all_options(GetAllOptionsCandidate {
            is_object: true,
            is_key_range: false,
            is_date: false,
            is_array: false,
            is_buffer_source: false,
        }));

        for candidate in [
            GetAllOptionsCandidate {
                is_object: false,
                is_key_range: false,
                is_date: false,
                is_array: false,
                is_buffer_source: false,
            },
            GetAllOptionsCandidate {
                is_object: true,
                is_key_range: true,
                is_date: false,
                is_array: false,
                is_buffer_source: false,
            },
            GetAllOptionsCandidate {
                is_object: true,
                is_key_range: false,
                is_date: true,
                is_array: false,
                is_buffer_source: false,
            },
            GetAllOptionsCandidate {
                is_object: true,
                is_key_range: false,
                is_date: false,
                is_array: true,
                is_buffer_source: false,
            },
            GetAllOptionsCandidate {
                is_object: true,
                is_key_range: false,
                is_date: false,
                is_array: false,
                is_buffer_source: true,
            },
        ] {
            assert!(!should_parse_get_all_options(candidate));
        }
    }
}
