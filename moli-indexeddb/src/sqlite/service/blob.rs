use super::*;
use moli_file_api::BlobReadCompletion;
use std::{io, ops::Range, sync::Weak};

pub(super) struct DatabaseBlobReader {
    pub(super) owner: Weak<Owner>,
    pub(super) runtime: RuntimeHandle,
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
                complete_read(
                    &delivered,
                    result.map_err(|error| io::Error::other(error.to_string())),
                );
            },
        );
        if let Err(error) = submitted {
            // Sealing the mailbox precedes Blob preservation. Wait for the
            // existing final receipt so StoredBlob's fallback sees either the
            // saved bytes or the preservation error, never a transient Closed.
            let mut receipt = owner.stop();
            let pending = complete.clone();
            let failure = error.to_string();
            let rejected = failure.clone();
            let worker = self.runtime.clone();
            if self
                .runtime
                .spawn(async move {
                    let _ = receipt.wait_for(|result| result.is_some()).await;
                    let _ = worker
                        .blocking(move || {
                            complete_read(&pending, Err(io::Error::other(failure)));
                        })
                        .await;
                })
                .is_err()
            {
                // Runtime shutdown seals task admission only after all owner
                // receipts, so preservation has already finished in this case.
                complete_read(&complete, Err(io::Error::other(rejected)));
            }
        }
    }
}

fn complete_read(complete: &Mutex<Option<BlobReadCompletion>>, result: io::Result<Vec<u8>>) {
    let complete = complete.lock().take();
    if let Some(complete) = complete {
        complete(result);
    }
}
