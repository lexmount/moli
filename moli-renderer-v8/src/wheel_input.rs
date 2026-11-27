//! Renderer identities and targets for the common wheel state machine.

use crate::{
    document_runtime::DomHandle,
    native_bridge::{WindowDocumentTaskTarget, element::WheelScrollTarget},
};

pub(crate) use moli_input::{WheelInput, WheelPhase, WheelSource};

#[derive(Clone, Debug)]
pub(crate) struct WheelEventTarget {
    pub node: DomHandle,
    pub owner: WindowDocumentTaskTarget,
    pub ancestors: Vec<DomHandle>,
}

pub(crate) type WheelInputController =
    moli_input::WheelInputController<WindowDocumentTaskTarget, WheelEventTarget, WheelScrollTarget>;
pub(crate) type PreparedWheel = moli_input::PreparedWheel<WheelEventTarget, WheelScrollTarget>;
