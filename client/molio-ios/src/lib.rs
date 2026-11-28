//! C ABI for the arm64 iPhoneOS application.

#![cfg(any(test, all(target_os = "ios", target_arch = "aarch64", target_abi = "")))]

#[cfg(feature = "browser")]
#[path = "../../molio/src/address_target.rs"]
mod address_target;
#[cfg(feature = "browser")]
mod browser;
#[cfg(feature = "browser")]
#[path = "../../molio/src/downloads.rs"]
mod downloads;
#[cfg(all(
    feature = "smoke",
    target_os = "ios",
    target_arch = "aarch64",
    target_abi = ""
))]
mod smoke;
#[cfg(feature = "browser")]
#[path = "../../molio/src/viewport_edit_sequence.rs"]
mod viewport_edit_sequence;
