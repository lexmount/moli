use anyhow::{Result, anyhow};
use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
        states::{Rawtext, Rcdata, ScriptData},
    },
};
use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WptStaticResourceReference {
    pub path: String,
    pub suffix: String,
}

impl WptStaticResourceReference {
    pub(crate) fn path_with_suffix(&self) -> String {
        format!("{}{}", self.path, self.suffix)
    }
}

pub(crate) fn extract_wpt_meta_script_references(source: &str) -> Vec<String> {
    let mut references = Vec::new();
    for line in source.lines() {
        let Some(meta) = strip_wpt_meta_prefix(line) else {
            break;
        };
        let meta = meta.trim_start();
        if let Some(reference) = meta.strip_prefix("script=").map(str::trim)
            && !reference.is_empty()
        {
            references.push(reference.to_owned());
        }
    }
    references
}

pub(crate) fn extract_wpt_meta_global_values(source: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in source.lines() {
        let Some(meta) = strip_wpt_meta_prefix(line) else {
            break;
        };
        let meta = meta.trim_start();
        if let Some(value) = meta.strip_prefix("global=").map(str::trim)
            && !value.is_empty()
        {
            values.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|global| !global.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    values
}

fn strip_wpt_meta_prefix(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix("//")?.trim_start();
    line.strip_prefix("META:")
}

pub(crate) fn extract_wpt_html_static_references(source: &str) -> Vec<String> {
    if source.is_empty() {
        return Vec::new();
    }
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(source));
    let tokenizer = Tokenizer::new(
        WptHtmlStaticReferenceSink::default(),
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.into_references()
}

pub(crate) fn extract_wpt_js_worker_constructor_references(source: &str) -> Vec<String> {
    extract_wpt_js_constructor_references(source, b"Worker")
}

pub(crate) fn extract_wpt_js_shared_worker_constructor_references(source: &str) -> Vec<String> {
    extract_wpt_js_constructor_references(source, b"SharedWorker")
}

fn extract_wpt_js_constructor_references(source: &str, constructor: &[u8]) -> Vec<String> {
    let mut references = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_at(bytes, index, b"//") {
            index = skip_line_comment(bytes, index + 2);
            continue;
        }
        if starts_with_at(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index + 2);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            index = skip_js_string_like(bytes, index);
            continue;
        }
        if !is_identifier_boundary_before(bytes, index) || !starts_with_at(bytes, index, b"new") {
            index += 1;
            continue;
        }

        let mut cursor = index + 3;
        if !is_identifier_boundary_after(bytes, cursor) {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor);
        if !starts_with_at(bytes, cursor, constructor) {
            index += 1;
            continue;
        }
        cursor += constructor.len();
        if !is_identifier_boundary_after(bytes, cursor) {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(cursor) != Some(&b'(') {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor + 1);
        let (reference, end) = match bytes.get(cursor).copied() {
            Some(quote @ (b'\'' | b'"')) => {
                let Some((reference, end)) = read_js_quoted_string(&bytes[cursor + 1..], quote)
                else {
                    index += 1;
                    continue;
                };
                (reference, cursor + 1 + end)
            }
            Some(byte) if byte.is_ascii_alphanumeric() => {
                let Some((reference, end)) = read_static_unquoted_resource_argument(bytes, cursor)
                else {
                    index += 1;
                    continue;
                };
                (reference, end)
            }
            _ => {
                index += 1;
                continue;
            }
        };
        let delimiter = skip_ascii_whitespace(bytes, end);
        if matches!(bytes.get(delimiter), Some(b',' | b')')) && !reference.trim().is_empty() {
            references.push(reference);
        }
        index = end;
    }
    references
}

pub(crate) fn extract_wpt_js_import_scripts_references(source: &str) -> Vec<String> {
    let mut references = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_at(bytes, index, b"//") {
            index = skip_line_comment(bytes, index + 2);
            continue;
        }
        if starts_with_at(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index + 2);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            index = skip_js_string_like(bytes, index);
            continue;
        }
        if !is_identifier_boundary_before(bytes, index)
            || !starts_with_at(bytes, index, b"importScripts")
        {
            index += 1;
            continue;
        }

        let mut cursor = index + "importScripts".len();
        if !is_identifier_boundary_after(bytes, cursor) {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(cursor) != Some(&b'(') {
            index += 1;
            continue;
        }
        let (arguments, end) = read_import_scripts_static_arguments(bytes, cursor + 1);
        for reference in arguments {
            if !reference.trim().is_empty() {
                references.push(reference);
            }
        }
        index = end;
    }
    references
}

