use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_index_names_follow_upgrade_commit_abort_and_utf16_identity() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("../fixtures/indexeddb-index-rename.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nindexRenameProbe('index-rename-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: renameChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert!(
            result["checks"].as_array().unwrap().len() > 150,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_index_cursors_capture_names_after_argument_conversion() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("../fixtures/indexeddb-index-cursor-conversion.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nindexCursorConversionProbe('index-cursor-conversion-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: cursorConversionChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(result["checks"].as_array().unwrap().len(), 24);
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_index_rename_keeps_queued_reads_and_cursor_labels_at_admission() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("../fixtures/indexeddb-index-cursor-rename.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nindexCursorRenameProbe('index-cursor-rename-{target}').then(finish,error=>finish({{state:'error',error:String(error)}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
    }
    server.shutdown().await;
    Ok(())
}
