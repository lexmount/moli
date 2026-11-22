use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_parser::HtmlFragmentStream;
use moli_v8_util::v8_string_to_u16_string;
use moli_webapi::streams::adapter::{new_writable_stream_object, set_resolved_promise};
use moli_webapi_declare::WebApiObject;

use crate::{
    context_bootstrap::sanitizer::SetHtmlOptions,
    custom_elements,
    document_runtime::{DomHandle, HtmlInsertionAction},
    native_bridge::{
        PageScriptHost, bridge::wrapped_handle_value,
        node::node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
    },
    util::{get_private_object, get_private_value},
    webidl,
};

use super::content::HtmlInsertionPosition;

const STREAM_ID: &str = "moli.HtmlFragmentStream.id";
const STREAM_TARGET: &str = "moli.HtmlFragmentStream.target";

type StreamState = Rc<RefCell<Option<ActiveFragmentStream>>>;
type StreamStore = Rc<RefCell<FragmentStreams>>;

struct ActiveFragmentStream {
    parser: HtmlFragmentStream,
    host: *mut PageScriptHost,
    target: DomHandle,
    leading_surrogate: Option<u16>,
}

struct FragmentStreamEntry {
    _sink: v8::Weak<v8::Object>,
    stream: StreamState,
}

#[derive(Default)]
struct FragmentStreams {
    next_id: u64,
    entries: HashMap<u64, FragmentStreamEntry>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FragmentStreamSink<'s> {
    #[webapi(slot = STREAM_ID)]
    id: v8::Local<'s, v8::BigInt>,
    #[webapi(slot = STREAM_TARGET)]
    target: v8::Local<'s, v8::Object>,
    #[webapi(method, callback = write_callback, length = 1)]
    write: (),
    #[webapi(method, callback = close_callback, length = 0)]
    close: (),
    #[webapi(method, callback = abort_callback, length = 1)]
    abort: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTML streaming")]
struct StreamHtmlArgs<'s> {
    #[webidl(dictionary)]
    options: SetHtmlOptions<'s>,
}

fn new_fragment_writable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    stream: ActiveFragmentStream,
) -> Option<v8::Local<'s, v8::Object>> {
    let store = scope.get_slot::<StreamStore>().cloned().unwrap_or_else(|| {
        let store = StreamStore::default();
        scope.set_slot(store.clone());
        store
    });
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("HTML fragment stream identities exhausted");
        store.next_id
    };
    let value = v8::BigInt::new_from_u64(scope, id);
    let sink = FragmentStreamSink::new(value, target).bind(scope).ok()?;
    // The sink is a private adapter object. Its algorithms must not read
    // author properties such as Object.prototype.start.
    let null = v8::null(scope);
    sink.set_prototype(scope, null.into())
        .filter(|changed| *changed)?;
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        sink,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(
        id,
        FragmentStreamEntry {
            _sink: weak,
            stream: Rc::new(RefCell::new(Some(stream))),
        },
    );
    Some(new_writable_stream_object(scope, Some(sink), 1.0, None))
}

fn stream_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
) -> Option<StreamState> {
    let value = get_private_value(scope, sink, STREAM_ID)?;
    let id = v8::Local::<v8::BigInt>::try_from(value).ok()?.u64_value().0;
    scope
        .get_slot::<StreamStore>()?
        .borrow()
        .entries
        .get(&id)
        .map(|entry| entry.stream.clone())
}

fn stream_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    stream: &ActiveFragmentStream,
) -> Option<*mut PageScriptHost> {
    let target = get_private_object(scope, sink, STREAM_TARGET)?;
    let (runtime, handle) = node_runtime_and_handle_from_object_or_detached(scope, target).ok()?;
    (runtime == stream.host && handle == stream.target).then_some(runtime)
}

fn throw_displaced_reference(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "HierarchyRequestError",
        3,
        "The reference child is no longer a child of the HTML stream's target.",
    );
}

fn abort_fragment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    stream: ActiveFragmentStream,
) {
    if let Some(runtime_ptr) = stream_runtime(scope, sink, &stream) {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.abort_html_fragment_stream(scope, runtime_ptr, stream.parser);
    }
}

