pub(in crate::context_bootstrap::indexed_db) fn database_registry_key(
    origin: &str,
    name: &str,
) -> String {
    format!("{origin}\u{0}{name}")
}
