use super::*;

const ACCEPTED_CSS: &str = ":root { --child-sri: accepted; }";
const ACCEPTED_INTEGRITY: &str =
    "sha384-Yr5qx5oiVQqAhfEAc+Rm07CfsvhCUAGi/lZHVLI9y0IAPJHPupEiTRh4HUmIjPNy";
const REJECTED_INTEGRITY: &str =
    "sha384-l1lMDaSG1lfPC9j+2P7n+VtRG5XC/UnboTCBEL/ImNiXFyX2gGDaOxkfHm3P2ick";

fn child_document(accepted: bool) -> String {
    let (path, integrity) = if accepted {
        ("/child-sri-accepted.css", ACCEPTED_INTEGRITY)
    } else {
        ("/child-sri-rejected.css", REJECTED_INTEGRITY)
    };
    format!(
        r#"<!doctype html>
<link id="sri" rel="stylesheet" href="{path}" integrity="{integrity}">
<script>
  globalThis.afterSri = true;
  document.addEventListener('DOMContentLoaded', () => globalThis.sriDcl = true);
</script>
<body>child stylesheet integrity</body>"#
    )
}

pub(super) fn fixture_response(path: &str, query: &str) -> (&'static str, String, String) {
    match path {
        "/child-sri-frame.html" => (
            "200 OK",
            "Content-Type: text/html\r\nCache-Control: no-store\r\n".to_owned(),
            child_document(query == "accepted"),
        ),
        "/child-sri-accepted.css" | "/child-sri-rejected.css" => (
            "200 OK",
            "Content-Type: text/css\r\nCache-Control: max-age=60\r\n".to_owned(),
            if path == "/child-sri-accepted.css" {
                ACCEPTED_CSS.to_owned()
            } else {
                "@import url('/child-sri-forbidden.css'); :root { --child-sri: rejected; }"
                    .to_owned()
            },
        ),
        "/child-sri-forbidden.css" => (
            "200 OK",
            "Content-Type: text/css\r\n".to_owned(),
            ":root { --child-sri-import: forbidden; }".to_owned(),
        ),
        _ => unreachable!("unknown child integrity fixture {path}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_parser_stylesheet_integrity_rejects_cssom_and_releases_lifecycle() -> Result<()> {
    // Each mode gets an independent browser cache: their referrer metadata differs.
    for mode in ["srcdoc", "url"] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!("{}/page.html", servers.origin))
            .await?;
        let documents = serde_json::json!({
            "rejected": child_document(false),
            "accepted": child_document(true),
        });
        let expression = format!(
            r#"(async () => {{
              const documents = {documents};
              const results = [];
              const mode = {mode:?};
              for (const kind of ['rejected', 'rejected-cache', 'accepted', 'accepted-cache']) {{
                  const accepted = kind.startsWith('accepted');
                  const frame = document.createElement('iframe');
                  const loaded = new Promise(resolve => frame.onload = resolve);
                  if (mode === 'srcdoc') frame.srcdoc = documents[accepted ? 'accepted' : 'rejected'];
                  else frame.src = '/child-sri-frame.html?' + (accepted ? 'accepted' : 'rejected');
                  document.body.append(frame);
                  await loaded;
                  const child = frame.contentWindow;
                  const doc = frame.contentDocument;
                  results.push({{
                    mode, kind,
                    sheet: doc.getElementById('sri').sheet !== null,
                    sheets: doc.styleSheets.length,
                    style: child.getComputedStyle(doc.documentElement).getPropertyValue('--child-sri').trim(),
                    script: child.afterSri === true,
                    dcl: child.sriDcl === true,
                    ready: doc.readyState
                  }});
                  frame.remove();
              }}
              return JSON.stringify(results);
            }})()"#,
        );
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await
        .expect("SRI rejection must release parser and iframe load blockers")?;
        let rows: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .unwrap_or_else(|| panic!("child integrity probe did not finish: {result}")),
        )?;
        let rows = rows.as_array().expect("iframe integrity results");
        assert_eq!(rows.len(), 4, "{mode}: {rows:?}");
        for row in rows {
            assert_eq!(row["mode"], mode);
            let accepted = row["kind"]
                .as_str()
                .expect("case kind")
                .starts_with("accepted");
            assert_eq!(row["sheet"], accepted, "SRI CSSOM admission: {row}");
            assert_eq!(
                row["sheets"],
                usize::from(accepted),
                "stylesheet list: {row}"
            );
            assert_eq!(
                row["style"],
                if accepted { "accepted" } else { "" },
                "unverified CSS: {row}"
            );
            assert_eq!(row["script"], true, "parser did not resume: {row}");
            assert_eq!(row["dcl"], true, "DOMContentLoaded did not finish: {row}");
            assert_eq!(
                row["ready"], "complete",
                "iframe load did not finish: {row}"
            );
        }
        let requests = servers.requests.lock();
        assert!(
            !requests
                .iter()
                .any(|r| r.path == "/child-sri-forbidden.css"),
            "rejected CSS must not start imports: {requests:?}"
        );
        for path in ["/child-sri-rejected.css", "/child-sri-accepted.css"] {
            assert_eq!(
                requests.iter().filter(|r| r.path == path).count(),
                1,
                "later iframe loads must use the cached physical response: {requests:?}"
            );
        }
    }
    Ok(())
}
