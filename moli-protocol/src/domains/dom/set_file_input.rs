//! Protocol-neutral file-input binding and result handling for automation.

use super::loaded_page_mut_for_owner;
use super::resolve::{
    DevToolsDomCommandTaskStep, DomCommandOutput, DomCommandTaskStep, PendingDomCommandDispatch,
    PendingDomCommandKind, PendingDomCommandStartError, devtools_dom_command_task_complete,
    dom_object_reference_id_for_owner,
};
use crate::automation::{
    DevToolsDomNodeReference, DevToolsError, DevToolsErrorKind, DevToolsRemoteHandleId,
    DevToolsSetFileInputFilesCommand, is_webdriver_bidi_node_shared_id,
};
use crate::conn::{CdpConnection, CommandOwnerScope};
use moli_core::page::{
    CompletedPageCommand, Page, PendingPageCommand, RendererDomBidiNodeBindingResolution,
    SelectedFile,
};

pub(super) fn start_devtools_set_file_input_files_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsSetFileInputFilesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let is_shared_node_id = is_webdriver_bidi_node_shared_id(command.object_id.as_str());
    if is_shared_node_id {
        let renderer_inspector_session_id =
            conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = loaded_page_mut_for_owner(conn, owner)
            .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
        let pending = page
            .start_document_bidi_node_binding(
                renderer_inspector_session_id,
                command.object_id.as_str().to_owned(),
            )
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles {
                object_id: command.object_id,
                files: command.files,
                append: command.append,
            },
            pending,
        }));
    }
    start_set_file_input_files_for_remote_reference(
        conn,
        command_id,
        owner,
        command.object_id,
        command.files,
        command.append,
    )
    .map(Some)
}

pub(super) fn complete_bidi_node_binding_for_set_file_input_files(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    object_id: DevToolsRemoteHandleId,
    files: Vec<SelectedFile>,
    append: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match start_set_file_input_files_after_bidi_binding_resolution(
        conn, command_id, owner, completion, object_id, files, append,
    ) {
        Ok(dispatch) => DomCommandTaskStep::Pending(Box::new(dispatch)),
        Err(error) => {
            out.push_error(error.code, error.message);
            DomCommandTaskStep::Complete
        }
    }
}

pub(super) fn complete_bidi_node_binding_for_set_file_input_files_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    object_id: DevToolsRemoteHandleId,
    files: Vec<SelectedFile>,
    append: bool,
) -> DevToolsDomCommandTaskStep {
    match start_set_file_input_files_after_bidi_binding_resolution(
        conn, command_id, owner, completion, object_id, files, append,
    ) {
        Ok(dispatch) => DevToolsDomCommandTaskStep::Pending(Box::new(dispatch)),
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::from(error))),
    }
}

pub(super) fn complete_set_file_input_files(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_set_file_input_files(completion) {
        Ok(Some(true)) => out.push_success(),
        Ok(Some(false)) => out.push_error(-32000, "UnableToSetFileInput"),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not set file input files: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_file_input_files_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_set_file_input_files_for_object_id(completion) {
        Ok(Some(true)) => out.push_success(),
        Ok(Some(false)) => out.push_error(-32000, "UnableToSetFileInput"),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not set file input files: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_file_input_files_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    match page.finish_set_file_input_files(completion) {
        Ok(Some(true)) => Ok(()),
        Ok(Some(false)) => Err(DevToolsError::new(
            DevToolsErrorKind::UnableToSetFileInput,
            "UnableToSetFileInput",
        )),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not set file input files: {error}"),
        )),
    }
}

pub(super) fn complete_set_file_input_files_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    match page.finish_set_file_input_files_for_object_id(completion) {
        Ok(Some(true)) => Ok(()),
        Ok(Some(false)) => Err(DevToolsError::new(
            DevToolsErrorKind::UnableToSetFileInput,
            "UnableToSetFileInput",
        )),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not set file input files: {error}"),
        )),
    }
}

fn start_set_file_input_files_after_bidi_binding_resolution(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    object_id: DevToolsRemoteHandleId,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    match finish_renderer_bidi_node_binding(conn, owner, completion)? {
        RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id) => {
            let page = loaded_page_mut_for_owner(conn, owner)
                .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
            start_set_file_input_files_for_reference_dispatch(
                page,
                command_id,
                owner,
                DevToolsDomNodeReference::BackendNodeId(backend_node_id),
                files,
                append,
            )
        }
        RendererDomBidiNodeBindingResolution::NotFound => {
            start_set_file_input_files_for_remote_reference(
                conn, command_id, owner, object_id, files, append,
            )
        }
    }
}

fn finish_renderer_bidi_node_binding(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
) -> Result<RendererDomBidiNodeBindingResolution, PendingDomCommandStartError> {
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    page.finish_document_bidi_node_binding(completion)
        .map_err(|error| {
            PendingDomCommandStartError::renderer_error(format!(
                "Could not resolve BiDi node binding: {error}"
            ))
        })
}

fn start_set_file_input_files_for_remote_reference(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    object_id: DevToolsRemoteHandleId,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let reference = dom_object_reference_id_for_owner(conn, owner, &object_id);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let object_id = reference;
    start_set_file_input_files_for_runtime_object(
        page,
        command_id,
        owner,
        renderer_inspector_session_id,
        object_id,
        files,
        append,
    )
}

fn start_set_file_input_files_for_reference_dispatch(
    page: &Page,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let pending = start_set_file_input_files_for_reference(page, reference, files, append)?;
    Ok(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::SetFileInputFiles,
        pending,
    })
}

fn start_set_file_input_files_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => page
            .start_set_file_input_files_for_backend_node_id(backend_node_id, files, append)
            .map_err(PendingDomCommandStartError::renderer_error),
    }
}

fn start_set_file_input_files_for_runtime_object(
    page: &Page,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    renderer_inspector_session_id: Option<String>,
    object_id: String,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let pending = page
        .start_set_file_input_files_for_object_id_in_inspector_session(
            renderer_inspector_session_id,
            &object_id,
            files,
            append,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::SetFileInputFilesObjectReference,
        pending,
    })
}
