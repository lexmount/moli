use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::HashMap;

fn utf16_css_bytes(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
    let mut bytes = if bom {
        if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        }
    } else {
        Vec::new()
    };
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    bytes
}

#[tokio::test(flavor = "current_thread")]
async fn css_stylesheet_decoding_uses_raw_bytes_across_owner_and_import_paths() {
    let mut observations = Vec::new();
    for document_encoding in ["UTF-8", "windows-1252"] {
        let legacy = document_encoding == "windows-1252";
        let fallback = if legacy {
            encoding_rs::WINDOWS_1252
        } else {
            encoding_rs::UTF_8
        };
        let encode = |text: &str, encoding: &'static encoding_rs::Encoding| {
            let (bytes, _, errors) = encoding.encode(text);
            assert!(!errors, "fixture must represent its CSS text exactly");
            bytes.into_owned()
        };
        let css = |name: &str, value: &str| format!("#{name} {{ --decoded: \"{value}\"; }}");
        let mut cases: Vec<(&str, Vec<u8>, &str, &str, Option<&str>)> = Vec::new();
        cases.push((
            "utf8",
            css("utf8", "æøåの").into_bytes(),
            "text/css; charset=utf-8",
            "æøåの",
            None,
        ));
        let mut utf8_bom = vec![0xef, 0xbb, 0xbf];
        utf8_bom.extend_from_slice(css("utf8-bom", "æøåの").as_bytes());
        cases.push((
            "utf8-bom",
            utf8_bom,
            "text/css; charset=windows-1252",
            "æøåの",
            None,
        ));
        cases.push((
            "utf16be",
            utf16_css_bytes(&css("utf16be", "æøåの"), true, true),
            "text/css; charset=utf-8",
            "æøåの",
            None,
        ));
        cases.push((
            "utf16le",
            utf16_css_bytes(&css("utf16le", "æøåの"), false, true),
            "text/css; charset=windows-1252",
            "æøåの",
            None,
        ));
        cases.push((
            "http-latin",
            encode(&css("http-latin", "€æ"), encoding_rs::WINDOWS_1252),
            "text/css; charset=windows-1252",
            "€æ",
            None,
        ));
        cases.push((
            "http-japanese",
            encode(&css("http-japanese", "目次"), encoding_rs::SHIFT_JIS),
            "text/css; charset=shift_jis",
            "目次",
            None,
        ));
        cases.push((
            "charset-latin",
            encode(
                &format!("@charset \"windows-1252\"; {}", css("charset-latin", "€æ")),
                encoding_rs::WINDOWS_1252,
            ),
            "text/css",
            "€æ",
            None,
        ));
        cases.push((
            "charset-japanese",
            encode(
                &format!(
                    "@charset \"shift_jis\"; {}",
                    css("charset-japanese", "目次")
                ),
                encoding_rs::SHIFT_JIS,
            ),
            "text/css",
            "目次",
            None,
        ));
        cases.push((
            "http-before-charset",
            encode(
                &format!(
                    "@charset \"shift_jis\"; {}",
                    css("http-before-charset", "€æ")
                ),
                encoding_rs::WINDOWS_1252,
            ),
            "text/css; charset=windows-1252",
            "€æ",
            None,
        ));
        cases.push((
            "charset-utf16",
            format!("@charset \"utf-16be\"; {}", css("charset-utf16", "æøåの")).into_bytes(),
            "text/css",
            "æøåの",
            None,
        ));
        cases.push((
            "environment",
            encode(&css("environment", "€"), fallback),
            "text/css",
            "€",
            None,
        ));
        cases.push((
            "link-charset",
            encode(&css("link-charset", "目次"), encoding_rs::SHIFT_JIS),
            "text/css",
            "目次",
            Some("shift_jis"),
        ));
        cases.push((
            "invalid-link-charset",
            encode(&css("invalid-link-charset", "€"), fallback),
            "text/css",
            "€",
            Some("not-an-encoding"),
        ));
        cases.push((
            "comment-before-charset",
            encode(
                &format!(
                    "/**/@charset \"shift_jis\"; {}",
                    css("comment-before-charset", "€")
                ),
                fallback,
            ),
            "text/css",
            "€",
            None,
        ));
        cases.push((
            "single-quote-charset",
            encode(
                &format!("@charset 'shift_jis'; {}", css("single-quote-charset", "€")),
                fallback,
            ),
            "text/css",
            "€",
            None,
        ));
        cases.push((
            "overlong-charset",
            encode(
                &format!(
                    "@charset \"{}utf-8\"; {}",
                    " ".repeat(1024),
                    css("overlong-charset", "€")
                ),
                fallback,
            ),
            "text/css",
            "€",
            None,
        ));
        cases.push((
            "bom-graph",
            utf16_css_bytes(
                &format!(
                    "@import url('/bom-child.css'); {}",
                    css("bom-graph", "æøåの")
                ),
                true,
                true,
            ),
            "text/css; charset=utf-8",
            "æøåの",
            None,
        ));
        cases.push((
            "japanese-graph",
            encode(
                &format!(
                    "@charset \"shift_jis\"; @import url('/japanese-child.css'); {}",
                    css("japanese-graph", "目次")
                ),
                encoding_rs::SHIFT_JIS,
            ),
            "text/css",
            "目次",
            None,
        ));
        let descriptors: Vec<_> = cases.iter().map(|(name, _, _, value, charset)| {
            serde_json::json!({"name": name, "expected": format!("\"{value}\""), "charset": charset})
        }).collect();
        let expected_checks = cases.len() * 2 + 7;
        let raw_length = cases
            .iter()
            .find(|case| case.0 == "utf16be")
            .unwrap()
            .1
            .len();
        let mut responses: HashMap<String, (Vec<u8>, String)> = cases
            .into_iter()
            .map(|(name, bytes, mime, _, _)| (format!("/{name}.css"), (bytes, mime.to_owned())))
            .collect();
        responses.insert(
            "/bom-child.css".to_owned(),
            (
                utf16_css_bytes(
                    "@import url('/bom-grandchild.css'); #bom-graph { --child: \"æøåの\"; }",
                    true,
                    false,
                ),
                "text/css".to_owned(),
            ),
        );
        responses.insert(
            "/bom-grandchild.css".to_owned(),
            (
                "#bom-graph { --grandchild: \"æøåの\"; }"
                    .as_bytes()
                    .to_vec(),
                "text/css; charset=utf-8".to_owned(),
            ),
        );
        let data_css = encode(
            "#japanese-graph { --data: \"目次\"; }",
            encoding_rs::SHIFT_JIS,
        );
        let japanese_child = format!(
            "@import url(data:text/css;base64,{}); #japanese-graph {{ --child: \"目次\"; }}",
            STANDARD.encode(data_css)
        );
        responses.insert(
            "/japanese-child.css".to_owned(),
            (
                encode(&japanese_child, encoding_rs::SHIFT_JIS),
                "text/css".to_owned(),
            ),
        );
        responses.insert(
            "/child-bom.css".to_owned(),
            (
                utf16_css_bytes("#child-target { --decoded: \"æøåの\"; }", false, true),
                "text/css; charset=utf-8".to_owned(),
            ),
        );
        let request_count = responses.len() + 1;
        let server = StaticHttpServer::spawn_with_raw_responder(request_count, move |_, request| {
            let (bytes, mime) = responses.get(&request.target).unwrap_or_else(|| panic!("unexpected stylesheet request {}", request.target));
            let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).into_bytes();
            response.extend_from_slice(bytes);
            response
        }).await;
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(
            server.base_url().join("/page.html").unwrap().as_str(),
            &loader,
        );
        vm.document_runtime
            .set_document_character_set(document_encoding);
        vm.eval(&format!(
            "globalThis.decodeCases = {}; globalThis.expectedRawLength = {raw_length};",
            serde_json::to_string(&descriptors).unwrap()
        ))
        .unwrap();
        vm.eval(r#"
globalThis.decodeDone = false;
globalThis.decodeEvents = {};
const loads = decodeCases.map(({name, charset}) => {
  const target = document.body.appendChild(document.createElement('div'));
  target.id = name;
  const link = document.createElement('link');
  link.rel = 'stylesheet'; link.href = '/' + name + '.css';
  if (charset !== null) link.charset = charset;
  const done = new Promise(resolve => {
    link.onload = () => { decodeEvents[name] = 'load'; resolve(); };
    link.onerror = () => { decodeEvents[name] = 'error'; resolve(); };
  });
  document.head.appendChild(link);
  return done;
});
const frame = document.createElement('iframe');
const frameDone = new Promise(resolve => { frame.onload = resolve; });
frame.srcdoc = '<!doctype html><link rel="stylesheet" href="/child-bom.css"><div id="child-target"></div>';
document.body.appendChild(frame);
Promise.all([...loads, frameDone]).then(() => fetch('/utf16be.css')).then(response => {
  const copy = response.clone();
  return Promise.all([response.arrayBuffer(), copy.text()]);
}).then(([bytes, text]) => {
  globalThis.decodeRaw = Array.from(new Uint8Array(bytes));
  globalThis.decodeFetchText = text;
  globalThis.decodeFrame = frame;
  globalThis.decodeDone = true;
}, error => { globalThis.decodeError = String(error); globalThis.decodeDone = true; });
"#).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(decodeDone)",
            "true",
            "external CSS byte decoding",
        )
        .await;
        let result = vm.eval(r#"
(() => {
  let checks = 0; const failures = [];
  const equal = (actual, expected, label) => {
    checks++; if (actual !== expected) failures.push({label, actual, expected});
  };
  equal(globalThis.decodeError, undefined, 'network workflow');
  for (const {name, expected} of decodeCases) {
    equal(decodeEvents[name], 'load', name + ': event');
    equal(getComputedStyle(document.getElementById(name)).getPropertyValue('--decoded').trim(), expected, name + ': text');
  }
  equal(getComputedStyle(document.getElementById('bom-graph')).getPropertyValue('--child').trim(), '"æøåの"', 'import inherits actual BOM encoding');
  equal(getComputedStyle(document.getElementById('bom-graph')).getPropertyValue('--grandchild').trim(), '"æøåの"', 'import HTTP overrides parent encoding');
  equal(getComputedStyle(document.getElementById('japanese-graph')).getPropertyValue('--child').trim(), '"目次"', 'import inherits charset encoding');
  equal(getComputedStyle(document.getElementById('japanese-graph')).getPropertyValue('--data').trim(), '"目次"', 'data import inherits parent encoding');
  const child = decodeFrame.contentDocument.getElementById('child-target');
  equal(decodeFrame.contentWindow.getComputedStyle(child).getPropertyValue('--decoded').trim(), '"æøåの"', 'child document BOM');
  equal(decodeRaw.length === expectedRawLength && decodeRaw[0] === 254 && decodeRaw[1] === 255 && decodeFetchText.includes('\u0000'), true, 'Fetch retains original bytes and its own UTF8 text policy');
  return JSON.stringify({checks, failures});
})()
"#).unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["checks"], expected_checks, "{result}");
        if result["failures"].as_array().unwrap().is_empty() {
            assert_eq!(server.finish().await.len(), request_count);
        } else {
            drop(server);
        }
        observations
            .push(serde_json::json!({"document_encoding": document_encoding, "result": result}));
    }
    assert!(
        observations
            .iter()
            .all(|r| r["result"]["failures"].as_array().unwrap().is_empty()),
        "{}",
        serde_json::to_string(&observations).unwrap()
    );
}
