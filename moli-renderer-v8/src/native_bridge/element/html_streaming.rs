use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_parser::HtmlFragmentStream;
use moli_v8_util::v8_string_to_u16_string;
use moli_webapi::streams::adapter::{
    new_writable_stream_object, rejected_promise_value, set_resolved_promise,
};
use moli_webapi_declare::WebApiObject;

use crate::{
    context_bootstrap::{CompliantParserOptions, UnsafeHtmlStreamArgs, sanitizer::SetHtmlOptions},
    custom_elements,
    document_runtime::HtmlInsertionAction,
    native_bridge::{
        PageScriptHost, bridge::wrapped_handle_value,
        node::node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
    },
    page_task_queue::{HtmlFragmentModuleTaskTarget, HtmlFragmentScriptTask},
    planning::PreparedScript,
    util::{get_private_object, get_private_value},
    webidl,
};

use super::content::HtmlInsertionPosition;

mod parser;
mod scripts;

use parser::{FragmentStreamOwner, FragmentStreamProgress};

const STREAM_ID: &str = "moli.HtmlFragmentStream.id";
const STREAM_TARGET: &str = "moli.HtmlFragmentStream.target";

pub(crate) enum HtmlFragmentModuleTerminal {
    Main(Box<crate::module_script_continuation::MainParserModuleGraphTerminalWork>),
    Child(Box<crate::document_script_scheduler::FrameDocumentModuleScriptReadyWork>),
    Unavailable(crate::document_runtime::DomHandle),
}

pub(crate) struct HtmlFragmentModuleStart {
    pub(crate) script: PreparedScript,
    pub(crate) still_in_preparation_document: bool,
}

type StreamState = Rc<FragmentStream>;
type StreamStore = Rc<RefCell<FragmentStreams>>;

struct FragmentStream {
    owner: FragmentStreamOwner,
    id: u64,
    progress: RefCell<FragmentStreamProgress>,
    writable: v8::Weak<v8::Object>,
}

struct FragmentStreamEntry {
    sink: v8::Weak<v8::Object>,
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
    owner: FragmentStreamOwner,
    parser: HtmlFragmentStream,
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
    let writable = new_writable_stream_object(scope, Some(sink), 1.0, None);
    let weak_store = Rc::downgrade(&store);
    let sink = v8::Weak::with_finalizer(
        scope,
        sink,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                remove_collected_stream(&mut store.borrow_mut(), id);
            }
        }),
    );
    let stream = Rc::new(FragmentStream {
        owner,
        id,
        progress: RefCell::new(FragmentStreamProgress::new(parser)),
        writable: v8::Weak::new(scope, writable),
    });
    store
        .borrow_mut()
        .entries
        .insert(id, FragmentStreamEntry { sink, stream });
    Some(writable)
}

fn remove_collected_stream(store: &mut FragmentStreams, id: u64) {
    if store.entries.get(&id).is_some_and(|entry| {
        entry.sink.is_empty()
            && entry
                .stream
                .progress
                .try_borrow()
                .is_ok_and(|progress| !progress.has_tasks())
    }) {
        store.entries.remove(&id);
    }
}

fn stream_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
) -> Option<StreamState> {
    let value = get_private_value(scope, sink, STREAM_ID)?;
    let id = v8::Local::<v8::BigInt>::try_from(value).ok()?.u64_value().0;
    stream_state_by_id(scope, id)
}

fn stream_state_by_id(scope: &mut v8::PinScope<'_, '_>, id: u64) -> Option<StreamState> {
    scope
        .get_slot::<StreamStore>()?
        .borrow()
        .entries
        .get(&id)
        .map(|entry| entry.stream.clone())
}

fn stream_target_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    stream: &FragmentStream,
) -> bool {
    get_private_object(scope, sink, STREAM_TARGET)
        .and_then(|target| node_runtime_and_handle_from_object_or_detached(scope, target).ok())
        == Some((stream.owner.host, stream.owner.target))
}

