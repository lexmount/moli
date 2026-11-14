use super::*;

enum FileInputTarget {
    ObjectId(String),
    Node(DevToolsDomNodeReference),
}

pub(super) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> NativeCommandStep {
    #[derive(serde::Deserialize)]
    struct Params {
        #[serde(flatten)]
        reference: NodeReferenceParams,
        files: Vec<String>,
    }
    let params: Params = match cmd.get_params().ok().flatten() {
        Some(params) => params,
        None => {
            return NativeCommandStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
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
            return NativeCommandStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
        };
        FileInputTarget::Node(reference)
    };
    native::start_prepared_operation(conn, cmd, async move {
        let files = crate::file_upload::selected_files_from_paths_async(params.files)
            .await
            .map_err(|error| StartError {
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
                        Command::DocumentNodeExistsForBackendNodeId { backend_node_id },
                        move |reply| match reply {
                            Ok(Reply::Bool(true)) => match files {
                                Ok(files) => Step::Continue(Operation::new(
                                    Command::SetFileInputFilesForBackendNodeId {
                                        backend_node_id,
                                        files,
                                        append: false,
                                    },
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
                        },
                    )
                }))
            }
        }
    })
}

fn project(reply: anyhow::Result<Reply>) -> Response {
    match reply {
        Ok(Reply::OptionalBool(Some(true))) => Response::success(json!({})),
        Ok(Reply::OptionalBool(Some(false))) => Response::error(-32000, "UnableToSetFileInput"),
        Ok(Reply::OptionalBool(None)) => node_not_found(),
        Err(error) => Response::error(-32000, format!("Could not set file input files: {error}")),
        _ => unreachable!("DOM file input reply"),
    }
}
