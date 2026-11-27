use std::collections::HashSet;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

pub(super) fn handle_is_in_style_subtrees(
    host: &DomHost,
    handle: DomHandle,
    roots: &HashSet<DomHandle>,
) -> bool {
    let mut current = Some(handle);
    let mut seen = HashSet::new();
    while let Some(candidate) = current.filter(|candidate| seen.insert(*candidate)) {
        if roots.contains(&candidate) {
            return true;
        }
        current = host
            .parent_node(candidate)
            .or_else(|| host.shadow_root_host(candidate));
    }
    false
}
