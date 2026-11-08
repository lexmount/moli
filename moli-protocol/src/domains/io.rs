#[cfg(test)]
use crate::conn::IoStreamState;
use anyhow::Result;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chromiumoxide_cdp::cdp::browser_protocol::io::{CloseParams, ReadParams, ResolveBlobParams};
use moli_core::page::{CompletedPageCommand, PendingPageCommand};
use serde_json::json;

use crate::{
    conn::{
        CapturedBody, CdpConnection, Cmd, CompletedFetchResponseBodyStreamReadDispatch,
        PendingFetchResponseBodyStreamRead, PendingFetchResponseBodyStreamReadDispatch,
        PendingFetchResponseBodyStreamReadStart,
    },
    domains::actions::IoAction,
    domains::command_output::CommandOutputPlan,
    domains::network::PreparedIoStreamRead,
};

const DEFAULT_IO_READ_SIZE: usize = 10 * 1024 * 1024;

pub(crate) struct PendingIoCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: PendingIoCommandKind,
}

pub(crate) struct CompletedIoCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: CompletedIoCommandKind,
}

struct IoPageCommand<T> {
    command: T,
    owner: crate::conn::CommandOwnerScope,
    host: Option<crate::conn::ProtocolPageHostScope>,
}

impl IoPageCommand<PendingPageCommand> {
    fn new(conn: &CdpConnection, session: Option<&str>, command: PendingPageCommand) -> Self {
        let owner = crate::conn::CommandOwnerScope::capture(conn, session);
        let host = crate::conn::ProtocolPageHostScope::capture(conn, &owner).ok();
        let owner = host
            .as_ref()
            .map(|host| host.owner().clone())
            .unwrap_or(owner);
        Self {
            command,
            owner,
            host,
        }
    }

    async fn wait(self) -> IoPageCommand<Result<CompletedPageCommand>> {
        IoPageCommand {
            command: self.command.wait().await,
            owner: self.owner,
            host: self.host,
        }
    }
}

enum PendingIoCommandKind {
    FetchResponseBodyRead(Box<PendingFetchResponseBodyStreamReadDispatch>),
    ResolveBlob(IoPageCommand<PendingPageCommand>),
    ReadBlob {
        pending: IoPageCommand<PendingPageCommand>,
        handle: String,
        offset: Option<usize>,
        size: Option<usize>,
    },
}

enum CompletedIoCommandKind {
    FetchResponseBodyRead(Box<CompletedFetchResponseBodyStreamReadDispatch>),
    ResolveBlob(IoPageCommand<Result<CompletedPageCommand>>),
    ReadBlob {
        completed: IoPageCommand<Result<CompletedPageCommand>>,
        handle: String,
        offset: Option<usize>,
        size: Option<usize>,
    },
}

pub(crate) enum IoCommandTaskStep {
    Workflow(Box<crate::conn::PreparedCommandWorkflow>),
    Pending(Box<PendingIoCommandDispatch>),
    Complete(CommandOutputPlan),
}

impl PendingIoCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedIoCommandDispatch {
        let kind = match self.kind {
            PendingIoCommandKind::FetchResponseBodyRead(pending) => {
                CompletedIoCommandKind::FetchResponseBodyRead(Box::new(pending.wait().await))
            }
            PendingIoCommandKind::ResolveBlob(pending) => {
                CompletedIoCommandKind::ResolveBlob(pending.wait().await)
            }
            PendingIoCommandKind::ReadBlob {
                pending,
                handle,
                offset,
                size,
            } => CompletedIoCommandKind::ReadBlob {
                completed: pending.wait().await,
                handle,
                offset,
                size,
            },
        };
        CompletedIoCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            kind,
        }
    }
}

impl CompletedIoCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_io_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> IoCommandTaskStep {
    match cmd.parse_action::<IoAction>() {
        Some(IoAction::Read) => start_read_command(conn, cmd),
        Some(IoAction::Close) => IoCommandTaskStep::Complete(close_command_output_plan(conn, cmd)),
        Some(IoAction::ResolveBlob) => start_resolve_blob_command(conn, cmd),
        None => IoCommandTaskStep::Complete(CommandOutputPlan::error(-32601, "UnknownMethod")),
    }
}

