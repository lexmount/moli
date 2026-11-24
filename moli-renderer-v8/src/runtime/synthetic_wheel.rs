use std::sync::Arc;

use parking_lot::Mutex;

use crate::{
    document_runtime::DomHandle,
    native_bridge::{WindowDocumentTaskTarget, element::WheelScrollTarget},
};

/// One mouse-wheel sequence, shared only with its in-flight renderer commands.
/// A new value begins a sequence; dropping it ends the sequence, including when
/// the protocol workflow is canceled. No renderer-global target survives it.
#[derive(Clone, Debug, Default)]
pub struct RendererSyntheticWheelGesture {
    pub(crate) targets: Arc<Mutex<SyntheticWheelTargets>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SyntheticWheelTargets {
    pub(crate) document: Option<WindowDocumentTaskTarget>,
    pub(crate) wheel: Option<(DomHandle, WindowDocumentTaskTarget)>,
    pub(crate) ancestors: Vec<DomHandle>,
    pub(crate) scroll_target: Option<WheelScrollTarget>,
}
