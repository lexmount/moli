use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexedDbError {
    Constraint(String),
    Data(String),
    InvalidAccess(String),
    InvalidState(String),
    NotFound(String),
    QuotaExceeded { quota: u64, requested: u64 },
    ReadOnly(String),
    Resource(String),
    TransactionInactive(String),
    Version(String),
    Io(String),
    Corruption(String),
    Serialization(String),
    Syntax(String),
}

impl fmt::Display for IndexedDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(message)
            | Self::Constraint(message)
            | Self::InvalidAccess(message)
            | Self::InvalidState(message)
            | Self::NotFound(message)
            | Self::ReadOnly(message)
            | Self::Resource(message)
            | Self::TransactionInactive(message)
            | Self::Version(message)
            | Self::Io(message)
            | Self::Corruption(message)
            | Self::Serialization(message)
            | Self::Syntax(message) => f.write_str(message),
            Self::QuotaExceeded { quota, requested } => write!(
                f,
                "IndexedDB write would exceed storage quota: requested {requested} bytes, quota {quota} bytes"
            ),
        }
    }
}

impl std::error::Error for IndexedDbError {}
