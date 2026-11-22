use moli_parser::HtmlSanitizer;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{
    content_security_policy::TrustedTypesForScriptRequirements,
    context_bootstrap::{context_host_ptr_from_global_bridge, sanitizer},
    util::{get_private_value, v8_string, v8str},
    web_api_interfaces, webidl,
};

use super::{
    TRUSTED_TYPES_DEFAULT_POLICY_SLOT, TrustedTypeKind,
    policy_callbacks::{
        CREATE_PARSER_OPTIONS_SLOT, TrustedTypePolicyCallbackOutcome, invoke_callback,
        policy_callback,
    },
    trusted_type_kind, trusted_type_string, trusted_type_string_or_throw,
};

const SANITIZER_SLOT: &str = "moli.TrustedHTMLParserOptions.sanitizer";
const RUN_SCRIPTS_SLOT: &str = "moli.TrustedHTMLParserOptions.runScripts";

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::TrustedHTMLParserOptions,
    constructor = "illegal",
    constructor_length = 0,
    readonly_prototype,
    prototype_to_string_tag = "TrustedHTMLParserOptions"
)]
struct TrustedHtmlParserOptionsTemplate {}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TrustedHTMLParserOptions, require_prototype)]
struct TrustedHtmlParserOptionsSlots<'s> {
    #[webapi(slot = RUN_SCRIPTS_SLOT)]
    run_scripts: v8::Local<'s, v8::Boolean>,
    #[webapi(slot = SANITIZER_SLOT)]
    sanitizer: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap) fn build_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    TrustedHtmlParserOptionsTemplate::build(scope)
}

#[derive(Default, WebApiObject)]
#[webapi(fragment = web_api_interfaces::TrustedTypePolicy)]
struct ParserOptionsPolicyMethods {
    #[webapi(method, callback = create_parser_options, length = 0, enumerable)]
    create_parser_options: (),
}

pub(in crate::context_bootstrap) fn finalize_policy_window_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    ParserOptionsPolicyMethods::default()
        .initialize(scope, prototype)
        .map_err(|error| {
            anyhow::anyhow!("failed to install TrustedTypePolicy window methods: {error}")
        })
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "SetHTMLUnsafeOptions")]
struct SetHtmlUnsafeOptions<'s> {
    #[webidl(default = false)]
    run_scripts: bool,
    #[webidl(converter = "raw")]
    sanitizer: Option<sanitizer::SanitizerArgument<'s>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ParseHTMLUnsafeOptions")]
struct ParseHtmlUnsafeOptions<'s> {
    #[webidl(converter = "raw")]
    sanitizer: Option<sanitizer::SanitizerArgument<'s>>,
}

impl<'s> SetHtmlUnsafeOptions<'s> {
    fn needs_vetting(&self) -> bool {
        self.run_scripts
            || self
                .sanitizer
                .as_ref()
                .is_some_and(|sanitizer| !sanitizer.is_empty_dictionary())
    }

    fn into_prepared(self, scope: &mut v8::PinScope<'s, '_>) -> Option<PreparedParserOptions> {
        let sanitizer = match self.sanitizer {
            Some(sanitizer) => Some(sanitizer.prepare_for_unsafe_html(scope)?),
            None => None,
        };
        Some(PreparedParserOptions {
            sanitizer,
            run_scripts: self.run_scripts,
        })
    }

    fn callback_argument(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        policy: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let context = policy.get_creation_context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let object = v8::Object::new(scope);
        let run_scripts = v8::Boolean::new(scope, self.run_scripts);
        let key = v8str(scope, "runScripts");
        object
            .create_data_property(scope, key.into(), run_scripts.into())?
            .then_some(())?;
        if let Some(sanitizer) = self.sanitizer.clone() {
            let sanitizer = sanitizer.snapshot(scope)?;
            let key = v8str(scope, "sanitizer");
            object
                .create_data_property(scope, key.into(), sanitizer.into())?
                .then_some(())?;
        }
        Some(object)
    }

    fn bind_trusted(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
        let sanitizer = match self.sanitizer {
            Some(sanitizer) => sanitizer.snapshot(scope)?.into(),
            None => v8::null(scope).into(),
        };
        TrustedHtmlParserOptionsSlots::new(v8::Boolean::new(scope, self.run_scripts), sanitizer)
            .bind(scope)
            .ok()
    }
}

struct PreparedParserOptions {
    sanitizer: Option<HtmlSanitizer>,
    run_scripts: bool,
}

