//! Native contracts: no Browser object registry participates in these tests.
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, pin::pin, rc::Rc};

pub(crate) fn assert_object_namespaces_and_groups(
    mut dispatch: impl FnMut(&str, &str) -> Vec<Value>,
) {
    let mut next_id = 0;
    let mut call = |session: &str, method: &str, params: Value| {
        next_id += 1;
        let messages = dispatch(
            session,
            &json!({"id":next_id,"method":method,"params":params}).to_string(),
        );
        messages
            .into_iter()
            .find(|message| message["id"] == next_id)
            .unwrap_or_else(|| panic!("missing response to {session} {method}"))
    };
    let a = call(
        "A",
        "Runtime.evaluate",
        json!({
            "expression":"({tag:'A',child:{answer:42}})","objectGroup":"shared-name"
        }),
    );
    let a_id = object_id(&a);
    let missing = call("B", "Runtime.getProperties", json!({"objectId":a_id}));
    assert!(missing.get("error").is_some(), "{missing}");
    let b = call(
        "B",
        "Runtime.evaluate",
        json!({
            "expression":"({tag:'B'})","objectGroup":"shared-name"
        }),
    );
    let b_id = object_id(&b);
    // IDs may be equal. Whichever string B receives must resolve in B's
    // namespace, rather than being rejected because A registered it first.
    let b_tag = call(
        "B",
        "Runtime.callFunctionOn",
        json!({
            "objectId":b_id,"functionDeclaration":"function(){return this.tag}","returnByValue":true
        }),
    );
    assert_eq!(b_tag["result"]["result"]["value"], "B", "{b_tag}");
    if a_id == b_id {
        let local = call(
            "B",
            "Runtime.callFunctionOn",
            json!({
                "objectId":a_id,"functionDeclaration":"function(){return this.tag}","returnByValue":true
            }),
        );
        assert_eq!(local["result"]["result"]["value"], "B", "{local}");
    }
    let properties = call(
        "A",
        "Runtime.getProperties",
        json!({"objectId":a_id,"ownProperties":true}),
    );
    let child = properties["result"]["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|property| property["name"] == "child")
        .unwrap()["value"]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let inherited = object_id(&call(
        "A",
        "Runtime.callFunctionOn",
        json!({
            "objectId":a_id,"functionDeclaration":"function(){return this.child}"
        }),
    ));
    let independent = object_id(&call(
        "A",
        "Runtime.callFunctionOn",
        json!({
            "objectId":a_id,"functionDeclaration":"function(){return this.child}","objectGroup":"independent"
        }),
    ));
    let promise = object_id(&call(
        "A",
        "Runtime.evaluate",
        json!({
            "expression":"Promise.resolve({answer:42})","objectGroup":"shared-name"
        }),
    ));
    let awaited = object_id(&call(
        "A",
        "Runtime.awaitPromise",
        json!({"promiseObjectId":promise}),
    ));
    assert_eq!(
        call(
            "A",
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":"shared-name"})
        )["result"],
        json!({})
    );
    for id in [a_id, child, inherited, promise, awaited] {
        let released = call("A", "Runtime.getProperties", json!({"objectId":id}));
        assert!(
            released.get("error").is_some(),
            "released native reference survived: {released}"
        );
    }
    for (session, id) in [("A", independent), ("B", b_id)] {
        let retained = call(session, "Runtime.getProperties", json!({"objectId":id}));
        assert!(
            retained.get("result").is_some(),
            "peer group was released: {retained}"
        );
    }
    let prototype = object_id(&call(
        "A",
        "Runtime.evaluate",
        json!({
            "expression":"(()=>{class Probe{};globalThis.nativeQueryProbe=new Probe;return Probe.prototype})()",
            "objectGroup":"prototype"
        }),
    ));
    let inherited_query = call(
        "A",
        "Runtime.queryObjects",
        json!({"prototypeObjectId":prototype}),
    );
    let inherited_query = inherited_query["result"]["objects"]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let ungrouped = call(
        "A",
        "Runtime.queryObjects",
        json!({"prototypeObjectId":prototype,"objectGroup":""}),
    );
    let ungrouped = ungrouped["result"]["objects"]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let grouped = call(
        "A",
        "Runtime.queryObjects",
        json!({"prototypeObjectId":prototype,"objectGroup":"query"}),
    );
    let grouped = grouped["result"]["objects"]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        "A",
        "Runtime.releaseObjectGroup",
        json!({"objectGroup":"prototype"}),
    );
    // Omitting the group inherits the prototype's group. Explicit empty and
    // independently named groups survive releasing the prototype's group.
    assert!(
        call(
            "A",
            "Runtime.getProperties",
            json!({"objectId":inherited_query})
        )
        .get("error")
        .is_some()
    );
    for id in [&ungrouped, &grouped] {
        assert!(
            call("A", "Runtime.getProperties", json!({"objectId":id}))
                .get("result")
                .is_some()
        );
    }
    call(
        "A",
        "Runtime.releaseObjectGroup",
        json!({"objectGroup":"query"}),
    );
    assert!(
        call("A", "Runtime.getProperties", json!({"objectId":grouped}))
            .get("error")
            .is_some()
    );
    assert!(
        call("A", "Runtime.getProperties", json!({"objectId":ungrouped}))
            .get("result")
            .is_some()
    );
    call("A", "Runtime.releaseObject", json!({"objectId":ungrouped}));
    assert!(
        call("A", "Runtime.getProperties", json!({"objectId":ungrouped}))
            .get("error")
            .is_some()
    );
}

