use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chromiumoxide_cdp::cdp::browser_protocol::io::{CloseParams, ReadParams, ResolveBlobParams};
use moli_core::page::{CompletedPageCommand, PendingPageCommand};
use serde_json::json;

use crate::{
    conn::{
        CapturedBody, CdpConnection, Cmd, CompletedFetchResponseBodyStreamReadDispatch,
        IoStreamState, PendingFetchResponseBodyStreamRead,
        PendingFetchResponseBodyStreamReadDispatch, PendingFetchResponseBodyStreamReadStart,
    },
    domains::actions::IoAction,
    domains::command_output::CommandOutputPlan,
    domains::network::TargetIoStreamRead,
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

enum PendingIoCommandKind {
    FetchResponseBodyRead(Box<PendingFetchResponseBodyStreamReadDispatch>),
    ResolveBlob(PendingPageCommand),
    ReadBlob {
        pending: PendingPageCommand,
        handle: String,
        offset: Option<usize>,
        size: Option<usize>,
    },
}

enum CompletedIoCommandKind {
    FetchResponseBodyRead(Box<CompletedFetchResponseBodyStreamReadDispatch>),
    ResolveBlob(Result<CompletedPageCommand, String>),
    ReadBlob {
        completed: Result<CompletedPageCommand, String>,
        handle: String,
        offset: Option<usize>,
        size: Option<usize>,
    },
}

pub(crate) enum IoCommandTaskStep {
    Pending(Box<PendingIoCommandDispatch>),
    Complete(CommandOutputPlan),
}

impl PendingIoCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedIoCommandDispatch {
        let kind = match self.kind {
            PendingIoCommandKind::FetchResponseBodyRead(pending) => {
                CompletedIoCommandKind::FetchResponseBodyRead(Box::new(pending.wait().await))
            }
            PendingIoCommandKind::ResolveBlob(pending) => CompletedIoCommandKind::ResolveBlob(
                pending.wait().await.map_err(|error| error.to_string()),
            ),
            PendingIoCommandKind::ReadBlob {
                pending,
                handle,
                offset,
                size,
            } => CompletedIoCommandKind::ReadBlob {
                completed: pending.wait().await.map_err(|error| error.to_string()),
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
) -> CommandOutputPlan {
    let session_id = completed.session_id.as_deref();
    match completed.kind {
        CompletedIoCommandKind::FetchResponseBodyRead(completed) => {
            let read = conn.finish_pending_fetch_response_body_stream_read_for_stream_owner(
                session_id, *completed,
            );
            read_fetch_response_body_stream_output_plan(read)
        }
        CompletedIoCommandKind::ResolveBlob(completed) => {
            complete_resolve_blob_command(conn, session_id, completed)
        }
        CompletedIoCommandKind::ReadBlob {
            completed,
            handle,
            offset,
            size,
        } => complete_read_blob_command(conn, session_id, completed, handle, offset, size),
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
            if let Some(read) = read_buffered_stream(conn, cmd.session_id, handle, offset, size) {
                return IoCommandTaskStep::Complete(read_output_plan(&read.bytes, read.eof));
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
            .map_err(|error| error.to_string())
        });
    match pending {
        Ok(pending) => IoCommandTaskStep::Pending(Box::new(PendingIoCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: PendingIoCommandKind::ResolveBlob(pending),
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
        .and_then(|page| {
            page.start_blob_bytes_for_uuid(uuid.to_owned())
                .map_err(|error| error.to_string())
        });
    match pending {
        Ok(pending) => IoCommandTaskStep::Pending(Box::new(PendingIoCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: PendingIoCommandKind::ReadBlob {
                pending,
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
    session_id: Option<&str>,
    completed: Result<CompletedPageCommand, String>,
) -> CommandOutputPlan {
    let uuid = completed.and_then(|completed| {
        conn.loaded_page_mut_for_protocol_access(session_id)
            .and_then(|page| {
                page.finish_resolve_blob_object(completed)
                    .map_err(|error| error.to_string())
            })
    });
    match uuid {
        Ok(uuid) => CommandOutputPlan::result(json!({ "uuid": uuid })),
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

fn complete_read_blob_command(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    completed: Result<CompletedPageCommand, String>,
    handle: String,
    offset: Option<usize>,
    size: Option<usize>,
) -> CommandOutputPlan {
    let bytes = completed
        .and_then(|completed| {
            conn.loaded_page_mut_for_protocol_access(session_id)
                .and_then(|page| {
                    page.finish_blob_bytes_for_uuid(completed)
                        .map_err(|error| error.to_string())
                })
        })
        .ok()
        .flatten();
    let Some(bytes) = bytes else {
        return CommandOutputPlan::error(-32000, "Read failed");
    };
    let Ok(slot) = conn.runtime_session_owner_slot_mut(session_id) else {
        return CommandOutputPlan::error(-32000, "Read failed");
    };
    slot.insert_io_stream_body_source(handle.clone(), CapturedBody::from_shared_bytes(bytes), 0);
    let Some(read) = read_buffered_stream(conn, session_id, &handle, offset, size) else {
        return CommandOutputPlan::error(-32000, "Read failed");
    };
    read_output_plan(&read.bytes, read.eof)
}

fn read_fetch_response_body_stream_output_plan(
    read: PendingFetchResponseBodyStreamRead,
) -> CommandOutputPlan {
    match read {
        PendingFetchResponseBodyStreamRead::NotFound => {
            CommandOutputPlan::error(-32000, "StreamHandleNotFound")
        }
        PendingFetchResponseBodyStreamRead::Read { bytes, eof } => read_output_plan(&bytes, eof),
        PendingFetchResponseBodyStreamRead::Failed(message) => {
            CommandOutputPlan::error(-32000, message)
        }
    }
}

fn read_output_plan(chunk: &[u8], eof: bool) -> CommandOutputPlan {
    match std::str::from_utf8(chunk) {
        Ok(data) => CommandOutputPlan::result(json!({
            "base64Encoded": false,
            "data": data,
            "eof": eof,
        })),
        Err(_) => CommandOutputPlan::result(json!({
            "base64Encoded": true,
            "data": encode_base64(chunk),
            "eof": eof,
        })),
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

fn read_buffered_stream(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    handle: &str,
    offset: Option<usize>,
    size: Option<usize>,
) -> Option<TargetIoStreamRead> {
    if let Some(read) = conn.read_io_stream_for_stream_owner(session_id, handle, offset, size) {
        return Some(read);
    }
    Some(read_io_stream_state(
        conn.global_io_streams.get_mut(handle)?,
        offset,
        size,
    ))
}

fn read_io_stream_state(
    stream: &mut IoStreamState,
    offset: Option<usize>,
    size: Option<usize>,
) -> TargetIoStreamRead {
    let stream_len = stream.len();
    let start = offset.unwrap_or(stream.offset).min(stream_len);
    stream.offset = start;
    let requested_len = size.unwrap_or(DEFAULT_IO_READ_SIZE);
    let bytes = stream.read_range(start, requested_len);
    let end = start.saturating_add(bytes.len()).min(stream_len);
    stream.offset = end;
    TargetIoStreamRead {
        bytes,
        eof: end >= stream_len,
    }
}

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
        let step = ctx.conn.start_command_dispatch(&raw);
        match step {
            CdpCommandTaskStep::Complete(outcome) => {
                let (messages, _) = outcome.into_parts();
                ctx.sent.extend(messages);
            }
            CdpCommandTaskStep::Pending(pending) => {
                let step = ctx
                    .conn
                    .complete_pending_command_dispatch(pending.wait().await)
                    .await;
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
        ctx.conn.browser_context = Some(bc);

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
        ctx.conn.browser_context = Some(bc);

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
        ctx.conn.global_io_streams.insert(
            "BROWSER-STREAM-1".into(),
            IoStreamState::from_bytes(vec![0, 255, b'a'], 0),
        );

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
        ctx.conn.browser_context = Some(bc);

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
        ctx.conn.browser_context = Some(bc);

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
        ctx.conn.browser_context = Some(bc);

        let handle = ctx
            .conn
            .open_io_stream_for_session_owner(Some("SID-background"), b"background-owner".to_vec())
            .expect("background target stream should open");
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

        let background_route = ctx
            .conn
            .target_session_route_for_target_id("TID-background")
            .expect("background target route");
        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(background_route));
        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 10,
                "method": "IO.read",
                "params": { "handle": handle, "size": 10 }
            }),
        )
        .await;
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        ctx.expect_result(
            10,
            json!({ "base64Encoded": false, "data": "background", "eof": false }),
            None,
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

        let handle = ctx
            .conn
            .open_io_stream_for_session_owner(Some("SID-background"), b"background-owner".to_vec())
            .expect("background target stream should open again");
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