enum ParserOptionsArgument<'s> {
    Dictionary(SetHtmlUnsafeOptions<'s>),
    Trusted(v8::Local<'s, v8::Object>),
}

impl<'s> ParserOptionsArgument<'s> {
    fn trusted(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Option<Self> {
        let owner = v8::Local::<v8::Object>::try_from(value).ok()?;
        web_api_interfaces::TrustedHTMLParserOptions::is_instance(scope, owner)
            .then_some(Self::Trusted(owner))
    }
}

fn trusted_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<PreparedParserOptions> {
    let value = get_private_value(scope, owner, SANITIZER_SLOT)?;
    let sanitizer = match v8::Local::<v8::Object>::try_from(value) {
        Ok(object) => Some(sanitizer::prepare_sanitizer_snapshot(scope, object)?),
        Err(_) => None,
    };
    let run_scripts = get_private_value(scope, owner, RUN_SCRIPTS_SLOT)?.is_true();
    Some(PreparedParserOptions {
        sanitizer,
        run_scripts,
    })
}

struct TrustedHtmlInput<'s>(v8::Local<'s, v8::Value>);

impl<'s> webidl::WebIdlConverter<'s> for TrustedHtmlInput<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if trusted_type_kind(scope, value) == Some(TrustedTypeKind::Html) {
            return Ok(Self(value));
        }
        let string = webidl::convert::<webidl::DomString16>(scope, value, context)?;
        crate::util::v8_string_from_utf16_units(scope, &string.0)
            .map(|value| Self(value.into()))
            .ok_or(webidl::WebIdlError::pending_exception(context))
    }
}

fn fragment_options_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ParserOptionsArgument<'s>, webidl::WebIdlError> {
    let value = args.get(index);
    if let Some(trusted) = ParserOptionsArgument::trusted(scope, value) {
        return Ok(trusted);
    }
    let context = webidl::Context::argument("HTML insertion", index as usize + 1);
    webidl::parse_dictionary::<SetHtmlUnsafeOptions<'s>>(scope, value, context)
        .map(|options| ParserOptionsArgument::Dictionary(options.unwrap_or_default()))
}

fn document_options_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ParserOptionsArgument<'s>, webidl::WebIdlError> {
    let value = args.get(index);
    if let Some(trusted) = ParserOptionsArgument::trusted(scope, value) {
        return Ok(trusted);
    }
    let context = webidl::Context::argument("Document.parseHTMLUnsafe", index as usize + 1);
    let options = webidl::parse_dictionary::<ParseHtmlUnsafeOptions<'s>>(scope, value, context)?;
    Ok(ParserOptionsArgument::Dictionary(SetHtmlUnsafeOptions {
        run_scripts: false,
        sanitizer: options.and_then(|options| options.sanitizer),
    }))
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTML insertion")]
pub(crate) struct UnsafeHtmlFragmentArgs<'s> {
    #[webidl(required, converter = "raw")]
    html: TrustedHtmlInput<'s>,
    #[webidl(with = fragment_options_argument)]
    options: ParserOptionsArgument<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.parseHTMLUnsafe")]
pub(in crate::context_bootstrap) struct UnsafeHtmlDocumentArgs<'s> {
    #[webidl(required, converter = "raw")]
    html: TrustedHtmlInput<'s>,
    #[webidl(with = document_options_argument)]
    options: ParserOptionsArgument<'s>,
}

enum CompliantParserOptions<'s> {
    Author(SetHtmlUnsafeOptions<'s>),
    Trusted(PreparedParserOptions),
}

pub(crate) struct PreparedUnsafeHtml<'s> {
    html: String,
    options: CompliantParserOptions<'s>,
}

impl<'s> PreparedUnsafeHtml<'s> {
    pub(crate) fn into_parser_input(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<(String, Option<HtmlSanitizer>, bool)> {
        let options = match self.options {
            CompliantParserOptions::Author(options) => options.into_prepared(scope)?,
            CompliantParserOptions::Trusted(options) => options,
        };
        Some((self.html, options.sanitizer, options.run_scripts))
    }
}

impl<'s> UnsafeHtmlFragmentArgs<'s> {
    pub(crate) fn prepare(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        requirements: TrustedTypesForScriptRequirements,
        sink: &'static str,
        api_name: &'static str,
    ) -> Option<PreparedUnsafeHtml<'s>> {
        prepare_html(
            scope,
            self.html,
            self.options,
            global,
            requirements,
            sink,
            api_name,
        )
    }
}

impl<'s> UnsafeHtmlDocumentArgs<'s> {
    pub(in crate::context_bootstrap) fn prepare(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        requirements: TrustedTypesForScriptRequirements,
    ) -> Option<PreparedUnsafeHtml<'s>> {
        prepare_html(
            scope,
            self.html,
            self.options,
            global,
            requirements,
            "Document parseHTMLUnsafe",
            "parseHTMLUnsafe",
        )
    }
}

