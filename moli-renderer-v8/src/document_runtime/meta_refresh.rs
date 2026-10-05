use super::*;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaRefreshNavigation {
    pub(crate) delay_ms: u32,
    pub(crate) url: Url,
}

#[derive(Debug)]
pub(crate) struct ScheduledMetaRefreshNavigation {
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) navigation: MetaRefreshNavigation,
    pub(crate) ready_at: Instant,
}

impl ScheduledMetaRefreshNavigation {
    pub(crate) fn into_internal_loading_task(
        self,
    ) -> (
        crate::page_task_queue::PageOwnedInternalLoadingTask,
        Instant,
    ) {
        let task = crate::page_task_queue::PageOwnedInternalLoadingTask::MetaRefreshNavigation(
            crate::page_task_queue::MainDocumentMetaRefreshNavigationTask::new(
                self.owner,
                self.navigation.delay_ms,
                self.navigation.url,
            ),
        );
        (task, self.ready_at)
    }
}

/// Document-owned state corresponding to Blink's `HttpRefreshScheduler`.
///
/// The scheduler owns one candidate and one exact-Document load boundary. The
/// Page task source owns the posted delayed payload; posting a new candidate
/// for the same owner replaces that payload rather than manufacturing a
/// JavaScript timer.
#[derive(Debug, Default)]
pub(super) struct MetaRefreshScheduler {
    candidate: Option<MetaRefreshNavigation>,
    load_finished_owner: Option<FrameDocumentTaskOwner>,
    armed_owner: Option<FrameDocumentTaskOwner>,
    ready_at: Option<Instant>,
}

impl DocumentRuntime {
    #[cfg(test)]
    pub(crate) fn top_level_meta_refresh_navigation(&self) -> Option<MetaRefreshNavigation> {
        let document_base_url = self
            .dom_host()
            .document_base_url()
            .unwrap_or_else(|| self.document_url().clone());
        meta_refresh_navigation_for_document(
            self.dom_host(),
            self.document_handle(),
            self.document_url(),
            &document_base_url,
        )
    }

    pub(crate) fn finish_top_level_meta_refresh_load(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        if !self.meta_refresh_scheduler.has_candidate() {
            if !self.document_sandbox_policy().allows_scripts {
                return None;
            }
            let document_base_url = self
                .dom_host()
                .document_base_url()
                .unwrap_or_else(|| self.document_url().clone());
            let candidates = meta_refresh_navigations_for_document(
                self.dom_host(),
                self.document_handle(),
                self.document_url(),
                &document_base_url,
            );
            self.meta_refresh_scheduler.note_candidates(candidates);
        }
        self.meta_refresh_scheduler.finish_load(owner)
    }

    pub(crate) fn consume_top_level_meta_refresh_navigation(
        &mut self,
        owner: FrameDocumentTaskOwner,
        delay_ms: u32,
        url: &Url,
    ) -> bool {
        self.meta_refresh_scheduler
            .consume_if_matches(owner, delay_ms, url)
    }

    pub(crate) fn note_top_level_meta_refresh_candidates(
        &mut self,
        candidates: Vec<MetaRefreshNavigation>,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        if !self.document_sandbox_policy().allows_scripts {
            return None;
        }
        self.meta_refresh_scheduler
            .note_candidates(candidates)
            .then(|| self.meta_refresh_scheduler.schedule_for_loaded_owner())
            .flatten()
    }

    pub(crate) fn prepare_top_level_meta_refresh_for_document_open(&mut self) {
        self.meta_refresh_scheduler.prepare_for_document_open();
    }

    pub(crate) fn rebind_top_level_meta_refresh_after_document_open(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        self.meta_refresh_scheduler.rebind_armed_navigation(owner)
    }
}

impl MetaRefreshScheduler {
    fn has_candidate(&self) -> bool {
        self.candidate.is_some()
    }

