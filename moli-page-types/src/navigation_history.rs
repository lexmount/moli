use std::sync::atomic::{AtomicU64, Ordering};
use url::Url;

static NEXT_NAVIGATION_HISTORY_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NAVIGATION_HISTORY_ENTRY_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NAVIGATION_HISTORY_ENTRY_KEY: AtomicU64 = AtomicU64::new(1);

/// Opaque identity shared by session-history entries that belong to the same
/// `Document`.
///
/// The serialized token is carried through the renderer's hidden Navigation
/// slots, but its contents have no meaning. In particular, callers must not
/// derive a new identity from a URL, a history index, or a previous token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationHistoryDocumentId(String);

impl NavigationHistoryDocumentId {
    pub fn allocate() -> Self {
        allocate_navigation_history_document_id(&NEXT_NAVIGATION_HISTORY_DOCUMENT_ID)
    }

    /// Restores an identity previously stored in a renderer-owned runtime
    /// slot. Equality remains opaque; the token is never parsed.
    pub fn from_serialized(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn allocate_navigation_history_document_id(counter: &AtomicU64) -> NavigationHistoryDocumentId {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Navigation History Document id allocator exhausted");
    NavigationHistoryDocumentId(format!("document-{raw}"))
}

/// Opaque identity for one Navigation API entry incarnation.
///
/// A replacement allocates a new id even when it retains the same session
/// history slot and therefore the same [`NavigationHistoryEntryKey`]. This is
/// deliberately independent from both the Document identity and history
/// index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationHistoryEntryId(String);

impl NavigationHistoryEntryId {
    pub fn allocate() -> Self {
        allocate_navigation_history_entry_id(&NEXT_NAVIGATION_HISTORY_ENTRY_ID)
    }

    pub fn from_serialized(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for NavigationHistoryEntryId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

fn allocate_navigation_history_entry_id(counter: &AtomicU64) -> NavigationHistoryEntryId {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Navigation History entry id allocator exhausted");
    NavigationHistoryEntryId(format!("entry-{raw}"))
}

/// Opaque identity for one session-history slot exposed to Navigation API.
///
/// Same-origin replacement retains the key; push and cross-origin
/// replacement allocate a fresh key. The token is never derived from a URL,
/// history index, or Document id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationHistoryEntryKey(String);

impl NavigationHistoryEntryKey {
    pub fn allocate() -> Self {
        allocate_navigation_history_entry_key(&NEXT_NAVIGATION_HISTORY_ENTRY_KEY)
    }

    pub fn from_serialized(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::ops::Deref for NavigationHistoryEntryKey {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

fn allocate_navigation_history_entry_key(counter: &AtomicU64) -> NavigationHistoryEntryKey {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Navigation History entry key allocator exhausted");
    NavigationHistoryEntryKey(format!("key-{raw}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationHistorySerializedEntry {
    pub url: String,
    pub history_state_json: Option<String>,
    pub navigation_state_json: Option<String>,
    pub referrer_policy: Option<String>,
    pub document_id: NavigationHistoryDocumentId,
    pub history_index: u32,
    pub index: u32,
    pub id: NavigationHistoryEntryId,
    pub key: NavigationHistoryEntryKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationHistoryEntrySeed {
    pub entries: Vec<NavigationHistorySerializedEntry>,
    pub current_index: u32,
    pub activation: Option<NavigationActivationSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationActivationSeed {
    pub entry: NavigationHistorySerializedEntry,
    pub from: Option<NavigationHistorySerializedEntry>,
    pub navigation_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTraversalSeedCandidate {
    pub current_url: Url,
    pub target_url: Url,
    pub seed: NavigationHistoryEntrySeed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationHistoryMutation {
    Push,
    Replace,
}

/// The browser-side session-history effect of a renderer-completed
/// same-document navigation.
///
/// Traversal is deliberately represented as a delta rather than another URL
/// insertion: URLs are not stable entry identities and may repeat in a
/// session-history list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameDocumentHistoryUpdate {
    Push,
    Replace,
    Traverse { delta: i64 },
}

impl NavigationHistoryMutation {
    pub const fn navigation_type(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Replace => "replace",
        }
    }
}

pub fn initial_navigation_history_seed(
    is_global_window: bool,
    href: &str,
) -> NavigationHistoryEntrySeed {
    if is_global_window && href != "about:blank" {
        let initial_document_id = NavigationHistoryDocumentId::allocate();
        let current_document_id = NavigationHistoryDocumentId::allocate();
        let entries = vec![
            navigation_history_entry(
                "about:blank",
                0,
                0,
                initial_document_id,
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
                None,
                None,
            ),
            navigation_history_entry(
                href,
                1,
                0,
                current_document_id,
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
                None,
                None,
            ),
        ];
        return NavigationHistoryEntrySeed {
            current_index: 1,
            activation: Some(NavigationActivationSeed {
                entry: entries[1].clone(),
                from: None,
                navigation_type: Some("push".to_owned()),
            }),
            entries,
        };
    }
    let entries = vec![navigation_history_entry(
        href,
        0,
        0,
        NavigationHistoryDocumentId::allocate(),
        NavigationHistoryEntryId::allocate(),
        NavigationHistoryEntryKey::allocate(),
        None,
        None,
    )];
    NavigationHistoryEntrySeed {
        current_index: 0,
        activation: (href != "about:blank").then(|| NavigationActivationSeed {
            entry: entries[0].clone(),
            from: None,
            navigation_type: Some("push".to_owned()),
        }),
        entries,
    }
}

pub fn child_browsing_context_single_entry_seed(url: Option<&Url>) -> NavigationHistoryEntrySeed {
    let url = url
        .map(|url| url.as_str().to_owned())
        .unwrap_or_else(|| "about:blank".to_owned());
    if url != "about:blank" {
        let entries = vec![
            navigation_history_entry(
                "about:blank",
                0,
                0,
                NavigationHistoryDocumentId::allocate(),
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
                None,
                None,
            ),
            navigation_history_entry(
                &url,
                1,
                0,
                NavigationHistoryDocumentId::allocate(),
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
                None,
                None,
            ),
        ];
        return NavigationHistoryEntrySeed {
            activation: Some(NavigationActivationSeed {
                entry: entries[1].clone(),
                from: None,
                navigation_type: Some("replace".to_owned()),
            }),
            entries,
            current_index: 1,
        };
    }
    let entries = vec![navigation_history_entry(
        &url,
        0,
        0,
        NavigationHistoryDocumentId::allocate(),
        NavigationHistoryEntryId::allocate(),
        NavigationHistoryEntryKey::allocate(),
        None,
        None,
    )];
    NavigationHistoryEntrySeed {
        activation: None,
        entries,
        current_index: 0,
    }
}

pub fn apply_child_browsing_context_navigation_to_entry_seed(
    seed: &mut NavigationHistoryEntrySeed,
    url: &Url,
    history_state_json: Option<String>,
    navigation_state_json: Option<String>,
) {
    let next_index = seed.current_index + 1;
    let current_navigation_index = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
        .map(|entry| entry.index)
        .unwrap_or(0);
    seed.entries
        .retain(|entry| entry.history_index <= seed.current_index);
    seed.entries.push(navigation_history_entry(
        url.as_str(),
        next_index,
        current_navigation_index + 1,
        NavigationHistoryDocumentId::allocate(),
        NavigationHistoryEntryId::allocate(),
        NavigationHistoryEntryKey::allocate(),
        history_state_json,
        navigation_state_json,
    ));
    seed.current_index = next_index;
    seed.activation = Some(NavigationActivationSeed {
        entry: seed
            .entries
            .iter()
            .find(|entry| entry.history_index == next_index)
            .cloned()
            .unwrap_or_else(|| seed.entries.last().cloned().unwrap()),
        from: visible_activation_from(
            seed.entries
                .iter()
                .find(|entry| entry.history_index == next_index.saturating_sub(1)),
            url,
        ),
        navigation_type: Some("push".to_owned()),
    });
}

fn visible_activation_from(
    previous_entry: Option<&NavigationHistorySerializedEntry>,
    destination_url: &Url,
) -> Option<NavigationHistorySerializedEntry> {
    previous_entry
        .filter(|entry| entry_is_same_origin_with_destination(entry, destination_url))
        .cloned()
}

fn entry_is_same_origin_with_destination(
    entry: &NavigationHistorySerializedEntry,
    destination_url: &Url,
) -> bool {
    entry.url == "about:blank"
        || Url::parse(&entry.url)
            .ok()
            .is_some_and(|entry_url| same_origin(&entry_url, destination_url))
}

pub fn replace_child_browsing_context_navigation_in_entry_seed(
    seed: &mut NavigationHistoryEntrySeed,
    url: &Url,
    history_state_json: Option<String>,
    navigation_state_json: Option<String>,
) {
    let current_index = seed.current_index;
    let current_navigation_index = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .map(|entry| entry.index)
        .unwrap_or(0);
    let previous_entry = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .cloned();
    let next_document_id = NavigationHistoryDocumentId::allocate();
    let next_key = replacement_entry_key(previous_entry.as_ref(), url);
    let next_entry = navigation_history_entry(
        url.as_str(),
        current_index,
        current_navigation_index,
        next_document_id,
        NavigationHistoryEntryId::allocate(),
        next_key,
        history_state_json,
        navigation_state_json,
    );
    if let Some(existing) = seed
        .entries
        .iter_mut()
        .find(|entry| entry.history_index == current_index)
    {
        *existing = next_entry.clone();
    } else {
        seed.entries.push(next_entry.clone());
    }
    seed.activation = Some(NavigationActivationSeed {
        entry: next_entry,
        from: visible_activation_from(previous_entry.as_ref(), url),
        navigation_type: Some("replace".to_owned()),
    });
}

pub fn apply_child_browsing_context_javascript_url_navigation_to_entry_seed(
    seed: &mut NavigationHistoryEntrySeed,
) {
    let current_index = seed.current_index;
    let Some(previous_entry) = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .cloned()
    else {
        return;
    };
    let next_document_id = NavigationHistoryDocumentId::allocate();
    let mut next_entry = previous_entry.clone();
    next_entry.document_id = next_document_id;
    next_entry.id = NavigationHistoryEntryId::allocate();
    if let Some(existing) = seed
        .entries
        .iter_mut()
        .find(|entry| entry.history_index == current_index)
    {
        *existing = next_entry.clone();
    }
    seed.activation = Some(NavigationActivationSeed {
        entry: next_entry,
        from: Some(previous_entry),
        navigation_type: Some("replace".to_owned()),
    });
}

pub fn cross_document_navigation_seed(
    mut entries: Vec<NavigationHistorySerializedEntry>,
    current_index: u32,
    current_navigation_index: u32,
    destination_url: &Url,
    mutation: NavigationHistoryMutation,
) -> NavigationHistoryEntrySeed {
    let current_entry_snapshot = entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .cloned();
    let (destination_entry, destination_index) = match mutation {
        NavigationHistoryMutation::Push => {
            entries.retain(|entry| entry.history_index <= current_index);
            let next_index = current_index + 1;
            let entry = navigation_history_entry(
                destination_url.as_str(),
                next_index,
                current_navigation_index + 1,
                NavigationHistoryDocumentId::allocate(),
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
                None,
                None,
            );
            entries.push(entry.clone());
            (entry, next_index)
        }
        NavigationHistoryMutation::Replace => {
            let next_document_id = NavigationHistoryDocumentId::allocate();
            let next_key = replacement_entry_key(current_entry_snapshot.as_ref(), destination_url);
            let entry = navigation_history_entry(
                destination_url.as_str(),
                current_index,
                current_navigation_index,
                next_document_id,
                NavigationHistoryEntryId::allocate(),
                next_key,
                None,
                None,
            );
            if let Some(existing) = entries
                .iter_mut()
                .find(|existing| existing.history_index == current_index)
            {
                *existing = entry.clone();
            } else {
                entries.push(entry.clone());
            }
            (entry, current_index)
        }
    };

    NavigationHistoryEntrySeed {
        entries,
        current_index: destination_index,
        activation: Some(NavigationActivationSeed {
            entry: destination_entry,
            from: visible_activation_from(current_entry_snapshot.as_ref(), destination_url),
            navigation_type: Some(mutation.navigation_type().to_owned()),
        }),
    }
}

pub fn reload_navigation_seed(
    entries: Vec<NavigationHistorySerializedEntry>,
    current_index: u32,
) -> Option<NavigationHistoryEntrySeed> {
    let current_entry = entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .cloned()?;
    Some(NavigationHistoryEntrySeed {
        entries,
        current_index,
        activation: Some(NavigationActivationSeed {
            entry: current_entry.clone(),
            from: Some(current_entry),
            navigation_type: Some("reload".to_owned()),
        }),
    })
}

pub fn traversal_navigation_seed_candidate(
    entries: Vec<NavigationHistorySerializedEntry>,
    current_index: u32,
    target_index: u32,
) -> Option<NavigationTraversalSeedCandidate> {
    let current_entry = entries
        .iter()
        .find(|entry| entry.history_index == current_index)
        .cloned()?;
    let target_entry = entries
        .iter()
        .find(|entry| entry.history_index == target_index)
        .cloned()?;
    if current_entry.document_id == target_entry.document_id {
        return None;
    }

    let current_url = Url::parse(&current_entry.url).ok()?;
    let target_url = Url::parse(&target_entry.url).ok()?;
    // Traversals only expose an activation source that belongs to the
    // destination's contiguous same-origin entry list.
    let traversed_indices = current_index.min(target_index)..=current_index.max(target_index);
    let from = entries
        .iter()
        .filter(|entry| traversed_indices.contains(&entry.history_index))
        .all(|entry| entry_is_same_origin_with_destination(entry, &target_url))
        .then_some(current_entry);
    Some(NavigationTraversalSeedCandidate {
        current_url,
        target_url: target_url.clone(),
        seed: NavigationHistoryEntrySeed {
            entries,
            current_index: target_index,
            activation: Some(NavigationActivationSeed {
                entry: target_entry,
                from,
                navigation_type: Some("traverse".to_owned()),
            }),
        },
    })
}

fn navigation_history_entry(
    url: &str,
    history_index: u32,
    index: u32,
    document_id: NavigationHistoryDocumentId,
    id: NavigationHistoryEntryId,
    key: NavigationHistoryEntryKey,
    history_state_json: Option<String>,
    navigation_state_json: Option<String>,
) -> NavigationHistorySerializedEntry {
    NavigationHistorySerializedEntry {
        url: url.to_owned(),
        history_state_json,
        navigation_state_json,
        referrer_policy: None,
        document_id,
        history_index,
        index,
        id,
        key,
    }
}

fn replacement_entry_key(
    previous_entry: Option<&NavigationHistorySerializedEntry>,
    url: &Url,
) -> NavigationHistoryEntryKey {
    if previous_entry
        .and_then(|entry| Url::parse(&entry.url).ok())
        .is_some_and(|previous_url| same_origin(&previous_url, url))
    {
        return previous_entry
            .map(|entry| entry.key.clone())
            .filter(|key| !key.is_empty())
            .unwrap_or_else(NavigationHistoryEntryKey::allocate);
    }
    NavigationHistoryEntryKey::allocate()
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.origin() == right.origin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_id(token: &str) -> NavigationHistoryDocumentId {
        NavigationHistoryDocumentId::from_serialized(token.to_owned())
    }

    fn entry_id(token: &str) -> NavigationHistoryEntryId {
        NavigationHistoryEntryId::from_serialized(token.to_owned())
    }

    fn entry_key(token: &str) -> NavigationHistoryEntryKey {
        NavigationHistoryEntryKey::from_serialized(token.to_owned())
    }

    #[test]
    fn initial_navigation_history_seed_preserves_global_about_blank_predecessor() {
        let seed = initial_navigation_history_seed(true, "https://example.test/page");
        assert_eq!(seed.current_index, 1);
        assert_eq!(seed.entries.len(), 2);
        assert_eq!(seed.entries[0].url, "about:blank");
        assert_eq!(seed.entries[1].url, "https://example.test/page");
        assert_ne!(seed.entries[0].document_id, seed.entries[1].document_id);
        assert_eq!(
            seed.activation
                .as_ref()
                .map(|activation| activation.navigation_type.as_deref()),
            Some(Some("push"))
        );

        let child_seed = initial_navigation_history_seed(false, "https://example.test/frame");
        assert_eq!(child_seed.current_index, 0);
        assert_eq!(child_seed.entries.len(), 1);
        assert_eq!(child_seed.entries[0].url, "https://example.test/frame");
    }

    #[test]
    fn initial_about_blank_navigation_seed_has_no_activation() {
        let global_seed = initial_navigation_history_seed(true, "about:blank");
        assert_eq!(global_seed.current_index, 0);
        assert_eq!(global_seed.entries.len(), 1);
        assert!(global_seed.activation.is_none());

        let child_seed = child_browsing_context_single_entry_seed(None);
        assert_eq!(child_seed.current_index, 0);
        assert_eq!(child_seed.entries.len(), 1);
        assert_eq!(child_seed.entries[0].url, "about:blank");
        assert!(child_seed.activation.is_none());
    }

    #[test]
    fn child_replace_same_url_generates_new_document_id_each_time() {
        let url = Url::parse("https://child.example/frame").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(None);

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url, None, None);
        let first = seed.entries[0].clone();

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url, None, None);
        let second = seed.entries[0].clone();

        assert_eq!(first.history_index, second.history_index);
        assert_eq!(second.index, first.index);
        assert_ne!(first.document_id, second.document_id);
        assert_ne!(first.id, second.id);
        assert_eq!(first.key, second.key);
        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.from.as_ref())
                .map(|entry| entry.document_id.as_str()),
            Some(first.document_id.as_str())
        );
    }

    #[test]
    fn cross_document_replace_same_url_allocates_a_fresh_document_id() {
        let destination = Url::parse("https://example.test/replaced").unwrap();
        let initial = vec![navigation_history_entry(
            "https://example.test/current",
            4,
            2,
            document_id("opaque-existing-document"),
            entry_id("entry-4"),
            entry_key("key-4"),
            None,
            None,
        )];

        let first = cross_document_navigation_seed(
            initial,
            4,
            2,
            &destination,
            NavigationHistoryMutation::Replace,
        );
        let first_document_id = first.entries[0].document_id.clone();
        let second = cross_document_navigation_seed(
            first.entries,
            4,
            2,
            &destination,
            NavigationHistoryMutation::Replace,
        );

        assert_ne!(first_document_id, second.entries[0].document_id);
        assert!(
            !second.entries[0]
                .document_id
                .as_str()
                .contains(destination.as_str())
        );
    }

    #[test]
    fn document_id_allocator_rejects_exhaustion_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX);
        let exhausted =
            std::panic::catch_unwind(|| allocate_navigation_history_document_id(&counter));

        assert!(exhausted.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn entry_identity_allocators_reject_exhaustion_without_wrapping() {
        let id_counter = AtomicU64::new(u64::MAX);
        let id_exhausted =
            std::panic::catch_unwind(|| allocate_navigation_history_entry_id(&id_counter));
        assert!(id_exhausted.is_err());
        assert_eq!(id_counter.load(Ordering::Relaxed), u64::MAX);

        let key_counter = AtomicU64::new(u64::MAX);
        let key_exhausted =
            std::panic::catch_unwind(|| allocate_navigation_history_entry_key(&key_counter));
        assert!(key_exhausted.is_err());
        assert_eq!(key_counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn push_after_back_allocates_fresh_identity_for_reused_history_index() {
        let first = Url::parse("https://example.test/first").unwrap();
        let second = Url::parse("https://example.test/second").unwrap();
        let replacement = Url::parse("https://example.test/replacement").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(None);
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &first, None, None);
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &second, None, None);
        let retired_forward_entry = seed.entries[2].clone();

        seed.current_index = 1;
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &replacement, None, None);

        let replacement_entry = &seed.entries[2];
        assert_eq!(
            replacement_entry.history_index,
            retired_forward_entry.history_index
        );
        assert_ne!(replacement_entry.id, retired_forward_entry.id);
        assert_ne!(replacement_entry.key, retired_forward_entry.key);
        assert_ne!(
            replacement_entry.id.as_str(),
            replacement_entry.document_id.as_str(),
            "entry incarnation identity must not be projected from Document identity"
        );
    }

    #[test]
    fn cross_origin_replace_allocates_a_fresh_entry_key() {
        let first = Url::parse("https://first.example/page").unwrap();
        let second = Url::parse("https://second.example/page").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(Some(&first));
        let previous = seed.entries[1].clone();

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &second, None, None);

        assert_eq!(seed.entries[1].history_index, previous.history_index);
        assert_ne!(seed.entries[1].id, previous.id);
        assert_ne!(seed.entries[1].key, previous.key);
    }

    #[test]
    fn child_activation_omits_from_for_cross_origin_navigation() {
        let first = Url::parse("http://127.0.0.1:1111/common/blank.html").unwrap();
        let second = Url::parse("http://127.0.0.1:2222/common/blank.html").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(Some(&first));

        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &second, None, None);

        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.navigation_type.as_deref()),
            Some("push")
        );
        assert!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.from.as_ref())
                .is_none()
        );

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &first, None, None);

        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.navigation_type.as_deref()),
            Some("replace")
        );
        assert!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.from.as_ref())
                .is_none()
        );
    }

    #[test]
    fn child_activation_keeps_initial_about_blank_from_entry() {
        let url = Url::parse("http://127.0.0.1:1111/common/blank.html").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(None);

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url, None, None);

        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.from.as_ref())
                .map(|entry| entry.url.as_str()),
            Some("about:blank")
        );
    }

    #[test]
    fn child_javascript_url_navigation_preserves_current_entry_url_and_key() {
        let url = Url::parse("http://127.0.0.1:1111/common/blank.html?1").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(None);
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &url, None, None);
        let before = seed.entries[1].clone();

        apply_child_browsing_context_javascript_url_navigation_to_entry_seed(&mut seed);

        assert_eq!(seed.entries.len(), 2);
        assert_eq!(seed.current_index, 1);
        assert_eq!(seed.entries[1].url, before.url);
        assert_eq!(seed.entries[1].key, before.key);
        assert_ne!(seed.entries[1].id, before.id);
        assert_ne!(seed.entries[1].document_id, before.document_id);
    }

    #[test]
    fn cross_document_navigation_seed_push_truncates_forward_history() {
        let destination = Url::parse("https://example.test/next").unwrap();
        let entries = vec![
            navigation_history_entry(
                "https://example.test/first",
                0,
                0,
                document_id("document-0"),
                entry_id("entry-0"),
                entry_key("key-0"),
                None,
                None,
            ),
            navigation_history_entry(
                "https://example.test/current",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
                None,
                None,
            ),
            navigation_history_entry(
                "https://example.test/forward",
                2,
                2,
                document_id("document-2"),
                entry_id("entry-2"),
                entry_key("key-2"),
                None,
                None,
            ),
        ];

        let seed = cross_document_navigation_seed(
            entries,
            1,
            1,
            &destination,
            NavigationHistoryMutation::Push,
        );

        assert_eq!(seed.current_index, 2);
        assert_eq!(seed.entries.len(), 3);
        assert_eq!(seed.entries[2].url, "https://example.test/next");
        assert_eq!(seed.entries[2].index, 2);
        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.from.as_ref())
                .map(|entry| entry.url.as_str()),
            Some("https://example.test/current")
        );
    }

    #[test]
    fn cross_document_navigation_seed_replace_preserves_history_index() {
        let destination = Url::parse("https://example.test/replaced").unwrap();
        let entries = vec![navigation_history_entry(
            "https://example.test/current",
            4,
            2,
            document_id("document-4"),
            entry_id("entry-4"),
            entry_key("key-4"),
            None,
            None,
        )];

        let seed = cross_document_navigation_seed(
            entries,
            4,
            2,
            &destination,
            NavigationHistoryMutation::Replace,
        );

        assert_eq!(seed.current_index, 4);
        assert_eq!(seed.entries.len(), 1);
        assert_eq!(seed.entries[0].history_index, 4);
        assert_eq!(seed.entries[0].index, 2);
        assert_eq!(seed.entries[0].url, "https://example.test/replaced");
        assert_eq!(
            seed.activation
                .as_ref()
                .and_then(|activation| activation.navigation_type.as_deref()),
            Some("replace")
        );
    }

    #[test]
    fn cross_document_navigation_seed_replace_preserves_forward_history() {
        let entries = (0..4)
            .map(|index| {
                let mut entry = navigation_history_entry(
                    &format!("https://example.test/page-{index}"),
                    index,
                    index,
                    NavigationHistoryDocumentId::allocate(),
                    NavigationHistoryEntryId::allocate(),
                    NavigationHistoryEntryKey::allocate(),
                    Some(format!("{{\"classic\":{index}}}")),
                    Some(format!("{{\"navigation\":{index}}}")),
                );
                entry.referrer_policy = Some("no-referrer".to_owned());
                entry
            })
            .collect::<Vec<_>>();
        for destination in [
            "https://example.test/replaced",
            "https://other.test/replaced",
        ] {
            let seed = cross_document_navigation_seed(
                entries.clone(),
                1,
                1,
                &Url::parse(destination).unwrap(),
                NavigationHistoryMutation::Replace,
            );
            assert_eq!(seed.current_index, 1);
            assert_eq!(seed.entries.len(), entries.len());
            assert_eq!(seed.entries[0], entries[0]);
            assert_eq!(seed.entries[2..], entries[2..]);
            assert_eq!(seed.entries[1].url, destination);
            assert_ne!(seed.entries[1].document_id, entries[1].document_id);
            assert_ne!(seed.entries[1].id, entries[1].id);
            assert_eq!(seed.activation.as_ref().unwrap().entry, seed.entries[1]);
        }
    }

    #[test]
    fn reload_navigation_seed_activates_current_entry_from_itself() {
        let entries = vec![navigation_history_entry(
            "https://example.test/current",
            7,
            3,
            document_id("document-7"),
            entry_id("entry-7"),
            entry_key("key-7"),
            None,
            None,
        )];

        let seed = reload_navigation_seed(entries, 7).expect("reload seed");
        let activation = seed.activation.as_ref().expect("activation");
        assert_eq!(seed.current_index, 7);
        assert_eq!(activation.entry.url, "https://example.test/current");
        assert_eq!(
            activation.from.as_ref().map(|entry| entry.url.as_str()),
            Some("https://example.test/current")
        );
        assert_eq!(activation.navigation_type.as_deref(), Some("reload"));
    }

    #[test]
    fn traversal_navigation_seed_candidate_rejects_same_document_id() {
        let entries = vec![
            navigation_history_entry(
                "https://example.test/current",
                0,
                0,
                document_id("document-1"),
                entry_id("entry-0"),
                entry_key("key-0"),
                None,
                None,
            ),
            navigation_history_entry(
                "https://example.test/target",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
                None,
                None,
            ),
        ];

        assert!(traversal_navigation_seed_candidate(entries, 0, 1).is_none());
    }

    #[test]
    fn traversal_navigation_seed_candidate_builds_traverse_activation() {
        let entries = vec![
            navigation_history_entry(
                "https://example.test/current",
                0,
                0,
                document_id("document-0"),
                entry_id("entry-0"),
                entry_key("key-0"),
                None,
                None,
            ),
            navigation_history_entry(
                "https://example.test/target",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
                None,
                None,
            ),
        ];

        let candidate =
            traversal_navigation_seed_candidate(entries, 0, 1).expect("traversal candidate");
        assert_eq!(
            candidate.current_url.as_str(),
            "https://example.test/current"
        );
        assert_eq!(candidate.target_url.as_str(), "https://example.test/target");
        assert_eq!(candidate.seed.current_index, 1);
        assert_eq!(
            candidate
                .seed
                .activation
                .as_ref()
                .and_then(|activation| activation.navigation_type.as_deref()),
            Some("traverse")
        );
    }

    #[test]
    fn traversal_activation_from_requires_a_contiguous_same_origin_region() {
        for (url, middle_url, visible) in [
            (
                "https://example.test/page",
                "https://example.test:443/middle",
                true,
            ),
            (
                "https://example.test/page",
                "http://example.test/middle",
                false,
            ),
            (
                "https://example.test/page",
                "https://other.test/middle",
                false,
            ),
            ("http://127.0.0.1/page", "http://127.0.0.1:80/middle", true),
            ("http://127.0.0.1/page", "http://127.0.0.2/middle", false),
            (
                "http://127.0.0.1/page",
                "http://127.0.0.1:8080/middle",
                false,
            ),
            ("http://[::1]/page", "http://[::2]/middle", false),
            (
                "https://example.test/page",
                "blob:https://example.test/fixture",
                true,
            ),
            (
                "https://example.test/page",
                "blob:https://other.test/fixture",
                false,
            ),
            ("https://example.test/page", "data:text/html,fixture", false),
        ] {
            let entries: Vec<_> = [url, middle_url, url]
                .into_iter()
                .enumerate()
                .map(|(index, url)| {
                    navigation_history_entry(
                        url,
                        index as u32,
                        index as u32,
                        NavigationHistoryDocumentId::allocate(),
                        NavigationHistoryEntryId::allocate(),
                        NavigationHistoryEntryKey::allocate(),
                        None,
                        None,
                    )
                })
                .collect();
            for (current, target) in [(0, 2), (2, 0)] {
                let candidate =
                    traversal_navigation_seed_candidate(entries.clone(), current, target)
                        .expect("distinct Documents with identical URLs require a traversal");
                let activation = candidate.seed.activation.unwrap();
                assert_eq!(activation.entry, entries[target as usize]);
                assert_eq!(
                    activation.from.as_ref(),
                    visible.then_some(&entries[current as usize]),
                    "{url} via {middle_url}, from {current} to {target}"
                );
            }
        }
    }
}
