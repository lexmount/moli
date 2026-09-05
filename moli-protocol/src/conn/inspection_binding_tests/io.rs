use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn blob_inspection_starts_without_protocol_document() {
    blob_inspection_round_trip(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn blob_inspection_completes_without_protocol_document() {
    blob_inspection_round_trip(true).await;
}

async fn blob_object(ctx: &mut TestContext, session: Option<&str>) -> String {
    ctx.process_async(
        json!({"id": 1, "sessionId": session, "method": "Runtime.evaluate", "params": {
            "expression": "globalThis.inspectionBlob = new Blob(['bound blob'])",
        }}),
    )
    .await;
    let response = ctx.take_response_by_id(1);
    response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Blob must be a real inspector remote object: {response}"))
        .to_owned()
}

async fn blob_inspection_round_trip(start_before_move: bool) {
    let mut ctx = dom_context().await;
    let session = "SID-blob-inspection";
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-dom-inspection", session.to_owned())
    );
    ctx.conn.commit_declared_session_fixtures_for_test();
    let object_id = blob_object(&mut ctx, Some(session)).await;
    let owner = CommandOwnerScope::capture(&ctx.conn, Some(session));
    let take_document = |ctx: &mut TestContext| {
        ctx.conn
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .page_slot_mut()
            .contents
            .main_frame
            .current_document
            .take()
            .unwrap()
    };
    let mut document = (!start_before_move).then(|| take_document(&mut ctx));
    let raw = json!({"id": 2, "sessionId": session, "method": "IO.resolveBlob", "params": {
        "objectId": object_id,
    }})
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Blob inspection must start through the live session binding without a Protocol Document"
    );
    if start_before_move {
        document = Some(take_document(&mut ctx));
    }
    let mut document = document.unwrap();
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(2))
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "Blob inspection completion: {response}"
    );
    let uuid = response["result"]["uuid"].as_str().unwrap();
    let completed = document
        .page
        .start_blob_bytes_for_uuid(uuid.to_owned())
        .unwrap()
        .wait()
        .await
        .unwrap();
    let bytes = document
        .page
        .finish_blob_bytes_for_uuid(completed)
        .unwrap()
        .unwrap();
    assert_eq!(
        &*bytes, b"bound blob",
        "the reply must resolve the actual session object"
    );
    assert!(
        !ctx.conn
            .runtime_session_owner_slot_for_owner(&owner)
            .unwrap()
            .has_loaded_page()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn blob_inspection_rejects_frozen_reply_after_rebind() {
    use crate::conn::{Cmd, ParsedCdpCommand};
    use crate::domains::io::{
        IoCommandTaskStep, complete_pending_io_command, try_start_io_command_dispatch,
    };

    let mut ctx = dom_context().await;
    let object_id = blob_object(&mut ctx, None).await;
    let raw =
        json!({"id": 2, "method": "IO.resolveBlob", "params": {"objectId": object_id}}).to_string();
    let parsed = ParsedCdpCommand::parse_str(&raw).unwrap();
    let cmd = Cmd::from_parsed(&parsed).unwrap();
    let IoCommandTaskStep::Pending(pending) = try_start_io_command_dispatch(&mut ctx.conn, &cmd)
    else {
        panic!("Blob inspection must start on the original renderer");
    };
    let completed = pending.wait().await;
    let old_document = ctx
        .conn
        .runtime_session_owner_slot_mut(None)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement</title>",
        None,
    )
    .await;
    let messages = complete_pending_io_command(&mut ctx.conn, completed)
        .into_background_events(Some(2), None)
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![json!({"id": 2, "error": {
            "code": -32000, "message": "Renderer attachment changed",
        }})]
    );
    drop(old_document);
}