pub(crate) fn extract_wpt_js_new_url_references(source: &str) -> Vec<String> {
    let mut references = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_at(bytes, index, b"//") {
            index = skip_line_comment(bytes, index + 2);
            continue;
        }
        if starts_with_at(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index + 2);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            index = skip_js_string_like(bytes, index);
            continue;
        }
        if !is_identifier_boundary_before(bytes, index) || !starts_with_at(bytes, index, b"new") {
            index += 1;
            continue;
        }

        let mut cursor = index + 3;
        if !is_identifier_boundary_after(bytes, cursor) {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor);
        if !starts_with_at(bytes, cursor, b"URL") {
            index += 1;
            continue;
        }
        cursor += 3;
        if !is_identifier_boundary_after(bytes, cursor) {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(cursor) != Some(&b'(') {
            index += 1;
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor + 1);
        let Some((&quote @ (b'\'' | b'"'), rest)) = bytes[cursor..].split_first() else {
            index += 1;
            continue;
        };
        let Some((reference, end)) = read_js_quoted_string(rest, quote) else {
            index += 1;
            continue;
        };
        let delimiter = skip_ascii_whitespace(bytes, cursor + 1 + end);
        if matches!(bytes.get(delimiter), Some(b',' | b')')) && !reference.trim().is_empty() {
            references.push(reference);
        }
        index = cursor + 1 + end;
    }
    references
}

fn read_import_scripts_static_arguments(bytes: &[u8], mut index: usize) -> (Vec<String>, usize) {
    let mut references = Vec::new();
    while index < bytes.len() {
        index = skip_ascii_whitespace(bytes, index);
        match bytes.get(index).copied() {
            Some(b')') => return (references, index + 1),
            Some(b',') => {
                index += 1;
            }
            Some(quote @ (b'\'' | b'"')) => {
                if let Some((reference, consumed)) =
                    read_js_quoted_string(&bytes[index + 1..], quote)
                {
                    index += 1 + consumed;
                    let delimiter = skip_ascii_whitespace(bytes, index);
                    if matches!(bytes.get(delimiter), Some(b',' | b')')) {
                        references.push(reference);
                    } else {
                        index = skip_js_argument_expression(bytes, index);
                    }
                } else {
                    return (references, bytes.len());
                }
            }
            Some(byte) if byte.is_ascii_alphanumeric() => {
                if let Some((reference, end)) = read_static_unquoted_resource_argument(bytes, index)
                {
                    index = end;
                    let delimiter = skip_ascii_whitespace(bytes, index);
                    if matches!(bytes.get(delimiter), Some(b',' | b')')) {
                        references.push(reference);
                    } else {
                        index = skip_js_argument_expression(bytes, index);
                    }
                } else {
                    index = skip_js_argument_expression(bytes, index);
                }
            }
            Some(_) => {
                index = skip_js_argument_expression(bytes, index);
            }
            None => return (references, bytes.len()),
        }
    }
    (references, bytes.len())
}

