use v8::RegExpCreationFlags;

use crate::{
    runtime::{RendererResourceTextSearchOutcome, RendererTextSearchMatch},
    util::v8_string,
};

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn search_text_by_lines(
        &mut self,
        text: &str,
        query: &str,
        case_sensitive: bool,
        is_regex: bool,
    ) -> anyhow::Result<Vec<RendererTextSearchMatch>> {
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(search_text_by_lines_in_scope(
                scope,
                text,
                query,
                case_sensitive,
                is_regex,
            ))
        })
    }

    pub(crate) fn search_child_frame_resource_by_lines(
        &mut self,
        frame_id: &str,
        url: &str,
        query: &str,
        case_sensitive: bool,
        is_regex: bool,
    ) -> anyhow::Result<RendererResourceTextSearchOutcome> {
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let Some(owner_handle) =
                host.child_browsing_context_owner_node_id_by_frame_id(frame_id)
            else {
                return Ok(RendererResourceTextSearchOutcome::FrameNotFound);
            };
            let Some(current_url) = host.child_browsing_context_current_url(owner_handle) else {
                return Ok(RendererResourceTextSearchOutcome::ContentUnavailable);
            };
            if !resource_urls_match(current_url.as_str(), url) {
                return Ok(RendererResourceTextSearchOutcome::ResourceNotFound);
            }
            if current_url.scheme() == "about" {
                return Ok(RendererResourceTextSearchOutcome::ResourceNotFound);
            }
            let Some(snapshot) = host.child_browsing_context_snapshot_markup(owner_handle) else {
                return Ok(RendererResourceTextSearchOutcome::ContentUnavailable);
            };
            let Some(source) = snapshot.content.text() else {
                return Ok(RendererResourceTextSearchOutcome::ContentUnavailable);
            };
            if source.is_empty() && !snapshot.resource_was_cached {
                return Ok(RendererResourceTextSearchOutcome::ContentUnavailable);
            }
            Ok(RendererResourceTextSearchOutcome::Matches(
                search_text_by_lines_in_scope(scope, source, query, case_sensitive, is_regex),
            ))
        })
    }
}

fn search_text_by_lines_in_scope(
    scope: &mut v8::PinScope<'_, '_>,
    text: &str,
    query: &str,
    case_sensitive: bool,
    is_regex: bool,
) -> Vec<RendererTextSearchMatch> {
    if text.is_empty() {
        return Vec::new();
    }

    // Chromium's inspector compiles search regexes in its own V8 context.
    // Keep page-modified globals and prototypes out of protocol search too.
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let pattern = if is_regex {
        query.to_owned()
    } else {
        escaped_search_pattern(query)
    };
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let Some(pattern) = v8_string(&scope, &pattern) else {
        return Vec::new();
    };
    let flags = if case_sensitive {
        RegExpCreationFlags::empty()
    } else {
        RegExpCreationFlags::IGNORE_CASE
    };
    let Some(regex) = v8::RegExp::new(&scope, pattern, flags) else {
        scope.reset();
        return Vec::new();
    };

    let mut matches = Vec::new();
    for (line_number, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let Some(subject) = v8_string(&scope, line) else {
            continue;
        };
        let Some(result) = regex.exec(&scope, subject) else {
            scope.reset();
            return Vec::new();
        };
        if !result.is_null() {
            matches.push(RendererTextSearchMatch {
                line_number,
                line_content: line.to_owned(),
            });
        }
    }
    matches
}

fn escaped_search_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len());
    for ch in query.chars() {
        if matches!(
            ch,
            '[' | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '+'
                | '-'
                | '*'
                | '.'
                | ','
                | '?'
                | '\\'
                | '^'
                | '$'
                | '|'
        ) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

fn resource_urls_match(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        url::Url::parse(value).ok().map(|mut url| {
            url.set_fragment(None);
            url
        })
    };
    match (normalize(left), normalize(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_search_escapes_the_same_metacharacters_as_v8_inspector() {
        assert_eq!(
            escaped_search_pattern("[](){}+-*.,?\\^$|/"),
            "\\[\\]\\(\\)\\{\\}\\+\\-\\*\\.\\,\\?\\\\\\^\\$\\|/"
        );
    }

    #[test]
    fn resource_url_identity_ignores_fragments() {
        assert!(resource_urls_match(
            "https://example.test/page#one",
            "https://example.test/page#two"
        ));
        assert!(!resource_urls_match(
            "https://example.test/page",
            "https://example.test/other"
        ));
    }
}
