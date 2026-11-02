use super::super::document_runtime::DomHandle;
use super::construction::CustomElementConstructionStack;
pub(crate) use super::definition_error::CustomElementDefineError;
use crate::dom::custom_elements::is_valid_custom_element_name as is_valid_dom_custom_element_name;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub(super) struct CustomElementDefinition {
    pub(super) constructor: v8::Global<v8::Function>,
    pub(super) observed_attributes: Rc<[String]>,
    pub(super) callbacks: CustomElementCallbacks,
    pub(super) disables_shadow: bool,
    pub(super) disables_internals: bool,
    pub(super) form_associated: bool,
    pub(super) extends_local_name: Option<Rc<str>>,
}

#[derive(Default)]
pub(super) struct CustomElementCallbacks {
    pub(super) connected: Option<v8::Global<v8::Function>>,
    pub(super) disconnected: Option<v8::Global<v8::Function>>,
    pub(super) connected_move: Option<v8::Global<v8::Function>>,
    pub(super) adopted: Option<v8::Global<v8::Function>>,
    pub(super) attribute_changed: Option<v8::Global<v8::Function>>,
    pub(super) form_associated: Option<v8::Global<v8::Function>>,
    pub(super) form_reset: Option<v8::Global<v8::Function>>,
    pub(super) form_disabled: Option<v8::Global<v8::Function>>,
    pub(super) form_state_restore: Option<v8::Global<v8::Function>>,
}

pub(super) struct PendingWhenDefined {
    pub(super) promise: v8::Global<v8::Promise>,
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

#[derive(Clone)]
pub(super) struct PendingInitialAttribute {
    pub(super) name: String,
    pub(super) namespace: Option<String>,
    pub(super) value: String,
}

#[derive(Default)]
pub(crate) struct CustomElementRegistryData {
    pub(super) definitions: RefCell<HashMap<String, Rc<CustomElementDefinition>>>,
    pub(super) pending_when_defined: RefCell<HashMap<String, PendingWhenDefined>>,
    pub(super) definition_is_running: Cell<bool>,
}

// A registry's definitions and promises can be shared by different native DOM
// owners. Native handles and construction stacks always belong to one owner.
pub(crate) struct CustomElementStore {
    pub(super) registry: Rc<CustomElementRegistryData>,
    pub(super) upgraded_handles: HashSet<DomHandle>,
    pub(super) upgraded_definition_names: HashMap<DomHandle, String>,
    pub(super) form_association_states: HashMap<DomHandle, Option<DomHandle>>,
    pub(super) form_disabled_states: HashMap<DomHandle, bool>,
    pub(super) construction_stack: CustomElementConstructionStack,
}

impl Default for CustomElementStore {
    fn default() -> Self {
        Self::for_registry(Rc::default())
    }
}

impl CustomElementStore {
    pub(crate) fn for_registry(registry: Rc<CustomElementRegistryData>) -> Self {
        Self {
            registry,
            upgraded_handles: HashSet::new(),
            upgraded_definition_names: HashMap::new(),
            form_association_states: HashMap::new(),
            form_disabled_states: HashMap::new(),
            construction_stack: CustomElementConstructionStack::default(),
        }
    }

    pub(crate) fn registry_data(&self) -> Rc<CustomElementRegistryData> {
        self.registry.clone()
    }
}

pub(crate) fn is_valid_custom_element_name(name: &str) -> bool {
    is_valid_dom_custom_element_name(name)
}
