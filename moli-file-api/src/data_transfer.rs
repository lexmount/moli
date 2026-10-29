const DRAG_MODIFIER_ALT: u8 = 1;
const DRAG_MODIFIER_CTRL: u8 = 2;
const DRAG_MODIFIER_META: u8 = 4;
const DRAG_MODIFIER_SHIFT: u8 = 8;

/// Permissions granted to a DataTransfer view of a native data store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragDataStoreMode {
    ReadWrite,
    ReadOnly,
    Protected,
    Disabled,
}

impl DragDataStoreMode {
    pub fn can_read_types(self) -> bool {
        self != Self::Disabled
    }

    pub fn can_read_data(self) -> bool {
        matches!(self, Self::ReadWrite | Self::ReadOnly)
    }

    pub fn can_write_data(self) -> bool {
        self == Self::ReadWrite
    }
}

#[derive(Clone, Copy)]
struct DragModifierState {
    alt: bool,
    ctrl: bool,
    meta: bool,
    shift: bool,
}

impl DragModifierState {
    fn from_mask(modifiers: u8) -> Self {
        Self {
            alt: modifiers & DRAG_MODIFIER_ALT != 0,
            ctrl: modifiers & DRAG_MODIFIER_CTRL != 0,
            meta: modifiers & DRAG_MODIFIER_META != 0,
            shift: modifiers & DRAG_MODIFIER_SHIFT != 0,
        }
    }
}

pub fn normalize_drag_data_type(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => String::new(),
        "text" => "text/plain".to_owned(),
        "url" => "text/uri-list".to_owned(),
        other => other.to_owned(),
    }
}

pub fn drag_effect_allowed_from_mask(mask: i32) -> &'static str {
    let copy = (mask & 1) != 0;
    let link = (mask & 2) != 0;
    let r#move = (mask & 16) != 0;
    match (copy, link, r#move) {
        (false, false, false) => "none",
        (true, false, false) => "copy",
        (false, true, false) => "link",
        (false, false, true) => "move",
        (true, true, false) => "copyLink",
        (true, false, true) => "copyMove",
        (false, true, true) => "linkMove",
        (true, true, true) => "all",
    }
}

/// Inverse of [`drag_effect_allowed_from_mask`] for the values a page can read
/// back from `DataTransfer.effectAllowed`.
pub fn drag_operations_mask_from_effect_allowed(effect_allowed: &str) -> i32 {
    match effect_allowed {
        "copy" => 1,
        "link" => 2,
        "move" => 16,
        "copyLink" => 3,
        "copyMove" => 17,
        "linkMove" => 18,
        "all" | "uninitialized" => 19,
        _ => 0,
    }
}

pub fn preferred_drop_effect_from_mask(mask: i32) -> &'static str {
    if (mask & 1) != 0 {
        "copy"
    } else if (mask & 2) != 0 {
        "link"
    } else if (mask & 16) != 0 {
        "move"
    } else {
        "none"
    }
}

pub fn modifier_drop_effect(modifiers: u8) -> Option<&'static str> {
    let state = DragModifierState::from_mask(modifiers);
    // Match the CDP modifier bitmask policy used by DragEvent construction:
    // explicit link gestures win, then copy gestures, then shift-move.
    if state.alt || (state.ctrl && state.shift) {
        Some("link")
    } else if state.ctrl || state.meta {
        Some("copy")
    } else if state.shift {
        Some("move")
    } else {
        None
    }
}

pub fn drop_effect_allowed_by_effect_allowed(
    effect_allowed: Option<&str>,
    drop_effect: &str,
) -> bool {
    let effect_allowed = effect_allowed.unwrap_or("uninitialized");
    match drop_effect {
        "copy" => matches!(
            effect_allowed,
            "copy" | "copyLink" | "copyMove" | "all" | "uninitialized"
        ),
        "link" => matches!(
            effect_allowed,
            "link" | "copyLink" | "linkMove" | "all" | "uninitialized"
        ),
        "move" => matches!(
            effect_allowed,
            "move" | "copyMove" | "linkMove" | "all" | "uninitialized"
        ),
        _ => false,
    }
}

pub fn valid_drop_effect(value: &str) -> bool {
    matches!(value, "none" | "copy" | "link" | "move")
}

pub fn valid_effect_allowed(value: &str) -> bool {
    matches!(
        value,
        "none"
            | "copy"
            | "copyLink"
            | "copyMove"
            | "link"
            | "linkMove"
            | "move"
            | "all"
            | "uninitialized"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTransferItemSummary {
    pub kind: String,
    pub item_type: String,
}

impl DataTransferItemSummary {
    pub fn new(kind: impl Into<String>, item_type: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            item_type: item_type.into(),
        }
    }

    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }

    pub fn is_string_type(&self, item_type: &str) -> bool {
        self.kind == "string" && self.item_type == item_type
    }
}

pub fn data_transfer_types_from_items(items: &[DataTransferItemSummary]) -> Vec<String> {
    let mut types = Vec::new();
    let mut has_files = false;
    for item in items {
        if item.is_file() {
            has_files = true;
            continue;
        }
        if item.kind == "string"
            && !item.item_type.is_empty()
            && !types.iter().any(|existing| existing == &item.item_type)
        {
            types.push(item.item_type.clone());
        }
    }
    if has_files {
        types.push("Files".to_owned());
    }
    types
}

pub fn contains_string_item_type(items: &[DataTransferItemSummary], item_type: &str) -> bool {
    items.iter().any(|item| item.is_string_type(item_type))
}