pub(crate) fn complete_pending_io_command(
    conn: &mut CdpConnection,
    completed: CompletedIoCommandDispatch,
) -> IoCommandTaskStep {
    let session_id = completed.session_id.as_deref();
    match completed.kind {
        CompletedIoCommandKind::FetchResponseBodyRead(completed) => {
            let read = conn.finish_pending_fetch_response_body_stream_read_for_stream_owner(
                session_id, *completed,
            );
            read_fetch_response_body_stream_step(read)
        }
        CompletedIoCommandKind::ResolveBlob(completed) => {
            IoCommandTaskStep::Complete(complete_resolve_blob_command(conn, completed))
        }
        CompletedIoCommandKind::ReadBlob {
            completed,
            handle,
            offset,
            size,
        } => complete_read_blob_command(conn, completed, handle, offset, size),
    }
}

fn start_read_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> IoCommandTaskStep {
    let params: ReadParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
        }
    };
    let Some(offset) = optional_i64_to_usize(params.offset) else {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
    };
    let Some(size) = optional_i64_to_usize(params.size) else {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
    };
    let handle = params.handle.as_ref();
    match conn.start_pending_fetch_response_body_stream_read_for_stream_owner(
        cmd.session_id,
        handle,
        offset,
        size,
    ) {
        PendingFetchResponseBodyStreamReadStart::Pending(pending) => {
            IoCommandTaskStep::Pending(Box::new(PendingIoCommandDispatch {
                command_id: cmd.id,
                session_id: cmd.session_id.map(str::to_owned),
                kind: PendingIoCommandKind::FetchResponseBodyRead(pending),
            }))
        }
        PendingFetchResponseBodyStreamReadStart::OffsetNotSupported => IoCommandTaskStep::Complete(
            CommandOutputPlan::error(-32000, "OffsetNotSupportedForStream"),
        ),
        PendingFetchResponseBodyStreamReadStart::NotFound => {
            if let Some(read) =
                prepare_buffered_stream_read(conn, cmd.session_id, handle, offset, size)
            {
                return buffered_read_workflow(read);
            }
            if let Some(uuid) = handle.strip_prefix("blob:") {
                return start_read_blob_command(conn, cmd, uuid, offset, size);
            }
            IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "StreamHandleNotFound"))
        }
    }
}

fn start_resolve_blob_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> IoCommandTaskStep {
    let params: ResolveBlobParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
        }
    };
    let inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let pending = conn
        .loaded_page_mut_for_protocol_access(cmd.session_id)
        .and_then(|page| {
            page.start_resolve_blob_object_in_inspector_session(
                inspector_session_id,
                params.object_id.as_ref().to_owned(),
            )
        });
    match pending {
        Ok(pending) => IoCommandTaskStep::Pending(Box::new(PendingIoCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: PendingIoCommandKind::ResolveBlob(IoPageCommand::new(
                conn,
                cmd.session_id,
                pending,
            )),
        })),
        Err(message) => IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

fn start_read_blob_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    uuid: &str,
    offset: Option<usize>,
    size: Option<usize>,
) -> IoCommandTaskStep {
    let pending = conn
        .loaded_page_mut_for_protocol_access(cmd.session_id)
        .and_then(|page| page.start_blob_bytes_for_uuid(uuid.to_owned()));
    match pending {
        Ok(pending) => IoCommandTaskStep::Pending(Box::new(PendingIoCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: PendingIoCommandKind::ReadBlob {
                pending: IoPageCommand::new(conn, cmd.session_id, pending),
                handle: format!("blob:{uuid}"),
                offset,
                size,
            },
        })),
        Err(_) => IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "Read failed")),
    }
}

fn complete_resolve_blob_command(
    conn: &mut CdpConnection,
    completed: IoPageCommand<Result<CompletedPageCommand>>,
) -> CommandOutputPlan {
    if completed
        .host
        .as_ref()
        .is_some_and(|host| !host.is_current(conn))
    {
        return CommandOutputPlan::error(-32000, crate::conn::NoDocumentLoaded);
    }
    let uuid = completed.command.and_then(|command| {
        conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)
            .and_then(|page| page.finish_resolve_blob_object(command))
    });
    match uuid {
        Ok(uuid) => CommandOutputPlan::result(json!({ "uuid": uuid })),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}

