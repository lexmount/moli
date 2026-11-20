use parking_lot::Mutex;
use std::sync::Arc;

/// The mutable domain component shared by Documents with the same origin.
/// It contains no arena handles or JavaScript references and can outlive a Window.
#[derive(Debug, Clone, Default)]
pub struct DocumentDomainOverride(Arc<Mutex<Option<String>>>);

impl PartialEq for DocumentDomainOverride {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Eq for DocumentDomainOverride {}

impl DocumentDomainOverride {
    pub fn get(&self) -> Option<String> {
        self.0.lock().clone()
    }

    pub fn set(&self, domain: String) {
        *self.0.lock() = Some(domain);
    }

    pub fn shares_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// An inherited origin's domain, independent of the response URL and frame lifetime.
#[derive(Debug, Clone)]
pub struct DocumentDomain {
    host: String,
    domain_override: DocumentDomainOverride,
}

impl DocumentDomain {
    pub fn new(host: String, domain_override: DocumentDomainOverride) -> Self {
        Self {
            host,
            domain_override,
        }
    }

    pub fn value(&self) -> String {
        self.domain_override
            .get()
            .unwrap_or_else(|| self.host.clone())
    }
}
