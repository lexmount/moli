use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_parser::HtmlSanitizer;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{util::get_private_value, web_api_interfaces, webidl};

mod config;
mod defaults;
mod input;
mod output;

use config::{Configuration, Name};
use input::{ConstructorInput, ElementInput, ElementName, ProcessingInstruction, SanitizerInput};

const STATE_SLOT: &str = "moli.Sanitizer.configuration";
type Store = Rc<RefCell<Configurations>>;
type State = Rc<RefCell<Configuration>>;

#[derive(Default)]
struct Configurations {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, State)>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Sanitizer)]
struct SanitizerSlots<'s> {
    #[webapi(slot = STATE_SLOT)]
    id: v8::Local<'s, v8::BigInt>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Sanitizer, constructor_callback = constructor, constructor_length = 0, enumerable)]
struct SanitizerTemplate {
    #[webapi(method, callback = get, length = 0)]
    get: (),
    #[webapi(method, callback = allow_element, length = 1)]
    allow_element: (),
    #[webapi(method, callback = remove_element, length = 1)]
    remove_element: (),
    #[webapi(method, callback = replace_element_with_children, length = 1)]
    replace_element_with_children: (),
    #[webapi(method, callback = allow_processing_instruction, length = 1)]
    allow_processing_instruction: (),
    #[webapi(method, callback = remove_processing_instruction, length = 1)]
    remove_processing_instruction: (),
    #[webapi(method, callback = allow_attribute, length = 1)]
    allow_attribute: (),
    #[webapi(method, callback = remove_attribute, length = 1)]
    remove_attribute: (),
    #[webapi(method, callback = set_comments, length = 1)]
    set_comments: (),
    #[webapi(method, callback = set_data_attributes, length = 1)]
    set_data_attributes: (),
    #[webapi(method = "setJavascriptURLs", callback = set_javascript_urls, length = 1)]
    set_javascript_urls: (),
    #[webapi(method, callback = remove_unsafe, length = 0)]
    remove_unsafe: (),
}

pub(super) fn build_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    SanitizerTemplate::build(scope)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Sanitizer")]
struct ConstructorArgs {
    #[webidl(converter = "raw", default = ConstructorInput::Default)]
    configuration: ConstructorInput,
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        webidl::throw_type_error(scope, "Sanitizer requires the new operator");
        return;
    }
    let Some(args_value) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(configuration) = new_configuration(scope, args_value.configuration, true) else {
        return;
    };
    if let Some(owner) = bind_configuration(scope, Some(args.this()), configuration) {
        rv.set(owner.into());
    }
}

fn bind_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: Option<v8::Local<'s, v8::Object>>,
    configuration: Configuration,
) -> Option<v8::Local<'s, v8::Object>> {
    let store = scope.get_slot::<Store>().cloned().unwrap_or_else(|| {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    });
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("Sanitizer identities exhausted");
        store.next_id
    };
    let value = v8::BigInt::new_from_u64(scope, id);
    let slots = SanitizerSlots::new(value);
    let owner = match owner {
        Some(owner) => {
            slots.initialize(scope, owner).ok()?;
            owner
        }
        None => slots.bind(scope).ok()?,
    };
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        owner,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (weak, Rc::new(RefCell::new(configuration))));
    Some(owner)
}

fn state<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) -> Option<State> {
    let value = get_private_value(scope, owner, STATE_SLOT)?;
    let id = v8::Local::<v8::BigInt>::try_from(value).ok()?.u64_value().0;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)
        .map(|entry| entry.1.clone())
}

fn new_configuration(
    scope: &mut v8::PinScope<'_, '_>,
    input: ConstructorInput,
    permissive: bool,
) -> Option<Configuration> {
    match input {
        ConstructorInput::Default => Some(defaults::DEFAULT_CONFIGURATION.clone()),
        ConstructorInput::Dictionary(input) => match input.canonicalize(permissive) {
            Ok(configuration) => Some(configuration),
            Err(message) => {
                webidl::throw_type_error(scope, message);
                None
            }
        },
    }
}

fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = state(scope, args.this()) else {
        return;
    };
    let configuration = state.borrow().clone();
    if let Some(value) = configuration.to_dictionary(scope) {
        rv.set(value.into());
    }
}

fn remove_unsafe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = state(scope, args.this()) {
        rv.set_bool(state.borrow_mut().remove_unsafe());
    }
}

macro_rules! modifier {
    ($callback:ident, $method:literal, $argument:ty, $operation:expr) => {
        fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            #[derive(webidl::WebIdlArgs)]
            #[webidl(prefix = $method)]
            struct Arguments {
                #[webidl(required, converter = "raw")]
                value: $argument,
            }
            let Some(parsed) = webidl::parse_args::<Arguments>(scope, &args) else {
                return;
            };
            if let Some(state) = state(scope, args.this()) {
                rv.set_bool($operation(&mut state.borrow_mut(), parsed.value));
            }
        }
    };
}

