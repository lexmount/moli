use std::{
    hash::{Hash, Hasher},
    mem::size_of,
    sync::{Arc, LazyLock, OnceLock, Weak},
};

use indexmap::IndexMap;
use moli_crypto::Sha256Context;
use parking_lot::Mutex;

use super::super::{source_key::StyleSourceKey, stylesheet::stylesheet_import_urls};
use super::store::StylesheetFontFaceDescriptor;

const RETAINED_BYTES_LIMIT: usize = 64 * 1024;

static CACHE: LazyLock<Mutex<SharedStyleSourceCache>> =
    LazyLock::new(|| Mutex::new(SharedStyleSourceCache::default()));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SharedStyleSourceCacheKey {
    css_fingerprint: [u8; 32],
    css_text_len: usize,
    base_url_fingerprint: [u8; 32],
    base_url_len: usize,
}

#[derive(Debug)]
pub(super) struct SharedStyleSourceContents {
    css_text: Arc<str>,
    base_url: Arc<url::Url>,
    css_fingerprint: [u8; 32],
    // Live owner sheets have their own parsed rule trees. Text-only callers
    // share one fallback parse for selector dependencies and import discovery.
    imports: OnceLock<Arc<[url::Url]>>,
    font_faces: OnceLock<Arc<[StylesheetFontFaceDescriptor]>>,
}

#[derive(Debug)]
struct SharedStyleSourceCache {
    entries: IndexMap<SharedStyleSourceCacheKey, Weak<SharedStyleSourceContents>>,
    retained_bytes: usize,
    retained_bytes_limit: usize,
}

pub(super) fn shared_style_source_contents(
    css_text: String,
    base_url: url::Url,
) -> Arc<SharedStyleSourceContents> {
    let key = SharedStyleSourceCacheKey::new(&css_text, &base_url);
    if let Some(cached) = CACHE.lock().lookup(&key, &css_text, &base_url) {
        return cached;
    }

    let source = Arc::new(SharedStyleSourceContents::new(
        css_text.into(),
        base_url,
        key.css_fingerprint,
    ));
    CACHE.lock().insert_or_get(key, source)
}

impl PartialEq for SharedStyleSourceContents {
    fn eq(&self, other: &Self) -> bool {
        self.matches_input(other.css_text(), other.base_url())
    }
}

impl Eq for SharedStyleSourceContents {}

impl Hash for SharedStyleSourceContents {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.css_fingerprint.hash(state);
        self.base_url.hash(state);
    }
}

impl SharedStyleSourceContents {
    fn new(css_text: Arc<str>, base_url: url::Url, css_fingerprint: [u8; 32]) -> Self {
        Self {
            css_text,
            base_url: Arc::new(base_url),
            css_fingerprint,
            imports: OnceLock::new(),
            font_faces: OnceLock::new(),
        }
    }

    pub(super) fn css_text(&self) -> &str {
        self.css_text.as_ref()
    }

    pub(super) fn css_text_handle(&self) -> Arc<str> {
        Arc::clone(&self.css_text)
    }

    pub(super) fn base_url(&self) -> &url::Url {
        self.base_url.as_ref()
    }

    pub(super) fn base_url_handle(&self) -> Arc<url::Url> {
        Arc::clone(&self.base_url)
    }

    pub(super) fn css_fingerprint(&self) -> [u8; 32] {
        self.css_fingerprint
    }

    pub(super) fn font_faces(&self) -> Arc<[StylesheetFontFaceDescriptor]> {
        Arc::clone(self.font_faces.get_or_init(|| {
            crate::css_style::parse_css_font_faces(self.css_text())
                .into_iter()
                .map(|face| StylesheetFontFaceDescriptor::new(face.family, face.source))
                .collect::<Vec<_>>()
                .into()
        }))
    }

    pub(super) fn import_urls(&self) -> &[url::Url] {
        self.import_urls_cached().as_ref()
    }

    pub(super) fn import_urls_handle(&self) -> Arc<[url::Url]> {
        Arc::clone(self.import_urls_cached())
    }

    fn matches_input(&self, css_text: &str, base_url: &url::Url) -> bool {
        self.css_text.as_ref() == css_text && self.base_url.as_ref() == base_url
    }

    fn import_urls_cached(&self) -> &Arc<[url::Url]> {
        self.imports
            .get_or_init(|| stylesheet_import_urls(self.css_text(), self.base_url()).into())
    }
}

impl SharedStyleSourceCacheKey {
    fn new(css_text: &str, base_url: &url::Url) -> Self {
        let base_url = base_url.as_str();
        Self {
            css_fingerprint: StyleSourceKey::css_fingerprint(css_text),
            css_text_len: css_text.len(),
            base_url_fingerprint: fingerprint(base_url),
            base_url_len: base_url.len(),
        }
    }
}

fn fingerprint(value: &str) -> [u8; 32] {
    let mut hasher = Sha256Context::new();
    hasher.update(value.len().to_le_bytes());
    hasher.update(value.as_bytes());
    hasher.finish()
}

impl Default for SharedStyleSourceCache {
    fn default() -> Self {
        Self::with_retained_bytes_limit(RETAINED_BYTES_LIMIT)
    }
}

impl SharedStyleSourceCache {
    fn with_retained_bytes_limit(retained_bytes_limit: usize) -> Self {
        Self {
            entries: IndexMap::new(),
            retained_bytes: 0,
            retained_bytes_limit,
        }
    }

    fn lookup(
        &mut self,
        key: &SharedStyleSourceCacheKey,
        css_text: &str,
        base_url: &url::Url,
    ) -> Option<Arc<SharedStyleSourceContents>> {
        let cached = self.remove_entry(key)?.upgrade()?;
        if !cached.matches_input(css_text, base_url) {
            return None;
        }
        self.insert_entry(*key, Arc::downgrade(&cached));
        Some(cached)
    }

