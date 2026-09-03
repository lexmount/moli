use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ops::Deref,
};

use crate::dom::NodeId;
use indexmap::IndexSet;
use url::Url;

use super::NodeData;
use super::{Element, LiveDomNodeMetadata, NativeDom, NativeNodeId, Node, NodeType};

mod clone;
mod collections;
mod document;
mod html_serialization;
mod mutation;
mod parser;
mod query_index;
mod stylesheet_candidates;
mod types;

pub use html_serialization::{ShadowRootInclusion, ShadowRootRegistryAttributePolicy};
pub use stylesheet_candidates::StylesheetCandidateTreeScopeSnapshots;
pub(crate) use stylesheet_candidates::{StylesheetCandidateChanges, StylesheetCandidateRegistries};

pub use self::mutation::{
    DomAttributeMutation, DomAttributeMutationOutcome, DomChildListMutation, DomMutationEffects,
    DomMutationRecord, DomMutationRecordBatch, DomMutationRecordKind, DomScriptMutationEffects,
    DomSlotAssignmentChange, DomSlotMutationEffects, DomStyleInvalidationInputs,
    DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, DomStylesheetOwnerTransitions,
    DomStylesheetOwnerTreeScopes, DomTreeMutationEffects, ScriptPrepareTrigger,
    ScriptPrepareTriggerKind,
};

use self::query_index::ElementQueryIndex;
use self::types::{
    CachedConnectedShadowRoots, CachedLiveCollection, LiveCollectionCacheKey,
    LiveCollectionCacheKind, MutationScope, NamedElementIndex, ShadowRootState,
    ShadowSlotNameIndex,
};
pub use self::types::{ConnectedShadowRootSnapshot, DomHandle, DomHost};
pub use self::types::{HostElementSnapshot, ShadowRootBindingSnapshot, ShadowRootInit};

fn is_html_frame_owner_candidate(local_name: &str, namespace: &str) -> bool {
    (namespace.is_empty() || namespace == "http://www.w3.org/1999/xhtml")
        && ["iframe", "frame", "embed", "object"]
            .into_iter()
            .any(|name| local_name.eq_ignore_ascii_case(name))
}

fn is_builtin_reassociateable_form_associated_element(element: &Element) -> bool {
    element.namespace() == "http://www.w3.org/1999/xhtml"
        && matches!(
            element.local_name(),
            "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
        )
}

fn is_parser_form_association_candidate(element: &Element) -> bool {
    // HTMLImageElement keeps a parser/ancestor form owner for legacy
    // behavior, but is neither listed nor reassociateable.
    is_builtin_reassociateable_form_associated_element(element)
        || (element.namespace() == "http://www.w3.org/1999/xhtml" && element.local_name() == "img")
}
