use super::super::range_algorithms::range_compare_point_internal;
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{callback_value_dom_handle, throw_dom_exception, wrapped_handle_value};
use crate::webidl;
use moli_webapi_declare::ObjectLiteralDeclaration;

fn throw_index_size_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "IndexSizeError", 1, message);
}

fn throw_not_found_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    // DOMException.NOT_FOUND_ERR == 8 per the legacy code table.
    throw_dom_exception(scope, "NotFoundError", 8, message);
}

fn selection_required_range_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    message: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let Ok(range) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, message);
        return None;
    };
    if range_boundary_container_object(scope, range, RangeBoundarySide::Start).is_none()
        || range_boundary_container_object(scope, range, RangeBoundarySide::End).is_none()
    {
        throw_type_error(scope, message);
        return None;
    }
    Some(range)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.getRangeAt")]
struct SelectionGetRangeAtArgs {
    #[webidl(
        required,
        converter = "unsigned_long",
        missing_message = "Selection.getRangeAt requires an index"
    )]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.containsNode")]
struct SelectionContainsNodeArgs<'s> {
    #[webidl(with = selection_contains_node_arg)]
    node: v8::Local<'s, v8::Object>,
    #[webidl(default = false)]
    allow_partial_containment: bool,
}

fn selection_contains_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Selection.containsNode requires a Node",
        ));
    }
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(index)) else {
        return Err(webidl::WebIdlError::custom_message(
            "Selection.containsNode requires a Node",
        ));
    };
    if callback_value_dom_handle(scope, node.into()).is_none() {
        return Err(webidl::WebIdlError::custom_message(
            "Selection.containsNode requires a Node",
        ));
    }
    Ok(node)
}

fn selection_contains_node_boundary_points<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    u32,
    v8::Local<'s, v8::Object>,
    u32,
    bool,
)> {
    let node_type = object_number_property(scope, node, "nodeType")? as u32;
    let is_character_data = matches!(node_type, 3 | 7 | 8);
    if is_character_data {
        return Some((node, 0, node, range_node_length(scope, node)?, true));
    }

    let parent = object_property_as_object(scope, node, "parentNode")?;
    let index = child_index(scope, parent, node)?;
    Some((parent, index, parent, index + 1, false))
}

fn range_belongs_to_current_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    start_node: v8::Local<'s, v8::Object>,
    end_node: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(document) =
        selection_owner_document(scope, selection).or_else(|| current_document_object(scope))
    else {
        return true;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(document) = callback_value_dom_handle(scope, document.into()) else {
        return false;
    };
    // Connectivity and ownership are native relationships. Author properties
    // named parentNode, nodeType or host must not change Selection membership
    // (or run script while checking an associated Range).
    let dom = unsafe { &*host_ptr }.dom_host();
    [start_node, end_node].into_iter().all(|node| {
        callback_value_dom_handle(scope, node.into()).is_some_and(|handle| {
            dom.is_connected_to_document(handle)
                && (handle == document || dom.owner_document_handle(handle) == Some(document))
        })
    })
}

pub(in crate::context_bootstrap) fn selection_get_range_at_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SelectionGetRangeAtArgs>(scope, &args) else {
        return;
    };
    if parsed.index != 0 {
        throw_index_size_error(scope, "Index or offset is out of range.");
        return;
    }
    if let Some(range) = selection_range(scope, args.this()) {
        let Some(start_node) =
            range_boundary_container_object(scope, range, RangeBoundarySide::Start)
        else {
            throw_index_size_error(scope, "Index or offset is out of range.");
            return;
        };
        let Some(end_node) = range_boundary_container_object(scope, range, RangeBoundarySide::End)
        else {
            throw_index_size_error(scope, "Index or offset is out of range.");
            return;
        };
        if !range_belongs_to_current_document(scope, args.this(), start_node, end_node) {
            throw_index_size_error(scope, "Index or offset is out of range.");
            return;
        }
        rv.set(range.into());
    } else {
        throw_index_size_error(scope, "Index or offset is out of range.");
    }
}

