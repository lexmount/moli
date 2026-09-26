use moli_session_history::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};
use std::sync::atomic::{AtomicU64, Ordering};
use url::Url;

static NEXT_NAVIGATION_HISTORY_ENTRY_ID: AtomicU64 = AtomicU64::new(1);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationHistorySerializedEntry {
    pub url: String,
    /// Authoritative structured state, preserved across renderer replacement.
    pub history_state: Option<moli_history::SerializedScriptValue>,
    pub navigation_state: Option<moli_history::SerializedScriptValue>,
    pub scroll_restoration: moli_history::ScrollRestoration,
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
    pub session_history: crate::SessionHistorySeed,
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

/// The browser-side effect of a committed traversable mutation, including
/// navigations in child Documents.
///
/// Traversal is deliberately represented as a delta rather than another URL
/// insertion: URLs are not stable entry identities and may repeat in a
/// session-history list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHistoryUpdateKind {
    Push,
    Replace,
    Traverse { delta: i64 },
    PruneAllButCurrent,
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
            ),
            navigation_history_entry(
                href,
                1,
                0,
                current_document_id,
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
            ),
        ];
        return NavigationHistoryEntrySeed {
            session_history: Default::default(),
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
    )];
    NavigationHistoryEntrySeed {
        session_history: Default::default(),
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
            ),
            navigation_history_entry(
                &url,
                1,
                0,
                NavigationHistoryDocumentId::allocate(),
                NavigationHistoryEntryId::allocate(),
                NavigationHistoryEntryKey::allocate(),
            ),
        ];
        return NavigationHistoryEntrySeed {
            session_history: Default::default(),
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
    )];
    NavigationHistoryEntrySeed {
        session_history: Default::default(),
        activation: None,
        entries,
        current_index: 0,
    }
}

pub fn apply_child_browsing_context_navigation_to_entry_seed(
    seed: &mut NavigationHistoryEntrySeed,
    url: &Url,
) {
    seed.session_history = crate::SessionHistorySeed {
        commit: crate::SessionHistoryCommit::Push,
        ..Default::default()
    };
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
        .filter(|entry| {
            entry.url == "about:blank"
                || Url::parse(&entry.url)
                    .ok()
                    .is_some_and(|previous_url| same_origin(&previous_url, destination_url))
        })
        .cloned()
}

pub fn replace_child_browsing_context_navigation_in_entry_seed(
    seed: &mut NavigationHistoryEntrySeed,
    url: &Url,
) {
    seed.session_history = crate::SessionHistorySeed {
        commit: crate::SessionHistoryCommit::Replace,
        ..Default::default()
    };
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
    seed.session_history = crate::SessionHistorySeed {
        commit: crate::SessionHistoryCommit::Replace,
        ..Default::default()
    };
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
        session_history: crate::SessionHistorySeed {
            commit: match mutation {
                NavigationHistoryMutation::Push => crate::SessionHistoryCommit::Push,
                NavigationHistoryMutation::Replace => crate::SessionHistoryCommit::Replace,
            },
            ..Default::default()
        },
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
        session_history: crate::SessionHistorySeed {
            commit: crate::SessionHistoryCommit::Replace,
            ..Default::default()
        },
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
    Some(NavigationTraversalSeedCandidate {
        current_url,
        target_url: target_url.clone(),
        seed: NavigationHistoryEntrySeed {
            session_history: crate::SessionHistorySeed {
                commit: crate::SessionHistoryCommit::Traverse,
                ..Default::default()
            },
            entries,
            current_index: target_index,
            activation: Some(NavigationActivationSeed {
                entry: target_entry,
                from: visible_activation_from(Some(&current_entry), &target_url),
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
) -> NavigationHistorySerializedEntry {
    NavigationHistorySerializedEntry {
        url: url.to_owned(),
        history_state: None,
        navigation_state: None,
        scroll_restoration: Default::default(),
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
    left.scheme() == right.scheme()
        && left.domain() == right.domain()
        && left.port_or_known_default() == right.port_or_known_default()
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

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url);
        let first = seed.entries[0].clone();

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url);
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
    fn entry_identity_allocators_reject_exhaustion_without_wrapping() {
        let id_counter = AtomicU64::new(u64::MAX);
        let id_exhausted =
            std::panic::catch_unwind(|| allocate_navigation_history_entry_id(&id_counter));
        assert!(id_exhausted.is_err());
        assert_eq!(id_counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn push_after_back_allocates_fresh_identity_for_reused_history_index() {
        let first = Url::parse("https://example.test/first").unwrap();
        let second = Url::parse("https://example.test/second").unwrap();
        let replacement = Url::parse("https://example.test/replacement").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(None);
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &first);
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &second);
        let retired_forward_entry = seed.entries[2].clone();

        seed.current_index = 1;
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &replacement);

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

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &second);

        assert_eq!(seed.entries[1].history_index, previous.history_index);
        assert_ne!(seed.entries[1].id, previous.id);
        assert_ne!(seed.entries[1].key, previous.key);
    }

    #[test]
    fn child_activation_omits_from_for_cross_origin_navigation() {
        let first = Url::parse("http://127.0.0.1:1111/common/blank.html").unwrap();
        let second = Url::parse("http://127.0.0.1:2222/common/blank.html").unwrap();
        let mut seed = child_browsing_context_single_entry_seed(Some(&first));

        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &second);

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

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &first);

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

        replace_child_browsing_context_navigation_in_entry_seed(&mut seed, &url);

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
        apply_child_browsing_context_navigation_to_entry_seed(&mut seed, &url);
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
            ),
            navigation_history_entry(
                "https://example.test/current",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
            ),
            navigation_history_entry(
                "https://example.test/forward",
                2,
                2,
                document_id("document-2"),
                entry_id("entry-2"),
                entry_key("key-2"),
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
                );
                entry.history_state = Some(moli_history::SerializedScriptValue::new(
                    vec![0, index as u8],
                    (),
                ));
                entry.navigation_state = Some(moli_history::SerializedScriptValue::new(
                    vec![1, index as u8],
                    (),
                ));
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
            ),
            navigation_history_entry(
                "https://example.test/target",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
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
            ),
            navigation_history_entry(
                "https://example.test/target",
                1,
                1,
                document_id("document-1"),
                entry_id("entry-1"),
                entry_key("key-1"),
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
}
