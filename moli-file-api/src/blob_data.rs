use std::{
    fmt, io,
    ops::Range,
    sync::{Arc, mpsc},
};

use parking_lot::Mutex;

pub type BlobReadCompletion = Box<dyn FnOnce(io::Result<Vec<u8>>) + Send>;

/// An immutable byte source. Implementations perform I/O on their owning
/// backend, then complete the read without entering a JavaScript context.
pub trait BlobSource: fmt::Debug + Send + Sync + 'static {
    fn read(&self, range: Range<usize>, complete: BlobReadCompletion);
}

/// Blob data is immutable. Slices and concatenation retain their source rather
/// than reading it; only a byte-consuming operation starts a source read.
#[derive(Clone)]
pub struct BlobData {
    backing: Arc<Backing>,
    offset: usize,
    length: usize,
}

enum Backing {
    Memory(Arc<[u8]>),
    Source(Arc<dyn BlobSource>),
    Parts(Vec<BlobData>),
}

impl fmt::Debug for BlobData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobData")
            .field("length", &self.length)
            .field("in_memory", &self.as_bytes().is_some())
            .finish_non_exhaustive()
    }
}

impl PartialEq for BlobData {
    fn eq(&self, other: &Self) -> bool {
        match (self.as_bytes(), other.as_bytes()) {
            (Some(left), Some(right)) => left == right,
            _ => {
                Arc::ptr_eq(&self.backing, &other.backing)
                    && self.offset == other.offset
                    && self.length == other.length
            }
        }
    }
}
impl Eq for BlobData {}

impl From<Vec<u8>> for BlobData {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from(Arc::<[u8]>::from(bytes))
    }
}

impl From<Arc<[u8]>> for BlobData {
    fn from(bytes: Arc<[u8]>) -> Self {
        let length = bytes.len();
        Self {
            backing: Arc::new(Backing::Memory(bytes)),
            offset: 0,
            length,
        }
    }
}

impl BlobData {
    pub fn from_source(source: Arc<dyn BlobSource>, length: usize) -> Self {
        Self {
            backing: Arc::new(Backing::Source(source)),
            offset: 0,
            length,
        }
    }