fn object_id(response: &Value) -> String {
    response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("missing native object reference: {response}"))
        .to_owned()
}

#[derive(Clone, Default)]
struct Output(Rc<RefCell<Vec<Value>>>);

impl v8::inspector::ChannelImpl for Output {
    fn send_response(&self, _: i32, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        let message = message.as_ref().expect("Inspector response buffer");
        self.0
            .borrow_mut()
            .push(serde_json::from_str(&message.string().to_string()).unwrap());
    }
    fn send_notification(&self, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        let message = message.as_ref().expect("Inspector notification buffer");
        self.0
            .borrow_mut()
            .push(serde_json::from_str(&message.string().to_string()).unwrap());
    }
    fn flush_protocol_notifications(&self) {}
}

struct Client;
impl v8::inspector::V8InspectorClientImpl for Client {}

struct NativeSession {
    session: Rc<v8::inspector::V8InspectorSession>,
    output: Output,
    context_id: i32,
}

// Sessions and contexts must drop before their Inspector and isolate.
struct NativeInspector {
    sessions: HashMap<&'static str, NativeSession>,
    contexts: Vec<v8::Global<v8::Context>>,
    inspector: v8::inspector::V8Inspector,
    isolate: v8::OwnedIsolate,
}

impl NativeInspector {
    fn new() -> Self {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let inspector = v8::inspector::V8Inspector::create(
            &mut isolate,
            v8::inspector::V8InspectorClient::new(Box::new(Client)),
        );
        Self {
            sessions: HashMap::new(),
            contexts: Vec::new(),
            inspector,
            isolate,
        }
    }

    fn attach(&mut self, name: &'static str, group: i32) {
        let index = usize::try_from(group - 1).unwrap();
        let context_id = {
            let scope = pin!(v8::HandleScope::new(&mut self.isolate));
            let scope = &mut scope.init();
            let context = if let Some(context) = self.contexts.get(index) {
                v8::Local::new(scope, context)
            } else {
                assert_eq!(index, self.contexts.len());
                let context = v8::Context::new(scope, Default::default());
                self.inspector.context_created(
                    context,
                    group,
                    v8::inspector::StringView::from(&b"native-contract"[..]),
                    v8::inspector::StringView::empty(),
                    v8::inspector::StringView::from(&b"{}"[..]),
                );
                self.contexts.push(v8::Global::new(scope, context));
                context
            };
            v8::inspector::V8Inspector::execution_context_id(context)
        };
        let output = Output::default();
        let session = self.inspector.connect(
            group,
            v8::inspector::Channel::new(Box::new(output.clone())),
            v8::inspector::StringView::empty(),
            v8::inspector::V8InspectorClientTrustLevel::FullyTrusted,
        );
        self.sessions.insert(
            name,
            NativeSession {
                session: Rc::new(session),
                output,
                context_id,
            },
        );
    }

