use super::*;

async fn assert_abort_signal_dependencies(scenario: &str, events: &[&str]) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nfinish(runAbortSignalDependencyProbe({}));",
        include_str!("../fixtures/runtime/abort_signal_dependencies.js"),
        serde_json::to_string(scenario)?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(10),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": [], "events": events}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_signal_any_flattens_nested_sources_and_marks_all_dependents_before_events()
-> Result<()> {
    assert_abort_signal_dependencies("graph", &["0", "1", "2", "3", "4", "5"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_signal_any_preserves_first_reason_and_event_order_during_reentrant_abort()
-> Result<()> {
    assert_abort_signal_dependencies(
        "reentrant",
        &["first", "second", "secondary", "shared", "nested"],
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_signal_any_runs_listener_removal_before_dispatch_and_snapshots_each_event()
-> Result<()> {
    assert_abort_signal_dependencies(
        "listeners",
        &["source", "first", "added", "handler", "last", "late-last"],
    )
    .await
}

async fn assert_abort_signal_events(scenarios: &[&str], events: &[&str]) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for scenario in scenarios {
        let source = format!(
            "{}\nrunAbortSignalEventProbe({}).then(finish, error => finish({{error: String(error)}}));",
            include_str!("../fixtures/runtime/abort_signal_events.js"),
            serde_json::to_string(scenario)?,
        );
        for target in ["window", "child", "worker"] {
            let observed = tokio::time::timeout(
                Duration::from_secs(10),
                super::event_dispatch::run_probe(&browser, &server, target, &source),
            )
            .await??;
            assert_eq!(
                observed,
                serde_json::json!({"errors": [], "events": events, "reads": 0}),
                "{scenario}/{target}"
            );
        }
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_signal_native_events_use_intrinsic_constructors_and_preserve_trust() -> Result<()> {
    assert_abort_signal_events(
        &[
            "native-original",
            "native-getter",
            "native-replacement",
            "native-missing",
        ],
        &["source", "composite", "stream", "timeout"],
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_signal_script_dispatch_validates_events_resets_trust_and_clears_dispatch_state()
-> Result<()> {
    assert_abort_signal_events(
        &["script"],
        &[
            "native",
            "replayed",
            "passive",
            "active",
            "passive",
            "tail",
            "pre-stopped",
        ],
    )
    .await
}