fn prepare_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    html: TrustedHtmlInput<'s>,
    options: ParserOptionsArgument<'s>,
    global: v8::Local<'s, v8::Object>,
    requirements: TrustedTypesForScriptRequirements,
    sink: &'static str,
    api_name: &'static str,
) -> Option<PreparedUnsafeHtml<'s>> {
    let options = match options {
        ParserOptionsArgument::Trusted(owner) => {
            let options = trusted_options(scope, owner)?;
            return prepared_with_trusted_options(scope, html, options);
        }
        ParserOptionsArgument::Dictionary(options) => options,
    };
    if requirements.requires_conversion() {
        let policy = get_private_value(scope, global, TRUSTED_TYPES_DEFAULT_POLICY_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        let callback = policy.and_then(|policy| {
            policy_callback(scope, policy, CREATE_PARSER_OPTIONS_SLOT)
                .map(|callback| (policy, callback))
        });
        let report_violation = match callback {
            Some((policy, callback)) => {
                let argument = options.callback_argument(scope, policy)?;
                let kind = v8str(scope, "TrustedHTMLParserOptions");
                let name = v8_string(scope, sink)?;
                match invoke_callback(
                    scope,
                    callback,
                    &[argument.into(), kind.into(), name.into()],
                    convert_policy_result,
                ) {
                    TrustedTypePolicyCallbackOutcome::Returned(Some(result)) => {
                        let options = result.into_prepared(scope)?;
                        return prepared_with_trusted_options(scope, html, options);
                    }
                    TrustedTypePolicyCallbackOutcome::Returned(None) => true,
                    TrustedTypePolicyCallbackOutcome::Abrupt => return None,
                    TrustedTypePolicyCallbackOutcome::Missing => options.needs_vetting(),
                }
            }
            None => options.needs_vetting(),
        };
        if report_violation {
            if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
                unsafe { &mut *host_ptr }
                    .dispatch_trusted_types_sink_csp_violation_event_for_global_best_effort(
                        scope, host_ptr, global, sink, "",
                    );
            }
            if requirements.is_enforced() {
                webidl::throw_type_error(scope, "This HTML sink requires trusted parser options.");
                return None;
            }
        }
    }

    let html = trusted_type_string_or_throw(
        scope,
        html.0,
        TrustedTypeKind::Html,
        requirements,
        sink,
        api_name,
        Some(global),
    )?;
    Some(PreparedUnsafeHtml {
        html,
        options: CompliantParserOptions::Author(options),
    })
}

fn prepared_with_trusted_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    html: TrustedHtmlInput<'s>,
    options: PreparedParserOptions,
) -> Option<PreparedUnsafeHtml<'s>> {
    let html = trusted_type_string(scope, html.0, TrustedTypeKind::Html).or_else(|| {
        html.0
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
    })?;
    Some(PreparedUnsafeHtml {
        html,
        options: CompliantParserOptions::Trusted(options),
    })
}

fn convert_policy_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<SetHtmlUnsafeOptions<'s>, ()> {
    let context = webidl::Context::member("TrustedTypePolicy.createParserOptions", "return value");
    webidl::parse_dictionary(scope, value, context)
        .map(Option::unwrap_or_default)
        .map_err(|error| webidl::throw_error(scope, &error))
}

fn create_parser_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = webidl::Context::argument("TrustedTypePolicy.createParserOptions", 1);
    let options =
        match webidl::parse_dictionary::<SetHtmlUnsafeOptions<'s>>(scope, args.get(0), context) {
            Ok(options) => options.unwrap_or_default(),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
    let policy = args.this();
    let Some(callback) = policy_callback(scope, policy, CREATE_PARSER_OPTIONS_SLOT) else {
        webidl::throw_type_error(scope, "The policy does not define createParserOptions.");
        return;
    };
    let Some(argument) = options.callback_argument(scope, policy) else {
        return;
    };
    let arguments = moli_v8_util::callback_arguments_with_first(&args, argument.into());
    let result = match invoke_callback(scope, callback, &arguments, convert_policy_result) {
        TrustedTypePolicyCallbackOutcome::Returned(Some(result)) => result,
        TrustedTypePolicyCallbackOutcome::Returned(None) => {
            webidl::throw_type_error(scope, "createParserOptions returned no parser options.");
            return;
        }
        TrustedTypePolicyCallbackOutcome::Abrupt | TrustedTypePolicyCallbackOutcome::Missing => {
            return;
        }
    };
    let Some(context) = policy.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(options) = result.bind_trusted(scope) {
        rv.set(options.into());
    }
}
