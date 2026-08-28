use crate::domains::{
    dom::DomPreparedOutputSlot, dom_storage::DomStoragePreparedOutputSlot,
    input::InputPreparedOutputSlot, network::NetworkPreparedOutputSlot,
    observable_output::ObservablePreparedOutputSlot, page::MainDocumentCommitPreparedOutput,
    page::PagePreparedOutputSlot, runtime::RuntimePreparedOutputSlot,
    target::TargetPreparedOutputSlot,
};

/// Closed ownership boundary for protocol output prepared by one domain.
///
/// This deliberately is an enum rather than `Any` plus a global slot
/// registry. Adding a new payload family now requires an exhaustive compiler-
/// checked decision here, and projection cannot downcast a value to a type that
/// was not named by the producer.
///
/// Payloads are consumed immediately into `ProtocolOutputPayloads`. Boxing the
/// Page variant would add an allocation to that move-only handoff solely to
/// shrink this transient enum.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(in crate::domains) enum ProtocolOutputPayload {
    Dom(DomPreparedOutputSlot),
    DomStorage(DomStoragePreparedOutputSlot),
    Input(InputPreparedOutputSlot),
    MainDocumentCommit(MainDocumentCommitPreparedOutput),
    Network(NetworkPreparedOutputSlot),
    Observable(ObservablePreparedOutputSlot),
    Page(PagePreparedOutputSlot),
    Runtime(RuntimePreparedOutputSlot),
    Target(TargetPreparedOutputSlot),
}

/// Move-only typed payloads prepared from concrete protocol-output records.
///
/// A batch may combine multiple domains, but has at most one aggregate per
/// domain. Domain aggregates merge already-frozen facts; they never query the
/// renderer or synthesize another output family.
#[derive(Debug, Default)]
pub(in crate::domains) struct ProtocolOutputPayloads {
    dom: Option<DomPreparedOutputSlot>,
    dom_storage: Option<DomStoragePreparedOutputSlot>,
    input: Option<InputPreparedOutputSlot>,
    main_document_commit: Option<MainDocumentCommitPreparedOutput>,
    network: Option<NetworkPreparedOutputSlot>,
    observable: Option<ObservablePreparedOutputSlot>,
    page: Option<PagePreparedOutputSlot>,
    runtime: Option<RuntimePreparedOutputSlot>,
    target: Option<TargetPreparedOutputSlot>,
}

impl ProtocolOutputPayloads {
    pub(in crate::domains) fn from_slot<S>(slot: S) -> Self
    where
        S: Into<ProtocolOutputPayload>,
    {
        let mut outputs = Self::default();
        outputs.extend_payload(slot.into());
        outputs
    }

    pub(in crate::domains) fn extend_payload(&mut self, payload: ProtocolOutputPayload) {
        macro_rules! merge {
            ($field:ident, $value:expr) => {{
                let value = $value;
                if let Some(existing) = self.$field.as_mut() {
                    existing.extend(value);
                } else {
                    self.$field = Some(value);
                }
            }};
        }
        match payload {
            ProtocolOutputPayload::Dom(value) => merge!(dom, value),
            ProtocolOutputPayload::DomStorage(value) => merge!(dom_storage, value),
            ProtocolOutputPayload::Input(value) => merge!(input, value),
            ProtocolOutputPayload::MainDocumentCommit(value) => {
                merge!(main_document_commit, value)
            }
            ProtocolOutputPayload::Network(value) => merge!(network, value),
            ProtocolOutputPayload::Observable(value) => merge!(observable, value),
            ProtocolOutputPayload::Page(value) => merge!(page, value),
            ProtocolOutputPayload::Runtime(value) => merge!(runtime, value),
            ProtocolOutputPayload::Target(value) => merge!(target, value),
        }
    }

    pub(in crate::domains) fn dom_mut(&mut self) -> Option<&mut DomPreparedOutputSlot> {
        self.dom.as_mut()
    }

    pub(in crate::domains) fn dom_storage_mut(
        &mut self,
    ) -> Option<&mut DomStoragePreparedOutputSlot> {
        self.dom_storage.as_mut()
    }

    pub(in crate::domains) fn input_mut(&mut self) -> Option<&mut InputPreparedOutputSlot> {
        self.input.as_mut()
    }

    pub(in crate::domains) fn main_document_commit_mut(
        &mut self,
    ) -> Option<&mut MainDocumentCommitPreparedOutput> {
        self.main_document_commit.as_mut()
    }

    pub(in crate::domains) fn network_mut(&mut self) -> Option<&mut NetworkPreparedOutputSlot> {
        self.network.as_mut()
    }

    pub(in crate::domains) fn observable_mut(
        &mut self,
    ) -> Option<&mut ObservablePreparedOutputSlot> {
        self.observable.as_mut()
    }

    pub(in crate::domains) fn page(&self) -> Option<&PagePreparedOutputSlot> {
        self.page.as_ref()
    }

    pub(in crate::domains) fn page_mut(&mut self) -> Option<&mut PagePreparedOutputSlot> {
        self.page.as_mut()
    }

    pub(in crate::domains) fn runtime_mut(&mut self) -> Option<&mut RuntimePreparedOutputSlot> {
        self.runtime.as_mut()
    }

    pub(in crate::domains) fn target_mut(&mut self) -> Option<&mut TargetPreparedOutputSlot> {
        self.target.as_mut()
    }

    #[cfg(test)]
    fn slot_count(&self) -> usize {
        [
            self.dom.is_some(),
            self.dom_storage.is_some(),
            self.input.is_some(),
            self.main_document_commit.is_some(),
            self.network.is_some(),
            self.observable.is_some(),
            self.page.is_some(),
            self.runtime.is_some(),
            self.target.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
    }
}

macro_rules! impl_payload_from {
    ($type:ty, $variant:ident) => {
        impl From<$type> for ProtocolOutputPayload {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }
    };
}

impl_payload_from!(DomPreparedOutputSlot, Dom);
impl_payload_from!(DomStoragePreparedOutputSlot, DomStorage);
impl_payload_from!(InputPreparedOutputSlot, Input);
impl_payload_from!(MainDocumentCommitPreparedOutput, MainDocumentCommit);
impl_payload_from!(NetworkPreparedOutputSlot, Network);
impl_payload_from!(ObservablePreparedOutputSlot, Observable);
impl_payload_from!(PagePreparedOutputSlot, Page);
impl_payload_from!(RuntimePreparedOutputSlot, Runtime);
impl_payload_from!(TargetPreparedOutputSlot, Target);

#[cfg(test)]
mod tests {
    use crate::domains::{network, observable_output};

    use super::ProtocolOutputPayloads;

    #[test]
    fn payloads_merge_by_closed_domain_variant() {
        let mut payloads =
            ProtocolOutputPayloads::from_slot(network::NetworkPreparedOutputSlot::from_outputs(
                network::NetworkPreparedOutputs::default(),
            ));
        payloads.extend_payload(
            observable_output::ObservablePreparedOutputSlot::from_outputs(
                observable_output::ObservablePreparedOutputs::default(),
            )
            .into(),
        );
        assert_eq!(payloads.slot_count(), 2);

        payloads.extend_payload(
            network::NetworkPreparedOutputSlot::from_outputs(
                network::NetworkPreparedOutputs::default(),
            )
            .into(),
        );
        assert_eq!(
            payloads.slot_count(),
            2,
            "a second value must merge into its compiler-known domain owner"
        );
    }
}
