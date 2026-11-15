use super::*;
use moli_core::page::PageInputExt;

#[tokio::test]
async fn legacy_key_dispatch_signature_defaults_to_standard_location() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    page.evaluate_runtime_expression_async("globalThis.__locations=[];document.addEventListener('keydown', e=>__locations.push([e.key,e.code,e.location]))").await?;
    // This is the public call shape supported before #2967. In particular,
    // its code string must not silently infer a non-standard location.
    let command = page.start_dispatch_key_event_with_outcome(
        "keydown",
        "Shift",
        "ShiftRight",
        "",
        0,
        false,
        false,
    )?;
    let completed = command.wait().await?;
    page.finish_dispatch_key_event_with_outcome(completed)?;
    let command = page.start_dispatch_key_event_with_location_outcome(
        "keydown",
        "Shift",
        "ShiftRight",
        "",
        0,
        false,
        false,
        2,
    )?;
    let completed = command.wait().await?;
    page.finish_dispatch_key_event_with_outcome(completed)?;
    let value = page
        .evaluate_runtime_expression_async("JSON.stringify(__locations)")
        .await?;
    assert_eq!(
        value["value"],
        serde_json::json!("[[\"Shift\",\"ShiftRight\",0],[\"Shift\",\"ShiftRight\",2]]")
    );
    server.shutdown().await;
    Ok(())
}
