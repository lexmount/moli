use moli_renderer_v8::RendererDomInspection;

use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use super::dom_inspection_for_owner;
use super::node_references::{NodeReferenceParams, devtools_node_reference_from_ids};
use super::resolve::{
    DevToolsDomCommandTaskStep, DomCommandOutput, DomCommandTaskStep, PendingDomCommandDispatch,
    PendingDomCommandKind, PendingDomCommandStartError, devtools_dom_command_task_complete,
    dom_object_reference_id_for_owner, start_document_node_snapshot_for_reference,
};
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::devtools_runtime::{
    DevToolsDomNodeReference, DevToolsError, DevToolsErrorKind, DevToolsRemoteHandleId,
    DevToolsSetFileInputFilesCommand, is_webdriver_bidi_node_shared_id,
};
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererDomBidiNodeBindingResolution, SelectedFile,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetFileInputFilesParams {
    #[serde(flatten)]
    reference: NodeReferenceParams,
    files: Vec<String>,
}

pub(super) fn build_cdp_set_file_input_files_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsSetFileInputFilesCommand>, PendingDomCommandStartError> {
    let params: SetFileInputFilesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(object_id) = params.reference.object_id else {
        return Ok(None);
    };
    let files = selected_files_from_paths(&params.files)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsSetFileInputFilesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        object_id: DevToolsRemoteHandleId::from(object_id),
        files,
        append: false,
    }))
}

pub(super) fn start_cdp_set_file_input_files_by_node_reference(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: SetFileInputFilesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    )
    .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_set_file_input_files_frontend_node_binding(
            conn,
            cmd.id,
            &owner,
            frontend_node_id,
            params.files,
            false,
        );
    }
    let inspection = dom_inspection_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    start_set_file_input_files_preflight_dispatch(
        &inspection,
        cmd.id,
        &owner,
        reference,
        params.files,
        false,
    )
    .map(Some)
}

pub(super) fn start_devtools_set_file_input_files_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsSetFileInputFilesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let is_shared_node_id = is_webdriver_bidi_node_shared_id(command.object_id.as_str());
    if is_shared_node_id {
        let inspection = dom_inspection_for_owner(conn, owner)
            .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
        let pending = inspection
            .start_document_bidi_node_binding(command.object_id.as_str().to_owned())
            .map(PendingPageCommand::from_inspector_main_route)
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

pub(super) fn complete_preflight(
    inspection: &RendererDomInspection<'_>,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    reference: DevToolsDomNodeReference,
    file_paths: Vec<String>,
    append: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let DevToolsDomNodeReference::BackendNodeId(_) = reference else {
        out.push_error(-32000, "Could not find node with given id");
        return DomCommandTaskStep::Complete;
    };
    let preflight = completion.finish_document_node_snapshot_for_backend_node_id();
    match preflight {
        Ok(Some(_)) => {}
        Ok(None) => {
            out.push_error(-32000, "Could not find node with given id");
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not preflight file input node: {error}"),
            );
            return DomCommandTaskStep::Complete;
        }
    }
    let files = match selected_files_from_paths(&file_paths) {
        Ok(files) => files,
        Err(error) => {
            out.push_error(error.code, error.message);
            return DomCommandTaskStep::Complete;
        }
    };
    match start_set_file_input_files_for_reference_dispatch(
        inspection, command_id, owner, reference, files, append,
    ) {
        Ok(dispatch) => DomCommandTaskStep::Pending(Box::new(dispatch)),
        Err(error) => {
            out.push_error(error.code, error.message);
            DomCommandTaskStep::Complete
        }
    }
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

pub(super) fn complete_frontend_node_binding_for_set_file_input_files(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    frontend_node_id: u32,
    file_paths: Vec<String>,
    append: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match start_set_file_input_files_after_frontend_binding_resolution(
        conn,
        command_id,
        owner,
        completion,
        frontend_node_id,
        file_paths,
        append,
    ) {
        Ok(dispatch) => DomCommandTaskStep::Pending(Box::new(dispatch)),
        Err(error) => {
            out.push_error(error.code, error.message);
            DomCommandTaskStep::Complete
        }
    }
}

pub(super) fn complete_frontend_node_binding_for_set_file_input_files_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    frontend_node_id: u32,
    file_paths: Vec<String>,
    append: bool,
) -> DevToolsDomCommandTaskStep {
    match start_set_file_input_files_after_frontend_binding_resolution(
        conn,
        command_id,
        owner,
        completion,
        frontend_node_id,
        file_paths,
        append,
    ) {
        Ok(dispatch) => DevToolsDomCommandTaskStep::Pending(Box::new(dispatch)),
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::from(error))),
    }
}