fn write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = stream_state(scope, args.this()) else {
        return;
    };
    let input = {
        v8::tc_scope!(let tc, scope);
        match args.get(0).to_string(tc) {
            Some(chunk) => Ok(v8_string_to_u16_string(tc, chunk).into_vec()),
            None => Err(tc.exception().map(|error| v8::Global::new(tc, error))),
        }
    };
    let Some(mut stream) = state.borrow_mut().take() else {
        return;
    };
    let mut chunk = match input {
        Ok(chunk) => chunk,
        Err(exception) => {
            abort_fragment(scope, args.this(), stream);
            if let Some(exception) = exception {
                let exception = v8::Local::new(scope, &exception);
                scope.throw_exception(exception);
            }
            return;
        }
    };
    let Some(runtime_ptr) = stream_runtime(scope, args.this(), &stream) else {
        throw_displaced_reference(scope);
        return;
    };
    if let Some(leading) = stream.leading_surrogate.take() {
        chunk.insert(0, leading);
    }
    // A DOMString chunk can end between the code units of one character.
    if chunk
        .last()
        .is_some_and(|unit| (0xd800..=0xdbff).contains(unit))
    {
        stream.leading_surrogate = chunk.pop();
    }
    let chunk = String::from_utf16_lossy(&chunk);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime
        .write_html_fragment_stream(scope, runtime_ptr, &stream.parser, &chunk)
        .is_err()
    {
        runtime.abort_html_fragment_stream(scope, runtime_ptr, stream.parser);
        throw_displaced_reference(scope);
        return;
    }
    state.replace(Some(stream));
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_state(scope, args.this()).and_then(|state| state.borrow_mut().take())
    else {
        return;
    };
    let Some(runtime_ptr) = stream_runtime(scope, args.this(), &stream) else {
        throw_displaced_reference(scope);
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if stream.leading_surrogate.is_some()
        && runtime
            .write_html_fragment_stream(scope, runtime_ptr, &stream.parser, "\u{fffd}")
            .is_err()
    {
        runtime.abort_html_fragment_stream(scope, runtime_ptr, stream.parser);
        throw_displaced_reference(scope);
        return;
    }
    if runtime
        .close_html_fragment_stream(scope, runtime_ptr, stream.parser)
        .is_err()
    {
        throw_displaced_reference(scope);
        return;
    }
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) =
        stream_state(scope, args.this()).and_then(|state| state.borrow_mut().take())
    {
        abort_fragment(scope, args.this(), stream);
    }
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn create_html_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    position: HtmlInsertionPosition,
) -> Option<v8::Local<'s, v8::Object>> {
    let parsed = webidl::parse_args::<StreamHtmlArgs>(scope, &args)?;
    let receiver = args.this();
    let (host_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
    let host = unsafe { &*host_ptr }.dom_host();
    let Ok(Some(target)) = position.target(host, handle) else {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "The target cannot accept HTML streaming.",
        );
        return None;
    };
    let context = host
        .shadow_root_host(target.context)
        .unwrap_or(target.context);
    if host.is_script_element(context) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "An HTML stream cannot target a script element.",
        );
        return None;
    }
    let reference = match target.action {
        HtmlInsertionAction::ReplaceAll => None,
        HtmlInsertionAction::InsertBefore(reference) => reference,
        HtmlInsertionAction::Replace(node) => host.next_sibling(node),
    };
    let sanitizer = parsed.options.prepare(scope)?;
    let realm = receiver.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, realm);
    let target_object =
        v8::Local::<v8::Object>::try_from(wrapped_handle_value(scope, host_ptr, target.parent)?)
            .ok()?;
    let runtime = unsafe { &mut *host_ptr };
    let parser =
        runtime.start_html_fragment_stream(scope, host_ptr, &target, reference, sanitizer)?;
    let stream = new_fragment_writable_stream(
        scope,
        target_object,
        ActiveFragmentStream {
            parser,
            host: host_ptr,
            target: target.parent,
            leading_surrogate: None,
        },
    )?;
    custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        let runtime = unsafe { &mut *host_ptr };
        match target.action {
            HtmlInsertionAction::ReplaceAll => {
                runtime.replace_all_children_appending_to_current_reaction_queue(
                    scope,
                    host_ptr,
                    target.parent,
                    None,
                );
            }
            HtmlInsertionAction::Replace(node) => {
                runtime.remove_child_appending_to_current_reaction_queue(
                    scope,
                    host_ptr,
                    target.parent,
                    node,
                );
            }
            HtmlInsertionAction::InsertBefore(_) => {}
        }
    });
    Some(stream)
}

macro_rules! stream_html_callback {
    ($name:ident, $position:ident) => {
        pub(in crate::native_bridge) fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            if let Some(stream) = create_html_stream(scope, args, HtmlInsertionPosition::$position)
            {
                rv.set(stream.into());
            }
        }
    };
}

stream_html_callback!(stream_html, Set);
stream_html_callback!(stream_append_html, Append);
stream_html_callback!(stream_prepend_html, Prepend);
stream_html_callback!(stream_before_html, Before);
stream_html_callback!(stream_after_html, After);
stream_html_callback!(stream_replace_with_html, Replace);
