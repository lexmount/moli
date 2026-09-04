use std::{
    hash::{Hash, Hasher},
    sync::{Arc as StdArc, LazyLock},
};

use moli_crypto::Sha256Context;
use moli_selector::StyloSourceDependencySummary;
use style::stylesheets::{CssRule, StylesheetInDocument};
use style_traits::ToCss;

use super::super::{
    source_id::{StyleScopeId, StyleSourceId, StyleSourceKind},
    source_key::{StyleSourceKey, StyleSourceSetKey},
};
use super::shared_cache::{SharedStyleSourceContents, shared_style_source_contents};
use crate::{
    document_runtime::DomHandle, protocol_types::EmulatedMediaOverrides,
    style_engine::StyleViewport,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StylesheetFontFaceDescriptor {
    family: StdArc<str>,
    source: StdArc<str>,
}

static EMPTY_STYLESHEET_FONT_FACES: LazyLock<StdArc<[StylesheetFontFaceDescriptor]>> =
    LazyLock::new(|| StdArc::from([]));

#[derive(Clone, Debug)]
pub(crate) struct StyloStylesheetSource {
    contents: StyloStylesheetSourceContents,
    /// Final response URL used as the external stylesheet parser base.
    pub(in crate::style_engine) base_url: StdArc<url::Url>,
    /// Stable URL exposed by the top-level `CSSStyleSheet.href`.
    sheet_url: StdArc<url::Url>,
    origin_clean: bool,
    cache_key: StyleSourceKey,
    source_id: Option<StyleSourceId>,
    adopted_client_id: Option<u64>,
    /// Top-level media list attached by the stylesheet owner.
    ///
    /// Live stylesheets carry the same list in Stylo's `Stylesheet::media`.
    /// Keeping the serialized value on the source also preserves it when a
    /// live sheet is projected into an independently parsed test/query world.
    media_text: StdArc<str>,
}

#[derive(Clone, Debug)]
enum StyloStylesheetSourceContents {
    Text {
        shared: StdArc<SharedStyleSourceContents>,
    },
    Live {
        stylesheet: style::servo_arc::Arc<style::stylesheets::Stylesheet>,
        id: crate::live_stylesheet::StylesheetId,
        contents_revision: u64,
        cascade_generation: u64,
        cascade_mutations: StdArc<
            parking_lot::Mutex<crate::live_stylesheet::LiveStylesheetCascadeMutationJournal>,
        >,
        derived_state: StdArc<crate::live_stylesheet::LiveStylesheetDerivedState>,
        shared_initial_contents: Option<StdArc<crate::live_stylesheet::SharedStylesheetContents>>,
    },
}

#[derive(Clone, Debug)]
pub(in crate::style_engine) enum LiveStylesheetCascadeUpdate {
    Full,
    Rules(Vec<crate::live_stylesheet::LiveStylesheetRuleMutation>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::style_engine) struct StyleSourceMetadata {
    pub(in crate::style_engine) dependency_summary: StyloSourceDependencySummary,
}

/// Immutable stylesheet source produced by processing an inline `<style>` owner.
///
/// The parser base belongs to the processed source, not to a later source-set
/// rebuild. The processing source owns the original owner text for parser-only
/// compatibility extensions without reconstructing parser state from the live
/// document or retaining a second copy on the owner record.
#[derive(Debug)]
pub(crate) struct OwnerStyleSheetSource {
    owner: DomHandle,
    source: StyloStylesheetSource,
}

impl StyloStylesheetSource {
    pub(crate) fn new(css_text: String, base_url: url::Url) -> Self {
        let shared = shared_style_source_contents(css_text, base_url);
        let cache_key =
            StyleSourceKey::from_css_fingerprint(shared.css_fingerprint(), shared.base_url());
        let base_url = shared.base_url_handle();
        Self {
            contents: StyloStylesheetSourceContents::Text { shared },
            sheet_url: StdArc::clone(&base_url),
            base_url,
            origin_clean: true,
            cache_key,
            source_id: None,
            adopted_client_id: None,
            media_text: StdArc::from(""),
        }
    }

    pub(crate) fn from_live_stylesheet(
        stylesheet: &crate::live_stylesheet::LiveStylesheetRef,
    ) -> Self {
        let base_url = stylesheet.base_url().clone();
        let parsed_stylesheet = stylesheet.stylesheet();
        let media_text = {
            let guard = parsed_stylesheet.shared_lock.read();
            parsed_stylesheet.media.read_with(&guard).to_css_string()
        };
        Self {
            contents: StyloStylesheetSourceContents::Live {
                stylesheet: parsed_stylesheet,
                id: stylesheet.id(),
                contents_revision: stylesheet.contents_revision(),
                cascade_generation: stylesheet.cascade_generation(),
                cascade_mutations: stylesheet.cascade_mutation_journal(),
                derived_state: stylesheet.derived_state(),
                shared_initial_contents: stylesheet.shared_initial_contents(),
            },
            base_url: StdArc::new(base_url.clone()),
            sheet_url: StdArc::new(base_url),
            origin_clean: true,
            cache_key: StyleSourceKey::from_live_stylesheet(
                stylesheet.id(),
                stylesheet.cascade_generation(),
            ),
            source_id: None,
            adopted_client_id: None,
            media_text: StdArc::from(media_text),
        }
    }

    pub(crate) fn with_source_id(mut self, source_id: Option<StyleSourceId>) -> Self {
        self.source_id = source_id;
        self
    }

    pub(in crate::style_engine) fn with_adopted_client_id(mut self, client_id: u64) -> Self {
        self.adopted_client_id = Some(client_id);
        self
    }

    pub(crate) fn with_origin_clean(mut self, origin_clean: bool) -> Self {
        self.origin_clean = origin_clean;
        self
    }

    pub(crate) fn with_sheet_url(mut self, sheet_url: url::Url) -> Self {
        self.sheet_url = StdArc::new(sheet_url);
        self
    }

    /// Installs an owner attribute media list on a text-backed source.
    ///
    /// A live stylesheet is already authoritative for CSSOM media mutations,
    /// so owner projection must not overwrite the list captured from it.
    pub(crate) fn with_owner_media_text(mut self, media_text: &str) -> Self {
        if matches!(&self.contents, StyloStylesheetSourceContents::Text { .. }) {
            self.media_text = StdArc::from(media_text);
        }
        self
    }

    fn with_media_text(mut self, media_text: StdArc<str>) -> Self {
        self.media_text = media_text;
        self
    }

    pub(crate) fn input_css_text(&self) -> Option<&str> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => Some(shared.css_text()),
            StyloStylesheetSourceContents::Live { .. } => None,
        }
    }

    pub(crate) fn serialized_css_text(&self) -> StdArc<str> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => shared.css_text_handle(),
            StyloStylesheetSourceContents::Live {
                stylesheet,
                contents_revision,
                derived_state,
                ..
            } => derived_state.serialized_css_text(*contents_revision, || {
                moli_css_parse::native_stylesheet_css_text_with_stylo(stylesheet)
            }),
        }
    }

    /// Whether this installed source's top-level media list is effective for
    /// compatibility paths that inspect serialized CSS outside Stylo.
    pub(crate) fn media_matches(
        &self,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) -> bool {
        crate::style_engine::source_owner::stylesheet_source_media_matches(
            &self.media_text,
            emulated_media,
            viewport,
        )
    }

    /// Projects this source into an independently parsed text source.
    ///
    /// A live stylesheet is owned by the `SharedRwLock` of the retained style
    /// world that parsed it. Passing that object to another `MoliStyleEngine`
    /// would pair it with an unrelated guard. Compatibility probes that use an
    /// isolated engine therefore serialize live sources at that boundary and
    /// let the isolated engine parse its own stylesheet object.
    pub(crate) fn independent_text_projection(&self) -> Self {
        let StyloStylesheetSourceContents::Live { .. } = &self.contents else {
            return self.clone();
        };
        let mut projection = Self::new(
            self.serialized_css_text().to_string(),
            self.base_url.as_ref().clone(),
        )
        .with_source_id(self.source_id.clone())
        .with_origin_clean(self.origin_clean)
        .with_sheet_url(self.sheet_url.as_ref().clone())
        .with_media_text(StdArc::clone(&self.media_text));
        projection.adopted_client_id = self.adopted_client_id;
        projection
    }

    fn processing_contents(&self) -> Option<&SharedStyleSourceContents> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => Some(shared),
            StyloStylesheetSourceContents::Live { .. } => None,
        }
    }

    pub(crate) fn parsed_stylesheet(
        &self,
    ) -> Option<style::servo_arc::Arc<style::stylesheets::Stylesheet>> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { .. } => None,
            StyloStylesheetSourceContents::Live { stylesheet, .. } => Some(stylesheet.clone()),
        }
    }

    pub(crate) fn live_stylesheet_id(&self) -> Option<crate::live_stylesheet::StylesheetId> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { .. } => None,
            StyloStylesheetSourceContents::Live { id, .. } => Some(*id),
        }
    }

    pub(in crate::style_engine) fn live_cascade_update_since(
        &self,
        previous: &Self,
    ) -> LiveStylesheetCascadeUpdate {
        let (
            StyloStylesheetSourceContents::Live {
                id,
                cascade_generation,
                cascade_mutations,
                ..
            },
            StyloStylesheetSourceContents::Live {
                id: previous_id,
                cascade_generation: previous_generation,
                ..
            },
        ) = (&self.contents, &previous.contents)
        else {
            return LiveStylesheetCascadeUpdate::Full;
        };
        if id != previous_id || previous_generation >= cascade_generation {
            return LiveStylesheetCascadeUpdate::Full;
        }

        let cascade_mutations = cascade_mutations.lock();
        let batches = cascade_mutations
            .iter()
            .filter(|batch| {
                batch.generation() > *previous_generation
                    && batch.generation() <= *cascade_generation
            })
            .collect::<Vec<_>>();
        let expected_batch_count = cascade_generation.saturating_sub(*previous_generation);
        if batches.len() as u64 != expected_batch_count
            || batches
                .first()
                .is_none_or(|batch| batch.generation() != previous_generation.saturating_add(1))
            || batches
                .last()
                .is_none_or(|batch| batch.generation() != *cascade_generation)
        {
            return LiveStylesheetCascadeUpdate::Full;
        }

        let mut rules = Vec::new();
        for batch in batches {
            match batch.mutation() {
                crate::live_stylesheet::LiveStylesheetCascadeMutation::Full => {
                    return LiveStylesheetCascadeUpdate::Full;
                }
                crate::live_stylesheet::LiveStylesheetCascadeMutation::Rules(changes) => {
                    rules.extend(changes.iter().cloned());
                }
            }
        }
        LiveStylesheetCascadeUpdate::Rules(rules)
    }

    pub(crate) fn shared_initial_contents(
        &self,
    ) -> Option<StdArc<crate::live_stylesheet::SharedStylesheetContents>> {
        match &self.contents {
            StyloStylesheetSourceContents::Live {
                shared_initial_contents,
                ..
            } => shared_initial_contents.as_ref().map(StdArc::clone),
            StyloStylesheetSourceContents::Text { .. } => None,
        }
    }

    pub(crate) fn base_url(&self) -> &url::Url {
        &self.base_url
    }

    pub(crate) fn sheet_url(&self) -> &url::Url {
        &self.sheet_url
    }

    pub(crate) fn origin_clean(&self) -> bool {
        self.origin_clean
    }

    pub(in crate::style_engine) fn cache_key(&self) -> StyleSourceKey {
        self.cache_key
    }

    pub(in crate::style_engine) fn source_dependency_summary(
        &self,
    ) -> StdArc<StyloSourceDependencySummary> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => shared.source_dependency_summary(),
            StyloStylesheetSourceContents::Live {
                stylesheet,
                contents_revision,
                cascade_generation,
                derived_state,
                ..
            } => derived_state.source_dependency_summary(
                *contents_revision,
                *cascade_generation,
                || {
                    super::super::stylesheet::style_source_metadata_for_stylesheet(stylesheet)
                        .dependency_summary
                },
            ),
        }
    }

    pub(crate) fn font_faces(&self) -> StdArc<[StylesheetFontFaceDescriptor]> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => shared.font_faces(),
            StyloStylesheetSourceContents::Live { .. } => {
                StdArc::clone(&EMPTY_STYLESHEET_FONT_FACES)
            }
        }
    }

    pub(crate) fn import_urls(&self) -> StdArc<[url::Url]> {
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => shared.import_urls_handle(),
            StyloStylesheetSourceContents::Live { stylesheet, .. } => {
                let guard = stylesheet.shared_lock.read();
                let mut urls = Vec::new();
                for rule in stylesheet.contents(&guard).rules(&guard) {
                    let CssRule::Import(rule) = rule else {
                        continue;
                    };
                    let Some(url) = rule.read_with(&guard).url.url() else {
                        continue;
                    };
                    if !urls.iter().any(|existing| existing == url.as_ref()) {
                        urls.push(url.as_ref().clone());
                    }
                }
                urls.into()
            }
        }
    }

    pub(in crate::style_engine) fn source_id(&self) -> Option<&StyleSourceId> {
        self.source_id.as_ref()
    }

    pub(crate) fn owner_style_sheet_owner(&self) -> Option<DomHandle> {
        self.source_id.as_ref().and_then(|source_id| {
            if let StyleSourceKind::OwnerStyleSheet { owner } = source_id.kind {
                Some(owner)
            } else {
                None
            }
        })
    }

    pub(crate) fn adopted_client_id(&self) -> Option<u64> {
        self.adopted_client_id
    }

    pub(in crate::style_engine) fn media_text(&self) -> &str {
        self.media_text.as_ref()
    }

    pub(in crate::style_engine) fn has_authoritative_runtime_media(&self) -> bool {
        matches!(&self.contents, StyloStylesheetSourceContents::Live { .. })
    }

    pub(in crate::style_engine) fn has_same_installation_identity(&self, other: &Self) -> bool {
        match (self.live_stylesheet_id(), other.live_stylesheet_id()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => {
                self.cache_key == other.cache_key
                    && self.base_url == other.base_url
                    && self.sheet_url == other.sheet_url
                    && self.origin_clean == other.origin_clean
            }
            _ => false,
        }
    }

    pub(in crate::style_engine) fn has_same_stylesheet_revision(&self, other: &Self) -> bool {
        self.base_url == other.base_url
            && self.sheet_url == other.sheet_url
            && self.origin_clean == other.origin_clean
            && self.cache_key == other.cache_key
            && self.media_text == other.media_text
            && match (&self.contents, &other.contents) {
                (
                    StyloStylesheetSourceContents::Text { shared: left },
                    StyloStylesheetSourceContents::Text { shared: right },
                ) => left == right,
                (
                    StyloStylesheetSourceContents::Live { .. },
                    StyloStylesheetSourceContents::Live { .. },
                ) => true,
                _ => false,
            }
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn shares_source_storage_for_test(&self, other: &Self) -> bool {
        match (&self.contents, &other.contents) {
            (
                StyloStylesheetSourceContents::Text { shared: left },
                StyloStylesheetSourceContents::Text { shared: right },
            ) => StdArc::ptr_eq(left, right),
            (
                StyloStylesheetSourceContents::Live {
                    stylesheet: left_stylesheet,
                    derived_state: left_derived_state,
                    ..
                },
                StyloStylesheetSourceContents::Live {
                    stylesheet: right_stylesheet,
                    derived_state: right_derived_state,
                    ..
                },
            ) => {
                style::servo_arc::Arc::ptr_eq(left_stylesheet, right_stylesheet)
                    && StdArc::ptr_eq(left_derived_state, right_derived_state)
            }
            _ => false,
        }
    }
}

