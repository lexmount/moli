//! Native channel-routing interfaces and constraints for the partial audio backend.

use super::*;
use webidl::WebIdlDictionary;

#[derive(Clone, Copy)]
enum Kind {
    Merger,
    Splitter,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Merger => "ChannelMergerNode",
            Self::Splitter => "ChannelSplitterNode",
        }
    }

    fn options_name(self) -> &'static str {
        match self {
            Self::Merger => "ChannelMergerOptions",
            Self::Splitter => "ChannelSplitterOptions",
        }
    }

    fn port_member(self) -> &'static str {
        match self {
            Self::Merger => "numberOfInputs",
            Self::Splitter => "numberOfOutputs",
        }
    }

    fn layout(self, ports: u32) -> node::ChannelLayout {
        match self {
            Self::Merger => node::ChannelLayout::Merger(ports),
            Self::Splitter => node::ChannelLayout::Splitter(ports),
        }
    }
}

struct Options {
    node: node::AudioNodeOptions,
    ports: u32,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: Kind,
) -> Result<Options, webidl::WebIdlError> {
    let mut options = Options {
        node: node::AudioNodeOptions::default(),
        ports: 6,
    };
    let Some(object) = webidl::dictionary_value(value, webidl::Context::argument(kind.name(), 2))?
    else {
        return Ok(options);
    };
    // Convert inherited members before derived members; state and range checks
    // must not prevent a later dictionary getter or conversion from running.
    options.node = node::AudioNodeOptions::parse_dictionary(scope, object)?;
    options.ports = webidl::optional_member_or::<webidl::UnsignedLong>(
        scope,
        object,
        kind.port_member(),
        webidl::Context::member(kind.options_name(), kind.port_member()),
        webidl::UnsignedLong(6),
    )?
    .0;
    Ok(options)
}

fn valid_port_count(scope: &mut v8::PinScope<'_, '_>, ports: u32) -> bool {
    if !(1..=32).contains(&ports) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "The number of channel ports must be between 1 and 32.",
        );
        return false;
    }
    true
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    kind: Kind,
    options: Options,
) -> bool {
    graph::initialize_node(scope, node, context);
    node::initialize_channel_layout(scope, node, kind.layout(options.ports));
    if !node::apply_options(scope, node, options.node) {
        return false;
    }
    graph::mark_unsupported_processor(scope, node);
    true
}

fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    kind: Kind,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Audio node constructors require the new operator.");
        return;
    }
    let context = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    let Some(context) = context
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "Audio node constructors require a BaseAudioContext.");
        return;
    };
    let options = match parse_options(scope, args.get(1), kind) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !valid_port_count(scope, options.ports) {
        return;
    }
    let node = args.this();
    web_api_interfaces::initialize(scope, node, kind.name())
        .expect("Audio node brand should initialize");
    if initialize(scope, node, context, kind, options) {
        rv.set(node.into());
    }
}

pub(in crate::context_bootstrap) fn channel_merger_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    construct(scope, args, rv, Kind::Merger);
}

pub(in crate::context_bootstrap) fn channel_splitter_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    construct(scope, args, rv, Kind::Splitter);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createChannelMerger")]
struct MergerArgs {
    #[webidl(default = 6)]
    number_of_inputs: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createChannelSplitter")]
struct SplitterArgs {
    #[webidl(default = 6)]
    number_of_outputs: u32,
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    kind: Kind,
    ports: u32,
) {
    if !valid_port_count(scope, ports) {
        return;
    }
    // Factory results belong to the AudioContext's relevant realm, even when
    // the method is borrowed from another Window. Conversions and exceptions
    // above still use the callee realm.
    let context = args
        .this()
        .get_creation_context(scope)
        .expect("Audio context should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let prototype =
        super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(scope, kind.name())
            .expect("Audio node prototype should exist");
    let node = v8::Object::new(scope);
    let _ = node.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, node, kind.name())
        .expect("Audio node brand should initialize");
    let options = Options {
        node: node::AudioNodeOptions::default(),
        ports,
    };
    if initialize(scope, node, args.this(), kind, options) {
        rv.set(node.into());
    }
}

pub(super) fn create_channel_merger<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<MergerArgs>(scope, &args) else {
        return;
    };
    create(scope, args, rv, Kind::Merger, parsed.number_of_inputs);
}

pub(super) fn create_channel_splitter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<SplitterArgs>(scope, &args) else {
        return;
    };
    create(scope, args, rv, Kind::Splitter, parsed.number_of_outputs);
}
