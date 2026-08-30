//! Registrable-site, schemeful-site, and SameSite URL comparison helpers.

mod public_suffix;
mod same_site;
mod schemeful;

pub use public_suffix::{
    StaticPublicSuffixList, host_is_public_suffix, public_suffix_list, registrable_site_host,
    same_site_hosts, site_key_for_host,
};
pub use same_site::same_site_urls;
pub use schemeful::schemeful_site_for_url;

#[cfg(test)]
mod tests;
