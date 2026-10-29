use super::*;
use std::time::Instant;

use crate::meta_refresh_scheduler::ScheduledMetaRefreshNavigation;

pub(super) struct MetaRefreshMutationCandidate {
    pub(super) document: DomHandle,
    pub(super) navigation: MetaRefreshNavigation,
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
                self.revision,
            ),
        );
        (task, self.ready_at)
    }
}

impl DocumentRuntime {
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

    pub(crate) fn take_top_level_meta_refresh_navigation(
        &mut self,
        owner: FrameDocumentTaskOwner,
        revision: u64,
    ) -> Option<MetaRefreshNavigation> {
        self.meta_refresh_scheduler.take_if_current(owner, revision)
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

    pub(crate) fn note_parser_meta_refresh_mutation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        effects: &DomMutationEffects,
    ) {
        let candidates =
            meta_refresh_navigations_from_mutation(self.dom_host(), effects, self.document_url());
        self.finish_meta_refresh_mutation_candidates(scope, host_ptr, candidates);
    }

    pub(super) fn finish_meta_refresh_mutation_candidates(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        candidates: Vec<MetaRefreshMutationCandidate>,
    ) {
        let mut top_level_refreshes = Vec::new();
        let mut child_refreshes = Vec::new();
        for candidate in candidates {
            if candidate.document == self.document_handle() {
                top_level_refreshes.push(candidate.navigation);
            } else {
                child_refreshes.push(candidate);
            }
        }
        if let Some(scheduled) = self.note_top_level_meta_refresh_candidates(top_level_refreshes) {
            let delay_ms = scheduled.navigation.delay_ms;
            let url = scheduled.navigation.url.clone();
            let owner = scheduled.owner;
            let (task, ready_at) = scheduled.into_internal_loading_task();
            tracing::debug!(
                owner = ?owner,
                %url,
                delay_ms,
                ?ready_at,
                "rescheduled top-level meta refresh after a DOM mutation"
            );
            if unsafe { &*host_ptr }
                .page_internal_loading_sender()
                .schedule_at(task, ready_at)
                .is_err()
            {
                tracing::debug!("dropped meta refresh because its Page source closed");
            }
        }
        for candidate in child_refreshes {
            unsafe { &mut *host_ptr }.note_child_meta_refresh_after_mutation(
                scope,
                self,
                candidate.document,
                candidate.navigation,
            );
        }
    }

    pub(crate) fn rebind_top_level_meta_refresh_after_document_open(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        self.meta_refresh_scheduler.rebind_armed_navigation(owner)
    }
}

pub(super) fn meta_refresh_navigations_from_mutation(
    dom_host: &DomHost,
    effects: &DomMutationEffects,
    document_url: &Url,
) -> Vec<MetaRefreshMutationCandidate> {
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
        if element.is_html_element("meta")
            && matches!(mutation.local_name(), "content" | "http-equiv")
            && !handles.contains(&mutation.target())
        {
            handles.push(mutation.target());
        }
    }
    if handles.is_empty() {
        return Vec::new();
    }
    handles
        .into_iter()
        .filter_map(|handle| {
            let document = dom_host.owner_document_handle(handle)?;
            if !dom_host.is_connected(handle) || dom_host.root_node_handle(handle) != Some(document)
            {
                return None;
            }
            let (owner_url, owner_base_url) = if document == document_handle {
                (document_url, dom_host.document_base_url())
            } else {
                (
                    dom_host
                        .document_url_for_handle(document)
                        .unwrap_or(document_url),
                    dom_host.document_base_url_for_handle(document),
                )
            };
            let owner_base_url = owner_base_url.unwrap_or_else(|| owner_url.clone());
            let navigation =
                meta_refresh_navigation_for_element(dom_host, handle, owner_url, &owner_base_url)?;
            Some(MetaRefreshMutationCandidate {
                document,
                navigation,
            })
        })
        .collect()
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
        let mut runtime = DocumentRuntime::from_document(document);
        let navigation = runtime
            .finish_top_level_meta_refresh_load(
                super::runtime_core::test_stylesheet_document_owner(),
            )
            .expect("top-level meta refresh navigation")
            .navigation;
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
        let mut runtime = DocumentRuntime::from_document(document);
        let navigation = runtime
            .finish_top_level_meta_refresh_load(
                super::runtime_core::test_stylesheet_document_owner(),
            )
            .expect("delayed top-level meta refresh navigation")
            .navigation;
        assert_eq!(navigation.delay_ms, 1_000);
        assert_eq!(
            navigation.url.as_str(),
            "https://example.test/dir/redirected.html"
        );
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
