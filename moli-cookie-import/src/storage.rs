use std::collections::BTreeMap;

// Browser sources merge by UTF-16 key without lossy Unicode conversion.
// Values keep owned UTF-8 strings when possible, including Firefox's SQL values.
pub(crate) type LocalStorage = BTreeMap<String, BTreeMap<Vec<u16>, DomString>>;

#[derive(Debug)]
pub(crate) enum DomString {
    Text(String),
    Utf16 { utf16: Vec<u16> },
}

impl From<String> for DomString {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Vec<u16>> for DomString {
    fn from(units: Vec<u16>) -> Self {
        match String::from_utf16(&units) {
            Ok(text) => Self::Text(text),
            Err(_) => Self::Utf16 { utf16: units },
        }
    }
}

impl DomString {
    pub(crate) fn into_units(self) -> Vec<u16> {
        match self {
            Self::Text(text) => text.encode_utf16().collect(),
            Self::Utf16 { utf16 } => utf16,
        }
    }
}

pub(crate) fn merge(target: &mut LocalStorage, source: LocalStorage) {
    for (origin, entries) in source {
        if !entries.is_empty() {
            target.entry(origin).or_default().extend(entries);
        }
    }
}

pub(crate) fn into_sqlite_areas(
    imported: LocalStorage,
) -> BTreeMap<String, Vec<(Vec<u16>, Vec<u16>)>> {
    imported
        .into_iter()
        .map(|(origin, entries)| {
            let key = moli_web_storage::web_storage_partitioned_area_key(&origin, &origin);
            let entries = entries
                .into_iter()
                .map(|(key, value)| (key, value.into_units()))
                .collect();
            (key, entries)
        })
        .collect()
}
