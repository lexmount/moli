//! One-area-at-a-time legacy decoding, preserving the existing DOMString
//! codec. UTF-16 code units stay lossless throughout the import.
use super::*;
use moli_sqlite::writeback::{WritebackError, WritebackOwner, WritebackPolicy};
use moli_sqlite::{OpenMode, RuntimeHandle};
use rusqlite::{Transaction, params};
use serde::de::{DeserializeSeed, Error, IgnoredAny, MapAccess, Visitor};
use std::io::BufReader;
use std::time::Duration;

pub fn create_sqlite_from_legacy_json(
    path: PathBuf,
    legacy: PathBuf,
    fingerprint: String,
    migration_id: String,
    runtime: RuntimeHandle,
) -> Result<()> {
    let owner = WritebackOwner::<sqlite::Batch>::open(
        path,
        OpenMode::Create,
        runtime,
        WritebackPolicy {
            delay: Duration::from_secs(5),
            early_commit_operations: None,
        },
        sqlite::initialize,
    )?;
    owner.client().query(move |connection| {
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM areas", [])?;
        match fs::File::open(&legacy) {
            Ok(file) if file.metadata().map_err(WritebackError::new)?.len() != 0 => {
                let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(file));
                LegacyFile(&tx)
                    .deserialize(&mut deserializer)
                    .map_err(WritebackError::new)?;
                deserializer.end().map_err(WritebackError::new)?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(WritebackError::new(error)),
        }
        tx.execute("DELETE FROM migration", [])?;
        tx.execute(
            "INSERT INTO migration VALUES (?1,?2,1)",
            params![fingerprint, migration_id],
        )?;
        tx.commit()?;
        Ok(())
    })?;
    checkpoint(&owner.client())?;
    owner.close()?;
    Ok(())
}
struct LegacyFile<'tx, 'conn>(&'tx Transaction<'conn>);
impl<'de> DeserializeSeed<'de> for LegacyFile<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for LegacyFile<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a versioned Web Storage JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        let (mut version, mut origins) = (false, false);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => {
                    if version {
                        return Err(A::Error::duplicate_field("version"));
                    }
                    let _: WebStorageJsonVersion = map.next_value()?;
                    version = true;
                }
                "origins" => {
                    if origins {
                        return Err(A::Error::duplicate_field("origins"));
                    }
                    map.next_value_seed(Areas(self.0))?;
                    origins = true;
                }
                _ => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        if !version {
            return Err(A::Error::missing_field("version"));
        }
        if !origins {
            return Err(A::Error::missing_field("origins"));
        }
        Ok(())
    }
}
struct Areas<'tx, 'conn>(&'tx Transaction<'conn>);
impl<'de> DeserializeSeed<'de> for Areas<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Areas<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Web Storage areas")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some((key, area)) = map.next_entry::<String, JsonWebStorageAreaFile>()? {
            import_area(self.0, &key, area).map_err(A::Error::custom)?;
        }
        Ok(())
    }
}
fn import_area(
    tx: &Transaction<'_>,
    key: &str,
    area: JsonWebStorageAreaFile,
) -> std::result::Result<(), WritebackError> {
    let values = area
        .entries
        .into_iter()
        .map(|entry| {
            (
                entry.key.into_web_storage_string(),
                entry.value.into_web_storage_string(),
            )
        })
        .collect::<HashMap<_, _>>();
    let usage = values
        .values()
        .try_fold(0usize, |sum, value| sum.checked_add(value.usage_bytes()))
        .ok_or_else(|| WritebackError::new("legacy localStorage usage overflow"))?;
    let id:i64=tx.query_row("INSERT INTO areas(storage_key,usage_bytes,entry_count) VALUES (?1,?2,?3) ON CONFLICT(storage_key) DO UPDATE SET usage_bytes=excluded.usage_bytes,entry_count=excluded.entry_count RETURNING area_id",params![key,i64::try_from(usage).map_err(WritebackError::new)?,i64::try_from(values.len()).map_err(WritebackError::new)?],|r|r.get(0))?;
    tx.execute("DELETE FROM entries WHERE area_id=?1", [id])?;
    let mut statement =
        tx.prepare_cached("INSERT INTO entries(area_id,key,value) VALUES (?1,?2,?3)")?;
    for (key, value) in values {
        statement.execute(params![
            id,
            sqlite::encode(key.as_units()),
            sqlite::encode(value.as_units())
        ])?;
    }
    Ok(())
}
pub fn inspect_sqlite_migration(
    path: PathBuf,
    runtime: RuntimeHandle,
) -> Result<Option<(String, String)>> {
    Ok(WritebackOwner::<sqlite::Batch>::inspect(
        path,
        runtime,
        |connection| {
            let application: i32 =
                connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
            let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if application != sqlite::APPLICATION_ID || version != 1 {
                return Err(WritebackError::new(
                    "unidentified localStorage migration database",
                ));
            }
            let count: i64 =
                connection.query_row("SELECT count(*) FROM migration", [], |r| r.get(0))?;
            if count != 1 {
                return Err(WritebackError::new("incomplete localStorage migration"));
            }
            Ok(Some(connection.query_row(
                "SELECT source_fingerprint,migration_id FROM migration WHERE complete=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?))
        },
    )?)
}
fn checkpoint(
    client: &moli_sqlite::writeback::WritebackClient<sqlite::Batch>,
) -> std::result::Result<(), WritebackError> {
    client.query(|connection| {
        let busy: i32 =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(WritebackError::new("localStorage checkpoint is busy"));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_sqlite::Runtime;
    #[test]
    fn a_late_legacy_format_error_rolls_back_already_imported_areas() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("old.json");
        let path = root.path().join("localstorage.sqlite");
        let runtime = Runtime::new().unwrap();
        fs::write(&legacy,r#"{"origins":{"a":{"entries":[{"key":"written-before-version","value":"v"}]}},"version":2}"#).unwrap();
        assert!(
            create_sqlite_from_legacy_json(
                path.clone(),
                legacy,
                "source".into(),
                "migration".into(),
                runtime.handle()
            )
            .is_err()
        );
        assert!(inspect_sqlite_migration(path.clone(), runtime.handle()).is_err());
        let (store, owner) =
            new_sqlite_web_storage_store(path, OpenMode::Existing, runtime.handle()).unwrap();
        assert_eq!(store.try_len("a").unwrap(), 0);
        owner.close().unwrap();
    }
    #[test]
    fn streaming_import_preserves_utf16_and_leaves_unrelated_areas_unloaded() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("old.json");
        let path = root.path().join("localstorage.sqlite");
        let runtime = Runtime::new().unwrap();
        let origins=(0..100).map(|i|(format!("area-{i}"),serde_json::json!({"entries":[{"key":{"utf16":[55296,0]},"value":{"utf16":[56320,0]}}]}))).collect::<BTreeMap<_,_>>();
        fs::write(
            &legacy,
            serde_json::to_vec(&serde_json::json!({"version":1,"origins":origins})).unwrap(),
        )
        .unwrap();
        create_sqlite_from_legacy_json(
            path.clone(),
            legacy,
            "source".into(),
            "migration".into(),
            runtime.handle(),
        )
        .unwrap();
        assert_eq!(
            inspect_sqlite_migration(path.clone(), runtime.handle()).unwrap(),
            Some(("source".into(), "migration".into()))
        );
        let (store, owner) =
            new_sqlite_web_storage_store(path, OpenMode::Existing, runtime.handle()).unwrap();
        let _view = store.lease_area("area-42").unwrap();
        assert_eq!(
            store.try_get_item_utf16("area-42", &[55296, 0]).unwrap(),
            Some(vec![56320, 0])
        );
        assert_eq!(store.try_len("area-99").unwrap(), 1);
        owner.close().unwrap();
    }
}
