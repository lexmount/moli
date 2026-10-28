use anyhow::Result;

use super::*;
use crate::conn::{CapturedBody, CommandWorkflowContext, ProtocolWorkflowDriver};
use crate::domains::network::collectors::NetworkDataCollectorIdentity;

/// An immutable body plus the existing collector lifetimes which authorized
/// this read. No connection reference or ambient target crosses the wait.
pub(crate) struct PreparedNetworkDataAccess {
    request_id: DevToolsRequestId,
    data_type: DevToolsNetworkDataType,
    session_id: Option<crate::automation::DevToolsSessionId>,
    body: CapturedBody,
    ownership: NetworkDataOwnership,
    disown: bool,
    output: NetworkDataOutput,
    limit: usize,
}

enum NetworkDataOwnership {
    Collector(NetworkDataCollectorIdentity),
    Any(Vec<NetworkDataCollectorIdentity>),
}

#[derive(Clone, Copy)]
enum NetworkDataOutput {
    Bytes,
    Empty,
}

impl PreparedNetworkDataAccess {
    pub(crate) fn prepare(
        conn: &CdpConnection,
        command: crate::automation::AutomationCommand,
    ) -> Result<Self, DevToolsError> {
        let (context, request_id, data_type, collector, disown, output) = match command {
            crate::automation::AutomationCommand::GetNetworkData(command) => {
                if command.disown && command.collector.is_none() {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::InvalidArgument,
                        NetworkDataAccessError::DisownRequiresCollector,
                    ));
                }
                (
                    command.context,
                    command.request_id,
                    command.data_type,
                    command.collector,
                    command.disown,
                    NetworkDataOutput::Bytes,
                )
            }
            crate::automation::AutomationCommand::DisownNetworkData(command) => (
                command.context,
                command.request_id,
                command.data_type,
                Some(command.collector_id),
                true,
                NetworkDataOutput::Empty,
            ),
            _ => unreachable!("network data preparation selects get or disown"),
        };
        // The collector is checked before selecting its body, preserving the
        // original admission error order. Identity comes from that map value,
        // because callers can remove and recreate an identical string id.
        let collector = collector
            .map(|id| conn.network_data_collectors.capture_collector(&id))
            .transpose()?;
        let source = network_data_body(
            conn,
            &request_id,
            data_type,
            context.session_id.as_ref().map(|id| id.as_str()),
            collector.as_ref().map(NetworkDataCollectorIdentity::id),
        )?;
        let body = source
            .captured_body()
            .map_err(|_| no_such_network_data())?
            .clone();
        let ownership = match collector {
            Some(collector) => NetworkDataOwnership::Collector(collector),
            None => NetworkDataOwnership::Any(
                conn.network_data_collectors
                    .capture_body_collectors(source.collector_ids()),
            ),
        };
        Ok(Self {
            request_id,
            data_type,
            session_id: context.session_id,
            body,
            ownership,
            disown,
            output,
            limit: conn.response_body_materialize_limit(),
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        let limit = self.limit;
        let output = self.output;
        let result = self
            .body
            .clone()
            .process_in_worker(move |body| {
                let bytes = body.materialize_bytes_limited(limit)?;
                Ok(match output {
                    NetworkDataOutput::Bytes => {
                        AutomationResult::NetworkData(network_data_result(bytes))
                    }
                    NetworkDataOutput::Empty => AutomationResult::Empty,
                })
            })
            .await
            .map_err(|_| no_such_network_data())?;
        context
            .with_state(driver, move |state, _| {
                state.finish_network_data_access(self, result)
            })
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?
    }

    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        result: AutomationResult,
    ) -> Result<AutomationResult, DevToolsError> {
        let collector = match &self.ownership {
            NetworkDataOwnership::Collector(collector) => {
                conn.network_data_collectors
                    .ensure_collector_is_current(collector)?;
                Some(collector.id())
            }
            NetworkDataOwnership::Any(_) => None,
        };
        let body = network_data_body(
            conn,
            &self.request_id,
            self.data_type,
            self.session_id.as_ref().map(|id| id.as_str()),
            collector,
        )?;
        if !body
            .captured_body()
            .is_ok_and(|body| body.same_storage(&self.body))
        {
            return Err(no_such_network_data());
        }
        let owned = match &self.ownership {
            NetworkDataOwnership::Collector(collector) => {
                conn.network_data_collectors.body_is_collected(
                    self.request_id.as_str(),
                    self.data_type,
                    collector.id(),
                    body.was_collected_by(collector.id().as_str()),
                )?
            }
            NetworkDataOwnership::Any(collectors) => collectors.iter().any(|collector| {
                conn.network_data_collectors.collector_is_current(collector)
                    && conn
                        .network_data_collectors
                        .body_is_collected(
                            self.request_id.as_str(),
                            self.data_type,
                            collector.id(),
                            body.was_collected_by(collector.id().as_str()),
                        )
                        .unwrap_or(false)
            }),
        };
        if !owned {
            return Err(no_such_network_data());
        }
        if self.disown {
            let NetworkDataOwnership::Collector(collector) = self.ownership else {
                unreachable!("disown admission requires a collector");
            };
            conn.network_data_collectors.disown_data(
                self.request_id.as_str(),
                self.data_type,
                collector.id(),
            )?;
        }
        Ok(result)
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum NetworkDataAccessError {
    #[error("network.getData disown requires collector")]
    DisownRequiresCollector,
    #[error("no such network data")]
    NoSuchData,
}