    fn dispatch(&mut self, name: &str, raw: &str) -> Vec<Value> {
        let session = &self.sessions[name];
        let mut request: Value = serde_json::from_str(raw).unwrap();
        if request["method"] == "Runtime.evaluate" {
            request["params"]["contextId"] = json!(session.context_id);
        }
        let scope = pin!(v8::HandleScope::new(&mut self.isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &self.contexts[0]);
        let scope = &mut v8::ContextScope::new(scope, context);
        session
            .session
            .dispatch_protocol_message(v8::inspector::StringView::from(
                request.to_string().as_bytes(),
            ));
        scope.perform_microtask_checkpoint();
        std::mem::take(&mut *session.output.0.borrow_mut())
    }

    fn call(&mut self, name: &str, method: &str, params: Value) -> Value {
        self.dispatch(
            name,
            &json!({"id":1,"method":method,"params":params}).to_string(),
        )
        .into_iter()
        .find(|message| message["id"] == 1)
        .expect("native response")
    }

    fn weak(&mut self, name: &str, id: &str) -> v8::Weak<v8::Object> {
        let scope = pin!(v8::HandleScope::new(&mut self.isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &self.contexts[0]);
        let scope = &mut v8::ContextScope::new(scope, context);
        let (value, context, _) = self.sessions[name]
            .session
            .unwrap_object(scope, v8::inspector::StringView::from(id.as_bytes()))
            .expect("unwrap native object in its producing session");
        assert_eq!(
            v8::inspector::V8Inspector::execution_context_id(context),
            self.sessions[name].context_id
        );
        v8::Weak::new(scope, value.try_cast::<v8::Object>().unwrap())
    }

    fn collect(&mut self) {
        self.isolate.clear_kept_objects();
        self.isolate.low_memory_notification();
    }
}

#[test]
fn native_inspector_session_owns_object_namespaces_and_groups() {
    let mut inspector = NativeInspector::new();
    inspector.attach("A", 1);
    inspector.attach("B", 1);
    assert_object_namespaces_and_groups(|session, request| inspector.dispatch(session, request));
}

#[test]
fn native_inspector_context_groups_do_not_fallback_to_another_context() {
    let mut inspector = NativeInspector::new();
    inspector.attach("A", 1);
    inspector.attach("C", 2);
    let id =
        object_id(&inspector.call("A", "Runtime.evaluate", json!({"expression":"({tag:'A'})"})));
    assert!(
        inspector
            .call("C", "Runtime.getProperties", json!({"objectId":id}))
            .get("error")
            .is_some()
    );
    let c_id =
        object_id(&inspector.call("C", "Runtime.evaluate", json!({"expression":"({tag:'C'})"})));
    assert!(
        inspector
            .call("A", "Runtime.getProperties", json!({"objectId":c_id}))
            .get("error")
            .is_some()
    );
    assert_eq!(
        inspector.call(
            "C",
            "Runtime.callFunctionOn",
            json!({"objectId":c_id,
        "functionDeclaration":"function(){return this.tag}","returnByValue":true})
        )["result"]["result"]["value"],
        "C"
    );
}

#[test]
fn native_inspector_detach_releases_grouped_and_ungrouped_roots_after_last_owner() {
    let mut inspector = NativeInspector::new();
    inspector.attach("A", 1);
    inspector.attach("B", 1);
    let grouped = object_id(&inspector.call(
        "A",
        "Runtime.evaluate",
        json!({"expression":"({tag:'grouped'})","objectGroup":"same"}),
    ));
    let grouped = inspector.weak("A", &grouped);
    let ungrouped = object_id(&inspector.call(
        "A",
        "Runtime.evaluate",
        json!({"expression":"({tag:'ungrouped'})"}),
    ));
    let ungrouped = inspector.weak("A", &ungrouped);
    let peer_id = object_id(&inspector.call(
        "B",
        "Runtime.evaluate",
        json!({"expression":"({tag:'peer'})","objectGroup":"same"}),
    ));
    let peer = inspector.weak("B", &peer_id);
    inspector.call("A", "Runtime.enable", json!({}));
    inspector.call("A", "Runtime.disable", json!({}));
    inspector.collect();
    assert!(
        !grouped.is_empty() && !ungrouped.is_empty(),
        "Runtime.disable is not session detach"
    );
    let in_flight = Rc::clone(&inspector.sessions["A"].session);
    inspector.sessions.remove("A");
    inspector.collect();
    assert!(
        !grouped.is_empty() && !ungrouped.is_empty(),
        "removing a map entry does not dispose an in-flight Rc"
    );
    drop(in_flight);
    inspector.collect();
    assert!(
        grouped.is_empty() && ungrouped.is_empty(),
        "the last native session owner releases all Inspector roots"
    );
    assert!(
        !peer.is_empty(),
        "a peer session's same-named group must survive"
    );
    assert!(
        inspector
            .call("B", "Runtime.getProperties", json!({"objectId":peer_id}))
            .get("result")
            .is_some()
    );
    inspector.attach("A", 1);
    assert!(
        inspector
            .call("A", "Runtime.getProperties", json!({"objectId":peer_id}))
            .get("error")
            .is_some(),
        "a fresh native session has no inherited objects or fallback to B"
    );
}
