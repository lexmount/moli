use crate::dom::native::SelectedFile;
use crate::native_bridge::element::{html_element_getter_receiver, html_element_setter_receiver};
use crate::util::{get_private_value, set_private_value, v8_string};
use std::fmt::Write as _;

use super::super::*;

const INPUT_FILES_CACHE_SLOT: &str = "__lmInputFiles";
const INPUT_FILES_CACHE_SIGNATURE_SLOT: &str = "__lmInputFilesSignature";

fn selected_files_cache_signature(selected_files: &[SelectedFile]) -> String {
    let mut signature = String::new();
    for file in selected_files {
        let mut bytes_hash = 0xcbf29ce484222325u64;
        for byte in &file.bytes {
            bytes_hash ^= u64::from(*byte);
            bytes_hash = bytes_hash.wrapping_mul(0x100000001b3);
        }
        let _ = write!(
            signature,
            "{}:{}:{}:{}:{}:{};",
            file.name.len(),
            file.name,
            file.mime_type.len(),
            file.mime_type,
            file.last_modified.to_bits(),
            bytes_hash
        );
    }
    signature
}

pub(crate) fn cache_input_files_from_selected_files<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: v8::Local<'s, v8::Object>,
    selected_files: &[SelectedFile],
) -> Option<v8::Local<'s, v8::Object>> {
    let context = input.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let signature = v8_string(scope, &selected_files_cache_signature(selected_files))?;
    let mut files = Vec::with_capacity(selected_files.len());
    for file in selected_files {
        let file_object = crate::context_bootstrap::build_file_object(scope, file)?;
        files.push(file_object);
    }
    let file_list = crate::context_bootstrap::build_file_list_object(scope, &files)?;
    set_private_value(scope, input, INPUT_FILES_CACHE_SLOT, file_list.into());
    set_private_value(
        scope,
        input,
        INPUT_FILES_CACHE_SIGNATURE_SLOT,
        signature.into(),
    );
    Some(file_list)
}

pub(in crate::native_bridge) fn input_files_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if html_element_getter_receiver(scope, args.this(), "HTMLInputElement", "files", "input")
        .is_none()
    {
        return;
    }
    if let Some(files) = input_files_for_object(scope, args.this()) {
        rv.set(files.into());
    } else {
        rv.set_null();
    }
}

pub(crate) fn input_files_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, object).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if !element.is_html_input() || element.input_type() != InputType::File {
        return None;
    }
    let selected_files = element.selected_files().to_vec();
    let current_signature = selected_files_cache_signature(&selected_files);
    if let Some(cached) = get_private_value(scope, object, INPUT_FILES_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let cache_matches = get_private_value(scope, object, INPUT_FILES_CACHE_SIGNATURE_SLOT)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == current_signature);
        if cache_matches {
            return Some(cached);
        }
    }

    cache_input_files_from_selected_files(scope, object, &selected_files)
}

pub(in crate::native_bridge) fn input_files_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_files_setter_on_object(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn input_files_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object_owner: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, object_owner, "HTMLInputElement", "files", "input")
    else {
        return;
    };
    if value.is_null_or_undefined() {
        return;
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.",
        );
        return;
    };
    // Web IDL checks the interface identity, independent of the object's realm
    // or mutable prototype. This conversion also precedes input-type checks.
    if !crate::context_bootstrap::is_file_list_object(scope, object) {
        throw_type_error(
            scope,
            "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.",
        );
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return;
    };
    if element.input_type() != InputType::File {
        return;
    }
    let Some(file_objects) = crate::context_bootstrap::file_list_files_from_object(scope, object)
    else {
        return;
    };

    let mut files = Vec::with_capacity(file_objects.len());
    for file_object in file_objects {
        let Some(file) = crate::context_bootstrap::selected_file_from_object(scope, file_object)
        else {
            throw_type_error(
                scope,
                "Failed to set the 'files' property on 'HTMLInputElement': FileList entries must be File objects.",
            );
            return;
        };
        files.push(file);
    }

    let signature = v8_string(scope, &selected_files_cache_signature(&files));
    let _ = unsafe { &mut *runtime_ptr }.set_input_files(handle, files);
    set_private_value(scope, object_owner, INPUT_FILES_CACHE_SLOT, object.into());
    if let Some(signature) = signature {
        set_private_value(
            scope,
            object_owner,
            INPUT_FILES_CACHE_SIGNATURE_SLOT,
            signature.into(),
        );
    }
}