    fn insert_or_get(
        &mut self,
        key: SharedStyleSourceCacheKey,
        source: Arc<SharedStyleSourceContents>,
    ) -> Arc<SharedStyleSourceContents> {
        if let Some(cached) = self.lookup(&key, source.css_text(), source.base_url()) {
            return cached;
        }
        if ENTRY_RETAINED_BYTES > self.retained_bytes_limit {
            return source;
        }
        self.insert_entry(key, Arc::downgrade(&source));
        self.evict_to_budget();
        source
    }

    fn insert_entry(
        &mut self,
        key: SharedStyleSourceCacheKey,
        source: Weak<SharedStyleSourceContents>,
    ) {
        self.retained_bytes = self.retained_bytes.saturating_add(ENTRY_RETAINED_BYTES);
        let replaced = self.entries.insert(key, source);
        debug_assert!(
            replaced.is_none(),
            "cache entry must be removed before insert"
        );
    }

    fn remove_entry(
        &mut self,
        key: &SharedStyleSourceCacheKey,
    ) -> Option<Weak<SharedStyleSourceContents>> {
        let entry = self.entries.shift_remove(key)?;
        self.retained_bytes = self.retained_bytes.saturating_sub(ENTRY_RETAINED_BYTES);
        Some(entry)
    }

    fn evict_to_budget(&mut self) {
        let previous_len = self.entries.len();
        self.entries.retain(|_, source| source.strong_count() > 0);
        let removed = previous_len.saturating_sub(self.entries.len());
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(removed.saturating_mul(ENTRY_RETAINED_BYTES));

        while self.retained_bytes > self.retained_bytes_limit {
            let Some((_, _)) = self.entries.shift_remove_index(0) else {
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(ENTRY_RETAINED_BYTES);
        }
    }
}

// Logical bytes retained solely by the fixed-size weak index. Shared CSS text
// and metadata are owner-held and therefore are not released by cache eviction.
const ENTRY_RETAINED_BYTES: usize = size_of::<SharedStyleSourceCacheKey>()
    + size_of::<Weak<SharedStyleSourceContents>>()
    + 2 * size_of::<usize>();

#[cfg(test)]
mod tests {
    use super::*;

    fn test_source(
        css_text: &str,
        base_url: &str,
    ) -> (
        SharedStyleSourceCacheKey,
        Arc<SharedStyleSourceContents>,
        url::Url,
    ) {
        let base_url = url::Url::parse(base_url).expect("valid test base URL");
        let key = SharedStyleSourceCacheKey::new(css_text, &base_url);
        let source = Arc::new(SharedStyleSourceContents::new(
            css_text.into(),
            base_url.clone(),
            key.css_fingerprint,
        ));
        (key, source, base_url)
    }

    #[test]
    fn weak_cache_does_not_keep_source_contents_alive() {
        let mut cache = SharedStyleSourceCache::with_retained_bytes_limit(ENTRY_RETAINED_BYTES);
        let css_text = ".probe { color: red; }";
        let (key, source, base_url) = test_source(css_text, "https://weak-cache.test/style.css");
        let weak_source = Arc::downgrade(&source);

        let retained = cache.insert_or_get(key, source);
        assert!(retained.imports.get().is_none());
        assert!(retained.font_faces().is_empty());
        assert!(retained.imports.get().is_none());
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.retained_bytes, ENTRY_RETAINED_BYTES);
        drop(retained);

        assert!(weak_source.upgrade().is_none());
        assert!(cache.lookup(&key, css_text, &base_url).is_none());
        assert!(cache.entries.is_empty());
        assert_eq!(cache.retained_bytes, 0);
    }

    #[test]
    fn weak_cache_evicts_oldest_index_to_byte_budget() {
        let mut cache = SharedStyleSourceCache::with_retained_bytes_limit(2 * ENTRY_RETAINED_BYTES);
        let first = test_source(".first {}", "https://byte-cache.test/first.css");
        let second = test_source(".second {}", "https://byte-cache.test/second.css");
        let third = test_source(".third {}", "https://byte-cache.test/third.css");

        let retained_sources = [
            cache.insert_or_get(first.0, Arc::clone(&first.1)),
            cache.insert_or_get(second.0, Arc::clone(&second.1)),
            cache.insert_or_get(third.0, Arc::clone(&third.1)),
        ];

        assert_eq!(retained_sources.len(), 3);
        assert!(!cache.entries.contains_key(&first.0));
        assert!(cache.entries.contains_key(&second.0));
        assert!(cache.entries.contains_key(&third.0));
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.retained_bytes, 2 * ENTRY_RETAINED_BYTES);
        assert!(cache.retained_bytes <= cache.retained_bytes_limit);
    }

    #[test]
    fn weak_cache_checks_full_input_after_fingerprint_hit() {
        let mut cache = SharedStyleSourceCache::with_retained_bytes_limit(ENTRY_RETAINED_BYTES);
        let first = test_source(".first {}", "https://collision.test/first.css");
        let second = test_source(".second {}", "https://collision.test/second.css");

        let retained_first = cache.insert_or_get(first.0, Arc::clone(&first.1));
        let retained_second = cache.insert_or_get(first.0, Arc::clone(&second.1));

        assert!(Arc::ptr_eq(&retained_first, &first.1));
        assert!(Arc::ptr_eq(&retained_second, &second.1));
        assert!(!Arc::ptr_eq(&retained_second, &first.1));
        assert!(
            cache
                .lookup(&first.0, first.1.css_text(), &first.2)
                .is_none()
        );
    }
}