    fn note_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = MetaRefreshNavigation>,
    ) -> bool {
        let mut changed = false;
        for candidate in candidates {
            if self
                .candidate
                .as_ref()
                .is_some_and(|current| current == &candidate)
            {
                // Mutation processing may rediscover an unchanged refresh
                // element. Blink keeps the already-posted task and its
                // original deadline for a semantically identical candidate.
                continue;
            }
            if self
                .candidate
                .as_ref()
                .is_some_and(|current| current.delay_ms < candidate.delay_ms)
            {
                continue;
            }
            self.candidate = Some(candidate);
            self.armed_owner = None;
            self.ready_at = None;
            changed = true;
        }
        changed
    }

    fn finish_load(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        self.load_finished_owner = Some(owner);
        self.schedule_for_loaded_owner()
    }

    fn schedule_for_loaded_owner(&mut self) -> Option<ScheduledMetaRefreshNavigation> {
        let owner = self.load_finished_owner?;
        let navigation = self.candidate.clone()?;
        if self.armed_owner.is_some() {
            return None;
        }
        let ready_at = self.ready_at.or_else(|| {
            Instant::now().checked_add(Duration::from_millis(u64::from(navigation.delay_ms)))
        })?;
        self.armed_owner = Some(owner);
        self.ready_at = Some(ready_at);
        Some(ScheduledMetaRefreshNavigation {
            owner,
            navigation,
            ready_at,
        })
    }

    fn consume_if_matches(
        &mut self,
        owner: FrameDocumentTaskOwner,
        delay_ms: u32,
        url: &Url,
    ) -> bool {
        let matches = self.armed_owner == Some(owner)
            && self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.delay_ms == delay_ms && candidate.url == *url);
        if matches {
            self.candidate = None;
            self.armed_owner = None;
            self.ready_at = None;
        }
        matches
    }

    fn prepare_for_document_open(&mut self) {
        self.load_finished_owner = None;
        self.armed_owner = None;
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| candidate.delay_ms == 0)
        {
            *self = Self::default();
        }
    }

    fn rebind_armed_navigation(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        let navigation = self.candidate.clone()?;
        let ready_at = self.ready_at?;
        self.armed_owner = Some(owner);
        Some(ScheduledMetaRefreshNavigation {
            owner,
            navigation,
            ready_at,
        })
    }
}

pub(super) fn meta_refresh_navigations_from_mutation(
    dom_host: &DomHost,
    effects: &DomMutationEffects,
    document_url: &Url,
) -> Vec<MetaRefreshNavigation> {
    let document_handle = dom_host.document_handle();
    let mut handles = Vec::new();
    for &root in effects.tree().connected_roots() {
        for handle in dom_host.elements_by_tag_name_ns(
            root,
            Some("http://www.w3.org/1999/xhtml"),
            "meta",
            true,
        ) {
            if !handles.contains(&handle) {
                handles.push(handle);
            }
        }
    }
    for mutation in effects.style().attribute_mutations() {
        if mutation.namespace().is_some() || !dom_host.is_connected(mutation.target()) {
            continue;
        }
        let Some(element) = dom_host.node(mutation.target()).and_then(Node::as_element) else {
            continue;
        };
        let local_name = element.normalized_attribute_name(mutation.local_name());
        if element.is_html_element("meta")
            && matches!(local_name.as_str(), "content" | "http-equiv")
            && !handles.contains(&mutation.target())
        {
            handles.push(mutation.target());
        }
    }
    if handles.is_empty() {
        return Vec::new();
    }
    let document_base_url = dom_host
        .document_base_url()
        .unwrap_or_else(|| document_url.clone());
    handles
        .into_iter()
        .filter(|&handle| {
            dom_host.owner_document_handle(handle) == Some(document_handle)
                && dom_host.is_connected(handle)
        })
        .filter_map(|handle| {
            meta_refresh_navigation_for_element(dom_host, handle, document_url, &document_base_url)
        })
        .collect()
}

