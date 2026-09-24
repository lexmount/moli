/// Opaque identifier for one SharedWorker client connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SharedWorkerClientId(u64);

impl SharedWorkerClientId {
    /// Build a client id from the registry allocator.
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }

    /// Return the numeric id for renderer-side private slots and diagnostics.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Opaque identifier for one SharedWorker instance slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SharedWorkerInstanceId(u64);

impl SharedWorkerInstanceId {
    /// Build an instance id from the registry allocator.
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }

    /// Rebuild an instance id from renderer/CDP state that only carries the
    /// diagnostic numeric form.
    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Return the numeric id for diagnostics.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}