impl PartialEq for StyloStylesheetSource {
    fn eq(&self, other: &Self) -> bool {
        self.base_url == other.base_url
            && self.sheet_url == other.sheet_url
            && self.origin_clean == other.origin_clean
            && self.cache_key == other.cache_key
            && self.source_id == other.source_id
            && self.adopted_client_id == other.adopted_client_id
            && self.media_text == other.media_text
            && match (&self.contents, &other.contents) {
                (
                    StyloStylesheetSourceContents::Text { shared: left },
                    StyloStylesheetSourceContents::Text { shared: right },
                ) => left == right,
                (
                    StyloStylesheetSourceContents::Live { .. },
                    StyloStylesheetSourceContents::Live { .. },
                ) => true,
                _ => false,
            }
    }
}

impl Eq for StyloStylesheetSource {}

impl Hash for StyloStylesheetSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.base_url.hash(state);
        self.sheet_url.hash(state);
        self.origin_clean.hash(state);
        self.cache_key.hash(state);
        self.source_id.hash(state);
        self.adopted_client_id.hash(state);
        self.media_text.hash(state);
        match &self.contents {
            StyloStylesheetSourceContents::Text { shared } => {
                0_u8.hash(state);
                shared.hash(state);
            }
            StyloStylesheetSourceContents::Live { .. } => 1_u8.hash(state),
        }
    }
}

