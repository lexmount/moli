//! Independent renderer DevTools command ingress paths.

pub(crate) mod io;
#[cfg(feature = "devtools")]
pub(crate) mod lane;
#[cfg(feature = "devtools")]
pub(crate) mod main;
