use super::*;

pub(super) fn opened_database_info(
    scope: &mut v8::PinScope<'_, '_>,
    database_name: &str,
    opened: &moli_indexeddb::OpenResult,
) -> Result<DatabaseInfo, moli_indexeddb::IndexedDbError> {
    match with_indexed_db_manager(scope, |manager| manager.database_info(opened.database)) {
        Ok(mut info) => {
            if let OpenDisposition::UpgradeNeeded { new_version, .. } = opened.disposition {
                info.version = new_version;
            }
            Ok(info)
        }
        Err(error) => match &opened.disposition {
            OpenDisposition::UpgradeNeeded {
                old_version: 0,
                new_version,
            } => Ok(DatabaseInfo {
                name: database_name.to_string(),
                version: *new_version,
                object_store_names: Vec::new(),
            }),
            _ => Err(error),
        },
    }
}
