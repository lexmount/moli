//! Read JSON-LD responses from a completed destination Document.

use moli_dom::native::{DomHost, NativeNodeId};

/// Collect parseable JSON-LD scripts in document order.
///
/// Legacy JSON comments are accepted for Chromium compatibility.
pub fn navigation_result(dom: &DomHost, document: NativeNodeId) -> String {
    let mut results = Vec::<serde_json::Value>::new();
    for handle in dom.script_handles_in_light_subtree(document) {
        if dom.is_html_element_named(handle, "script")
            && dom
                .node(handle)
                .and_then(|node| node.as_element())
                .and_then(|element| element.attribute("type"))
                == Some("application/ld+json")
            && let Some(text) = dom.text_content(handle)
            && let Ok(value) = serde_json::from_str(&strip_json_comments(&text))
        {
            results.push(value);
        }
    }
    serde_json::to_string(&results).expect("JSON-LD result")
}

// Chromium's legacy JSON-LD parser accepts comments, but JSON strings must
// retain literal comment markers and escaped quotes.
fn strip_json_comments(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => {
                index += 2;
                continue;
            }
            b'"' => quoted = !quoted,
            b'/' if !quoted && bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    bytes[index] = b' ';
                    index += 1;
                }
                continue;
            }
            b'/' if !quoted && bytes.get(index + 1) == Some(&b'*') => {
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    bytes[index] = b' ';
                    index += 1;
                }
                if index + 1 >= bytes.len() {
                    return String::new();
                }
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    String::from_utf8(bytes).expect("comment replacement preserves UTF-8")
}