pub(in crate::context_bootstrap) fn selection_get_composed_ranges_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let empty = v8::Array::new(scope, 0);
    if !selection_has_range(scope, args.this()) {
        rv.set(empty.into());
        return;
    }
    let Some(shadow_roots) = selection_get_composed_range_shadow_roots(scope, &args) else {
        return;
    };

    let (start_container, start_offset, end_container, end_offset) =
        if let (Some(start_node), Some(end_node)) = (
            selection_composed_start_node(scope, args.this()),
            selection_composed_end_node(scope, args.this()),
        ) {
            (
                start_node,
                selection_composed_start_offset(scope, args.this()),
                end_node,
                selection_composed_end_offset(scope, args.this()),
            )
        } else {
            let Some(anchor_node) = selection_anchor_node(scope, args.this()) else {
                rv.set(empty.into());
                return;
            };
            let Some(focus_node) = selection_focus_node(scope, args.this()) else {
                rv.set(empty.into());
                return;
            };
            let anchor_offset = selection_anchor_offset(scope, args.this());
            let focus_offset = selection_focus_offset(scope, args.this());
            match selection_composed_boundary_order(
                scope,
                anchor_node,
                anchor_offset,
                focus_node,
                focus_offset,
            )
            .or_else(|| {
                Some(boundary_order(
                    scope,
                    anchor_node,
                    anchor_offset,
                    focus_node,
                    focus_offset,
                ))
            }) {
                Some(std::cmp::Ordering::Greater) => {
                    (focus_node, focus_offset, anchor_node, anchor_offset)
                }
                _ => (anchor_node, anchor_offset, focus_node, focus_offset),
            }
        };

    let Some((start_container, start_offset)) = selection_rescope_composed_boundary(
        scope,
        start_container,
        start_offset,
        &shadow_roots,
        false,
    ) else {
        rv.set(empty.into());
        return;
    };
    let Some((end_container, end_offset)) =
        selection_rescope_composed_boundary(scope, end_container, end_offset, &shadow_roots, true)
    else {
        rv.set(empty.into());
        return;
    };
    let Some(static_range) = selection_new_static_range(
        scope,
        start_container,
        start_offset,
        end_container,
        end_offset,
    ) else {
        rv.set(empty.into());
        return;
    };
    let ranges = v8::Array::new(scope, 1);
    let _ = ranges.set_index(scope, 0, static_range.into());
    rv.set(ranges.into());
}

fn selection_get_composed_range_shadow_roots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Vec<DomHandle>> {
    if args.length() == 0 {
        return Some(Vec::new());
    }
    let options_value = args.get(0);
    if options_value.is_null_or_undefined() {
        return Some(Vec::new());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(options_value) else {
        throw_type_error(
            scope,
            "Selection.getComposedRanges options must be an object.",
        );
        return None;
    };
    let Some(shadow_roots_value) = options.get(scope, v8str(scope, "shadowRoots").into()) else {
        return Some(Vec::new());
    };
    if shadow_roots_value.is_null_or_undefined() {
        return Some(Vec::new());
    }
    let Ok(shadow_roots_array) = v8::Local::<v8::Array>::try_from(shadow_roots_value) else {
        throw_type_error(
            scope,
            "Selection.getComposedRanges shadowRoots must be a sequence.",
        );
        return None;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Some(Vec::new());
    };
    let mut roots = Vec::with_capacity(shadow_roots_array.length() as usize);
    for index in 0..shadow_roots_array.length() {
        let Some(value) = shadow_roots_array.get_index(scope, index) else {
            continue;
        };
        let Some(handle) = callback_value_dom_handle(scope, value) else {
            throw_type_error(
                scope,
                "Selection.getComposedRanges shadowRoots must contain ShadowRoot objects.",
            );
            return None;
        };
        if !unsafe { &*host_ptr }.dom_host().is_shadow_root(handle) {
            throw_type_error(
                scope,
                "Selection.getComposedRanges shadowRoots must contain ShadowRoot objects.",
            );
            return None;
        }
        roots.push(handle);
    }
    Some(roots)
}

fn selection_rescope_composed_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
    allowed_shadow_roots: &[DomHandle],
    is_end: bool,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let original = callback_value_dom_handle(scope, node.into())?;
    let (handle, offset) = {
        let runtime = unsafe { &*host_ptr };
        let dom_host = runtime.dom_host();
        let mut current = original;
        let mut current_offset = offset;
        while let Some(root) = dom_host.containing_shadow_root(current) {
            if selection_shadow_root_allowed(dom_host, root, allowed_shadow_roots) {
                break;
            }
            let Some(host) = dom_host.shadow_root_host(root) else {
                break;
            };
            let Some(parent) = dom_host.node(host).and_then(|node| node.parent_node()) else {
                break;
            };
            let Some(index) = dom_host.child_index(parent, host) else {
                break;
            };
            current = parent;
            current_offset = u32::try_from(index + usize::from(is_end)).ok()?;
        }
        (current, current_offset)
    };
    wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| (object, offset))
}