fn complete_read_blob_command(
    conn: &mut CdpConnection,
    completed: IoPageCommand<Result<CompletedPageCommand>>,
    handle: String,
    offset: Option<usize>,
    size: Option<usize>,
) -> IoCommandTaskStep {
    if completed
        .host
        .as_ref()
        .is_some_and(|host| !host.is_current(conn))
    {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, IoReadFailed));
    }
    let bytes = completed
        .command
        .and_then(|completion| {
            conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)
                .and_then(|page| page.finish_blob_bytes_for_uuid(completion))
        })
        .ok()
        .flatten();
    let Some(bytes) = bytes else {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, IoReadFailed));
    };
    // Finishing the accepted Native read is allowed during cleanup; opening
    // another protocol resource requires the original operation authority.
    if completed
        .host
        .as_ref()
        .is_some_and(|host| !host.can_start_operation(conn))
    {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, IoReadFailed));
    }
    let Some(slot) = conn.runtime_session_owner_slot_mut_for_owner(&completed.owner) else {
        return IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, IoReadFailed));
    };
    // A concurrent read may have installed this same immutable Blob already.
    // Keep its cursor and admission order instead of replacing that resource.
    if slot.io_stream(&handle).is_none() {
        slot.insert_io_stream_body_source(
            handle.clone(),
            CapturedBody::from_shared_bytes(bytes),
            0,
        );
    }
    let stream = slot
        .io_stream(&handle)
        .expect("the original Blob stream was just installed or retained");
    buffered_read_workflow(stream.prepare_read(offset, size))
}

fn read_fetch_response_body_stream_step(
    read: PendingFetchResponseBodyStreamRead,
) -> IoCommandTaskStep {
    match read {
        PendingFetchResponseBodyStreamRead::NotFound => {
            IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "StreamHandleNotFound"))
        }
        PendingFetchResponseBodyStreamRead::Read { bytes, eof } => io_read_workflow(async move {
            crate::conn::run_body_storage_work(move || Ok(read_output_value(&bytes, eof))).await
        }),
        PendingFetchResponseBodyStreamRead::Failed(error) => {
            IoCommandTaskStep::Complete(CommandOutputPlan::error(-32000, format!("{error:#}")))
        }
    }
}

pub(crate) fn read_output_value(chunk: &[u8], eof: bool) -> serde_json::Value {
    match std::str::from_utf8(chunk) {
        Ok(data) => json!({"base64Encoded": false, "data": data, "eof": eof}),
        Err(_) => json!({"base64Encoded": true, "data": encode_base64(chunk), "eof": eof}),
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    BASE64_STANDARD.encode(bytes)
}

fn optional_i64_to_usize(value: Option<i64>) -> Option<Option<usize>> {
    value.map(usize::try_from).transpose().ok()
}

fn close_command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: CloseParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if !remove_stream(conn, cmd.session_id, params.handle.as_ref()) {
        return CommandOutputPlan::error(-32000, "StreamHandleNotFound");
    }
    CommandOutputPlan::success()
}

fn prepare_buffered_stream_read(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    handle: &str,
    offset: Option<usize>,
    size: Option<usize>,
) -> Option<PreparedIoStreamRead> {
    if let Some(stream) = conn.io_stream_for_stream_owner(session_id, handle) {
        return Some(stream.prepare_read(offset, size));
    }
    Some(
        conn.global_io_streams
            .get(handle)?
            .prepare_read(offset, Some(size.unwrap_or(DEFAULT_IO_READ_SIZE))),
    )
}

fn buffered_read_workflow(read: PreparedIoStreamRead) -> IoCommandTaskStep {
    io_read_workflow(read.run())
}