    pub fn concat(parts: Vec<Self>) -> io::Result<Self> {
        // Keep the backing graph flat so repeated Blob construction cannot
        // create recursive native destruction on a small Worker stack.
        let mut parts: Vec<_> = parts.into_iter().flat_map(|part| part.leaves()).collect();
        if parts.is_empty() {
            return Ok(Vec::new().into());
        }
        if parts.len() == 1 {
            return Ok(parts.pop().unwrap());
        }
        let length = parts.iter().try_fold(0usize, |total, part| {
            total
                .checked_add(part.len())
                .ok_or_else(|| io::Error::other("Blob size overflow"))
        })?;
        if parts.iter().all(|part| part.as_bytes().is_some()) {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(length)
                .map_err(|_| io::Error::other("Unable to allocate Blob bytes"))?;
            for part in parts {
                bytes.extend_from_slice(part.as_bytes().expect("memory part"));
            }
            return Ok(bytes.into());
        }
        Ok(Self {
            backing: Arc::new(Backing::Parts(parts)),
            offset: 0,
            length,
        })
    }

    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &*self.backing {
            Backing::Memory(bytes) => Some(&bytes[self.offset..self.offset + self.length]),
            _ => None,
        }
    }

    pub fn shared_bytes(&self) -> Option<Arc<[u8]>> {
        match &*self.backing {
            Backing::Memory(bytes) if self.offset == 0 && self.length == bytes.len() => {
                Some(bytes.clone())
            }
            Backing::Memory(_) => self.as_bytes().map(Arc::from),
            _ => None,
        }
    }

    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.length);
        Self {
            backing: self.backing.clone(),
            offset: self.offset + range.start,
            length: range.len(),
        }
    }

    pub fn read_all(&self, complete: BlobReadCompletion) {
        self.read(0..self.length, complete);
    }

    pub fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        if range.start > range.end || range.end > self.length {
            complete(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid Blob range",
            )));
            return;
        }
        let mut pieces = self.slice(range).leaves();
        if pieces.is_empty() {
            complete(Ok(Vec::new()));
            return;
        }
        if pieces.len() == 1 {
            pieces.pop().unwrap().read_piece(complete);
            return;
        }
        let result = Arc::new(Mutex::new(ReadParts {
            pending: pieces.len(),
            pieces: (0..pieces.len()).map(|_| None).collect(),
            complete: Some(complete),
        }));
        for (index, piece) in pieces.into_iter().enumerate() {
            let result = result.clone();
            piece.read_piece(Box::new(move |bytes| {
                ReadParts::finish(&result, index, bytes)
            }));
        }
    }

    fn leaves(&self) -> Vec<Self> {
        let mut pieces = Vec::new();
        let mut pending = vec![self.clone()];
        while let Some(data) = pending.pop() {
            if data.is_empty() {
                continue;
            }
            match &*data.backing {
                Backing::Parts(parts) => {
                    let mut offset = 0;
                    let end = data.offset + data.length;
                    let mut children = Vec::new();
                    for part in parts {
                        let next = offset + part.len();
                        let start = data.offset.max(offset);
                        let stop = end.min(next);
                        if start < stop {
                            children.push(part.slice(start - offset..stop - offset));
                        }
                        offset = next;
                        if offset >= end {
                            break;
                        }
                    }
                    // Expand without recursive traversal of nested Blobs.
                    pending.extend(children.into_iter().rev());
                }
                _ => pieces.push(data),
            }
        }
        pieces
    }

    fn read_piece(self, complete: BlobReadCompletion) {
        match &*self.backing {
            Backing::Memory(bytes) => {
                complete(Ok(bytes[self.offset..self.offset + self.length].to_vec()))
            }
            Backing::Source(source) => {
                let keep_alive = self.clone();
                source.read(
                    self.offset..self.offset + self.length,
                    Box::new(move |result| {
                        complete(result.and_then(|bytes| {
                            if bytes.len() != keep_alive.length {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Invalid Blob read length",
                                ));
                            }
                            Ok(bytes)
                        }));
                    }),
                );
            }
            Backing::Parts(_) => unreachable!("parts were flattened before reading"),
        }
    }

    /// For synchronous consumers (e.g. FileReaderSync) on a blocking-capable
    /// thread. Promise-based APIs use read() and their existing owner task.
    pub fn read_blocking(&self) -> io::Result<Vec<u8>> {
        let (send, receive) = mpsc::sync_channel(1);
        self.read_all(Box::new(move |bytes| {
            let _ = send.send(bytes);
        }));
        receive
            .recv()
            .map_err(|_| io::Error::other("Blob read was cancelled"))?
    }
}

struct ReadParts {
    pending: usize,
    pieces: Vec<Option<Vec<u8>>>,
    complete: Option<BlobReadCompletion>,
}

