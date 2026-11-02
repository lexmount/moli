use super::*;

const STARTUP_SOURCE: &str = "globalThis.__startupCount = (globalThis.__startupCount || 0) + 1";

fn startup_config() -> NavigationRuntimeConfig {
    NavigationRuntimeConfig::default().with_document_start_scripts(vec![STARTUP_SOURCE.to_owned()])
}

fn assert_startup_scripts(context: &BrowserContext) {
    let scripts = context.default_document_start_script_descriptors();
    assert_eq!(scripts.len(), 1, "one startup script per context");
    assert_eq!(scripts[0].source, STARTUP_SOURCE);
    assert!(scripts[0].registry_key.is_none());
    assert!(context.default_document_start_scripts.is_empty());
}

#[test]
fn startup_scripts_survive_materialized_connection_initialization() {
    let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        startup_config(),
    );
    conn.insert_browser_context(conn.new_browser_context("default".to_owned()));
    assert_startup_scripts(conn.browser_context.as_ref().unwrap());

    let context = conn
        .new_ephemeral_browser_context("ephemeral".to_owned())
        .unwrap();
    assert_startup_scripts(&context);
}

#[test]
fn startup_scripts_survive_deferred_engine_materialization_reset_and_rebind() {
    let mut conn = CdpConnection::new_with_deferred_navigation_runtime(
        crate::CdpInitialStoragePartition::memory(),
        startup_config(),
    );
    assert!(!conn.standalone_navigation_engine.is_materialized());
    assert_startup_scripts(&conn.new_browser_context("before-engine".to_owned()));

    conn.active_navigation_engine();
    assert!(conn.standalone_navigation_engine.is_materialized());
    assert!(conn.release_idle_navigation_engine_memory_if_idle().reset);
    conn.insert_browser_context(conn.new_browser_context("after-reset".to_owned()));

    let config = conn.standalone_navigation_engine.runtime_config();
    let context = conn.browser_context.as_mut().unwrap();
    context.bind_page_navigation_engines(config.clone(), None);
    context.bind_page_navigation_engines(config, None);
    assert_startup_scripts(context);
}

#[test]
fn startup_scripts_do_not_share_protocol_preload_identifiers() {
    let mut conn = CdpConnection::new_with_deferred_navigation_runtime(
        crate::CdpInitialStoragePartition::memory(),
        startup_config(),
    );
    conn.insert_browser_context(conn.new_browser_context("default".to_owned()));
    let context = conn.browser_context.as_mut().unwrap();
    assert!(context.remove_default_document_start_script("1").is_none());
    let script = context
        .default_document_start_script_descriptors()
        .remove(0);
    let identifier = context.record_default_document_start_script(&script);
    assert_eq!(
        identifier, "1",
        "startup scripts must not consume protocol IDs"
    );
    assert!(
        context
            .remove_default_document_start_script(&identifier)
            .is_some()
    );
    assert_startup_scripts(context);
}
