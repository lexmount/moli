use crate::{document_runtime::DomHandle, dom::native::DomMutationEffects};

#[derive(Clone, Copy)]
pub(super) enum TreeMutationSideEffectSource {
    JsDomApi,
    ParserTreeSink,
}

#[derive(Clone, Copy)]
pub(in crate::document_runtime::mutation_commands) enum TreeReactionDispatchPolicy {
    DispatchNow,
    AppendToCurrentQueue,
}

#[derive(Clone, Copy)]
pub(super) enum TreeNoncePolicy {
    HideInsertedContentAttributes,
    PreserveInsertedContentAttributes,
}

#[derive(Clone, Copy)]
pub(super) enum TreeMutationObserverPolicy {
    Queue,
    Suppress,
    ReplaceChild {
        removed: DomHandle,
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    },
}

#[derive(Clone, Copy)]
pub(super) struct TreeMutationSourceProfile {
    pub(super) source: TreeMutationSideEffectSource,
    pub(super) reaction_policy: TreeReactionDispatchPolicy,
    pub(super) nonce_policy: TreeNoncePolicy,
    pub(super) preserve_connection: bool,
    pub(super) upgrade_connected_subtrees: bool,
    pub(super) queue_parser_details_toggle_events: bool,
    pub(super) queue_resource_followups: bool,
    pub(super) observers: TreeMutationObserverPolicy,
}

impl TreeNoncePolicy {
    pub(super) fn hides_inserted_content_attributes(self) -> bool {
        matches!(self, Self::HideInsertedContentAttributes)
    }
}

impl TreeMutationSourceProfile {
    pub(super) fn subresource_request_initiator_type(
        self,
    ) -> crate::types::SubresourceRequestInitiatorType {
        match self.source {
            TreeMutationSideEffectSource::ParserTreeSink => {
                crate::types::SubresourceRequestInitiatorType::Parser
            }
            TreeMutationSideEffectSource::JsDomApi => {
                crate::types::SubresourceRequestInitiatorType::Script
            }
        }
    }

    pub(super) fn js_dom_api_with(
        reaction_policy: TreeReactionDispatchPolicy,
        nonce_policy: TreeNoncePolicy,
    ) -> Self {
        Self {
            source: TreeMutationSideEffectSource::JsDomApi,
            reaction_policy,
            nonce_policy,
            preserve_connection: false,
            upgrade_connected_subtrees: true,
            queue_parser_details_toggle_events: false,
            queue_resource_followups: true,
            observers: TreeMutationObserverPolicy::Queue,
        }
    }

    pub(super) fn js_dom_api() -> Self {
        Self::js_dom_api_with(
            TreeReactionDispatchPolicy::DispatchNow,
            TreeNoncePolicy::HideInsertedContentAttributes,
        )
    }

    pub(super) fn parser_tree_sink() -> Self {
        Self {
            source: TreeMutationSideEffectSource::ParserTreeSink,
            reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
            nonce_policy: TreeNoncePolicy::HideInsertedContentAttributes,
            preserve_connection: true,
            upgrade_connected_subtrees: false,
            queue_parser_details_toggle_events: true,
            queue_resource_followups: true,
            observers: TreeMutationObserverPolicy::Queue,
        }
    }

    pub(super) fn js_dom_api_appending_to_current_reaction_queue() -> Self {
        Self::js_dom_api_with(
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
            TreeNoncePolicy::HideInsertedContentAttributes,
        )
    }

    pub(super) fn js_dom_move_appending_to_current_reaction_queue() -> Self {
        Self {
            preserve_connection: true,
            ..Self::js_dom_api_with(
                TreeReactionDispatchPolicy::AppendToCurrentQueue,
                TreeNoncePolicy::PreserveInsertedContentAttributes,
            )
        }
    }

    pub(super) fn html_fragment_insertion_appending_to_current_reaction_queue() -> Self {
        Self {
            source: TreeMutationSideEffectSource::JsDomApi,
            reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
            nonce_policy: TreeNoncePolicy::HideInsertedContentAttributes,
            preserve_connection: false,
            upgrade_connected_subtrees: false,
            queue_parser_details_toggle_events: true,
            queue_resource_followups: true,
            observers: TreeMutationObserverPolicy::Queue,
        }
    }

    pub(super) fn suppressing_observers(self) -> Self {
        Self {
            observers: TreeMutationObserverPolicy::Suppress,
            ..self
        }
    }

    pub(super) fn suppresses_observers(self) -> bool {
        !matches!(self.observers, TreeMutationObserverPolicy::Queue)
    }

    pub(super) fn replacing_child(
        self,
        removed: DomHandle,
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) -> Self {
        Self {
            observers: TreeMutationObserverPolicy::ReplaceChild {
                removed,
                previous_sibling,
                next_sibling,
            },
            ..self
        }
    }

    pub(super) fn apply_insertion_observer_policy(
        self,
        effects: &mut DomMutationEffects,
        parent: DomHandle,
        roots: &[DomHandle],
        records_enabled: bool,
    ) {
        if self.suppresses_observers() {
            effects.suppress_child_list_mutations_for_target(parent);
        }
        if effects.did_change()
            && records_enabled
            && let TreeMutationObserverPolicy::ReplaceChild {
                removed,
                previous_sibling,
                next_sibling,
            } = self.observers
        {
            // replaceChild's combined record precedes insertion's script and
            // selectedcontent post-connection steps. Replace-all queues its
            // record only after insertion returns, using Suppress instead.
            effects.queue_child_list_mutation(
                parent,
                roots,
                &[removed],
                previous_sibling,
                next_sibling,
            );
        }
    }
}
