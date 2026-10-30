use std::{
    collections::BTreeMap,
    fmt, io,
    ops::Range,
    sync::{Arc, Weak},
};

use moli_file_api::{BlobData, BlobReadCompletion, BlobSource};
use parking_lot::Mutex;

use super::*;

/// The service routes reads onto the database's existing SQLite worker.
pub(super) trait BlobReader: Send + Sync {
    fn read(&self, id: i64, size: usize, range: Range<usize>, complete: BlobReadCompletion);
}

pub(super) struct BlobSnapshots {
    reader: Arc<dyn BlobReader>,
    live: Mutex<BTreeMap<i64, Weak<StoredBlob>>>,
}

impl BlobSnapshots {
    pub(super) fn new(reader: Arc<dyn BlobReader>) -> Arc<Self> {
        Arc::new(Self {
            reader,
            live: Mutex::new(BTreeMap::new()),
        })
    }

    pub(super) fn snapshot(self: &Arc<Self>, id: i64, size: usize) -> BlobData {
        let mut live = self.live.lock();
        let blob = live.get(&id).and_then(Weak::upgrade).unwrap_or_else(|| {
            let blob = Arc::new(StoredBlob {
                id,
                size,
                reader: self.reader.clone(),
                snapshots: Arc::downgrade(self),
                saved: Arc::new(Mutex::new(None)),
            });
            live.insert(id, Arc::downgrade(&blob));
            blob
        });
        BlobData::from_source(blob, size)
    }

    pub(super) fn has_live(&self) -> bool {
        !self.live.lock().is_empty()
    }

    pub(super) fn preserve_all(&self, database: &Database) {
        let blobs: Vec<_> = self
            .live
            .lock()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for blob in blobs {
            // Retirement must close even when the persistent source is no
            // longer readable. Keep that error for later byte consumers.
            if let Err(error) = blob.preserve(database) {
                blob.saved
                    .lock()
                    .get_or_insert_with(|| Err(error.to_string()));
            }
        }
    }

    pub(super) fn preserve(&self, database: &Database, ids: &[i64]) -> Result<(), IndexedDbError> {
        let blobs: Vec<_> = {
            let live = self.live.lock();
            ids.iter()
                .filter_map(|id| live.get(id).and_then(Weak::upgrade))
                .collect()
        };
        for blob in blobs {
            blob.preserve(database)?;
        }
        Ok(())
    }

    pub(super) fn fail(&self, error: &IndexedDbError) {
        let blobs: Vec<_> = self
            .live
            .lock()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for blob in blobs {
            blob.saved
                .lock()
                .get_or_insert_with(|| Err(error.to_string()));
        }
    }
}

type SavedBytes = Arc<Mutex<Option<Result<Arc<[u8]>, String>>>>;

struct StoredBlob {
    id: i64,
    size: usize,
    reader: Arc<dyn BlobReader>,
    snapshots: Weak<BlobSnapshots>,
    saved: SavedBytes,
}

impl StoredBlob {
    fn preserve(&self, database: &Database) -> Result<(), IndexedDbError> {
        if let Some(saved) = self.saved.lock().as_ref() {
            return saved
                .as_ref()
                .map(|_| ())
                .map_err(|error| IndexedDbError::Io(error.clone()));
        }
        let bytes = database
            .read_chunks("blob_chunks", "object_id", self.id, self.size as u64)
            .map(Arc::from)?;
        *self.saved.lock() = Some(Ok(bytes));
        Ok(())
    }
}

impl fmt::Debug for StoredBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredBlob")
            .field("id", &self.id)
            .field("size", &self.size)
            .finish()
    }
}

fn saved_range(saved: &SavedBytes, range: &Range<usize>) -> Option<io::Result<Vec<u8>>> {
    saved.lock().as_ref().map(|bytes| match bytes {
        Ok(bytes) => Ok(bytes[range.clone()].to_vec()),
        Err(error) => Err(io::Error::other(error.clone())),
    })
}

impl BlobSource for StoredBlob {
    fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        if let Some(bytes) = saved_range(&self.saved, &range) {
            complete(bytes);
            return;
        }
        let saved = self.saved.clone();
        let retry = range.clone();
        self.reader.read(
            self.id,
            self.size,
            range,
            Box::new(move |result| {
                // A rejected read completes after owner retirement has preserved
                // the bytes or its error. A handed-out Blob outlives its service.
                complete(match result {
                    Err(error) => saved_range(&saved, &retry).unwrap_or(Err(error)),
                    result => result,
                });
            }),
        );
    }
}

impl Drop for StoredBlob {
    fn drop(&mut self) {
        if let Some(snapshots) = self.snapshots.upgrade() {
            let mut live = snapshots.live.lock();
            if live
                .get(&self.id)
                .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
            {
                live.remove(&self.id);
            }
        }
    }
}

impl Database {
    /// Validate chunk metadata and fetch only chunks intersecting the read.
    /// No row identifier or connection is borrowed by a returned Blob.
    pub(super) fn read_blob_range(
        &self,
        id: i64,
        expected: usize,
        range: Range<usize>,
    ) -> Result<Vec<u8>, IndexedDbError> {
        let size: Option<i64> = self
            .connection
            .query_row(
                "SELECT size FROM external_objects WHERE object_id=?1 AND kind IN ('blob','file')",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if size.and_then(|size| usize::try_from(size).ok()) != Some(expected) {
            return Err(corruption("IndexedDB Blob source changed"));
        }
        if range.start > range.end || range.end > expected {
            return Err(corruption("Invalid Blob read range"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(range.len())
            .map_err(|_| IndexedDbError::Io("Unable to allocate Blob read".into()))?;
        if range.is_empty() {
            return Ok(bytes);
        }
        // Our writer fills every chunk except the last. Seek directly to the
        // requested ordinals; a stream read must not rescan the whole body.
        let first = range.start / CHUNK_BYTES;
        let last = (range.end - 1) / CHUNK_BYTES;
        let count = expected.div_ceil(CHUNK_BYTES);
        let extra: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM blob_chunks WHERE object_id=?1 AND chunk_no>=?2)",
            rusqlite::params![id, count as i64],
            |row| row.get(0),
        )?;
        if extra {
            return Err(corruption("IndexedDB Blob has extra chunks"));
        }
        let mut statement = self.connection.prepare_cached("SELECT chunk_no, bytes FROM blob_chunks WHERE object_id=?1 AND chunk_no BETWEEN ?2 AND ?3 ORDER BY chunk_no")?;
        let mut rows = statement.query(rusqlite::params![id, first as i64, last as i64])?;
        let mut ordinal = first;
        while let Some(row) = rows.next()? {
            let chunk = row
                .get_ref(1)?
                .as_blob()
                .map_err(|_| corruption("Invalid Blob chunk"))?;
            let offset = ordinal * CHUNK_BYTES;
            let length = CHUNK_BYTES.min(expected - offset);
            if row.get::<_, i64>(0)? != ordinal as i64 || chunk.len() != length {
                return Err(corruption("Invalid Blob chunks"));
            }
            bytes.extend_from_slice(
                &chunk[range.start.saturating_sub(offset)..length.min(range.end - offset)],
            );
            ordinal += 1;
        }
        if ordinal != last + 1 {
            return Err(corruption("IndexedDB Blob is truncated"));
        }
        Ok(bytes)
    }
}
