//! Cookie cache and import-file formats for Moli.
//!
//! This crate owns disk formats that convert to and from the browser-facing
//! `StoredCookie` model. Cookie policy and mutation behavior stay in
//! `moli-cookie-jar`.

mod atomic_file;
mod cache;
mod netscape;

pub use cache::{load_cookie_cache, save_cookie_cache};
pub use netscape::{CookieFileSaveReport, load_cookie_file, save_cookie_file};

#[cfg(test)]
mod tests;
