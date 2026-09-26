use super::pipe_disturbed::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn font_face_sets_validate_load_and_check_queries_with_css_shorthand_syntax() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("../fixtures/font-query.js");
    for target in ["window", "child"] {
        let source =
            format!("({fixture})().then(finish, error => finish({{error: String(error)}}));");
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "{target}: {result}"
        );
        assert_eq!(
            result["rows"].as_array().unwrap().len(),
            24,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}