modifier!(
    allow_element,
    "Sanitizer.allowElement",
    ElementInput,
    |configuration: &mut Configuration, value: ElementInput| {
        let (name, attributes) = value.into_element();
        configuration.allow_element(name, attributes)
    }
);
modifier!(
    remove_element,
    "Sanitizer.removeElement",
    ElementName,
    |configuration: &mut Configuration, value: ElementName| {
        configuration.remove_element(value.0)
    }
);
modifier!(
    replace_element_with_children,
    "Sanitizer.replaceElementWithChildren",
    ElementName,
    |configuration: &mut Configuration, value: ElementName| {
        configuration.replace_element_with_children(value.0)
    }
);
modifier!(
    allow_attribute,
    "Sanitizer.allowAttribute",
    Name,
    Configuration::allow_attribute
);
modifier!(
    remove_attribute,
    "Sanitizer.removeAttribute",
    Name,
    Configuration::remove_attribute
);
modifier!(
    allow_processing_instruction,
    "Sanitizer.allowProcessingInstruction",
    ProcessingInstruction,
    |configuration: &mut Configuration, value: ProcessingInstruction| {
        configuration.processing_instructions.allow(value.0)
    }
);
modifier!(
    remove_processing_instruction,
    "Sanitizer.removeProcessingInstruction",
    ProcessingInstruction,
    |configuration: &mut Configuration, value: ProcessingInstruction| {
        configuration.processing_instructions.remove(value.0)
    }
);
modifier!(
    set_comments,
    "Sanitizer.setComments",
    webidl::Boolean,
    |configuration: &mut Configuration, value: webidl::Boolean| {
        std::mem::replace(&mut configuration.comments, value.0) != value.0
    }
);
modifier!(
    set_data_attributes,
    "Sanitizer.setDataAttributes",
    webidl::Boolean,
    |configuration: &mut Configuration, value: webidl::Boolean| {
        configuration.set_data_attributes(value.0)
    }
);
modifier!(
    set_javascript_urls,
    "Sanitizer.setJavascriptURLs",
    webidl::Boolean,
    |configuration: &mut Configuration, value: webidl::Boolean| {
        std::mem::replace(&mut configuration.javascript_urls, value.0) != value.0
    }
);

#[derive(Clone)]
pub(super) struct SanitizerArgument<'s>(SanitizerInput<'s>);

impl<'s> webidl::WebIdlConverter<'s> for SanitizerArgument<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<SanitizerInput<'s>>(scope, value, context).map(Self)
    }
}

impl<'s> SanitizerArgument<'s> {
    pub(super) fn is_empty_dictionary(&self) -> bool {
        matches!(&self.0, SanitizerInput::New(ConstructorInput::Dictionary(input)) if input.is_empty())
    }

    fn configuration(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        permissive: bool,
    ) -> Option<Configuration> {
        match self.0 {
            SanitizerInput::Existing(owner) => snapshot_configuration(scope, owner),
            SanitizerInput::New(input) => new_configuration(scope, input, permissive),
        }
    }

    pub(super) fn snapshot(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let configuration = self.configuration(scope, true)?;
        bind_configuration(scope, None, configuration)
    }

    pub(super) fn prepare_for_unsafe_html(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<HtmlSanitizer> {
        Some(self.configuration(scope, true)?.to_parser_config())
    }
}

fn snapshot_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<Configuration> {
    let Some(state) = state(scope, owner) else {
        webidl::throw_type_error(scope, "The Sanitizer has no configuration.");
        return None;
    };
    let configuration = state.borrow().clone();
    Some(configuration)
}

pub(super) fn prepare_sanitizer_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<HtmlSanitizer> {
    Some(snapshot_configuration(scope, owner)?.to_parser_config())
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "SetHTMLOptions")]
pub(crate) struct SetHtmlOptions<'s> {
    #[webidl(converter = "raw")]
    sanitizer: Option<SanitizerArgument<'s>>,
}

impl<'s> SetHtmlOptions<'s> {
    pub(crate) fn prepare(self, scope: &mut v8::PinScope<'s, '_>) -> Option<HtmlSanitizer> {
        let mut configuration = match self.sanitizer {
            Some(input) => input.configuration(scope, false)?,
            None => defaults::DEFAULT_CONFIGURATION.clone(),
        };
        configuration.remove_unsafe();
        Some(configuration.to_parser_config())
    }
}