impl MetaRefreshNavigation {
    pub(crate) fn parse(
        content: &str,
        document_url: &Url,
        document_base_url: &Url,
    ) -> Option<Self> {
        meta_refresh_navigation_from_content(content, document_url, document_base_url)
    }
}

#[cfg(test)]
fn meta_refresh_navigation_for_document(
    dom_host: &DomHost,
    document_handle: DomHandle,
    document_url: &Url,
    document_base_url: &Url,
) -> Option<MetaRefreshNavigation> {
    meta_refresh_navigations_for_document(
        dom_host,
        document_handle,
        document_url,
        document_base_url,
    )
    .into_iter()
    .next()
}

fn meta_refresh_navigations_for_document(
    dom_host: &DomHost,
    document_handle: DomHandle,
    document_url: &Url,
    document_base_url: &Url,
) -> Vec<MetaRefreshNavigation> {
    dom_host
        .html_elements_by_local_name_in_document_tree_order(document_handle, "meta")
        .into_iter()
        .filter_map(|handle| {
            meta_refresh_navigation_for_element(dom_host, handle, document_url, document_base_url)
        })
        .collect()
}

fn meta_refresh_navigation_for_element(
    dom_host: &DomHost,
    handle: DomHandle,
    document_url: &Url,
    document_base_url: &Url,
) -> Option<MetaRefreshNavigation> {
    let element = dom_host.node(handle)?.as_element()?;
    let http_equiv = element.attribute("http-equiv")?;
    if !http_equiv.eq_ignore_ascii_case("refresh") {
        return None;
    }
    meta_refresh_navigation_from_content(
        element.attribute("content")?,
        document_url,
        document_base_url,
    )
}

fn meta_refresh_navigation_from_content(
    content: &str,
    document_url: &Url,
    document_base_url: &Url,
) -> Option<MetaRefreshNavigation> {
    let mut position = 0;
    skip_meta_refresh_html_spaces(content, &mut position);
    let time_start = position;
    while let Some(character) = content[position..].chars().next() {
        if matches!(character, ',' | ';') || is_meta_refresh_html_space(character) {
            break;
        }
        position += character.len_utf8();
    }
    let delay_seconds = parse_meta_refresh_delay_seconds(&content[time_start..position])?;
    let delay_ms = u32::try_from(delay_seconds.checked_mul(1_000)?).ok()?;

    skip_meta_refresh_html_spaces(content, &mut position);
    if content[position..].starts_with(',') || content[position..].starts_with(';') {
        position += 1;
    }
    skip_meta_refresh_html_spaces(content, &mut position);
    let url = meta_refresh_target_url(&content[position..], document_url, document_base_url)?;
    Some(MetaRefreshNavigation { delay_ms, url })
}

fn parse_meta_refresh_delay_seconds(value: &str) -> Option<u64> {
    let mut second_full_stop = None;
    let mut full_stop_count = 0;
    for (index, character) in value.char_indices() {
        if character == '.' {
            full_stop_count += 1;
            if full_stop_count == 2 {
                second_full_stop = Some(index);
            }
        } else if !character.is_ascii_digit() {
            return None;
        }
    }
    let number = value[..second_full_stop.unwrap_or(value.len())]
        .parse::<f64>()
        .ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let seconds = number.floor();
    // Blink's HttpRefreshScheduler rejects delays beyond INT32_MAX / 1000
    // seconds instead of clamping them into a different observable delay.
    const MAX_SCHEDULED_DELAY_SECONDS: u64 = i32::MAX as u64 / 1_000;
    if seconds > MAX_SCHEDULED_DELAY_SECONDS as f64 {
        return None;
    }
    Some(seconds as u64)
}

fn meta_refresh_target_url(
    value: &str,
    document_url: &Url,
    document_base_url: &Url,
) -> Option<Url> {
    let target = strip_optional_meta_refresh_url_prefix(value);
    let target = trim_meta_refresh_quoted_url(target);
    let target = target.trim_matches(is_meta_refresh_html_space);
    if target.is_empty() {
        return Some(document_url.clone());
    }
    let url = document_base_url.join(target).ok()?;
    (url.scheme() != "javascript").then_some(url)
}

