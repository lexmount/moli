use super::*;

pub(super) fn prepare(conn: &CdpConnection, cmd: &Cmd<'_>) -> Result<Operation, StartError> {
    #[derive(serde::Deserialize)]
    struct Params {
        #[serde(flatten)]
        reference: NodeReferenceParams,
        files: Vec<String>,
    }
    let params: Params = cmd
        .get_params()
        .ok()
        .flatten()
        .ok_or_else(StartError::invalid_params)?;
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    // File access belongs to Browser admission. For node ids, retain an error
    // until the renderer preflight has succeeded, preserving node-error priority.
    let files = crate::domains::dom::set_file_input::selected_files_from_paths(&params.files);
    if let Some(object_id) = params.reference.object_id {
        let object_id = dom_object_reference_id_for_owner(
            conn,
            &owner,
            &DevToolsRemoteHandleId::from(object_id),
        );
        return Ok(Operation::new(
            Command::set_file_input_files_for_object_id(session, object_id, files?, false),
            project,
        ));
    }
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    )
    .ok_or_else(StartError::invalid_params)?;
    Ok(with_backend(
        session,
        reference,
        OwnerTurn,
        move |backend_node_id| {
            Operation::then(
                Command::DocumentNodeSnapshotForBackendNodeId {
                    backend_node_id,
                    depth: 0,
                    pierce: false,
                },
                move |reply| match reply {
                    Ok(Reply::OptionalDocumentNodeObjectSnapshot(snapshot))
                        if snapshot.is_some() =>
                    {
                        match files {
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
                        }
                    }
                    Ok(Reply::OptionalDocumentNodeObjectSnapshot(_)) => {
                        Step::Complete(node_not_found())
                    }
                    Err(error) => Step::Complete(Response::error(
                        -32000,
                        format!("Could not preflight file input node: {error}"),
                    )),
                    _ => unreachable!("DOM file input preflight reply"),
                },
            )
        },
    ))
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
