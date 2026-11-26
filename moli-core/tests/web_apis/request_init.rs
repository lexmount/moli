use super::*;

async fn assert_request_init_validation(scenario: &str, targets: &[&str]) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunRequestInitValidationProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/request_init_validation.js"),
        serde_json::to_string(scenario)?,
        serde_json::to_string(&server.url("/compat/child-dynamic-markup-document"))?,
    );
    for target in targets {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn request_and_fetch_reject_invalid_init_before_abort() -> Result<()> {
    assert_request_init_validation("invalid", &["window", "child", "worker"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_and_fetch_accept_valid_init_and_preserve_abort_reason() -> Result<()> {
    assert_request_init_validation("valid", &["window", "child", "worker"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_and_fetch_validate_inherited_modes_without_consuming_invalid_input() -> Result<()>
{
    assert_request_init_validation("inheritance", &["window", "child", "worker"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_init_errors_and_rejected_promises_use_the_function_realm() -> Result<()> {
    assert_request_init_validation("realm", &["window", "child"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_init_window_accepts_only_null_without_consuming_invalid_input() -> Result<()> {
    assert_request_init_validation("window", &["window", "child", "worker"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_init_getter_exceptions_precede_construction_validation() -> Result<()> {
    assert_request_init_validation("getters", &["window", "child", "worker"]).await
}
