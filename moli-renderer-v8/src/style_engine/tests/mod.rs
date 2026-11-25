use super::invalidation::handle_is_in_style_subtrees;
use super::source_dirty::StyleSourceDirtyReason;
use super::*;
use std::collections::HashSet;

use crate::dom::native::{NativeDom, NativeNodeId};
use selectors::{Element as SelectorsElement, matching::ElementSelectorFlags};
use style::dom::TElement;
use style::servo_arc::Arc as ServoArc;

fn test_host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://example.test/").expect("valid test url"),
    ))
}

mod char_child;
mod dependency;
mod invalidator;
mod lifecycle;
mod native_snapshots;
mod registered_properties;
mod slotted_focus;
mod subtree_cache;
mod subtree_context;