impl StylesheetFontFaceDescriptor {
    pub(super) fn new(family: String, source: String) -> Self {
        Self {
            family: StdArc::from(family),
            source: StdArc::from(source),
        }
    }

    pub(crate) fn family(&self) -> &str {
        self.family.as_ref()
    }

    pub(crate) fn source(&self) -> &str {
        self.source.as_ref()
    }
}

impl OwnerStyleSheetSource {
    pub(crate) fn new(owner: DomHandle, css_text: String, parser_base: url::Url) -> Self {
        let source = StyloStylesheetSource::new(css_text, parser_base);
        debug_assert!(source.processing_contents().is_some());
        Self { owner, source }
    }

    pub(in crate::style_engine) fn matches_processing_input(
        &self,
        css_text: &str,
        parser_base: &url::Url,
    ) -> bool {
        self.css_text() == css_text && self.source.base_url() == parser_base
    }

    pub(crate) fn css_text(&self) -> &str {
        self.source
            .processing_contents()
            .map(SharedStyleSourceContents::css_text)
            .expect("owner processing source must remain text-backed")
    }

    pub(crate) fn parser_base(&self) -> &url::Url {
        self.source.base_url()
    }

    pub(in crate::style_engine) fn source(&self) -> &StyloStylesheetSource {
        &self.source
    }

