//! C ABI for the arm64 iPhoneOS application.

#![cfg(all(target_os = "ios", target_arch = "aarch64", target_abi = ""))]

#[cfg(feature = "browser")]
#[path = "../../src/address_target.rs"]
mod address_target;
#[cfg(feature = "browser")]
mod browser;
#[cfg(feature = "browser")]
#[path = "../../src/downloads.rs"]
mod downloads;
#[cfg(feature = "smoke")]
mod smoke;
