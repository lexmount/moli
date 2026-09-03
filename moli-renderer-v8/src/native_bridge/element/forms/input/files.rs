use crate::dom::native::SelectedFile;
use crate::util::{
    get_private_value, global_constructor_prototype, set_private_value, v8_string, v8str,
};
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
    input_files_getter_from_object(scope, args.this(), &mut rv);
}

fn input_files_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        rv.set_null();
        return;
    };
    if !element.is_html_input() || element.input_type() != InputType::File {
        rv.set_null();
        return;
    }
    let selected_files = element.selected_files().to_vec();
    let current_signature = selected_files_cache_signature(&selected_files);
    if let Some(cached) = get_private_value(scope, object, INPUT_FILES_CACHE_SLOT) {
        let cache_matches = get_private_value(scope, object, INPUT_FILES_CACHE_SIGNATURE_SLOT)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == current_signature);
        if cache_matches {
            rv.set(cached);
            return;
        }
    }

    let Some(file_list) = cache_input_files_from_selected_files(scope, object, &selected_files)
    else {
        rv.set_null();
        return;
    };
    rv.set(file_list.into());
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
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, object_owner)
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return;
    };
    if !element.is_html_input() || element.input_type() != InputType::File {
        return;
    }
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
    if !object_has_file_list_prototype(scope, object) {
        throw_type_error(
            scope,
            "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.",
        );
        return;
    }
    let length = object
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);

    let mut files = Vec::with_capacity(length as usize);
    for index in 0..length {
        let Some(value) = object.get_index(scope, index) else {
            continue;
        };
        let Ok(file_object) = v8::Local::<v8::Object>::try_from(value) else {
            throw_type_error(
                scope,
                "Failed to set the 'files' property on 'HTMLInputElement': FileList entries must be File objects.",
            );
            return;
        };
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

fn object_has_file_list_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    global_constructor_prototype(scope, "FileList").is_some_and(|prototype| {
        object
            .get_prototype(scope)
            .is_some_and(|candidate| candidate.strict_equals(prototype.into()))
    })
}
