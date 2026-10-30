use super::*;
use moli_file_api::BlobReadCompletion;
use std::{io, ops::Range, sync::Weak};

pub(super) struct DatabaseBlobReader {
    pub(super) owner: Weak<Owner>,
}

impl super::super::blobs::BlobReader for DatabaseBlobReader {
    fn read(&self, object: i64, size: usize, range: Range<usize>, complete: BlobReadCompletion) {
        let Some(owner) = self.owner.upgrade() else {
            complete(Err(io::Error::other("Blob database is retired")));
            return;
        };
        let complete = Arc::new(Mutex::new(Some(complete)));
        let delivered = complete.clone();
        let submitted = owner.submit(
            move |owner| {
                let config = owner.config;
                let entry = owner.entry()?;
                entry.connect(config)?;
                entry
                    .database
                    .as_ref()
                    .expect("connected Blob database")
                    .read_blob_range(object, size, range)
            },
            move |result| {
                if let Some(complete) = delivered.lock().take() {
                    complete(result.map_err(|error| io::Error::other(error.to_string())));
                }
            },
        );
        if let Err(error) = submitted
            && let Some(complete) = complete.lock().take()
        {
            complete(Err(io::Error::other(error.to_string())));
        }
    }
}
