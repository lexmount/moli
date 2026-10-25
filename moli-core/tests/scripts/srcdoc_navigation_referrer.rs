use super::*;

async fn check_navigation_referrer_sources(policies_and_snapshots: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut cases = Vec::new();
    if policies_and_snapshots {
        for scope in ["srcdoc", "nested-srcdoc"] {
            for cross in [false, true] {
                for policy in ["no-referrer", "origin", ""] {
                    cases.push((scope, cross, "assign", policy, false));
                }
            }
        }
        for scope in ["http", "srcdoc"] {
            for cross in [false, true] {
                cases.push((scope, cross, "assign", "unsafe-url", true));
            }
        }
    } else {
        for scope in ["http", "srcdoc", "nested-srcdoc", "opaque-srcdoc"] {
            for cross in [false, true] {
                for action in ["assign", "replace", "post"] {
                    cases.push((scope, cross, action, "unsafe-url", false));
                }
            }
        }
    }
    for (scope, cross, action, policy, snapshot) in &cases {
        let id = format!("{scope}-{cross}-{action}-{policy}-{snapshot}");
        let destination_origin = if *cross {
            &servers.cross_origin
        } else {
            &servers.origin
        };
        let path = format!("/navigation-referrer-target?id={id}");
        let config = serde_json::json!({
            "id": id,
            "scope": scope,
            "action": action,
            "policy": policy,
            "snapshot": snapshot,
            "destination": format!("{destination_origin}{path}"),
        });
        let mut page = browser
            .fetch(&format!(
                "{}/page.html?navigation-referrer={id}#ignored-fragment",
                servers.origin,
            ))
            .await?;
        let expression = format!(
            "({})({config}).then(JSON.stringify)",
            include_str!("srcdoc_navigation_referrer.js"),
        );
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let observed: serde_json::Value =
            serde_json::from_str(result["value"].as_str().expect("navigation result"))?;
        let opaque = *scope == "opaque-srcdoc";
        let expected_referrer = if opaque || *policy == "no-referrer" {
            String::new()
        } else if *policy == "origin" || policy.is_empty() && *cross {
            format!("{}/", servers.origin)
        } else {
            let mut source = url::Url::parse(observed["sourceUrl"].as_str().unwrap())?;
            source.set_fragment(None);
            source.to_string()
        };
        assert_eq!(
            observed["documentReferrer"], expected_referrer,
            "{id}: {observed}"
        );
        assert_eq!(
            observed["senderOrigin"],
            if opaque { "null" } else { destination_origin },
            "{id}: {observed}"
        );
        let requests = servers.requests.lock();
        let matching = requests
            .iter()
            .filter(|request| request.path == path)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "{id}: {matching:?}");
        let request = matching[0];
        assert_eq!(
            request.referer.as_deref(),
            (!expected_referrer.is_empty()).then_some(expected_referrer.as_str()),
            "{id}: {request:?}"
        );
        assert_eq!(
            request.origin.as_deref(),
            if *action == "post" {
                Some(if opaque { "null" } else { &servers.origin })
            } else {
                None
            },
            "{id}: {request:?}"
        );
        assert_eq!(
            request.method,
            if *action == "post" { "POST" } else { "GET" }
        );
    }
    assert_eq!(
        servers
            .requests
            .lock()
            .iter()
            .filter(|request| request.path.starts_with("/navigation-referrer-target?"))
            .count(),
        cases.len(),
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_navigation_keeps_native_referrer_sources_and_opaque_post_origins() -> Result<()> {
    check_navigation_referrer_sources(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_navigation_freezes_referrer_url_and_policy_before_beforeunload() -> Result<()> {
    check_navigation_referrer_sources(true).await
}

pub(super) fn fixture_response(path: &str, query: &str) -> (&'static str, String, String) {
    let field = if path == "/navigation-referrer-source" {
        "html"
    } else {
        "id"
    };
    let value = url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(name, value)| (name == field).then(|| value.into_owned()))
        .expect("navigation fixture input");
    let html = if field == "html" {
        value
    } else {
        format!(
            "<!doctype html><script>top.postMessage({{navigationReferrerId:{},referrer:document.referrer}},'*');</script>",
            serde_json::to_string(&value).unwrap(),
        )
    };
    (
        "200 OK",
        "Content-Type: text/html\r\nCache-Control: no-store\r\n".to_owned(),
        html,
    )
}