fn strip_optional_meta_refresh_url_prefix(value: &str) -> &str {
    let Some(after_url) = value
        .get(..3)
        .filter(|prefix| prefix.eq_ignore_ascii_case("url"))
    else {
        return value;
    };
    let mut position = after_url.len();
    skip_meta_refresh_html_spaces(value, &mut position);
    let rest = &value[position..];
    let Some(after_equals) = rest.strip_prefix('=') else {
        return value;
    };
    let mut position = 0;
    skip_meta_refresh_html_spaces(after_equals, &mut position);
    &after_equals[position..]
}

fn trim_meta_refresh_quoted_url(value: &str) -> &str {
    let Some(quotation_mark @ ('"' | '\'')) = value.chars().next() else {
        return value;
    };
    let value = &value[quotation_mark.len_utf8()..];
    value
        .rfind(quotation_mark)
        .map_or(value, |closing_quote| &value[..closing_quote])
}

fn skip_meta_refresh_html_spaces(value: &str, position: &mut usize) {
    while let Some(character) = value[*position..].chars().next() {
        if !is_meta_refresh_html_space(character) {
            break;
        }
        *position += character.len_utf8();
    }
}

const fn is_meta_refresh_html_space(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{000c}' | '\r' | ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replacement_document_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            super::runtime_core::test_stylesheet_document_owner().scheduler_lane_id,
            super::runtime_core::test_stylesheet_document_owner().local_window_id,
            crate::frame_owner_model::DocumentId(2),
        )
    }

    fn base_url() -> Url {
        Url::parse("https://example.test/dir/page.html").unwrap()
    }

    #[test]
    fn meta_refresh_parses_bare_relative_target() {
        let document_url = base_url();
        let navigation =
            meta_refresh_navigation_from_content("0;redirected.html", &document_url, &document_url)
                .unwrap();
        assert_eq!(navigation.delay_ms, 0);
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/dir/redirected.html"
        );
    }

    #[test]
    fn meta_refresh_parses_url_equals_quoted_target_and_floors_fractional_seconds() {
        let document_url = base_url();
        let navigation = meta_refresh_navigation_from_content(
            "1.25; URL = '../next.html#done'",
            &document_url,
            &document_url,
        )
        .unwrap();
        assert_eq!(navigation.delay_ms, 1_000);
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/next.html#done"
        );
    }

    #[test]
    fn meta_refresh_without_target_reloads_current_document() {
        let document_url = base_url();
        let document_base_url = Url::parse("https://cdn.example.test/assets/").unwrap();
        let navigation =
            meta_refresh_navigation_from_content("0", &document_url, &document_base_url).unwrap();
        assert_eq!(navigation.delay_ms, 0);
        assert_eq!(navigation.url, document_url);
    }

    #[test]
    fn meta_refresh_relative_target_resolves_against_document_base_url() {
        let document_url = Url::parse("about:srcdoc").unwrap();
        let document_base_url = base_url();
        let navigation =
            meta_refresh_navigation_from_content("0;next.html", &document_url, &document_base_url)
                .unwrap();
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/dir/next.html"
        );
    }

    #[test]
    fn srcdoc_meta_refresh_keeps_document_url_separate_from_fallback_base() {
        let document_url = Url::parse("about:srcdoc").unwrap();
        let document_base_url = base_url();

        let reload =
            meta_refresh_navigation_from_content("0", &document_url, &document_base_url).unwrap();
        assert_eq!(reload.url, document_url);

        let relative = meta_refresh_navigation_from_content(
            "0;redirected.html",
            &document_url,
            &document_base_url,
        )
        .unwrap();
        assert_eq!(
            relative.url.as_str(),
            "https://example.test/dir/redirected.html"
        );
    }

    #[test]
    fn document_runtime_finds_top_level_meta_refresh() {
        let document = crate::parser::HtmlParser::SCRIPTING_ENABLED.parse(
            base_url(),
            r#"<!doctype html><head><meta http-equiv="refresh" content="0;redirected.html"></head>"#
                .to_owned(),
        );
        let runtime = DocumentRuntime::from_document(document);
        let navigation = runtime
            .top_level_meta_refresh_navigation()
            .expect("top-level meta refresh navigation");
        assert_eq!(navigation.delay_ms, 0);
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/dir/redirected.html"
        );
    }

    #[test]
    fn sandbox_without_allow_scripts_blocks_meta_refresh_at_creation_time() {
        let document = crate::parser::HtmlParser::SCRIPTING_ENABLED.parse(
            base_url(),
            r#"<!doctype html><head><meta http-equiv="refresh" content="0;redirected.html"></head>"#
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::from_document(document);
        runtime.set_response_content_security_policies(&[String::from("sandbox")]);

        assert!(
            runtime
                .finish_top_level_meta_refresh_load(
                    super::runtime_core::test_stylesheet_document_owner()
                )
                .is_none(),
            "the sandboxed automatic-features flag must reject the refresh when it is discovered"
        );
    }

    #[test]
    fn document_runtime_preserves_delayed_top_level_meta_refresh() {
        let document = crate::parser::HtmlParser::SCRIPTING_ENABLED.parse(
            base_url(),
            r#"<!doctype html><head><meta http-equiv="refresh" content="1.5;redirected.html"></head>"#
                .to_owned(),
        );
        let runtime = DocumentRuntime::from_document(document);
        let navigation = runtime
            .top_level_meta_refresh_navigation()
            .expect("delayed top-level meta refresh navigation");
        assert_eq!(navigation.delay_ms, 1_000);
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/dir/redirected.html"
        );
    }

    #[test]
    fn scheduler_keeps_the_earliest_delay_and_replaces_equal_candidates() {
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([
            MetaRefreshNavigation {
                delay_ms: 500,
                url: Url::parse("https://example.test/first").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 1_000,
                url: Url::parse("https://example.test/ignored").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 200,
                url: Url::parse("https://example.test/earlier").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 200,
                url: Url::parse("https://example.test/equal-and-later").unwrap(),
            },
        ]));
        let scheduled = scheduler
            .finish_load(super::runtime_core::test_stylesheet_document_owner())
            .expect("accepted candidate should schedule after load");
        assert_eq!(scheduled.navigation.delay_ms, 200);
        assert_eq!(
            scheduled.navigation.url.as_str(),
            "https://example.test/equal-and-later"
        );
    }

    #[test]
    fn scheduler_rediscovery_of_an_identical_candidate_preserves_the_armed_deadline() {
        let owner = super::runtime_core::test_stylesheet_document_owner();
        let navigation = MetaRefreshNavigation {
            delay_ms: 60_000,
            url: Url::parse("https://example.test/final").unwrap(),
        };
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([navigation.clone()]));
        let original = scheduler
            .finish_load(owner)
            .expect("load should arm the original refresh");

        assert!(
            !scheduler.note_candidates([navigation.clone()]),
            "rediscovering the same parsed candidate must be a no-op"
        );
        assert_eq!(scheduler.ready_at, Some(original.ready_at));
        assert_eq!(scheduler.armed_owner, Some(owner));
        assert!(
            scheduler.schedule_for_loaded_owner().is_none(),
            "an identical candidate must not post a replacement payload"
        );
        assert!(scheduler.consume_if_matches(owner, navigation.delay_ms, &navigation.url));
    }

    #[test]
    fn scheduler_rebinds_an_armed_nonzero_refresh_without_resetting_its_deadline() {
        let original_owner = super::runtime_core::test_stylesheet_document_owner();
        let replacement_owner = replacement_document_owner();
        let target = Url::parse("https://example.test/final").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 60_000,
            url: target.clone(),
        }]));
        let original = scheduler
            .finish_load(original_owner)
            .expect("load should arm the refresh");

        scheduler.prepare_for_document_open();
        let rebound = scheduler
            .rebind_armed_navigation(replacement_owner)
            .expect("document.open should rebind an active nonzero refresh");

        assert_eq!(rebound.owner, replacement_owner);
        assert_eq!(rebound.ready_at, original.ready_at);
        assert!(!scheduler.consume_if_matches(original_owner, 60_000, &target));
        assert!(scheduler.consume_if_matches(replacement_owner, 60_000, &target));
    }

    #[test]
    fn scheduler_cancels_a_zero_delay_refresh_during_document_open() {
        let owner = super::runtime_core::test_stylesheet_document_owner();
        let target = Url::parse("https://example.test/final").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 0,
            url: target.clone(),
        }]));
        assert!(scheduler.finish_load(owner).is_some());

        scheduler.prepare_for_document_open();

        assert!(
            scheduler
                .rebind_armed_navigation(replacement_document_owner())
                .is_none()
        );
        assert!(!scheduler.consume_if_matches(owner, 0, &target));
    }

    #[test]
    fn scheduler_rejects_a_rebound_payload_after_a_new_candidate_supersedes_it() {
        let original_owner = super::runtime_core::test_stylesheet_document_owner();
        let replacement_owner = replacement_document_owner();
        let old_target = Url::parse("https://example.test/old").unwrap();
        let new_target = Url::parse("https://example.test/new").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 60_000,
            url: old_target.clone(),
        }]));
        assert!(scheduler.finish_load(original_owner).is_some());
        scheduler.prepare_for_document_open();
        assert!(
            scheduler
                .rebind_armed_navigation(replacement_owner)
                .is_some()
        );

        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 1_000,
            url: new_target.clone(),
        }]));
        assert!(
            !scheduler.consume_if_matches(replacement_owner, 60_000, &old_target),
            "a queued payload must prove it still matches scheduler state before navigating"
        );
        let replacement = scheduler
            .finish_load(replacement_owner)
            .expect("the new candidate should arm after the replacement load");
        assert_eq!(replacement.navigation.url, new_target);
        assert_eq!(replacement.navigation.delay_ms, 1_000);
    }

    #[test]
    fn meta_refresh_parser_matches_the_wpt_pragma_directive_matrix() {
        const CURRENT: &str = "__current__";
        let cases: &[(&str, Option<(u32, &str)>)] = &[
            ("", None),
            ("1", Some((1_000, CURRENT))),
            ("1 ", Some((1_000, CURRENT))),
            ("1\t", Some((1_000, CURRENT))),
            ("1\r", Some((1_000, CURRENT))),
            ("1\n", Some((1_000, CURRENT))),
            ("1\u{000c}", Some((1_000, CURRENT))),
            ("1;", Some((1_000, CURRENT))),
            ("1,", Some((1_000, CURRENT))),
            ("1; url=foo", Some((1_000, "foo"))),
            ("1, url=foo", Some((1_000, "foo"))),
            ("1 url=foo", Some((1_000, "foo"))),
            ("1;\turl=foo", Some((1_000, "foo"))),
            ("1,\turl=foo", Some((1_000, "foo"))),
            ("1\turl=foo", Some((1_000, "foo"))),
            ("1;\rurl=foo", Some((1_000, "foo"))),
            ("1,\rurl=foo", Some((1_000, "foo"))),
            ("1\rurl=foo", Some((1_000, "foo"))),
            ("1;\nurl=foo", Some((1_000, "foo"))),
            ("1,\nurl=foo", Some((1_000, "foo"))),
            ("1\nurl=foo", Some((1_000, "foo"))),
            ("1;\u{000c}url=foo", Some((1_000, "foo"))),
            ("1,\u{000c}url=foo", Some((1_000, "foo"))),
            ("1\u{000c}url=foo", Some((1_000, "foo"))),
            ("1url=foo", None),
            ("1x;url=foo", None),
            ("1 x;url=foo", Some((1_000, "x;url=foo"))),
            ("1;;url=foo", Some((1_000, ";url=foo"))),
            ("  1  ;  url  =  foo", Some((1_000, "foo"))),
            ("  1  ,  url  =  foo", Some((1_000, "foo"))),
            ("  1  ;  foo", Some((1_000, "foo"))),
            ("  1  ,  foo", Some((1_000, "foo"))),
            ("  1  url  =  foo", Some((1_000, "foo"))),
            ("1; url=foo ", Some((1_000, "foo"))),
            ("1; url=f\to\no", Some((1_000, "foo"))),
            ("1; url=\"foo\"bar", Some((1_000, "foo"))),
            ("1; url='foo'bar", Some((1_000, "foo"))),
            ("1; url=\"foo'bar", Some((1_000, "foo'bar"))),
            ("1; url foo", Some((1_000, "url foo"))),
            ("1; urlfoo", Some((1_000, "urlfoo"))),
            ("1; urfoo", Some((1_000, "urfoo"))),
            ("1; ufoo", Some((1_000, "ufoo"))),
            ("1; \"foo\"bar", Some((1_000, "foo"))),
            ("; foo", None),
            (";foo", None),
            (", foo", None),
            (",foo", None),
            ("foo", None),
            ("+1; url=foo", None),
            ("-1; url=foo", None),
            ("+0; url=foo", None),
            ("-0; url=foo", None),
            ("0; url=foo", Some((0, "foo"))),
            ("+1; foo", None),
            ("-1; foo", None),
            ("+0; foo", None),
            ("-0; foo", None),
            ("0; foo", Some((0, "foo"))),
            ("+1", None),
            ("-1", None),
            ("+0", None),
            ("-0", None),
            ("0", Some((0, CURRENT))),
            ("1.9; url=foo", Some((1_000, "foo"))),
            ("1.9..5.; url=foo", Some((1_000, "foo"))),
            (".9; url=foo", Some((0, "foo"))),
            ("0.9; url=foo", Some((0, "foo"))),
            ("0...9; url=foo", Some((0, "foo"))),
            ("0...; url=foo", Some((0, "foo"))),
            ("1e0; url=foo", None),
            ("1e1; url=foo", None),
            ("10e-1; url=foo", None),
            ("-0.1; url=foo", None),
        ];
        let document_url = base_url();

        for &(input, expected) in cases {
            let actual = meta_refresh_navigation_from_content(input, &document_url, &document_url);
            match expected {
                None => assert_eq!(actual, None, "input {input:?}"),
                Some((delay_ms, CURRENT)) => {
                    let actual = actual.unwrap_or_else(|| panic!("input {input:?}"));
                    assert_eq!(actual.delay_ms, delay_ms, "input {input:?}");
                    assert_eq!(actual.url, document_url, "input {input:?}");
                }
                Some((delay_ms, target)) => {
                    let actual = actual.unwrap_or_else(|| panic!("input {input:?}"));
                    assert_eq!(actual.delay_ms, delay_ms, "input {input:?}");
                    assert_eq!(
                        actual.url,
                        document_url.join(target).unwrap(),
                        "input {input:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn meta_refresh_rejects_javascript_urls_and_excessive_delays() {
        let document_url = base_url();
        assert_eq!(
            meta_refresh_navigation_from_content(
                "0;url=javascript:globalThis.x=2",
                &document_url,
                &document_url,
            ),
            None
        );
        assert_eq!(
            meta_refresh_navigation_from_content(
                "2147484;url=next.html",
                &document_url,
                &document_url,
            ),
            None
        );
        assert_eq!(
            meta_refresh_navigation_from_content(
                "2147483;url=next.html",
                &document_url,
                &document_url,
            )
            .expect("Blink's maximum whole-second refresh delay remains valid")
            .delay_ms,
            2_147_483_000
        );
    }
}
