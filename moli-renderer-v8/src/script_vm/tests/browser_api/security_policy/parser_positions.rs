use super::*;

const OBSERVER: &str = r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="script-src 'nonce-allowed' 'report-sample'"><script nonce="allowed">
globalThis.parserReports = [];
document.addEventListener('securitypolicyviolation', event => parserReports.push({
  id: event.target.id, trusted: event.isTrusted, blockedURI: event.blockedURI,
  directive: event.effectiveDirective, sample: event.sample,
  line: event.lineNumber, column: event.columnNumber
}));
</script>"#;

const COMPLETION: &str = "<script nonce='allowed'>globalThis.parserCompleted = true;</script>";

fn position_after(source: &str) -> (u64, u64) {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    (
        normalized.bytes().filter(|&byte| byte == b'\n').count() as u64 + 1,
        normalized
            .rsplit('\n')
            .next()
            .unwrap()
            .encode_utf16()
            .count() as u64
            + 1,
    )
}

fn child_parser_result(source: &str) -> serde_json::Value {
    let mut vm = new_storage_page_task_executor_test_vm("https://child-parser-csp-position.test/");
    vm.eval(&format!(
        "const frame = document.createElement('iframe'); frame.srcdoc = {}; document.body.appendChild(frame);",
        serde_json::to_string(source).unwrap(),
    ))
    .unwrap();
    serde_json::from_str(
        &vm.eval_after_selected_page_tasks(
            "JSON.stringify({completed: frame.contentWindow.parserCompleted, ran: frame.contentWindow.blockedRan === true, reports: frame.contentWindow.parserReports})",
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn child_parser_inline_csp_reports_original_html_and_svg_script_start_positions() {
    for (prefix, opening, closing) in [
        ("<body>", "<script id='blocked'>", "</script>"),
        (
            "<body>é😀&amp;&#10;&#x1f600;&notit;&#xQ;",
            "<script id='blocked'>",
            "</script>",
        ),
        (
            "<body>one\rtwo\r\n😀",
            "<script\r\n id='blocked' data-x='😀&amp;&#xQ;'>",
            "</script>",
        ),
        (
            "<body><svg><![CDATA[😀\r\n]]>",
            "<script id='blocked'>",
            "</script></svg>",
        ),
        (
            "<body><svg>é😀",
            "<script id='blocked' data-x='&notit;'>",
            "</script></svg>",
        ),
    ] {
        let before = format!("{OBSERVER}{prefix}{opening}");
        let (line, column) = position_after(&before);
        let script = "globalThis.blockedRan = true;";
        let result = child_parser_result(&format!("{before}{script}{closing}{COMPLETION}"));
        assert_eq!(
            result,
            serde_json::json!({
                "completed": true, "ran": false,
                "reports": [{"id": "blocked", "trusted": true, "blockedURI": "inline",
                    "directive": "script-src-elem", "sample": script, "line": line, "column": column}],
            }),
            "prefix={prefix:?}, opening={opening:?}"
        );
    }
}

#[test]
fn child_parser_inline_csp_keeps_distinct_columns_for_same_line_scripts() {
    let first_start = format!("{OBSERVER}<body>😀<script id='first'>");
    let first_script = "globalThis.blockedRan = true;";
    let second_start = format!("{first_start}{first_script}</script>&amp;<script id='second'>");
    let source = format!("{second_start}{first_script}</script>{COMPLETION}");
    let result = child_parser_result(&source);
    let reports: Vec<_> = [("first", &first_start), ("second", &second_start)]
        .into_iter()
        .map(|(id, before)| {
            let (line, column) = position_after(before);
            serde_json::json!({"id": id, "trusted": true, "blockedURI": "inline",
            "directive": "script-src-elem", "sample": first_script, "line": line, "column": column})
        })
        .collect();
    assert_eq!(
        result,
        serde_json::json!({"completed": true, "ran": false, "reports": reports})
    );
}

#[test]
fn child_parser_document_write_keeps_generated_and_following_script_positions_unknown() {
    let writer = r#"<script nonce='allowed'>document.write('<scr' + 'ipt id="generated">globalThis.blockedRan = true;</scr' + 'ipt>');</script>"#;
    let source = format!(
        "{OBSERVER}<body>{writer}<script id='original'>globalThis.blockedRan = true;</script>{COMPLETION}"
    );
    let result = child_parser_result(&source);
    let reports: Vec<_> = ["generated", "original"].into_iter().map(|id| {
        serde_json::json!({"id": id, "trusted": true, "blockedURI": "inline",
            "directive": "script-src-elem", "sample": "globalThis.blockedRan = true;", "line": 0, "column": 0})
    }).collect();
    assert_eq!(
        result,
        serde_json::json!({"completed": true, "ran": false, "reports": reports})
    );
}
