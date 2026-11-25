use moli_crypto::Sha256Context;

/// Revision key for one installed stylesheet source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct StyleSourceKey {
    pub(super) fingerprint: [u8; 32],
}

impl StyleSourceKey {
    pub(super) fn css_fingerprint(css_text: &str) -> [u8; 32] {
        let mut hasher = Sha256Context::new();
        let css_text = css_text.as_bytes();
        hasher.update(css_text.len().to_le_bytes());
        hasher.update(css_text);
        hasher.finish()
    }

    pub(super) fn from_css_fingerprint(css_fingerprint: [u8; 32], base_url: &url::Url) -> Self {
        let mut hasher = Sha256Context::new();
        hasher.update(css_fingerprint);
        let mut base_url = base_url.clone();
        base_url.set_fragment(None);
        let base_url = base_url.as_str().as_bytes();
        hasher.update(base_url.len().to_le_bytes());
        hasher.update(base_url);
        Self {
            fingerprint: hasher.finish(),
        }
    }

    pub(super) fn from_live_stylesheet(
        stylesheet_id: crate::live_stylesheet::StylesheetId,
        cascade_generation: u64,
    ) -> Self {
        let mut hasher = Sha256Context::new();
        hasher.update(b"live-stylesheet");
        hasher.update(stylesheet_id.get().to_le_bytes());
        hasher.update(cascade_generation.to_le_bytes());
        Self {
            fingerprint: hasher.finish(),
        }
    }
}