fn write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_state(scope, args.this()) else {
        return;
    };
    let input = {
        v8::tc_scope!(let tc, scope);
        match args.get(0).to_string(tc) {
            Some(chunk) => Ok(v8_string_to_u16_string(tc, chunk).into_vec()),
            None => Err(tc.exception().map(|error| v8::Global::new(tc, error))),
        }
    };
    let chunk = match input {
        Ok(chunk) => chunk,
        Err(exception) => {
            if let Some(exception) = exception {
                let exception = v8::Local::new(scope, &exception);
                stream.abort_parser(scope, Some(exception));
                if let Some(promise) = rejected_promise_value(scope, exception) {
                    rv.set(promise);
                }
            } else {
                stream.abort_parser(scope, None);
            }
            return;
        }
    };
    if !stream_target_is_current(scope, args.this(), &stream) || !stream.reference_is_valid() {
        let error = stream.invalid_reference(scope);
        if let Some(promise) = rejected_promise_value(scope, error) {
            rv.set(promise);
        }
        return;
    }
    stream.progress.borrow_mut().append_chunk(chunk);
    if let Err(error) = stream.run_fragment_parser(scope) {
        if let Some(promise) = rejected_promise_value(scope, error) {
            rv.set(promise);
        }
        return;
    }
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_state(scope, args.this()) else {
        return;
    };
    let Some(close) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = close.get_promise(scope);
    let global = v8::Global::new(scope, close);
    stream.progress.borrow_mut().start_close(global);
    if stream_target_is_current(scope, args.this(), &stream) {
        // A close error is reported through the promise returned to the sink.
        let _ = stream.run_fragment_parser(scope);
    } else {
        stream.invalid_reference(scope);
    }
    rv.set(promise.into());
}

fn abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) = stream_state(scope, args.this()) {
        let reason = args.get(0);
        stream.abort_parser(scope, Some(reason));
    }
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

pub(crate) fn apply_html_fragment_script_task(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    task: HtmlFragmentScriptTask,
) -> bool {
    let Some(stream) = stream_state_by_id(scope, task.stream) else {
        return false;
    };
    let target = task.owner.target();
    if stream.owner.host != host_ptr || stream.owner.script_target != Some(target) {
        return false;
    }
    let host = unsafe { &mut *host_ptr };
    let Some(context) =
        host.current_html_fragment_script_context(scope, target, stream.owner.document)
    else {
        let dispatched = stream.retire_script_tasks(scope);
        remove_finished_stream(scope, task.stream);
        return dispatched;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let dispatch_scope = target.dispatch_scope();
    let previous = dispatch_scope.enter(scope);
    let dispatched = stream.apply_script_task(scope, task.kind);
    dispatch_scope.restore(scope, previous);
    remove_finished_stream(scope, task.stream);
    dispatched
}

fn fragment_module_stream(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: HtmlFragmentModuleTaskTarget,
) -> Option<StreamState> {
    let stream = stream_state_by_id(scope, target.stream)?;
    if stream.owner.host != host_ptr || stream.owner.script_target != Some(target.document) {
        return None;
    }
    let host = unsafe { &*host_ptr };
    if host.current_window_document_task_target_for_dispatch_scope(target.document.dispatch_scope())
        != Some(target.document)
    {
        stream.retire_script_tasks(scope);
        remove_finished_stream(scope, target.stream);
        return None;
    }
    Some(stream)
}

pub(crate) fn take_html_fragment_module_start(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: HtmlFragmentModuleTaskTarget,
) -> Option<HtmlFragmentModuleStart> {
    let stream = fragment_module_stream(scope, host_ptr, target)?;
    let script = stream.take_module_start(target.job)?;
    let still_in_preparation_document =
        stream.script_is_in_preparation_document(target.document, script.node_id);
    Some(HtmlFragmentModuleStart {
        script,
        still_in_preparation_document,
    })
}

pub(crate) fn accept_html_fragment_module_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: HtmlFragmentModuleTaskTarget,
    terminal: HtmlFragmentModuleTerminal,
) -> bool {
    let Some(stream) = fragment_module_stream(scope, host_ptr, target) else {
        return false;
    };
    let accepted = stream.accept_module_terminal(target.document, target.job, terminal);
    remove_finished_stream(scope, target.stream);
    accepted
}

pub(crate) fn take_html_fragment_module_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: HtmlFragmentModuleTaskTarget,
) -> Option<HtmlFragmentModuleTerminal> {
    let stream = fragment_module_stream(scope, host_ptr, target)?;
    let work = stream.take_ready_module(target.document, target.job);
    remove_finished_stream(scope, target.stream);
    work
}

pub(crate) fn finish_html_fragment_module_turn(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: HtmlFragmentModuleTaskTarget,
    failure: Option<crate::document_runtime::DomHandle>,
) {
    let Some(stream) = fragment_module_stream(scope, host_ptr, target) else {
        return;
    };
    if let Some(node) = failure {
        stream.with_script_context(scope, node, |scope, _| {
            stream.dispatch_script_event(scope, node, "error");
        });
    }
    stream.finish_module_turn();
    remove_finished_stream(scope, target.stream);
}