fn read_static_unquoted_resource_argument(
    bytes: &[u8],
    mut index: usize,
) -> Option<(String, usize)> {
    let start = index;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'$'))
    {
        index += 1;
    }
    let token = std::str::from_utf8(&bytes[start..index]).ok()?;
    if matches!(token, "Infinity" | "NaN" | "undefined" | "null")
        || token.bytes().all(|byte| byte.is_ascii_digit())
    {
        Some((token.to_owned(), index))
    } else {
        None
    }
}

fn skip_js_argument_expression(bytes: &[u8], mut index: usize) -> usize {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    while index < bytes.len() {
        if starts_with_at(bytes, index, b"//") {
            index = skip_line_comment(bytes, index + 2);
            continue;
        }
        if starts_with_at(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index + 2);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            index = skip_js_string_like(bytes, index);
            continue;
        }
        match bytes[index] {
            b'(' => paren_depth = paren_depth.saturating_add(1),
            b'[' => bracket_depth = bracket_depth.saturating_add(1),
            b'{' => brace_depth = brace_depth.saturating_add(1),
            b')' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => return index,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => return index,
            _ => {}
        }
        index += 1;
    }
    index
}

fn starts_with_at(haystack: &[u8], index: usize, needle: &[u8]) -> bool {
    haystack
        .get(index..)
        .is_some_and(|tail| tail.starts_with(needle))
}

fn skip_line_comment(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    while index + 1 < bytes.len() {
        if starts_with_at(bytes, index, b"*/") {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_js_string_like(bytes: &[u8], mut index: usize) -> usize {
    let quote = bytes[index];
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn is_identifier_boundary_before(bytes: &[u8], index: usize) -> bool {
    index == 0 || !is_js_identifier_byte(bytes[index - 1])
}

fn is_identifier_boundary_after(bytes: &[u8], index: usize) -> bool {
    index >= bytes.len() || !is_js_identifier_byte(bytes[index])
}

fn is_js_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn read_js_quoted_string(bytes: &[u8], quote: u8) -> Option<(String, usize)> {
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte == quote => return Some((output, index + 1)),
            b'\\' => {
                index += 1;
                if index >= bytes.len() {
                    return None;
                }
                output.push(bytes[index] as char);
                index += 1;
            }
            byte => {
                output.push(byte as char);
                index += 1;
            }
        }
    }
    None
}

pub(crate) fn resolve_wpt_static_resource_reference(
    base_path: &str,
    reference: &str,
    absolute_reference_prefix: &str,
) -> Result<Option<WptStaticResourceReference>> {
    if is_external_resource_reference(reference) {
        return Ok(None);
    }

    let (reference_path, suffix) = split_resource_reference(reference);
    if reference_path.is_empty() {
        return Ok(None);
    }

    let candidate = if let Some(root_relative) = reference_path.strip_prefix('/') {
        let mut path = PathBuf::new();
        let prefix = absolute_reference_prefix.trim_matches('/');
        if !prefix.is_empty() {
            path.push(prefix);
        }
        path.push(root_relative);
        path
    } else {
        let mut path = Path::new(base_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        path.push(reference_path);
        path
    };

    let path = normalize_root_relative_path(&candidate)?;
    Ok(Some(WptStaticResourceReference {
        path,
        suffix: suffix.to_owned(),
    }))
}

#[derive(Default)]
struct WptHtmlStaticReferenceSink {
    references: RefCell<Vec<String>>,
}

impl WptHtmlStaticReferenceSink {
    fn into_references(self) -> Vec<String> {
        self.references.into_inner()
    }

    fn collect_src_reference(&self, tag: &Tag) {
        let Some(src) = tag
            .attrs
            .iter()
            .find(|attr| AsRef::<str>::as_ref(&attr.name.local).eq_ignore_ascii_case("src"))
            .map(|attr| attr.value.to_string())
        else {
            return;
        };
        if src.trim().is_empty() {
            return;
        }
        self.references.borrow_mut().push(src);
    }
}

impl TokenSink for WptHtmlStaticReferenceSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }

        match AsRef::<str>::as_ref(&tag.name) {
            "script" => {
                self.collect_src_reference(&tag);
                TokenSinkResult::RawData(ScriptData)
            }
            "iframe" => {
                self.collect_src_reference(&tag);
                TokenSinkResult::RawData(Rawtext)
            }
            "noscript" | "style" | "xmp" | "noembed" | "noframes" => {
                TokenSinkResult::RawData(Rawtext)
            }
            "title" | "textarea" => TokenSinkResult::RawData(Rcdata),
            "plaintext" => TokenSinkResult::Plaintext,
            _ => TokenSinkResult::Continue,
        }
    }
}

