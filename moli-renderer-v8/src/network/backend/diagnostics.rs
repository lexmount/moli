use super::SharedMemoryResourceCacheDiagnostics;
use crate::network::loads::DetachedKeepaliveLoadDiagnostics;

/// Snapshot of the browser-context scoped resource disk pool.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserResourceDiskPoolDiagnostics {
    pub may_write: bool,
    pub disk_footprint_bytes: u64,
    pub free_bytes: usize,
    pub free_chunk_count: usize,
}

impl From<moli_disk_pool::DiskPoolDiagnostics> for BrowserResourceDiskPoolDiagnostics {
    fn from(diagnostics: moli_disk_pool::DiskPoolDiagnostics) -> Self {
        Self {
            may_write: diagnostics.may_write,
            disk_footprint_bytes: diagnostics.disk_footprint_bytes,
            free_bytes: diagnostics.free_bytes,
            free_chunk_count: diagnostics.free_chunk_count,
        }
    }
}

/// Snapshot of encoded image bytes managed by the browser-runtime image pool.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserParkableImageDiagnostics {
    pub image_count: usize,
    pub mutable_count: usize,
    pub resident_count: usize,
    pub parking_count: usize,
    pub resident_with_disk_backup_count: usize,
    pub parked_count: usize,
    pub retained_memory_bytes: usize,
    pub retained_disk_bytes: usize,
}

impl From<moli_parkable_image::ParkableImageManagerDiagnostics>
    for BrowserParkableImageDiagnostics
{
    fn from(diagnostics: moli_parkable_image::ParkableImageManagerDiagnostics) -> Self {
        Self {
            image_count: diagnostics.image_count,
            mutable_count: diagnostics.mutable_count,
            resident_count: diagnostics.resident_count,
            parking_count: diagnostics.parking_count,
            resident_with_disk_backup_count: diagnostics.resident_with_disk_backup_count,
            parked_count: diagnostics.parked_count,
            retained_memory_bytes: diagnostics.retained_memory_bytes,
            retained_disk_bytes: diagnostics.retained_disk_bytes,
        }
    }
}

/// Snapshot of one browser-context scoped renderer resource runtime.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserResourceRuntimeDiagnostics {
    pub runtime_id: u64,
    pub memory_cache: SharedMemoryResourceCacheDiagnostics,
    pub disk_pool: Option<BrowserResourceDiskPoolDiagnostics>,
    pub parkable_images: BrowserParkableImageDiagnostics,
    pub(crate) detached_keepalive_loads: DetachedKeepaliveLoadDiagnostics,
}
