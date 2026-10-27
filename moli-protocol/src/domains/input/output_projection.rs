use anyhow::Result;

use super::*;
use crate::conn::CommandWorkflowContext;
use crate::domains::activity::ProtocolOutputSlot;

pub(crate) enum PreparedInputOutputProjection {
    Downloads(Vec<Result<crate::conn::PreparedDownloadActivationDelivery>>),
    FileChoosers {
        owner: CommandOwnerScope,
        activations: Vec<file_chooser::PreparedFileChooserActivation>,
    },
}

impl PreparedInputOutputProjection {
    pub(in crate::domains) fn take(
        slot: ProtocolOutputSlot,
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        payloads: &mut ProtocolOutputPayloads,
    ) -> Option<Self> {
        let input = payloads.input_mut()?;
        match slot {
            ProtocolOutputSlot::Download => {
                let activations = input.take_download_activations()?;
                Some(Self::prepare_downloads(conn, owner, activations))
            }
            ProtocolOutputSlot::FileChooser => Some(Self::FileChoosers {
                owner: owner.clone(),
                activations: input.take_file_chooser_activations()?,
            }),
            _ => unreachable!("only Input projection slots prepare these activations"),
        }
    }

    pub(super) fn prepare_downloads(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        activations: Vec<RendererPendingDownloadActivation>,
    ) -> Self {
        Self::Downloads(
            activations
                .into_iter()
                .map(|activation| {
                    conn.prepare_pending_download_activation_delivery(owner, activation, true)
                })
                .collect(),
        )
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) {
        let mut events = Vec::new();
        match self {
            Self::Downloads(deliveries) => {
                for delivery in deliveries {
                    match delivery {
                        Ok(delivery) => delivery.run(&mut events, context.context_mut()).await,
                        Err(error) => {
                            tracing::warn!(%error, "pending download activation failed during protocol output projection");
                        }
                    }
                }
            }
            Self::FileChoosers { owner, activations } => {
                for activation in activations {
                    file_chooser::emit_prepared_activation(driver, &mut events, &owner, activation)
                        .await;
                }
            }
        }
        let mut output = CommandOutputPlan::default();
        output.extend_background_events(events);
        context.retain_output(output);
    }
}
