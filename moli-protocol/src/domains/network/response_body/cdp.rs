use super::*;
use crate::conn::{CommandOwnerScope, ProtocolPageHostScope};
use crate::domains::network::{
    NetworkCommandTaskStep, PendingNetworkCommandDispatch, PendingNetworkCommandKind,
    PendingNetworkCommandWork,
};

#[derive(Clone, Copy)]
enum BodyKind {
    Response,
    PostData,
}

pub(in crate::domains::network) struct PreparedCdpBodyRead {
    owner: CommandOwnerScope,
    host: Option<ProtocolPageHostScope>,
    request_id: String,
    session_id: Option<String>,
    primary_session_id: Option<String>,
    body: CapturedBody,
    kind: BodyKind,
    limit: usize,
}

pub(in crate::domains::network) struct CompletedCdpBodyRead {
    prepared: PreparedCdpBodyRead,
    result: Result<Value>,
}

impl BodyKind {
    fn missing(self) -> BodyReadError {
        match self {
            Self::Response => BodyReadError::ResponseNotFound,
            Self::PostData => BodyReadError::RequestNotFound,
        }
    }

    fn capture(
        self,
        slot: &TargetRuntimeSlot,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Result<CapturedBody> {
        let body = match self {
            Self::Response => {
                let body = slot
                    .captured_response_body(request_id)
                    .ok_or(self.missing())?;
                anyhow::ensure!(body.is_visible_to_session(session_id), self.missing());
                body.captured_body()?
            }
            Self::PostData => {
                let body = slot.captured_request_body(request_id).ok_or_else(|| {
                    if slot
                        .captured_response_body(request_id)
                        .is_some_and(|body| body.is_visible_to_session(session_id))
                    {
                        BodyReadError::NoPostData
                    } else {
                        self.missing()
                    }
                })?;
                anyhow::ensure!(body.is_visible_to_session(session_id), self.missing());
                body.captured_body()
            }
        };
        Ok(body.clone())
    }
}

impl PreparedCdpBodyRead {
    pub(in crate::domains::network) async fn run(self) -> CompletedCdpBodyRead {
        let kind = self.kind;
        let limit = self.limit;
        let result = self
            .body
            .clone()
            .process_in_worker(move |body| {
                let bytes = body.materialize_bytes_limited(limit)?;
                if matches!(kind, BodyKind::PostData) {
                    anyhow::ensure!(!bytes.is_empty(), BodyReadError::NoPostData);
                }
                let (text, base64_encoded) = encode_cdp_body(bytes);
                Ok(match kind {
                    BodyKind::Response => json!({"body": text, "base64Encoded": base64_encoded}),
                    BodyKind::PostData => {
                        json!({"postData": text, "base64Encoded": base64_encoded})
                    }
                })
            })
            .await;
        CompletedCdpBodyRead {
            prepared: self,
            result,
        }
    }
}

impl CompletedCdpBodyRead {
    pub(in crate::domains::network) fn session_id(&self) -> Option<&str> {
        self.prepared.session_id.as_deref()
    }

    pub(in crate::domains::network) fn finish(self, conn: &CdpConnection) -> CommandOutputPlan {
        let original = self.prepared;
        let current = (|| -> Result<()> {
            anyhow::ensure!(
                original
                    .host
                    .as_ref()
                    .is_none_or(|host| host.is_current(conn)),
                original.kind.missing()
            );
            let slot = conn
                .runtime_session_owner_slot_for_owner(&original.owner)
                .ok_or(original.kind.missing())?;
            anyhow::ensure!(
                network_events_enabled_for_session(
                    slot,
                    original.session_id.as_deref(),
                    original.primary_session_id.as_deref()
                ),
                original.kind.missing()
            );
            let body = original.kind.capture(
                slot,
                &original.request_id,
                original.session_id.as_deref(),
            )?;
            anyhow::ensure!(body.same_storage(&original.body), original.kind.missing());
            Ok(())
        })();
        match current.and(self.result) {
            Ok(value) => CommandOutputPlan::result(value),
            Err(error) => CommandOutputPlan::error(-32000, error),
        }
    }
}

fn start_body_read(conn: &CdpConnection, cmd: &Cmd<'_>, kind: BodyKind) -> NetworkCommandTaskStep {
    let prepared = (|| -> Result<PreparedCdpBodyRead, CommandOutputPlan> {
        let request_id = cmd
            .params
            .and_then(|params| params.get("requestId"))
            .and_then(Value::as_str)
            .ok_or_else(|| CommandOutputPlan::error(-32602, BodyReadError::InvalidParams))?;
        if conn.browser_context.is_none() {
            return Err(CommandOutputPlan::error(
                -31998,
                BodyReadError::BrowserContextNotLoaded,
            ));
        }
        let primary_session_id = conn.runtime_session_owner_primary_session_id(cmd.session_id);
        let slot = conn
            .runtime_session_owner_slot(cmd.session_id)
            .ok_or_else(|| CommandOutputPlan::error(-32000, kind.missing()))?;
        if !network_events_enabled_for_session(slot, cmd.session_id, primary_session_id.as_deref())
        {
            return Err(CommandOutputPlan::error(-32000, kind.missing()));
        }
        let body = kind
            .capture(slot, request_id, cmd.session_id)
            .map_err(|error| CommandOutputPlan::error(-32000, error))?;
        let owner = CommandOwnerScope::capture(conn, cmd.session_id);
        let host = ProtocolPageHostScope::capture(conn, &owner).ok();
        let owner = host
            .as_ref()
            .map(|host| host.owner().clone())
            .unwrap_or(owner);
        Ok(PreparedCdpBodyRead {
            owner,
            host,
            request_id: request_id.to_owned(),
            session_id: cmd.session_id.map(str::to_owned),
            primary_session_id,
            body,
            kind,
            limit: conn.response_body_materialize_limit(),
        })
    })();
    match prepared {
        Ok(prepared) => NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
            command_id: cmd.id,
            owner_scope: prepared.owner.clone(),
            kind: PendingNetworkCommandKind::ReadBody,
            pending: PendingNetworkCommandWork::Body(Box::new(prepared)),
        }),
        Err(plan) => NetworkCommandTaskStep::Complete(plan),
    }
}

pub(in crate::domains::network) fn start_response_body(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    start_body_read(conn, cmd, BodyKind::Response)
}

pub(in crate::domains::network) fn start_request_post_data(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    start_body_read(conn, cmd, BodyKind::PostData)
}

#[derive(Debug, thiserror::Error)]
enum BodyReadError {
    #[error("InvalidParams")]
    InvalidParams,
    #[error("BrowserContextNotLoaded")]
    BrowserContextNotLoaded,
    #[error("No resource with given identifier found")]
    ResponseNotFound,
    #[error("No resource with given id was found")]
    RequestNotFound,
    #[error("No post data available for the request")]
    NoPostData,
}
