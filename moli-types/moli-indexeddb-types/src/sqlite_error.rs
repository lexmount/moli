use crate::IndexedDbError;

impl From<rusqlite::Error> for IndexedDbError {
    fn from(error: rusqlite::Error) -> Self {
        match moli_sqlite::classify_error(&error) {
            moli_sqlite::ErrorClass::Constraint => Self::Constraint(error.to_string()),
            moli_sqlite::ErrorClass::Corruption => Self::Corruption(error.to_string()),
            _ => Self::Io(error.to_string()),
        }
    }
}
