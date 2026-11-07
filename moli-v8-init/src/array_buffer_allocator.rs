/// Create an allocator for Window and Worker ArrayBuffer backing stores.
///
/// The pointer-compressed V8 archives on these platforms interpose libc
/// allocation with PartitionAlloc. Its direct-map size limit is smaller than
/// V8's maximum ArrayBuffer length, and exceeding it aborts the process rather
/// than returning an allocation failure. Use demand-zero anonymous mappings
/// for large buffers, retaining libc allocation for small buffers.
pub fn new_array_buffer_allocator() -> v8::UniqueRef<v8::Allocator> {
    #[cfg(any(
        all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    {
        partition_alloc::new_allocator()
    }
    #[cfg(not(any(
        all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )))]
    {
        v8::new_default_allocator()
    }
}

#[cfg(any(
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
mod partition_alloc {
    use std::ffi::c_void;

    // This selects an allocation strategy, not a maximum supported length.
    // Keep comfortably below PartitionAlloc's private direct-map size limit.
    const MAPPING_THRESHOLD: usize = 1 << 30;
    static HANDLE: () = ();
    static VTABLE: v8::RustAllocatorVtable<()> = v8::RustAllocatorVtable {
        allocate,
        allocate_uninitialized,
        free,
        drop: drop_handle,
    };

    pub(super) fn new_allocator() -> v8::UniqueRef<v8::Allocator> {
        // SAFETY: Both the stateless handle and its matching vtable live for
        // the process lifetime. Every callback is thread-safe and avoids V8.
        unsafe { v8::new_rust_allocator(&HANDLE, &VTABLE) }
    }

    fn map(length: usize) -> *mut c_void {
        // SAFETY: A private anonymous mapping has no file or borrowed memory.
        // The kernel zeroes its pages and validates the requested length.
        let data = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if data == libc::MAP_FAILED {
            std::ptr::null_mut()
        } else {
            data
        }
    }

    unsafe extern "C" fn allocate(_: &(), length: usize) -> *mut c_void {
        if length >= MAPPING_THRESHOLD {
            map(length)
        } else {
            // SAFETY: calloc returns zeroed storage or null on failure.
            unsafe { libc::calloc(1, length) }
        }
    }

    unsafe extern "C" fn allocate_uninitialized(_: &(), length: usize) -> *mut c_void {
        if length >= MAPPING_THRESHOLD {
            map(length)
        } else {
            // SAFETY: malloc returns owned storage or null on failure.
            unsafe { libc::malloc(length) }
        }
    }

    unsafe extern "C" fn free(_: &(), data: *mut c_void, length: usize) {
        if data.is_null() {
            return;
        }
        // V8 supplies the original allocation length, so it also identifies
        // the allocation strategy without a shared allocation registry.
        if length >= MAPPING_THRESHOLD {
            // SAFETY: data and length identify the mapping returned by map.
            unsafe { libc::munmap(data, length) };
        } else {
            // SAFETY: data came from calloc/malloc and is freed only once.
            unsafe { libc::free(data) };
        }
    }

    unsafe extern "C" fn drop_handle(_: *const ()) {
        // HANDLE is static and owns no resources.
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn large_array_buffer_backing_store_outlives_isolate_and_moves_between_threads() {
            crate::ensure_v8_initialized(|| v8::new_default_platform(0, false).make_shared());
            let length = (1_usize << 31) + 17;
            let backing_store = {
                let mut isolate = v8::Isolate::new(
                    v8::CreateParams::default()
                        .array_buffer_allocator(crate::new_array_buffer_allocator()),
                );
                let backing_store =
                    v8::ArrayBuffer::new_backing_store(&mut isolate, length).make_shared();
                for offset in [0, 4095, 1 << 30, length - 1] {
                    assert_eq!(backing_store[offset].get(), 0);
                    backing_store[offset].set(63);
                }
                backing_store
            };

            // The source isolate has been disposed. The shared allocator must
            // remain alive until the last backing-store owner releases it.
            std::thread::spawn(move || {
                assert_eq!(backing_store.byte_length(), length);
                for offset in [0, 4095, 1 << 30, length - 1] {
                    assert_eq!(backing_store[offset].get(), 63);
                }
                drop(backing_store);
            })
            .join()
            .expect("backing store can be released on another thread");
        }
    }
}