fn is_external_resource_reference(reference: &str) -> bool {
    let trimmed = reference.trim_start();
    if trimmed.starts_with("//") {
        return true;
    }

    let first_separator = trimmed.find(['/', '?', '#']).unwrap_or(trimmed.len());
    trimmed[..first_separator].contains(':')
}

fn split_resource_reference(reference: &str) -> (&str, &str) {
    let index = reference.find(['?', '#']).unwrap_or(reference.len());
    reference.split_at(index)
}

fn normalize_root_relative_path(path: &Path) -> Result<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(anyhow!(
                        "WPT static resource dependency escapes root: {}",
                        path.display()
                    ));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!(
                    "WPT static resource dependency must stay root-relative: {}",
                    path.display()
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(anyhow!("empty WPT static resource dependency"));
    }

    let mut parts = Vec::new();
    for component in normalized.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_str().ok_or_else(|| {
                    anyhow!("WPT static resource path contains non-UTF-8 segment")
                })?;
                parts.push(segment.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("WPT static resource path is not root-relative"));
            }
        }
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_meta_script_references() {
        assert_eq!(
            extract_wpt_meta_script_references(
                "// META: title=Blob\n// META: script=../support/Blob.js\n'use strict';\n",
            ),
            ["../support/Blob.js"]
        );
    }

    #[test]
    fn extracts_meta_without_space_after_comment_marker() {
        assert_eq!(
            extract_wpt_meta_global_values("//META: global=worker\n"),
            ["worker"]
        );
        assert_eq!(
            extract_wpt_meta_script_references("//META: script=../support/helper.js\n"),
            ["../support/helper.js"]
        );
    }

    #[test]
    fn extracts_html_static_references() {
        assert_eq!(
            extract_wpt_html_static_references(
                r#"
<!doctype html>
<script src="/resources/testharness.js"></script>
<script src='resources/helper.js'></script>
<script async src=../support/fixture.js></script>
<script>const ignored = '<script src="late.js">';</script>
<iframe src=frame.xml></iframe>
"#,
            ),
            [
                "/resources/testharness.js",
                "resources/helper.js",
                "../support/fixture.js",
                "frame.xml"
            ]
        );
    }

    #[test]
    fn extracts_static_worker_constructor_references() {
        assert_eq!(
            extract_wpt_js_worker_constructor_references(
                r#"
new Worker("worker.js");
new Worker('module-worker.js?pipe=sub', { type: "module" });
const ignoredString = "new Worker('ignored-string.js')";
// new Worker("ignored-line-comment.js")
/* new Worker("ignored-block-comment.js") */
new window.Worker("ignored-qualified.js");
new Worker(dynamicName);
new Worker("ignored-prefix.js" + suffix);
new Worker(undefined);
new Worker(null);
new Worker(1);
new Worker(Infinity);
new Worker(NaN);
"#,
            ),
            [
                "worker.js",
                "module-worker.js?pipe=sub",
                "undefined",
                "null",
                "1",
                "Infinity",
                "NaN"
            ]
        );
    }

    #[test]
    fn extracts_static_shared_worker_constructor_references() {
        assert_eq!(
            extract_wpt_js_shared_worker_constructor_references(
                r#"
new SharedWorker("shared-worker.js");
new SharedWorker('module-shared-worker.js?pipe=sub', { type: "module" });
const ignoredString = "new SharedWorker('ignored-string.js')";
// new SharedWorker("ignored-line-comment.js")
/* new SharedWorker("ignored-block-comment.js") */
new window.SharedWorker("ignored-qualified.js");
new SharedWorker(dynamicName);
new SharedWorker("ignored-prefix.js" + suffix);
new SharedWorker(undefined);
new SharedWorker(null);
new SharedWorker(1);
new SharedWorker(Infinity);
new SharedWorker(NaN);
"#,
            ),
            [
                "shared-worker.js",
                "module-shared-worker.js?pipe=sub",
                "undefined",
                "null",
                "1",
                "Infinity",
                "NaN"
            ]
        );
    }

    #[test]
    fn extracts_static_import_scripts_references() {
        assert_eq!(
            extract_wpt_js_import_scripts_references(
                r#"
importScripts("first.js", 'second.js?pipe=sub');
self.importScripts(undefined, null, 1);
importScripts(dynamicName, "after-dynamic.js");
importScripts("ignored-prefix.js" + suffix);
const ignoredString = "importScripts('ignored-string.js')";
// importScripts("ignored-line-comment.js")
/* importScripts("ignored-block-comment.js") */
importScripts(`ignored-template.js`);
"#,
            ),
            [
                "first.js",
                "second.js?pipe=sub",
                "undefined",
                "null",
                "1",
                "after-dynamic.js"
            ]
        );
    }

    #[test]
    fn extracts_static_new_url_references() {
        assert_eq!(
            extract_wpt_js_new_url_references(
                r#"
new URL("support/frame.html", location);
new URL('/workers/support/helper.js', location.href);
const ignoredString = "new URL('ignored-string.js', location)";
// new URL("ignored-line-comment.js", location)
/* new URL("ignored-block-comment.js", location) */
new URL(dynamicName, location);
"#,
            ),
            ["support/frame.html", "/workers/support/helper.js"]
        );
    }

    #[test]
    fn stops_extracting_meta_script_references_after_leading_meta_block() {
        assert_eq!(
            extract_wpt_meta_script_references(
                "// META: script=../support/first.js\n'use strict';\n// META: script=../support/late.js\n",
            ),
            ["../support/first.js"]
        );
    }

    #[test]
    fn resolves_relative_meta_script_reference() -> Result<()> {
        let resolved = resolve_wpt_static_resource_reference(
            "FileAPI/blob/Blob-constructor.any.js",
            "../support/Blob.js",
            "",
        )?
        .expect("relative helper should resolve");

        assert_eq!(resolved.path, "FileAPI/support/Blob.js");
        assert_eq!(resolved.suffix, "");
        Ok(())
    }

    #[test]
    fn resolves_root_relative_meta_script_reference_with_prefix() -> Result<()> {
        let resolved = resolve_wpt_static_resource_reference(
            "upstream/html/dom/basic.any.js",
            "/common/gc.js?variant=1",
            "upstream",
        )?
        .expect("root-relative helper should resolve");

        assert_eq!(resolved.path, "upstream/common/gc.js");
        assert_eq!(resolved.suffix, "?variant=1");
        Ok(())
    }

    #[test]
    fn ignores_external_meta_script_references() -> Result<()> {
        assert_eq!(
            resolve_wpt_static_resource_reference(
                "upstream/url/basic.any.js",
                "https://example.test/helper.js",
                "upstream",
            )?,
            None
        );
        assert_eq!(
            resolve_wpt_static_resource_reference(
                "upstream/url/basic.any.js",
                "//example.test/helper.js",
                "upstream",
            )?,
            None
        );
        Ok(())
    }

    #[test]
    fn rejects_escaping_meta_script_references() {
        let error =
            resolve_wpt_static_resource_reference("url/basic.any.js", "../../outside.js", "")
                .expect_err("escaping helper should fail");
        assert!(error.to_string().contains("escapes root"));
    }
}
