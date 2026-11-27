//! Shared CDP file-input parsing and asynchronous Browser preparation.

use super::native::node_not_found;
use super::*;
use crate::conn::{
    CommandWorkflowContext, PreparedCommandWorkflow, ProtocolWorkflowDriver, RuntimePageScope,
};
use crate::domains::native::{self, NativeCommandStep, with_backend_node as with_backend};
use moli_core::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeOperationStep as Step,
    RendererNativeProtocolResponse as Response, RendererPageCommand as Command,
    RendererPageReply as Reply,
};

enum FileInputTarget {
    ObjectId(String),
    Node(DevToolsDomNodeReference),
}

struct FileInputRequest {
    target: FileInputTarget,
    paths: Vec<String>,
    session: Option<String>,
}

fn parse(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<FileInputRequest, PendingDomCommandStartError> {
    #[derive(serde::Deserialize)]
    struct Params {
        #[serde(flatten)]
        reference: NodeReferenceParams,
        files: Vec<String>,
    }
    let params: Params = match cmd.get_params().ok().flatten() {
        Some(params) => params,
        None => {
            return Err(PendingDomCommandStartError::invalid_params());
        }
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let target = if let Some(object_id) = params.reference.object_id {
        FileInputTarget::ObjectId(dom_object_reference_id_for_owner(
            conn,
            &owner,
            &DevToolsRemoteHandleId::from(object_id),
        ))
    } else {
        let Some(reference) = devtools_node_reference_from_ids(
            params.reference.node_id,
            params.reference.backend_node_id,
        ) else {
            return Err(PendingDomCommandStartError::invalid_params());
        };
        FileInputTarget::Node(reference)
    };
    if !params.files.is_empty() {
        conn.local_file_policy()
            .check_host_file_access()
            .map_err(PendingDomCommandStartError::renderer_error)?;
    }
    Ok(FileInputRequest {
        target,
        paths: params.files,
        session,
    })
}

pub(super) fn start_native(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> NativeCommandStep {
    let request = match parse(conn, cmd) {
        Ok(request) => request,
        Err(error) => {
            return NativeCommandStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            ));
        }
    };
    let FileInputRequest {
        target,
        paths,
        session,
    } = request;
    native::start_prepared_operation(conn, cmd, async move {
        let files = crate::file_upload::selected_files_from_paths_async(paths)
            .await
            .map_err(|error| PendingDomCommandStartError {
                code: -32000,
                message: error.to_string(),
            });
        // Retain the result until the node preflight succeeds, so a bad node
        // keeps precedence over a bad path. No filesystem work enters a Page.
        match target {
            FileInputTarget::ObjectId(object_id) => {
                let files = files.map_err(|error| anyhow::anyhow!(error.message))?;
                Ok(Operation::new(
                    Command::set_file_input_files_for_object_id(session, object_id, files, false),
                    project,
                ))
            }
            FileInputTarget::Node(reference) => {
                Ok(with_backend(session, reference, move |backend_node_id| {
                    Operation::then(
                        Command::DevTools(
                            RendererDevToolsPageCommand::DocumentNodeExistsForBackendNodeId {
                                backend_node_id,
                            },
                        ),
                        move |reply| {
                            match reply {
                            Ok(Reply::Bool(true)) => match files {
                                Ok(files) => Step::Continue(Operation::new(
                                    Command::DevTools(RendererDevToolsPageCommand::SetFileInputFilesForBackendNodeId {
                                        backend_node_id,
                                        files,
                                        append: false,
                                    }),
                                    project,
                                )),
                                Err(error) => {
                                    Step::Complete(Response::error(error.code, error.message))
                                }
                            },
                            Ok(Reply::Bool(false)) => Step::Complete(node_not_found()),
                            Err(error) => Step::Complete(Response::error(
                                -32000,
                                format!("Could not preflight file input node: {error}"),
                            )),
                            _ => unreachable!("DOM file input preflight reply"),
                        }
                        },
                    )
                }))
            }
        }
    })
}

fn project(reply: anyhow::Result<Reply>) -> Response {
    match reply {
        Ok(Reply::DevTools(RendererDevToolsPageReply::OptionalBool(Some(true)))) => {
            Response::success(json!({}))
        }
        Ok(Reply::DevTools(RendererDevToolsPageReply::OptionalBool(Some(false)))) => {
            Response::error(-32000, "UnableToSetFileInput")
        }
        Ok(Reply::DevTools(RendererDevToolsPageReply::OptionalBool(None))) => node_not_found(),
        Err(error) => Response::error(-32000, format!("Could not set file input files: {error}")),
        _ => unreachable!("DOM file input reply"),
    }
}

/// Internal CDP with AdapterReply uses the same Browser admission, but retains its typed
/// reply through the existing workflow instead of publishing a Native terminal.
pub(in crate::domains::dom) fn start_adapter_reply(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> super::super::DomCommandDispatchStep {
    use super::super::DomCommandDispatchStep;
    let request = match parse(conn, cmd) {
        Ok(request) => request,
        Err(error) => {
            return DomCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            ));
        }
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let scope = match RuntimePageScope::capture(conn, &owner) {
        Ok(scope) => scope,
        Err(error) => {
            return DomCommandDispatchStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    let access = conn.protocol_state_access();
    DomCommandDispatchStep::Workflow(
        PreparedCommandWorkflow::new(move |mut context| async move {
            let mut driver = ProtocolWorkflowDriver::new(access);
            let result = run_adapter_reply(request, scope, &mut driver, &mut context).await;
            let plan = match result {
                Ok(Some(true)) => CommandOutputPlan::success(),
                Ok(Some(false)) => CommandOutputPlan::error(-32000, "UnableToSetFileInput"),
                Ok(None) => CommandOutputPlan::error(-32000, "Could not find node with given id"),
                Err(error) => CommandOutputPlan::error(error.code, error.message),
            };
            context.complete(plan)
        })
        .cancel_on_waiter_drop(),
    )
}

async fn run_adapter_reply(
    request: FileInputRequest,
    scope: RuntimePageScope,
    driver: &mut ProtocolWorkflowDriver,
    context: &mut CommandWorkflowContext,
) -> Result<Option<bool>, PendingDomCommandStartError> {
    let FileInputRequest {
        target,
        paths,
        session,
    } = request;
    let files = crate::file_upload::selected_files_from_paths_async(paths).await;
    let start = match target {
        FileInputTarget::ObjectId(object_id) => {
            let files = files.map_err(PendingDomCommandStartError::renderer_error)?;
            return query(
                &scope,
                driver,
                context,
                move |page| {
                    page.start_set_file_input_files_for_object_id_in_inspector_session(
                        session, &object_id, files, false,
                    )
                },
                Page::finish_set_file_input_files_for_object_id,
            )
            .await;
        }
        FileInputTarget::Node(reference) => {
            let backend = match reference {
                DevToolsDomNodeReference::BackendNodeId(id) => id,
                DevToolsDomNodeReference::FrontendNodeId(id) => {
                    let binding = query(
                        &scope,
                        driver,
                        context,
                        move |page| page.start_document_frontend_node_binding(session, id),
                        Page::finish_document_frontend_node_binding,
                    )
                    .await?;
                    match binding {
                        moli_core::page::RendererDomFrontendNodeBindingResolution::BackendNodeId(id) => id,
                        moli_core::page::RendererDomFrontendNodeBindingResolution::NotFound => return Err(PendingDomCommandStartError::node_not_found()),
                    }
                }
            };
            let exists = query(
                &scope,
                driver,
                context,
                move |page| page.start_document_node_exists_for_backend_node_id(backend),
                Page::finish_document_node_exists,
            )
            .await?;
            if !exists {
                return Err(PendingDomCommandStartError::node_not_found());
            }
            let files = files.map_err(PendingDomCommandStartError::renderer_error)?;
            move |page: &mut Page| {
                page.start_set_file_input_files_for_backend_node_id(backend, files, false)
            }
        }
    };
    query(
        &scope,
        driver,
        context,
        start,
        Page::finish_set_file_input_files,
    )
    .await
}

async fn query<T: 'static>(
    scope: &RuntimePageScope,
    driver: &mut ProtocolWorkflowDriver,
    context: &mut CommandWorkflowContext,
    start: impl FnOnce(&mut Page) -> anyhow::Result<PendingPageCommand>,
    finish: impl FnOnce(&mut Page, CompletedPageCommand) -> anyhow::Result<T> + 'static,
) -> Result<T, PendingDomCommandStartError> {
    let prepared = driver
        .with_state(|state| state.prepare_dom_page_query(scope, start, finish))
        .map_err(PendingDomCommandStartError::renderer_error)?
        .map_err(PendingDomCommandStartError::renderer_error)?;
    prepared
        .run(
            driver,
            context.context_mut().renderer_output_predecessor_mut(),
        )
        .await
        .map_err(PendingDomCommandStartError::renderer_error)
}