fn selection_shadow_root_allowed(
    dom_host: &moli_dom::native::DomHost,
    root: DomHandle,
    allowed_shadow_roots: &[DomHandle],
) -> bool {
    allowed_shadow_roots
        .iter()
        .copied()
        .any(|allowed| selection_shadow_including_contains(dom_host, root, allowed))
}

fn selection_shadow_including_contains(
    dom_host: &moli_dom::native::DomHost,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = dom_host
            .node(handle)
            .and_then(|entry| entry.parent_node())
            .or_else(|| dom_host.shadow_root_host(handle));
    }
    false
}

fn selection_new_static_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    start_container: v8::Local<'s, v8::Object>,
    start_offset: u32,
    end_container: v8::Local<'s, v8::Object>,
    end_offset: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global.get(scope, v8str(scope, "StaticRange").into())?;
    let ctor = v8::Local::<v8::Function>::try_from(ctor).ok()?;
    let init = ObjectLiteralDeclaration::bind(scope);
    init.set_string_property(scope, "startContainer", start_container.into());
    init.set_string_property(
        scope,
        "startOffset",
        v8::Integer::new_from_unsigned(scope, start_offset).into(),
    );
    init.set_string_property(scope, "endContainer", end_container.into());
    init.set_string_property(
        scope,
        "endOffset",
        v8::Integer::new_from_unsigned(scope, end_offset).into(),
    );
    ctor.new_instance(scope, &[init.into_value()])
}

pub(in crate::context_bootstrap) fn selection_add_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(range) =
        selection_required_range_arg(scope, &args, "Selection.addRange requires a Range")
    else {
        return;
    };
    if let Some(current) = selection_range(scope, args.this()) {
        if current.strict_equals(range.into()) {
            return;
        }
        return;
    }
    let Some(start_node) = range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        return;
    };
    let Some(end_node) = range_boundary_container_object(scope, range, RangeBoundarySide::End)
    else {
        return;
    };
    if !range_belongs_to_current_document(scope, args.this(), start_node, end_node) {
        return;
    }
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    let direction = if range_is_collapsed(scope, range) {
        "none"
    } else {
        "forward"
    };
    selection_store(
        scope,
        args.this(),
        range,
        start_node,
        start_offset,
        end_node,
        end_offset,
        direction,
    );
    selection_dispatch_change(scope);
}

pub(in crate::context_bootstrap) fn selection_remove_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(range) =
        selection_required_range_arg(scope, &args, "Selection.removeRange requires a Range")
    else {
        return;
    };
    let Some(current) = selection_range(scope, args.this()) else {
        throw_not_found_error(scope, "The specified range was not found.");
        return;
    };
    if !current.strict_equals(range.into()) {
        throw_not_found_error(scope, "The specified range was not found.");
        return;
    }
    selection_clear(scope, args.this());
    selection_dispatch_change(scope);
}

pub(in crate::context_bootstrap) fn selection_remove_all_ranges_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let had_range = selection_has_range(scope, args.this());
    selection_clear(scope, args.this());
    if had_range {
        selection_dispatch_change(scope);
    }
}

pub(in crate::context_bootstrap) fn selection_contains_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SelectionContainsNodeArgs>(scope, &args) else {
        return;
    };
    let Some(range) = selection_range(scope, args.this()) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some((start_container, start_offset, end_container, end_offset, is_character_data)) =
        selection_contains_node_boundary_points(scope, parsed.node)
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let node_start = range_compare_point_internal(scope, range, start_container, start_offset);
    let node_end = range_compare_point_internal(scope, range, end_container, end_offset);

    let result = if parsed.allow_partial_containment {
        node_start != 1 && node_end != -1
    } else {
        is_character_data && node_start == 0 && node_end == 0
    };

    rv.set(v8::Boolean::new(scope, result).into());
}
