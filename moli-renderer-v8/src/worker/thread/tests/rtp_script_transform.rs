use super::*;
use crate::worker::WorkerMessage;
use crate::worker::rtp_tasks::{RtpTransformTask, TransformLease};

#[tokio::test]
async fn worker_rtp_transform_tasks_wait_for_module_bootstrap_and_preserve_message_order() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let bootstrap = if matches!(&kind, WorkerScriptKind::Module) {
            "await new Promise(resolve => setTimeout(resolve, 0));"
        } else {
            ""
        };
        let source = format!(
            r#"{bootstrap}
            const order = [];
            addEventListener('rtctransform', event => {{
              const t = event.transformer;
              order.push('transform:' + t.options.tag);
              const writer = t.writable.getWriter();
              const getter = Object.getOwnPropertyDescriptor(RTCRtpScriptTransformer.prototype, 'options').get;
              let brand = false;
              try {{ getter.call(new Proxy(t, {{}})); }} catch (e) {{ brand = e instanceof TypeError; }}
              postMessage({{kind:'transform', tag:t.options.tag, trusted:event.isTrusted,
                target:event.target===self, current:event.currentTarget===self,
                eventBrand:event instanceof RTCTransformEvent && event instanceof Event,
                transformerBrand:t instanceof RTCRtpScriptTransformer && t instanceof EventTarget,
                readable:t.readable instanceof ReadableStream,
                writable:t.writable instanceof WritableStream,
                infinity:writer.desiredSize===Infinity,
                stable:t.options===getter.call(t), brand,
                nested:t.options.nested.value}});
              writer.releaseLock();
            }});
            onrtctransform = event => order.push('handler:' + event.transformer.options.tag);
            onmessage = event => postMessage({{kind:'message', data:event.data, order:order.slice()}});
            postMessage({{kind:'ready'}});
            "#,
        );
        let mut worker = spawn_worker_with_request_client_and_kind(
            source,
            "https://rtp-transform.test/worker.js".into(),
            worker_test_request_client(),
            kind,
        );
        let lease = TransformLease::new(1);
        worker.queue_task(WorkerMessage::RtpTransform(RtpTransformTask::Create {
            id: 1,
            options: Box::new(serialize_test_value("({tag:'first',nested:{value:7}})")),
            lease: lease.clone(),
        }));
        worker.post_message(serialize_test_string("after"));
        assert_eq!(recv_post_json(&mut worker).await, r#"{"kind":"ready"}"#);
        let facts: serde_json::Value =
            serde_json::from_str(&recv_post_json(&mut worker).await).unwrap();
        assert_eq!(facts["kind"], "transform", "{facts}");
        assert_eq!(facts["tag"], "first", "{facts}");
        assert_eq!(facts["nested"], 7, "{facts}");
        for name in [
            "trusted",
            "target",
            "current",
            "eventBrand",
            "transformerBrand",
            "readable",
            "writable",
            "infinity",
            "stable",
            "brand",
        ] {
            assert_eq!(facts[name], true, "{name}: {facts}");
        }
        let delivery: serde_json::Value =
            serde_json::from_str(&recv_post_json(&mut worker).await).unwrap();
        assert_eq!(
            delivery,
            serde_json::json!({"kind":"message","data":"after","order":["transform:first","handler:first"]})
        );
        lease.retire();
        worker.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_rtp_transform_retirement_closes_locked_streams_without_author_catch_lookup() {
    ensure_v8();
    let mut worker = spawn_worker(
        r#"
        let inherited = 0, catches = 0, completed;
        for (const key of ['start','abort']) Object.defineProperty(Object.prototype,key,{configurable:true,get(){inherited++;throw Error('inherited sink algorithm');}});
        onrtctransform = event => {
          const t=event.transformer, reader=t.readable.getReader(), writer=t.writable.getWriter();
          completed=Promise.all([
            reader.read().then(result=>result.done&&result.value===undefined),
            reader.closed.then(()=>true),
            writer.closed.then(()=>false,error=>error===undefined)
          ]).then(values=>values.every(Boolean)&&t.readable.locked&&t.writable.locked);
          Object.defineProperty(Promise.prototype,'catch',{configurable:true,get(){catches++;throw Error('author catch');}});
          postMessage(['ready',t.options===undefined]);
        };
        onmessage=async()=>postMessage({closed:await completed,inherited,catches});
        "#.into(),
        "https://rtp-transform.test/retire.js".into(),
    );
    let lease = TransformLease::new(2);
    worker.queue_task(WorkerMessage::RtpTransform(RtpTransformTask::Create {
        id: 2,
        options: Box::new(serialize_test_value("undefined")),
        lease: lease.clone(),
    }));
    assert_eq!(recv_post_json(&mut worker).await, "[\"ready\",true]");
    lease.retire();
    worker.post_message(serialize_test_string("observe"));
    assert_eq!(
        recv_post_json(&mut worker).await,
        r#"{"closed":true,"inherited":0,"catches":0}"#
    );
    worker.terminate_and_join();
}

#[tokio::test]
async fn worker_rtp_script_interfaces_are_dedicated_only() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://rtp-transform.test".into(),
        "https://rtp-transform.test".into(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "rtp".into(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://rtp-transform.test/").unwrap(),
        },
    ] {
        let dedicated = matches!(&kind, super::super::WorkerGlobalKind::Dedicated { .. });
        let mut worker = spawn_test_worker_with_options(WorkerSpawnOptions::new(
            "console.log(JSON.stringify([typeof RTCRtpScriptTransformer,typeof RTCTransformEvent,typeof KeyFrameRequestEvent,'onrtctransform' in self,typeof RTCRtpScriptTransform]));".into(),
            "https://rtp-transform.test/exposure.js".into(),
        ).with_global_kind(kind));
        let WorkerToParentMessage::Console(console) =
            timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap()
        else {
            panic!("worker exposure probe must report its result");
        };
        let actual: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        let exposed = if dedicated { "function" } else { "undefined" };
        assert_eq!(
            actual,
            serde_json::json!([exposed, exposed, exposed, dedicated, "undefined"])
        );
        worker.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_rtp_keyframe_events_keep_domstrings_and_native_receivers() {
    ensure_v8();
    let mut worker = spawn_worker(
        r#"
        const facts=[];
        for(const [value,expected] of [[undefined,null],[null,'null'],['\ud800','\ud800'],['f','f']]) {
          const e=new KeyFrameRequestEvent('request',value);
          facts.push(e instanceof Event && e instanceof KeyFrameRequestEvent && e.type==='request' && e.rid===expected && !e.bubbles && !e.cancelable && !e.composed && !e.isTrusted);
        }
        const getter=Object.getOwnPropertyDescriptor(KeyFrameRequestEvent.prototype,'rid').get;
        for(const value of [{},Object.create(KeyFrameRequestEvent.prototype),new Proxy(new KeyFrameRequestEvent('x'),{})]) {
          try {getter.call(value);facts.push(false);} catch(e) {facts.push(e instanceof TypeError);}
        }
        for(const C of [RTCTransformEvent,RTCRtpScriptTransformer]) {
          try {new C();facts.push(false);} catch(e) {facts.push(e instanceof TypeError);}
        }
        let conversions=0;
        try {new KeyFrameRequestEvent({toString(){conversions++;throw 17;}});} catch(e) {facts.push(e===17&&conversions===1);}
        postMessage(facts);close();
        "#.into(),
        "https://rtp-transform.test/events.js".into(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&recv_post_json(&mut worker).await).unwrap(),
        serde_json::Value::Array(vec![serde_json::Value::Bool(true); 10])
    );
    worker.terminate_and_join();
}