fn remove_finished_stream(scope: &mut v8::PinScope<'_, '_>, id: u64) {
    if let Some(store) = scope.get_slot::<StreamStore>() {
        remove_collected_stream(&mut store.borrow_mut(), id);
    }
}

enum StreamOptions<'s> {
    Safe(SetHtmlOptions<'s>),
    Unsafe(CompliantParserOptions<'s>),
}

fn unsafe_stream_sink(position: &HtmlInsertionPosition, shadow_root: bool) -> &'static str {
    match (position, shadow_root) {
        (HtmlInsertionPosition::Set, false) => "Element streamHTMLUnsafe",
        (HtmlInsertionPosition::Set, true) => "ShadowRoot streamHTMLUnsafe",
        (HtmlInsertionPosition::Append, false) => "Element streamAppendHTMLUnsafe",
        (HtmlInsertionPosition::Append, true) => "ShadowRoot streamAppendHTMLUnsafe",
        (HtmlInsertionPosition::Prepend, false) => "Element streamPrependHTMLUnsafe",
        (HtmlInsertionPosition::Prepend, true) => "ShadowRoot streamPrependHTMLUnsafe",
        (HtmlInsertionPosition::Before, _) => "Node streamBeforeHTMLUnsafe",
        (HtmlInsertionPosition::After, _) => "Node streamAfterHTMLUnsafe",
        (HtmlInsertionPosition::Replace, _) => "Node streamReplaceWithHTMLUnsafe",
    }
}

fn create_html_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    position: HtmlInsertionPosition,
    unsafe_markup: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    let options = if unsafe_markup {
        let parsed = webidl::parse_args::<UnsafeHtmlStreamArgs>(scope, &args)?;
        let (host_ptr, handle) =
            node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
        // Adoption updates the policy's node document while preserving the
        // original wrapper and the realm in which the method was installed.
        let context =
            crate::native_bridge::node_owner_document_relevant_context(scope, host_ptr, handle)?;
        let global = context.global(scope);
        let host = unsafe { &*host_ptr };
        let sink = unsafe_stream_sink(&position, host.dom_host().is_shadow_root(handle));
        let requirements = host
            .owner_dispatch_scope_for_node(handle)
            .and_then(|owner| host.trusted_types_for_script_requirements_for_owner(owner))
            .unwrap_or_else(|| {
                host.trusted_types_for_script_requirements_for_global(scope, global)
            });
        StreamOptions::Unsafe(parsed.prepare(scope, global, requirements, sink)?)
    } else {
        let parsed = webidl::parse_args::<StreamHtmlArgs>(scope, &args)?;
        StreamOptions::Safe(parsed.options)
    };
    // A default policy can move the node before the insertion target is selected.
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
    let (sanitizer, run_scripts) = match options {
        StreamOptions::Safe(options) => (Some(options.prepare(scope)?), false),
        StreamOptions::Unsafe(options) => options.into_parser_options(scope)?,
    };
    let realm = receiver.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, realm);
    let target_object =
        v8::Local::<v8::Object>::try_from(wrapped_handle_value(scope, host_ptr, target.parent)?)
            .ok()?;
    let runtime = unsafe { &mut *host_ptr };
    let owner = FragmentStreamOwner::new(runtime, host_ptr, target.parent, reference)?;
    let parser = runtime.start_html_fragment_stream(
        scope,
        host_ptr,
        &target,
        reference,
        sanitizer,
        run_scripts,
    )?;
    let stream = new_fragment_writable_stream(scope, target_object, owner, parser)?;
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
    ($name:ident, $position:ident, $unsafe_markup:literal) => {
        pub(in crate::native_bridge) fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            if let Some(stream) = create_html_stream(
                scope,
                args,
                HtmlInsertionPosition::$position,
                $unsafe_markup,
            ) {
                rv.set(stream.into());
            }
        }
    };
}

stream_html_callback!(stream_html, Set, false);
stream_html_callback!(stream_append_html, Append, false);
stream_html_callback!(stream_prepend_html, Prepend, false);
stream_html_callback!(stream_before_html, Before, false);
stream_html_callback!(stream_after_html, After, false);
stream_html_callback!(stream_replace_with_html, Replace, false);

stream_html_callback!(stream_html_unsafe, Set, true);
stream_html_callback!(stream_append_html_unsafe, Append, true);
stream_html_callback!(stream_prepend_html_unsafe, Prepend, true);
stream_html_callback!(stream_before_html_unsafe, Before, true);
stream_html_callback!(stream_after_html_unsafe, After, true);
stream_html_callback!(stream_replace_with_html_unsafe, Replace, true);
