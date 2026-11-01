use super::*;
use url::Url;

#[tokio::test]
async fn nested_worker_startup_uses_response_coep_and_reports_after_construction() {
    ensure_v8();
    use crate::cross_origin_isolation::CrossOriginEmbedderPolicy as Coep;
    for kind in ["classic", "module"] {
        for (enforced, report_only, child, outcome, network, dispositions) in [
            (
                Coep::None,
                Coep::None,
                Coep::RequireCorp,
                "started",
                "TypeError",
                vec![],
            ),
            (
                Coep::RequireCorp,
                Coep::None,
                Coep::Credentialless,
                "started",
                "opaque",
                vec![],
            ),
            (
                Coep::Credentialless,
                Coep::None,
                Coep::RequireCorp,
                "started",
                "TypeError",
                vec![],
            ),
            (
                Coep::RequireCorp,
                Coep::None,
                Coep::None,
                "error",
                "",
                vec!["enforce"],
            ),
            (
                Coep::None,
                Coep::RequireCorp,
                Coep::None,
                "started",
                "opaque",
                vec!["reporting"],
            ),
            (
                Coep::RequireCorp,
                Coep::Credentialless,
                Coep::None,
                "error",
                "",
                vec!["reporting", "enforce"],
            ),
        ] {
            let coep = match child {
                Coep::None => "",
                Coep::RequireCorp => "Cross-Origin-Embedder-Policy: require-corp\r\n",
                Coep::Credentialless => "Cross-Origin-Embedder-Policy: credentialless\r\n",
            };
            let source = "self.onmessage=async url=>{let result;try{result=(await fetch(url.data,{mode:'no-cors',credentials:'include'})).type;}catch(e){result=e.name;}postMessage(result);};";
            let mut paths = vec![(
                "/child.js",
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\n{coep}Content-Length: {}\r\nConnection: close\r\n\r\n{source}",
                    source.len()
                ),
                Duration::ZERO,
            )];
            if outcome == "started" {
                paths.push(("/resource","HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned(),Duration::ZERO));
            }
            let (base, server) = spawn_raw_path_response_http_server(paths).await;
            let owner_url = format!("{base}/parent.js");
            let child_url = format!("{base}/child.js");
            let mut resource = Url::parse(&format!("{base}/resource")).unwrap();
            resource
                .set_host(Some(if resource.host_str() == Some("localhost") {
                    "127.0.0.1"
                } else {
                    "localhost"
                }))
                .unwrap();
            let source = format!(
                r#"
                let result;const reports=[];const expected={count};
                const worker=new Worker({url},{{type:{kind}}});
                const observer=new ReportingObserver(list=>{{reports.push(...list.map(r=>r.toJSON()));finish();}});observer.observe();
                function finish(){{if(result&&reports.length===expected){{postMessage({{...result,reports}});worker.terminate();close();}}}}
                worker.onmessage=event=>{{result={{outcome:'started',network:event.data}};finish();}};
                worker.onerror=event=>{{event.preventDefault();result={{outcome:'error',network:'',eventTag:Object.prototype.toString.call(event)}};finish();}};
                worker.postMessage({resource});
            "#,
                count = dispositions.len(),
                url = serde_json::to_string(&child_url).unwrap(),
                kind = serde_json::to_string(kind).unwrap(),
                resource = serde_json::to_string(resource.as_str()).unwrap()
            );
            let mut parent = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(source, owner_url.clone()).with_policy_context(
                    crate::types::SubresourcePolicyContext {
                        cross_origin_embedder_policy: enforced,
                        cross_origin_embedder_policy_report_only: report_only,
                        ..Default::default()
                    },
                ),
            );
            let actual: serde_json::Value =
                serde_json::from_str(&recv_post_json(&mut parent).await).unwrap();
            assert_eq!(actual["outcome"], outcome, "{kind}/{enforced:?}/{child:?}");
            assert_eq!(actual["network"], network);
            if outcome == "error" {
                assert_eq!(actual["eventTag"], "[object Event]");
            }
            let reports = actual["reports"].as_array().unwrap();
            assert_eq!(reports.len(), dispositions.len());
            for (report, disposition) in reports.iter().zip(dispositions) {
                assert_eq!(report["type"], "coep");
                assert_eq!(report["url"], owner_url);
                assert_eq!(
                    report["body"],
                    serde_json::json!({"type":"worker initialization","blockedURL":child_url,"disposition":disposition})
                );
            }
            parent.terminate_and_join();
            server.await.unwrap();
        }
    }
}
