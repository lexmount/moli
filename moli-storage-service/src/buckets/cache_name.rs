use std::fmt;

/// CacheStorage names are DOMStrings. Identity and persistence retain UTF-16
/// code units, including unpaired surrogates, rather than a lossy UTF-8 view.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StorageBucketCacheName(Vec<u16>);

impl StorageBucketCacheName {
    pub fn from_utf16(units: Vec<u16>) -> Self {
        Self(units)
    }

    pub fn as_utf16(&self) -> &[u16] {
        &self.0
    }

    pub(super) fn file_component(&self) -> String {
        if let Ok(text) = String::from_utf16(&self.0) {
            return super::encode_storage_bucket_cache_component(&text);
        }
        // The legacy codec percent-escapes '~', so this prefix cannot alias
        // any legacy UTF-8 name. Well-formed names retain their existing paths.
        let mut component = String::from("~utf16-");
        for unit in &self.0 {
            use std::fmt::Write;
            write!(component, "{unit:04x}").expect("writing to a String cannot fail");
        }
        component
    }

    pub(super) fn from_file_component(component: &str) -> anyhow::Result<Self> {
        let Some(hex) = component.strip_prefix("~utf16-") else {
            return super::decode_storage_bucket_cache_component(component).map(Self::from);
        };
        anyhow::ensure!(
            hex.is_ascii() && hex.len().is_multiple_of(4),
            "invalid UTF-16 cache name path"
        );
        let units = hex
            .as_bytes()
            .chunks_exact(4)
            .map(|chunk| {
                u16::from_str_radix(std::str::from_utf8(chunk).expect("ASCII checked"), 16)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(units))
    }
}

impl From<&str> for StorageBucketCacheName {
    fn from(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }
}

impl From<String> for StorageBucketCacheName {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&String> for StorageBucketCacheName {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&StorageBucketCacheName> for StorageBucketCacheName {
    fn from(value: &StorageBucketCacheName) -> Self {
        value.clone()
    }
}

impl fmt::Display for StorageBucketCacheName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Diagnostics only. This projection never participates in identity.
        formatter.write_str(&String::from_utf16_lossy(&self.0))
    }
}