fn io_read_workflow(
    work: impl std::future::Future<Output = Result<serde_json::Value>> + 'static,
) -> IoCommandTaskStep {
    IoCommandTaskStep::Workflow(Box::new(
        crate::conn::PreparedCommandWorkflow::new(move |context| async move {
            match work.await {
                Ok(value) => context.complete(CommandOutputPlan::result(value)),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .cancel_on_waiter_drop(),
    ))
}

#[cfg(test)]
fn read_io_stream_state(
    stream: &mut IoStreamState,
    offset: Option<usize>,
    size: Option<usize>,
) -> crate::domains::network::TargetIoStreamRead {
    stream.read_for_test(offset, Some(size.unwrap_or(DEFAULT_IO_READ_SIZE)))
}

#[derive(Debug, thiserror::Error)]
#[error("Read failed")]
struct IoReadFailed;

fn remove_stream(conn: &mut CdpConnection, session_id: Option<&str>, handle: &str) -> bool {
    if conn.close_pending_fetch_response_body_stream_for_stream_owner(session_id, handle) {
        return true;
    }
    if conn.close_io_stream_for_stream_owner(session_id, handle) {
        return true;
    }
    conn.global_io_streams.remove(handle).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DEFAULT_IO_READ_SIZE, read_io_stream_state};
    use crate::{
        conn::{BrowserContext, CdpCommandTaskStep, IoStreamState, PageTargetHost},
        testing::TestContext,
    };

    async fn process_via_command_dispatch(ctx: &mut TestContext, msg: serde_json::Value) {
        let raw = serde_json::to_string(&msg).expect("test command should serialize");
        let step = ctx.with_fixture(|conn| conn.start_command_dispatch(&raw));
        match step {
            CdpCommandTaskStep::Complete(outcome) => {
                let (messages, _) = outcome.into_parts();
                ctx.sent.extend(messages);
            }
            CdpCommandTaskStep::Pending(pending) => {
                let step = {
                    let completed = ctx.drive_owned_wait(pending.wait()).await;
                    ctx.state
                        .with_state(|state| {
                            state.complete_command_dispatch_with_context(
                                completed,
                                &mut crate::conn::CommandDispatchContext::default(),
                            )
                        })
                        .expect("live test owner")
                };
                let CdpCommandTaskStep::Complete(outcome) = step else {
                    panic!("IO.read should complete after one pending body-stream read");
                };
                let (messages, _) = outcome.into_parts();
                ctx.sent.extend(messages);
            }
        }
    }

    #[tokio::test]
    async fn read_supports_offsets_and_eof() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.insert_io_stream("STREAM-1".into(), b"abcdef".to_vec(), 0);
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));

        ctx.process_async(json!({
            "id": 1,
            "method": "IO.read",
            "params": { "handle": "STREAM-1", "size": 2 }
        }))
        .await;
        ctx.expect_result(
            1,
            json!({ "base64Encoded": false, "data": "ab", "eof": false }),
            None,
        );

        ctx.process_async(json!({
            "id": 2,
            "method": "IO.read",
            "params": { "handle": "STREAM-1", "offset": 4 }
        }))
        .await;
        ctx.expect_result(
            2,
            json!({ "base64Encoded": false, "data": "ef", "eof": true }),
            None,
        );
    }

    #[tokio::test]
    async fn close_removes_stream_handle() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.insert_io_stream("STREAM-1".into(), b"abcdef".to_vec(), 0);
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));

        ctx.process_async(json!({
            "id": 3,
            "method": "IO.close",
            "params": { "handle": "STREAM-1" }
        }))
        .await;
        ctx.expect_result(3, json!({}), None);

        ctx.process_async(json!({
            "id": 4,
            "method": "IO.read",
            "params": { "handle": "STREAM-1" }
        }))
        .await;
        ctx.expect_error(4, -32000, "StreamHandleNotFound");
    }

    #[tokio::test]
    async fn read_supports_browser_level_stream_handles_without_browser_context() {
        let mut ctx = TestContext::new();
        ctx.with_fixture(|conn| {
            conn.global_io_streams.insert(
                "BROWSER-STREAM-1".into(),
                IoStreamState::from_bytes(vec![0, 255, b'a'], 0),
            )
        });

        ctx.process_async(json!({
            "id": 5,
            "method": "IO.read",
            "params": { "handle": "BROWSER-STREAM-1" }
        }))
        .await;
        ctx.expect_result(
            5,
            json!({ "base64Encoded": true, "data": "AP9h", "eof": true }),
            None,
        );
    }

    #[tokio::test]
    async fn read_large_stream_handle_uses_captured_body_backing() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.insert_io_stream("STREAM-2".into(), vec![b'x'; 1024 * 1024 + 8], 0);
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));

        ctx.process_async(json!({
            "id": 6,
            "method": "IO.read",
            "params": { "handle": "STREAM-2", "offset": 1048574, "size": 8 }
        }))
        .await;
        ctx.expect_result(
            6,
            json!({ "base64Encoded": false, "data": "xxxxxxxx", "eof": false }),
            None,
        );
    }

    #[test]
    fn buffered_stream_default_read_size_matches_chromium() {
        let mut stream = IoStreamState::from_bytes(vec![b'x'; DEFAULT_IO_READ_SIZE + 3], 0);

        let first = read_io_stream_state(&mut stream, None, None);
        assert_eq!(first.bytes.len(), DEFAULT_IO_READ_SIZE);
        assert!(!first.eof);

        let second = read_io_stream_state(&mut stream, None, None);
        assert_eq!(second.bytes, b"xxx");
        assert!(second.eof);
    }

    #[tokio::test]
    async fn read_command_dispatch_handles_buffered_stream_without_fallback() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.insert_io_stream("STREAM-DISPATCH".into(), b"dispatch".to_vec(), 0);
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 7,
                "method": "IO.read",
                "params": { "handle": "STREAM-DISPATCH", "size": 4 }
            }),
        )
        .await;

        ctx.expect_result(
            7,
            json!({ "base64Encoded": false, "data": "disp", "eof": false }),
            None,
        );
    }

    #[tokio::test]
    async fn target_scoped_stream_handle_requires_matching_session_owner() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-io-owner".to_owned());
        bc.set_active_target_id("TID-active".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        bc.insert_page_target_host(PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank#background".to_owned(),
        ));
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));

        let handle = ctx.with_fixture(|conn| {
            conn.open_io_stream_for_session_owner(
                Some("SID-background"),
                b"background-owner".to_vec(),
            )
            .expect("background target stream should open")
        });
        assert_eq!(handle, "BID-io-owner:TID-background:STREAM-1");

        ctx.process_async(json!({
            "id": 8,
            "sessionId": "SID-active",
            "method": "IO.read",
            "params": { "handle": handle, "size": 10 }
        }))
        .await;
        ctx.expect_error(8, -32000, "StreamHandleNotFound");
        ctx.process_async(json!({
            "id": 9,
            "sessionId": "SID-active",
            "method": "IO.close",
            "params": { "handle": handle }
        }))
        .await;
        ctx.expect_error(9, -32000, "StreamHandleNotFound");

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 10,
                "sessionId": "SID-background",
                "method": "IO.read",
                "params": { "handle": handle, "size": 10 }
            }),
        )
        .await;
        ctx.expect_result(
            10,
            json!({ "base64Encoded": false, "data": "background", "eof": false }),
            Some("SID-background"),
        );

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 11,
                "sessionId": "SID-background",
                "method": "IO.close",
                "params": { "handle": handle }
            }),
        )
        .await;
        ctx.expect_result(11, json!({}), Some("SID-background"));

        let handle = ctx.with_fixture(|conn| {
            conn.open_io_stream_for_session_owner(
                Some("SID-background"),
                b"background-owner".to_vec(),
            )
            .expect("background target stream should open again")
        });
        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 12,
                "sessionId": "SID-background",
                "method": "IO.close",
                "params": { "handle": handle }
            }),
        )
        .await;
        ctx.expect_result(12, json!({}), Some("SID-background"));

        ctx.process_async(json!({
            "id": 13,
            "sessionId": "SID-background",
            "method": "IO.read",
            "params": { "handle": handle }
        }))
        .await;
        ctx.expect_error(13, -32000, "StreamHandleNotFound");
    }
}