pub fn clear_data_removes_item(item: &DataTransferItemSummary, target_type: Option<&str>) -> bool {
    if item.kind != "string" {
        return false;
    }
    target_type.is_none_or(|target_type| item.item_type == target_type)
}

pub fn child_entry_full_path(parent_path: Option<&str>, name: &str) -> String {
    match parent_path {
        Some(parent) => format!("{}/{}", parent.trim_end_matches('/'), name),
        None => format!("/{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_legacy_drag_data_type_aliases() {
        assert_eq!(normalize_drag_data_type(" text "), "text/plain");
        assert_eq!(normalize_drag_data_type("URL"), "text/uri-list");
        assert_eq!(
            normalize_drag_data_type("Application/JSON"),
            "application/json"
        );
        assert_eq!(normalize_drag_data_type(" "), "");
    }

    #[test]
    fn maps_drag_operation_masks_to_effect_tokens() {
        assert_eq!(drag_effect_allowed_from_mask(0), "none");
        assert_eq!(drag_effect_allowed_from_mask(1), "copy");
        assert_eq!(drag_effect_allowed_from_mask(2), "link");
        assert_eq!(drag_effect_allowed_from_mask(16), "move");
        assert_eq!(drag_effect_allowed_from_mask(1 | 2), "copyLink");
        assert_eq!(drag_effect_allowed_from_mask(1 | 16), "copyMove");
        assert_eq!(drag_effect_allowed_from_mask(2 | 16), "linkMove");
        assert_eq!(drag_effect_allowed_from_mask(1 | 2 | 16), "all");

        assert_eq!(preferred_drop_effect_from_mask(1 | 2 | 16), "copy");
        assert_eq!(preferred_drop_effect_from_mask(2 | 16), "link");
        assert_eq!(preferred_drop_effect_from_mask(16), "move");
        assert_eq!(preferred_drop_effect_from_mask(0), "none");
    }

    #[test]
    fn drag_modifier_drop_effect_maps_cdp_modifier_masks_by_precedence() {
        assert_eq!(modifier_drop_effect(0), None);
        assert_eq!(modifier_drop_effect(DRAG_MODIFIER_CTRL), Some("copy"));
        assert_eq!(modifier_drop_effect(DRAG_MODIFIER_META), Some("copy"));
        assert_eq!(modifier_drop_effect(DRAG_MODIFIER_SHIFT), Some("move"));
        assert_eq!(modifier_drop_effect(DRAG_MODIFIER_ALT), Some("link"));
        assert_eq!(
            modifier_drop_effect(DRAG_MODIFIER_CTRL | DRAG_MODIFIER_SHIFT),
            Some("link")
        );
        assert_eq!(
            modifier_drop_effect(DRAG_MODIFIER_ALT | DRAG_MODIFIER_META),
            Some("link")
        );
        assert_eq!(
            modifier_drop_effect(DRAG_MODIFIER_CTRL | DRAG_MODIFIER_META),
            Some("copy")
        );
    }

    #[test]
    fn drag_modifier_drop_effect_respects_effect_allowed_filter() {
        assert!(drop_effect_allowed_by_effect_allowed(Some("all"), "copy"));
        assert!(drop_effect_allowed_by_effect_allowed(Some("all"), "link"));
        assert!(drop_effect_allowed_by_effect_allowed(Some("all"), "move"));
        assert!(drop_effect_allowed_by_effect_allowed(
            Some("copyLink"),
            "copy"
        ));
        assert!(drop_effect_allowed_by_effect_allowed(
            Some("copyLink"),
            "link"
        ));
        assert!(!drop_effect_allowed_by_effect_allowed(
            Some("copyLink"),
            "move"
        ));
        assert!(!drop_effect_allowed_by_effect_allowed(Some("copy"), "link"));
    }

    #[test]
    fn validates_effect_tokens() {
        assert!(valid_drop_effect("copy"));
        assert!(valid_drop_effect("none"));
        assert!(!valid_drop_effect("copyLink"));

        assert!(valid_effect_allowed("copyLink"));
        assert!(valid_effect_allowed("uninitialized"));
        assert!(!valid_effect_allowed("invalid"));
    }

    #[test]
    fn projects_data_transfer_types_from_item_summaries() {
        let items = vec![
            DataTransferItemSummary::new("string", "text/plain"),
            DataTransferItemSummary::new("string", "text/html"),
            DataTransferItemSummary::new("string", "text/plain"),
            DataTransferItemSummary::new("string", ""),
            DataTransferItemSummary::new("file", "image/png"),
        ];

        assert_eq!(
            data_transfer_types_from_items(&items),
            vec![
                "text/plain".to_owned(),
                "text/html".to_owned(),
                "Files".to_owned()
            ]
        );
        assert!(contains_string_item_type(&items, "text/html"));
        assert!(!contains_string_item_type(&items, "image/png"));
    }

    #[test]
    fn clear_data_removal_rule_only_targets_string_items() {
        let text = DataTransferItemSummary::new("string", "text/plain");
        let html = DataTransferItemSummary::new("string", "text/html");
        let file = DataTransferItemSummary::new("file", "text/plain");

        assert!(clear_data_removes_item(&text, None));
        assert!(clear_data_removes_item(&text, Some("text/plain")));
        assert!(!clear_data_removes_item(&html, Some("text/plain")));
        assert!(!clear_data_removes_item(&file, None));
    }

    #[test]
    fn builds_child_entry_full_paths() {
        assert_eq!(child_entry_full_path(None, "file.txt"), "/file.txt");
        assert_eq!(
            child_entry_full_path(Some("/parent/"), "child.txt"),
            "/parent/child.txt"
        );
    }
}