pub(super) fn complete_set_file_input_files(
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match completion.finish_set_file_input_files() {
        Ok(Some(true)) => out.push_success(),
        Ok(Some(false)) => out.push_error(-32000, "UnableToSetFileInput"),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not set file input files: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_file_input_files_object_reference(
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match completion.finish_set_file_input_files_for_object_id() {
        Ok(Some(true)) => out.push_success(),
        Ok(Some(false)) => out.push_error(-32000, "UnableToSetFileInput"),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not set file input files: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_file_input_files_result(
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    match completion.finish_set_file_input_files() {
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
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    match completion.finish_set_file_input_files_for_object_id() {
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
    match finish_renderer_bidi_node_binding(completion)? {
        RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id) => {
            let inspection = dom_inspection_for_owner(conn, owner)
                .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
            start_set_file_input_files_for_reference_dispatch(
                &inspection,
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

fn start_set_file_input_files_after_frontend_binding_resolution(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    frontend_node_id: u32,
    file_paths: Vec<String>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let reference = finish_renderer_frontend_node_binding(completion, frontend_node_id)?;
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    start_set_file_input_files_preflight_dispatch(
        &inspection,
        command_id,
        owner,
        reference,
        file_paths,
        append,
    )
}

fn finish_renderer_bidi_node_binding(
    completion: CompletedPageCommand,
) -> Result<RendererDomBidiNodeBindingResolution, PendingDomCommandStartError> {
    completion
        .finish_document_bidi_node_binding()
        .map_err(|error| {
            PendingDomCommandStartError::renderer_error(format!(
                "Could not resolve BiDi node binding: {error}"
            ))
        })
}

fn finish_renderer_frontend_node_binding(
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> Result<DevToolsDomNodeReference, PendingDomCommandStartError> {
    super::frontend_binding::finish_reference(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
}

fn start_set_file_input_files_frontend_node_binding(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    frontend_node_id: u32,
    file_paths: Vec<String>,
    append: bool,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_document_frontend_node_binding(frontend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles {
            frontend_node_id,
            file_paths,
            append,
        },
        pending,
    }))
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
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let object_id = reference;
    start_set_file_input_files_for_runtime_object(
        &inspection,
        command_id,
        owner,
        object_id,
        files,
        append,
    )
}

fn start_set_file_input_files_preflight_dispatch(
    inspection: &RendererDomInspection<'_>,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
    file_paths: Vec<String>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let DevToolsDomNodeReference::BackendNodeId(_) = reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    let pending =
        start_document_node_snapshot_for_reference(inspection, reference.clone(), 0, false)?;
    Ok(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::SetFileInputFilesPreflight {
            reference,
            file_paths,
            append,
        },
        pending,
    })
}

fn start_set_file_input_files_for_reference_dispatch(
    inspection: &RendererDomInspection<'_>,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let pending = start_set_file_input_files_for_reference(inspection, reference, files, append)?;
    Ok(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::SetFileInputFiles,
        pending,
    })
}

fn start_set_file_input_files_for_reference(
    inspection: &RendererDomInspection<'_>,
    reference: DevToolsDomNodeReference,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => inspection
            .start_set_file_input_files_for_backend_node_id(backend_node_id, files, append)
            .map(PendingPageCommand::from_inspector_main_route)
            .map_err(PendingDomCommandStartError::renderer_error),
    }
}

fn start_set_file_input_files_for_runtime_object(
    inspection: &RendererDomInspection<'_>,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    object_id: String,
    files: Vec<SelectedFile>,
    append: bool,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let pending = inspection
        .start_set_file_input_files_for_object_id_in_inspector_session(&object_id, files, append)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::SetFileInputFilesObjectReference,
        pending,
    })
}

fn selected_files_from_paths(
    paths: &[String],
) -> Result<Vec<SelectedFile>, PendingDomCommandStartError> {
    paths
        .iter()
        .map(|path| selected_file_from_path(path))
        .collect()
}

fn selected_file_from_path(path: &str) -> Result<SelectedFile, PendingDomCommandStartError> {
    let file_path = Path::new(path);
    let metadata = fs::metadata(file_path).map_err(|_| PendingDomCommandStartError {
        code: -32000,
        message: format!("File not found : {path}"),
    })?;
    if !metadata.is_file() {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: format!("File not found : {path}"),
        });
    }
    let bytes = fs::read(file_path).map_err(|error| PendingDomCommandStartError {
        code: -32000,
        message: format!("could not read file for DOM.setFileInputFiles: {error}"),
    })?;
    let name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned();
    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as f64)
                .unwrap_or(0.0)
        });
    Ok(SelectedFile {
        bytes,
        mime_type: String::new(),
        name,
        last_modified,
    })
}