impl ReadParts {
    fn finish(this: &Mutex<Self>, index: usize, bytes: io::Result<Vec<u8>>) {
        let done = {
            let mut state = this.lock();
            if state.complete.is_none() {
                return;
            }
            match bytes {
                Err(error) => Some((state.complete.take().unwrap(), Err(error))),
                Ok(bytes) => {
                    state.pieces[index] = Some(bytes);
                    state.pending -= 1;
                    if state.pending != 0 {
                        return;
                    }
                    let pieces = std::mem::take(&mut state.pieces);
                    Some((state.complete.take().unwrap(), Ok(pieces)))
                }
            }
        };
        if let Some((complete, pieces)) = done {
            complete(pieces.and_then(|pieces| {
                let length = pieces.iter().try_fold(0usize, |total, piece| {
                    total
                        .checked_add(piece.as_ref().unwrap().len())
                        .ok_or_else(|| io::Error::other("Blob size overflow"))
                })?;
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(length)
                    .map_err(|_| io::Error::other("Unable to allocate Blob bytes"))?;
                for piece in pieces {
                    bytes.extend(piece.unwrap());
                }
                Ok(bytes)
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct CountingSource {
        bytes: Vec<u8>,
        reads: AtomicUsize,
    }

    impl BlobSource for CountingSource {
        fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
            self.reads.fetch_add(1, Ordering::Relaxed);
            complete(Ok(self.bytes[range].to_vec()));
        }
    }

    #[test]
    fn metadata_cloning_concatenation_and_slicing_keep_sources_lazy() {
        let source = Arc::new(CountingSource {
            bytes: b"abcdef".to_vec(),
            reads: AtomicUsize::new(0),
        });
        let data = BlobData::from_source(source.clone(), 6);
        let parts = BlobData::concat(vec![
            b"<".to_vec().into(),
            data.slice(1..5),
            b">".to_vec().into(),
        ])
        .unwrap();
        let nested = BlobData::concat(vec![
            b"[".to_vec().into(),
            parts.clone(),
            b"]".to_vec().into(),
        ])
        .unwrap();
        assert_eq!(source.reads.load(Ordering::Relaxed), 0);
        assert_eq!(parts.len(), 6);
        assert_eq!(nested.len(), 8);
        assert_eq!(parts.slice(0..1).read_blocking().unwrap(), b"<");
        assert_eq!(source.reads.load(Ordering::Relaxed), 0);
        assert_eq!(nested.slice(2..6).read_blocking().unwrap(), b"bcde");
        assert_eq!(nested.read_blocking().unwrap(), b"[<bcde>]");
        assert_eq!(source.reads.load(Ordering::Relaxed), 2);
    }

    struct DeferredSource(Mutex<Vec<(Range<usize>, BlobReadCompletion)>>);

    impl fmt::Debug for DeferredSource {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("DeferredSource")
        }
    }

    impl BlobSource for DeferredSource {
        fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
            self.0.lock().push((range, complete));
        }
    }

    #[test]
    fn asynchronous_source_completion_order_does_not_change_blob_byte_order() {
        let source = Arc::new(DeferredSource(Mutex::new(Vec::new())));
        let first = BlobData::from_source(source.clone(), 3);
        let second = BlobData::from_source(source.clone(), 3);
        let data = BlobData::concat(vec![first, b"|".to_vec().into(), second]).unwrap();
        let (send, receive) = mpsc::channel();
        data.read_all(Box::new(move |result| send.send(result).unwrap()));
        assert!(receive.try_recv().is_err());
        let (_, later) = source.0.lock().pop().unwrap();
        later(Ok(b"two".to_vec()));
        assert!(receive.try_recv().is_err());
        let (_, earlier) = source.0.lock().pop().unwrap();
        earlier(Ok(b"one".to_vec()));
        assert_eq!(receive.try_recv().unwrap().unwrap(), b"one|two");
    }

    #[test]
    fn failed_blob_reads_complete_once_and_discard_later_bytes() {
        let source = Arc::new(DeferredSource(Mutex::new(Vec::new())));
        let data = BlobData::concat(vec![
            BlobData::from_source(source.clone(), 3),
            BlobData::from_source(source.clone(), 3),
        ])
        .unwrap();
        let (send, receive) = mpsc::channel();
        data.read_all(Box::new(move |result| send.send(result).unwrap()));
        let (_, later) = source.0.lock().pop().unwrap();
        later(Err(io::Error::other("injected read failure")));
        assert!(receive.try_recv().unwrap().is_err());
        let (_, earlier) = source.0.lock().pop().unwrap();
        earlier(Ok(b"one".to_vec()));
        assert!(receive.try_recv().is_err());
    }
}
