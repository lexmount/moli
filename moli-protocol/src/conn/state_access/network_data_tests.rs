use super::*;
use crate::automation::*;
use crate::conn::{AutomationCommandTaskStep, CapturedBody, ProtocolStateOwner};

fn context() -> AutomationContext {
    AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: None,
        target_id: None,
        browser_context_id: None,
    }
}

fn install_collector(conn: &mut CdpConnection, body: CapturedBody) {
    conn.network_data_collectors
        .add_collector(DevToolsAddNetworkDataCollectorCommand {
            context: context(),
            collector_id: "reusable".into(),
            data_types: vec![DevToolsNetworkDataType::Response],
            max_encoded_data_size: 4_000_000,
            target_ids: Vec::new(),
            browser_context_ids: Vec::new(),
        })
        .unwrap();
    record_body(conn, body);
}

fn record_body(conn: &mut CdpConnection, body: CapturedBody) {
    conn.network_data_collectors.record_collected_body(
        "REQ-original".into(),
        DevToolsNetworkDataType::Response,
        body,
        ["reusable".into()],
        true,
    );
}

fn read_command(disown: bool) -> AutomationCommand {
    AutomationCommand::GetNetworkData(DevToolsGetNetworkDataCommand {
        context: context(),
        request_id: "REQ-original".into(),
        data_type: DevToolsNetworkDataType::Response,
        collector: Some("reusable".into()),
        disown,
    })
}

async fn finish(
    handle: &ProtocolStateHandle,
    mut step: AutomationCommandTaskStep,
) -> Result<AutomationResult, DevToolsError> {
    loop {
        step = match step {
            AutomationCommandTaskStep::Complete(output) => {
                let (result, scheduler) = output.into_parts();
                assert!(scheduler.is_empty());
                return result;
            }
            AutomationCommandTaskStep::Pending(pending) => {
                let completed = pending.wait().await;
                handle
                    .with_state(move |state| {
                        state
                            .connection
                            .complete_automation_command_dispatch(completed)
                    })
                    .unwrap()
            }
        };
    }
}

#[tokio::test]
async fn network_data_late_disown_cannot_revoke_a_recreated_collector() {
    tokio::task::LocalSet::new()
        .run_until(Box::pin(async {
            for command in [
                read_command(true),
                AutomationCommand::DisownNetworkData(DevToolsDisownNetworkDataCommand {
                    context: context(),
                    request_id: "REQ-original".into(),
                    data_type: DevToolsNetworkDataType::Response,
                    collector_id: "reusable".into(),
                }),
            ] {
                let mut conn = CdpConnection::new();
                install_collector(&mut conn, CapturedBody::from_bytes(b"old".to_vec()));
                let owner = ProtocolStateOwner::new(conn);
                let handle = owner.handle();
                let pending = handle
                    .with_state(|state| state.connection.start_automation_command_dispatch(command))
                    .unwrap();
                assert!(matches!(pending, AutomationCommandTaskStep::Pending(_)));
                // Retire A after admission but before its read task is polled.
                // B uses the same caller-supplied collector and request ids.
                handle
                    .with_state(|state| {
                        state
                            .connection
                            .network_data_collectors
                            .remove_collector(&"reusable".into())
                            .unwrap();
                        install_collector(
                            state.connection,
                            CapturedBody::from_bytes(b"new".to_vec()),
                        );
                    })
                    .unwrap();
                assert_eq!(
                    finish(&handle, pending).await.unwrap_err().kind,
                    DevToolsErrorKind::NoSuchNetworkCollector
                );
                let read = handle
                    .with_state(|state| {
                        state
                            .connection
                            .start_automation_command_dispatch(read_command(false))
                    })
                    .unwrap();
                let AutomationResult::NetworkData(result) = finish(&handle, read).await.unwrap()
                else {
                    panic!("new body result")
                };
                assert_eq!(result.value, "new");
            }
        }))
        .await;
}

#[tokio::test]
async fn network_data_admitted_read_cannot_disown_a_replacement_body() {
    tokio::task::LocalSet::new()
        .run_until(Box::pin(async {
            let mut conn = CdpConnection::new();
            install_collector(
                &mut conn,
                CapturedBody::from_bytes_spooled(vec![b'a'; 1_100_000]),
            );
            let owner = ProtocolStateOwner::new(conn);
            let handle = owner.handle();
            let AutomationCommandTaskStep::Pending(pending) = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(read_command(true))
                })
                .unwrap()
            else {
                panic!("owned body read")
            };
            // Admission froze the real disk-backed source. Replace its record
            // before polling the external read, while keeping the collector.
            handle
                .with_state(|state| {
                    record_body(
                        state.connection,
                        CapturedBody::from_bytes(b"replacement".to_vec()),
                    )
                })
                .unwrap();
            let completed = pending.wait().await;
            let step = handle
                .with_state(|state| {
                    state
                        .connection
                        .complete_automation_command_dispatch(completed)
                })
                .unwrap();
            let Err(error) = finish(&handle, step).await else {
                panic!("old body read must not succeed against a replacement record");
            };
            assert_eq!(error.kind, DevToolsErrorKind::NoSuchNetworkData);
            let read = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(read_command(false))
                })
                .unwrap();
            let AutomationResult::NetworkData(result) = finish(&handle, read).await.unwrap() else {
                panic!("replacement body")
            };
            assert_eq!(result.value, "replacement");
        }))
        .await;
}

#[tokio::test]
async fn network_data_ready_result_does_not_repeat_disown_against_a_new_collector() {
    tokio::task::LocalSet::new()
        .run_until(Box::pin(async {
            let mut conn = CdpConnection::new();
            install_collector(
                &mut conn,
                CapturedBody::from_bytes_spooled(vec![b'a'; 1_100_000]),
            );
            let owner = ProtocolStateOwner::new(conn);
            let handle = owner.handle();
            let AutomationCommandTaskStep::Pending(pending) = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(read_command(true))
                })
                .unwrap()
            else {
                panic!("owned body read")
            };
            let completed = pending.wait().await;
            // Domain work has already committed when the ordinary task becomes
            // ready. Its final reply must not execute that disown a second time.
            handle
                .with_state(|state| {
                    state
                        .connection
                        .network_data_collectors
                        .remove_collector(&"reusable".into())
                        .unwrap();
                    install_collector(
                        state.connection,
                        CapturedBody::from_bytes(b"replacement".to_vec()),
                    );
                })
                .unwrap();
            let step = handle
                .with_state(|state| {
                    state
                        .connection
                        .complete_automation_command_dispatch(completed)
                })
                .unwrap();
            let AutomationResult::NetworkData(result) = finish(&handle, step).await.unwrap() else {
                panic!("original ready result")
            };
            assert_eq!(result.value.len(), 1_100_000);
            assert!(result.value.bytes().all(|byte| byte == b'a'));
            let read = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(read_command(false))
                })
                .unwrap();
            let AutomationResult::NetworkData(result) = finish(&handle, read).await.unwrap() else {
                panic!("replacement body")
            };
            assert_eq!(result.value, "replacement");
        }))
        .await;
}
