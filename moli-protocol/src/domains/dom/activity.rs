use moli_core::page::{
    RendererDevToolsAgentToken, RendererDomMutationEvent, RendererDomMutationEventBatch,
};

use crate::conn::{BackgroundProtocolEvent, CdpConnection, CommandOwnerScope};
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink, ProtocolOutputSlot,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::domains) struct DomPreparedOutputs {
    batches: Vec<DomMutationBatch>,
}

#[derive(Clone, Debug, PartialEq)]
struct DomMutationBatch {
    attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
    events: Vec<RendererDomMutationEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DomPreparedOutputSlot {
    outputs: DomPreparedOutputs,
}

pub(in crate::domains) const SLOT_DOM_MUTATIONS: ProtocolOutputSlot =
    ProtocolOutputSlot::DomMutations;

pub(in crate::domains) async fn project_dom_mutations_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    let Some(batches) = prepared_outputs
        .and_then(ProtocolOutputPayloads::dom_mut)
        .and_then(DomPreparedOutputSlot::take_batches)
    else {
        return;
    };
    append_dom_mutation_batches_to_background_events(
        conn,
        batches,
        context.command.protocol_events_mut(),
    );
}

fn append_dom_mutation_batches_to_background_events(
    conn: &CdpConnection,
    batches: Vec<DomMutationBatch>,
    out: &mut Vec<BackgroundProtocolEvent>,
) {
    for batch in batches {
        if !conn.target_page_protocol_attachment_identity_is_current(&batch.attachment)
            || !super::dom_agent_enabled_for_session(conn, batch.attachment.session_id())
        {
            continue;
        }
        let session_id = batch.attachment.session_id();
        out.extend(
            batch
                .events
                .into_iter()
                .filter_map(|event| renderer_dom_mutation_background_event(event, session_id)),
        );
    }
}

impl DomPreparedOutputs {
    pub(in crate::domains) fn from_renderer_dom_mutation_event_batches_for_stream(
        conn: &CdpConnection,
        source_owner: &CommandOwnerScope,
        source_renderer_agent: RendererDevToolsAgentToken,
        batches: &[RendererDomMutationEventBatch],
    ) -> Self {
        Self::from_renderer_dom_mutation_event_batches_with_source_agent(
            conn,
            source_owner,
            source_renderer_agent,
            batches,
        )
    }

    fn from_renderer_dom_mutation_event_batches_with_source_agent(
        conn: &CdpConnection,
        source_owner: &CommandOwnerScope,
        source_renderer_agent: RendererDevToolsAgentToken,
        batches: &[RendererDomMutationEventBatch],
    ) -> Self {
        let current_attachment = conn
            .runtime_session_owner_slot_for_route(
                source_owner.session_id(),
                source_owner.session_owner_route(),
            )
            .ok()
            .and_then(|slot| slot.current_renderer_attachment());
        Self {
            batches: batches
                .iter()
                .cloned()
                .filter_map(|mut batch| {
                    if batch.renderer_agent_attachment_id().is_none()
                        && current_attachment.is_some_and(|attachment| {
                            attachment.agent_token() == source_renderer_agent
                        })
                    {
                        batch.bind_renderer_agent_attachment(
                            current_attachment
                                .expect("matched renderer attachment must remain present")
                                .id(),
                        );
                    }
                    if batch.events.is_empty()
                        || batch.renderer_agent_attachment_id()
                            != current_attachment.map(|attachment| attachment.id())
                    {
                        return None;
                    }
                    let attachment = conn
                        .target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                            source_owner,
                            batch.session.wire_session_id(),
                        )?;
                    Some(DomMutationBatch {
                        attachment,
                        events: batch.events,
                    })
                })
                .collect(),
        }
    }

    pub(in crate::domains) fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    pub(in crate::domains) fn append_to_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.is_empty() {
            return;
        }
        sink.push_produced_slot(SLOT_DOM_MUTATIONS);
        sink.push_prepared_payload(DomPreparedOutputSlot { outputs: self }.into());
    }

    fn extend(&mut self, other: Self) {
        self.batches.extend(other.batches);
    }
}

impl DomPreparedOutputSlot {
    pub(in crate::domains) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    fn take_batches(&mut self) -> Option<Vec<DomMutationBatch>> {
        (!self.outputs.batches.is_empty()).then(|| std::mem::take(&mut self.outputs.batches))
    }
}

fn renderer_dom_mutation_background_event(
    event: RendererDomMutationEvent,
    session_id: Option<&str>,
) -> Option<BackgroundProtocolEvent> {
    Some(match event {
        RendererDomMutationEvent::AttributeModified {
            node_id,
            name,
            value,
        } => BackgroundProtocolEvent::dom_attribute_modified(session_id, node_id, &name, &value),
        RendererDomMutationEvent::AttributeRemoved { node_id, name } => {
            BackgroundProtocolEvent::dom_attribute_removed(session_id, node_id, &name)
        }
        RendererDomMutationEvent::CharacterDataModified {
            node_id,
            character_data,
        } => BackgroundProtocolEvent::dom_character_data_modified(
            session_id,
            node_id,
            &character_data,
        ),
        RendererDomMutationEvent::ChildNodeCountUpdated {
            node_id,
            child_node_count,
        } => BackgroundProtocolEvent::dom_child_node_count_updated(
            session_id,
            node_id,
            child_node_count,
        ),
        RendererDomMutationEvent::SetChildNodes {
            parent_node_id,
            nodes,
        } => BackgroundProtocolEvent::dom_set_child_nodes(
            session_id,
            parent_node_id,
            nodes
                .iter()
                .map(|node| super::node_payload::node_snapshot_to_cdp(node, None, None))
                .collect::<Option<Vec<_>>>()?,
        ),
        RendererDomMutationEvent::ChildNodeInserted {
            parent_node_id,
            previous_node_id,
            node,
        } => BackgroundProtocolEvent::dom_child_node_inserted(
            session_id,
            parent_node_id,
            previous_node_id,
            super::node_payload::node_snapshot_to_cdp(&node, None, None)?,
        ),
        RendererDomMutationEvent::ChildNodeRemoved {
            parent_node_id,
            node_id,
        } => BackgroundProtocolEvent::dom_child_node_removed(session_id, parent_node_id, node_id),
    })
}