    pub(crate) fn import_urls(&self) -> &[url::Url] {
        self.source
            .processing_contents()
            .expect("owner processing source must remain text-backed")
            .import_urls()
    }

    pub(crate) fn font_faces(&self) -> StdArc<[StylesheetFontFaceDescriptor]> {
        self.source.font_faces()
    }

    pub(crate) fn owner(&self) -> DomHandle {
        self.owner
    }
}

pub(in crate::style_engine) fn stylesheet_sources_cache_key(
    sources: &[StyloStylesheetSource],
) -> StyleSourceSetKey {
    let mut hasher = Sha256Context::new();
    hasher.update(sources.len().to_le_bytes());
    for source in sources {
        hasher.update(source.cache_key().fingerprint);
        let media_text = source.media_text().as_bytes();
        hasher.update(media_text.len().to_le_bytes());
        hasher.update(media_text);
        update_style_source_identity_hash(&mut hasher, source.source_id());
        match source.adopted_client_id() {
            Some(client_id) => {
                hasher.update([1]);
                hasher.update(client_id.to_le_bytes());
            }
            None => hasher.update([0]),
        }
    }
    StyleSourceSetKey {
        len: sources.len(),
        fingerprint: hasher.finish(),
    }
}

fn update_style_source_identity_hash(
    hasher: &mut Sha256Context,
    source_id: Option<&StyleSourceId>,
) {
    let Some(source_id) = source_id else {
        hasher.update([0]);
        return;
    };
    hasher.update([1]);
    update_style_scope_identity_hash(hasher, source_id.scope_id);
    match &source_id.kind {
        StyleSourceKind::OwnerStyleSheet { owner } => {
            hasher.update([0]);
            hasher.update(owner.index().to_le_bytes());
        }
        StyleSourceKind::LinkedStyleSheet { owner } => {
            hasher.update([1]);
            hasher.update(owner.index().to_le_bytes());
        }
        StyleSourceKind::DocumentAdoptedStyleSheet { client_id } => {
            hasher.update([2]);
            hasher.update(client_id.to_le_bytes());
        }
        StyleSourceKind::ShadowRootAdoptedStyleSheet { client_id } => {
            hasher.update([3]);
            hasher.update(client_id.to_le_bytes());
        }
    }
}

