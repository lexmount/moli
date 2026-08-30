use psl_types::{List as Psl, Suffix};

/// Public-suffix lookup needed by cookie Domain-attribute validation.
///
/// This object-safe boundary lets browser embedders provide either a static
/// generated table or another implementation of [`psl_types::List`].
pub trait CookiePublicSuffixList: std::fmt::Debug + Send + Sync {
    /// Returns whether `domain` exactly matches an explicitly listed suffix.
    fn is_public_suffix(&self, domain: &[u8]) -> bool;
}

impl<T> CookiePublicSuffixList for T
where
    T: Psl + std::fmt::Debug + Send + Sync,
{
    fn is_public_suffix(&self, domain: &[u8]) -> bool {
        self.suffix(domain)
            // The implicit wildcard must not reject arbitrary top-level names.
            .filter(Suffix::is_known)
            .is_some_and(|suffix| suffix == domain)
    }
}
