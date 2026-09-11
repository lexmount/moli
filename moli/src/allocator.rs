//! Process-wide allocator configuration for the `moli` executable.
//!
//! Pointer-compressed rusty_v8 archives are linked without their allocator
//! shim, so pointer encoding and process allocator policy remain independent.
//! The low-resource jemalloc profile below favors returning short-lived page
//! allocations to the operating system over maximizing allocator throughput.
//! Keep this configuration beside the allocator declaration: changing either
//! side requires another deterministic DOM and full Spider Bench A/B.
//!
//! `override_allocator_on_supported_platforms` makes jemalloc export the
//! unprefixed C allocation symbols on Linux. This distinction is important:
//! `#[global_allocator]` alone only routes Rust allocations through jemalloc,
//! while V8 and the other native libraries continue to call libc. Exporting
//! `malloc`, `free`, and their companion symbols gives the whole executable
//! one allocation domain and matches the process-wide `LD_PRELOAD` experiment.

use core::ffi::c_char;

// The sys crate must be visible to rustc so its `#[used]` symbol references
// force the unprefixed allocator objects out of the static archive. Merely
// enabling the dependency feature is not sufficient for all linkers.
use tikv_jemalloc_sys as _;

#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

const MALLOC_CONF: &[u8] =
    b"abort_conf:true,narenas:1,tcache_max:1024,dirty_decay_ms:250,muzzy_decay_ms:0\0";

union MallocConfPointer {
    bytes: &'static u8,
    c_char: &'static c_char,
}

// jemalloc reads this exported pointer before `main`, so configuring it after
// startup would be too late for allocator metadata and early runtime objects.
// Linux uses the unprefixed symbol because the feature overrides the C
// allocator process-wide. Platforms on which tikv-jemalloc deliberately keeps
// a private namespace must use its `_rjem_` configuration symbol instead.
#[allow(non_upper_case_globals)]
#[cfg_attr(
    any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly"
    ),
    unsafe(export_name = "_rjem_malloc_conf")
)]
#[cfg_attr(
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly"
    )),
    unsafe(no_mangle)
)]
pub static malloc_conf: Option<&'static c_char> = Some(unsafe {
    MallocConfPointer {
        bytes: &MALLOC_CONF[0],
    }
    .c_char
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malloc_conf_is_nul_terminated_low_resource_profile() {
        assert_eq!(MALLOC_CONF.last(), Some(&0));
        assert!(MALLOC_CONF.starts_with(b"abort_conf:true,narenas:1,"));
        assert!(
            !MALLOC_CONF
                .windows(17)
                .any(|part| part == b"background_thread")
        );
        let decay = b"dirty_decay_ms:250";
        assert!(MALLOC_CONF.windows(decay.len()).any(|part| part == decay));
    }

    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly"
    )))]
    #[test]
    fn libc_malloc_symbol_is_overridden_process_wide() {
        unsafe extern "C" {
            fn malloc(size: usize) -> *mut core::ffi::c_void;
        }

        assert!(
            std::ptr::fn_addr_eq(
                malloc as unsafe extern "C" fn(usize) -> *mut core::ffi::c_void,
                tikv_jemalloc_sys::malloc as unsafe extern "C" fn(usize) -> *mut core::ffi::c_void
            ),
            "V8's archive allocator shim must not override Moli's process allocator"
        );
    }
}