fn update_style_scope_identity_hash(hasher: &mut Sha256Context, scope_id: StyleScopeId) {
    match scope_id {
        StyleScopeId::Document(document) => {
            hasher.update([0]);
            hasher.update(document.index().to_le_bytes());
        }
        StyleScopeId::ShadowRoot(root) => {
            hasher.update([1]);
            hasher.update(root.index().to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use style::{context::QuirksMode, shared_lock::SharedRwLock, stylesheets::AllowImportRules};

    use super::{LiveStylesheetCascadeUpdate, StyloStylesheetSource};
    use crate::live_stylesheet::{
        LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY, LiveStylesheetRegistry,
    };

    #[test]
    fn live_cascade_mutation_journal_is_bounded_with_safe_full_fallback() {
        let registry = LiveStylesheetRegistry::default();
        let stylesheet = registry.create(
            ".target { color: rgb(0, 0, 0); }",
            url::Url::parse("https://cascade-journal.test/style.css").expect("stylesheet URL"),
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
            SharedRwLock::new(),
        );
        let initial = StyloStylesheetSource::from_live_stylesheet(&stylesheet);

        stylesheet
            .replace_rule(".target { color: rgb(1, 2, 3); }", 0)
            .expect("first exact rule replacement");
        let first_update = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        assert!(matches!(
            first_update.live_cascade_update_since(&initial),
            LiveStylesheetCascadeUpdate::Rules(changes) if !changes.is_empty()
        ));

        let lagging_installation = first_update;
        for index in 0..2_048 {
            stylesheet
                .replace_rule(
                    &format!(
                        ".target {{ color: rgb({}, {}, {}); }}",
                        index % 251,
                        (index + 1) % 251,
                        (index + 2) % 251,
                    ),
                    0,
                )
                .expect("stress replacement should remain valid");
        }
        assert_eq!(
            stylesheet.cascade_mutation_journal().lock().len(),
            LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY,
            "old rule trees must not remain strongly retained without bound",
        );
        let after_stress = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        assert!(matches!(
            after_stress.live_cascade_update_since(&lagging_installation),
            LiveStylesheetCascadeUpdate::Full
        ));

        let caught_up_installation = after_stress;
        stylesheet
            .replace_rule(".target { color: rgb(7, 8, 9); }", 0)
            .expect("latest exact rule replacement");
        let latest = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        assert!(matches!(
            latest.live_cascade_update_since(&caught_up_installation),
            LiveStylesheetCascadeUpdate::Rules(changes) if !changes.is_empty()
        ));
    }

    #[test]
    fn live_cascade_journal_capacity_boundary_keeps_exact_updates() {
        let registry = LiveStylesheetRegistry::default();
        let stylesheet = registry.create(
            ".target { color: rgb(0, 0, 0); }",
            url::Url::parse("https://cascade-journal-boundary.test/style.css")
                .expect("stylesheet URL"),
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
            SharedRwLock::new(),
        );
        let one_generation_too_old = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        stylesheet
            .replace_rule(".target { color: rgb(1, 2, 3); }", 0)
            .expect("boundary seed replacement");
        let oldest_exact_predecessor = StyloStylesheetSource::from_live_stylesheet(&stylesheet);

        for index in 0..LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY {
            stylesheet
                .replace_rule(&format!(".target {{ z-index: {}; }}", index % 100), 0)
                .expect("boundary replacement should remain valid");
        }
        let current = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        assert!(matches!(
            current.live_cascade_update_since(&oldest_exact_predecessor),
            LiveStylesheetCascadeUpdate::Rules(changes) if !changes.is_empty()
        ));
        assert!(matches!(
            current.live_cascade_update_since(&one_generation_too_old),
            LiveStylesheetCascadeUpdate::Full
        ));
    }

    #[test]
    fn imported_child_mutations_bound_both_child_and_ancestor_journals() {
        let registry = LiveStylesheetRegistry::default();
        let root = registry.create(
            "@import './child.css'; .root { display: block; }",
            url::Url::parse("https://cascade-import-journal.test/root.css")
                .expect("root stylesheet URL"),
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
            SharedRwLock::new(),
        );
        let request = root
            .pending_import_requests()
            .into_iter()
            .next()
            .expect("pending child import");
        let child = registry
            .install_import_response(
                root.id(),
                root.contents_revision(),
                request.edge_id,
                ".child { color: rgb(0, 0, 0); }",
                request.url,
                true,
                true,
            )
            .expect("child import should install");
        let root_before_mutation = StyloStylesheetSource::from_live_stylesheet(&root);
        let child_before_mutation = StyloStylesheetSource::from_live_stylesheet(&child);

        child
            .replace_rule(".child { color: rgb(1, 2, 3); }", 0)
            .expect("child exact replacement");
        let root_after_mutation = StyloStylesheetSource::from_live_stylesheet(&root);
        let child_after_mutation = StyloStylesheetSource::from_live_stylesheet(&child);
        assert!(matches!(
            child_after_mutation.live_cascade_update_since(&child_before_mutation),
            LiveStylesheetCascadeUpdate::Rules(changes) if !changes.is_empty()
        ));
        assert!(matches!(
            root_after_mutation.live_cascade_update_since(&root_before_mutation),
            LiveStylesheetCascadeUpdate::Full
        ));

        for index in 0..(LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY * 2 + 3) {
            child
                .replace_rule(&format!(".child {{ z-index: {}; }}", index % 100), 0)
                .expect("import stress replacement should remain valid");
        }
        assert_eq!(
            child.cascade_mutation_journal().lock().len(),
            LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY,
        );
        assert_eq!(
            root.cascade_mutation_journal().lock().len(),
            LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY,
            "descendant mutations must not grow an ancestor journal without bound",
        );
    }
}