#[cfg(test)]
mod owned_stream_tests {
    use super::*;
    use crate::conn::{CdpCommandTaskStep, IoStreamState};

    #[tokio::test]
    async fn buffered_read_survives_close_without_advancing_a_reused_stream_handle() {
        tokio::task::LocalSet::new().run_until(Box::pin(async {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        ctx.with_fixture(|conn| conn.global_io_streams.insert("BROWSER-STREAM-reused".into(), IoStreamState::from_bytes(b"old body".to_vec(), 0)));
        let raw = json!({"id":1,"method":"IO.read","params":{"handle":"BROWSER-STREAM-reused","size":3}}).to_string();
        let CdpCommandTaskStep::Pending(read) = ctx.with_fixture(|conn| conn.start_command_dispatch(&raw)) else { panic!("read owns an external wait") };
        let close = ctx.with_fixture(|conn| conn.start_command_dispatch(&json!({"id":2,"method":"IO.close","params":{"handle":"BROWSER-STREAM-reused"}}).to_string()));
        let CdpCommandTaskStep::Complete(closed) = close else { panic!("close must progress before the read task is polled") };
        let (messages, _) = closed.into_parts();
        assert_eq!(messages[0], json!({"id":2,"result":{}}));
        ctx.with_fixture(|conn| conn.global_io_streams.insert("BROWSER-STREAM-reused".into(), IoStreamState::from_bytes(b"new body".to_vec(), 0)));
        let completed = ctx.drive_owned_wait(read.wait()).await;
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let mut step = ctx.state.with_state(|state| state.complete_command_dispatch_with_context(completed, &mut command_context)).unwrap();
        loop {
            step = match step {
                CdpCommandTaskStep::Complete(output) => {
                    let (messages, _) = output.into_parts();
                    assert_eq!(messages[0], json!({"id":1,"result":{"data":"old","base64Encoded":false,"eof":false}}));
                    break;
                }
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = ctx.drive_owned_wait(pending.wait()).await;
                    ctx.state.with_state(|state| state.complete_command_dispatch_with_context(completed, &mut command_context)).unwrap()
                },
            };
        }
        let value = ctx.with_fixture(|conn| conn.global_io_streams.get("BROWSER-STREAM-reused").unwrap().prepare_read(None, Some(3)).run()).await.unwrap();
        assert_eq!(value["data"], "new");
        })).await;
    }

    #[tokio::test]
    async fn blob_read_completion_cannot_open_a_stream_while_its_owner_is_closing() {
        tokio::task::LocalSet::new().run_until(Box::pin(async {
            for close_target in [false, true] {
                let mut context = crate::testing::TestContext::new();
                let mut browser = crate::conn::BrowserContext::new("BID-blob-closing".into());
                browser.set_active_target_id("TID-blob-closing");
                browser.attach_active_session("SID-blob-closing");
                context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
                context.install_navigation_fixture_for_session_owner("about:blank", Some("SID-blob-closing")).await;
                context.process_async(json!({"id":1,"sessionId":"SID-blob-closing","method":"Runtime.evaluate","params":{"expression":"new Blob(['closing-body'])"}})).await;
                let object = context.take_response_by_id(1)["result"]["result"]["objectId"].as_str().unwrap().to_owned();
                context.process_async(json!({"id":2,"sessionId":"SID-blob-closing","method":"IO.resolveBlob","params":{"objectId":object}})).await;
                let uuid = context.take_response_by_id(2)["result"]["uuid"].as_str().unwrap().to_owned();
                let handle = format!("blob:{uuid}");
                let raw = json!({"id":3,"sessionId":"SID-blob-closing","method":"IO.read","params":{"handle":handle}}).to_string();
                let CdpCommandTaskStep::Pending(read) = context.with_fixture(|conn| conn.start_command_dispatch(&raw)) else { panic!("the Blob bytes have an original Native read") };
                let completed = read.wait().await;
                let owner = crate::conn::CommandOwnerScope::for_session("SID-blob-closing");
                let host = context.with_fixture(|conn| crate::conn::ProtocolPageHostScope::capture(conn, &owner)).unwrap();
                if close_target {
                    context.with_fixture(|conn| conn.mark_page_target_closing(&owner));
                } else {
                    let route = context.with_fixture(|conn| conn.session_route(Some("SID-blob-closing")).unwrap());
                    let plan = crate::conn::SessionDisposalPlan::for_session_route("SID-blob-closing", &route).unwrap();
                    context.with_fixture(|conn| conn.mark_session_closing(&plan));
                }
                context.with_fixture(|conn| { assert!(host.is_current(conn), "Closing keeps the original read receipt's owner") });
                context.with_fixture(|conn| { assert!(!host.can_start_operation(conn)) });
                let CdpCommandTaskStep::Complete(output) = context.state.with_state(|state| state.complete_command_dispatch_with_context(completed, &mut crate::conn::CommandDispatchContext::default())).unwrap() else { panic!("a closing owner must not reopen a stream") };
                let (messages, _) = output.into_parts();
                assert_eq!(messages[0], json!({"id":3,"sessionId":"SID-blob-closing","error":{"code":-32000,"message":"Read failed"}}));
                context.with_fixture(|conn| { assert!(conn.runtime_session_owner_slot_for_owner(host.owner()).unwrap().io_stream(&handle).is_none()) });
            }
        })).await;
    }
}
