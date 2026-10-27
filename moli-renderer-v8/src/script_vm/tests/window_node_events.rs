use super::window_node_worlds::child_node_world;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn isolated_node_events_project_receivers_targets_and_share_dispatch_state() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://node-events.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "node-events").await;
        vm.eval(
            r#"
            globalThis.defaultFacts=[];
            const target=frame.contentDocument.getElementById('childTarget');
            target.defaultOnly='default-node';
            target.addEventListener('node-world',function(e){
                defaultFacts.push([this===target,e===original,e.target===target,
                    e.currentTarget===target,e.isolatedOnly===undefined]);
            });'ready'
        "#,
        )
        .unwrap();
        vm.eval_in_isolated_context(child,r#"
            globalThis.node=document.getElementById('childTarget');
            globalThis.facts=[];
            node.addEventListener('node-world',function(e){
                globalThis.savedEvent=e;
                facts.push([this===node,this.defaultOnly===undefined,e.defaultOnly===undefined,
                    e instanceof CustomEvent,e.target===node,e.currentTarget===node,
                    e.composedPath()[0]===node,e.composedPath().at(-1)===window,
                    window.event===e,e.detail==='scalar-payload']);
                this.isolatedOnly='isolated-node';e.isolatedOnly='isolated-event';e.preventDefault();
            });
            const listener={handleEvent(e){facts.push([this===listener,e===savedEvent,
                e.defaultPrevented,e.currentTarget===node]);}};
            node.addEventListener('node-world',listener);'ready'
        "#).unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(vm.eval(r#"
            globalThis.original=new CustomEvent('node-world',{bubbles:true,cancelable:true,detail:'scalar-payload'});
            original.defaultOnly='default-event';
            JSON.stringify([frame.contentDocument.getElementById('childTarget').dispatchEvent(original),
                original.defaultPrevented,original.currentTarget===null,original.composedPath().length===0,
                original.isolatedOnly===undefined,frame.contentDocument.getElementById('childTarget').isolatedOnly===undefined])
        "#).unwrap(),"[false,true,true,true,true,true]","{policy:?}");
        assert_eq!(
            vm.eval("JSON.stringify(defaultFacts)").unwrap(),
            "[[true,true,true,true,true]]",
            "{policy:?}"
        );
        assert_eq!(
            vm.eval_in_isolated_context(child, "JSON.stringify(facts)")
                .unwrap(),
            "[[true,true,true,true,true,true,true,true,true,true],[true,true,true,true]]",
            "{policy:?}"
        );
        assert_eq!(vm.eval_in_isolated_context(child,"JSON.stringify([savedEvent.currentTarget===null,savedEvent.composedPath().length===0,savedEvent.defaultPrevented])").unwrap(),"[true,true,true]");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "projection must not request layout"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn adopted_node_events_use_current_document_window_in_the_listener_world() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://adopted-node-events.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "adopted-node-events").await;
        vm.eval_in_isolated_context(
            child,
            r#"
            globalThis.adopted=parent.document.createElement('p');
            document.adoptNode(adopted);document.body.append(adopted);
            globalThis.facts=[];
            adopted.addEventListener('adopt-world',function(e){facts.push([
                this===adopted,this instanceof parent.HTMLParagraphElement,
                !(this instanceof HTMLParagraphElement),e instanceof CustomEvent,
                !(e instanceof parent.CustomEvent),e.target===adopted,
                e.currentTarget===adopted,e.target.ownerDocument===document,
                e.composedPath()[0]===adopted,e.composedPath().at(-1)===window]);});'ready'
        "#,
        )
        .unwrap();
        assert_eq!(vm.eval("String(frame.contentDocument.querySelector('p').dispatchEvent(new CustomEvent('adopt-world',{bubbles:true})))").unwrap(),"true");
        assert_eq!(
            vm.eval_in_isolated_context(child, "JSON.stringify(facts)")
                .unwrap(),
            "[[true,true,true,true,true,true,true,true,true,true]]",
            "{policy:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn inert_document_factories_preserve_native_execution_context_for_node_events() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://inert-node-events.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "inert-node-events").await;
        vm.eval(
            r#"
            globalThis.injectFacts=[];
            addEventListener('factory-inject',e=>{
                const node=e.relatedTarget;
                const event=new CustomEvent('factory-node');event.defaultOnly='default';
                injectFacts.push([node instanceof Node,node.dispatchEvent(event)]);
            });'ready'
        "#,
        )
        .unwrap();
        let facts=vm.eval_in_isolated_context(child,r#"(()=>{
            const childParser=new DOMParser(),parentParser=new parent.DOMParser();
            const childImpl=document.implementation,parentImpl=parent.document.implementation;
            const factories=[
                [true,true,()=>childParser.parseFromString('<p/>','text/html')],
                [false,false,()=>parentParser.parseFromString('<p/>','text/html')],
                [true,true,()=>childImpl.createHTMLDocument('child')],
                [false,false,()=>parentImpl.createHTMLDocument('parent')],
                [true,true,()=>parentImpl.createHTMLDocument.call(childImpl,'borrowed')],
                [true,true,()=>parentParser.parseFromString.call(childParser,'<p/>','text/html')],
                [true,true,()=>document.cloneNode(false)],
                [true,true,()=>parent.Node.prototype.cloneNode.call(document,false)],
                [true,true,()=>new Document()],[false,false,()=>new parent.Document()],
                [true,true,()=>Reflect.construct(DOMParser,[],parent.DOMParser).parseFromString('<p/>','text/html')],
                [false,false,()=>Reflect.construct(parent.DOMParser,[],DOMParser).parseFromString('<p/>','text/html')],
                [false,true,()=>Reflect.construct(Document,[],parent.Document)],
                [true,false,()=>Reflect.construct(parent.Document,[],Document)]];
            const facts=[];
            for(const [wrapperChild,eventChild,factory] of factories){
                const doc=factory(),node=doc.createElement('button');
                const row=[doc.defaultView===null,wrapperChild===(doc instanceof Document),wrapperChild!== (doc instanceof parent.Document)];
                node.addEventListener('factory-node',function(e){row.push(this===node,e.target===node,
                    eventChild===(e instanceof CustomEvent),eventChild!==(e instanceof parent.CustomEvent),e.defaultOnly===undefined);});
                parent.dispatchEvent(new FocusEvent('factory-inject',{relatedTarget:node}));facts.push(row);
            }
            return JSON.stringify(facts);
        })()"#).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
            serde_json::to_value(vec![vec![true; 8]; 14]).unwrap(),
            "{policy:?}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(injectFacts)").unwrap(),
            "[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn retired_frame_nodes_and_inert_factories_distinguish_execution_context_presence() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://retired-node-events.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "retired-node-events").await;
        vm.eval_in_isolated_context(
            child,
            r#"
            parent.savedFactories={parser:new DOMParser(),implementation:document.implementation,
                sourceDocument:document,sourceNode:document.createElement('input'),
                inert:document.implementation.createHTMLDocument('before'),constructor:Document};
            parent.savedFactories.sourceNode.type='checkbox';'ready'
        "#,
        )
        .unwrap();
        let parent = vm
            .page_isolated_world_contexts
            .contexts_with_ids()
            .find_map(|(id, world)| (!world.explicit && world.child_handle.is_none()).then_some(id))
            .unwrap();
        vm.eval("frame.srcdoc='<body>replacement</body>'; 'navigate'")
            .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "retire Node source Document")
            .await;
        let facts=vm.eval_in_isolated_context(parent,r#"(()=>{
            const old=savedFactories.sourceNode;
            let invalid;try{old.dispatchEvent({});}catch(e){invalid=e.name;}
            let nested;document.addEventListener('nested',e=>{try{old.dispatchEvent(e);}catch(error){nested=error.name;}},{once:true});
            document.dispatchEvent(new Event('nested'));
            const documents=[savedFactories.sourceDocument,savedFactories.inert,
                savedFactories.parser.parseFromString('<p/>','text/html'),
                savedFactories.implementation.createHTMLDocument('after'),
                savedFactories.inert.implementation.createHTMLDocument('again'),
                savedFactories.inert.cloneNode(false),new savedFactories.constructor()];
            const dispatches=documents.map(doc=>{let called=0;const node=doc.createElement('button');
                node.addEventListener('after',()=>called++);const e=new Event('after');
                const result=node.dispatchEvent(e);return [result,called,e.currentTarget===null];});
            const click=old.dispatchEvent(new MouseEvent('click',{bubbles:true}));
            return JSON.stringify({invalid,nested,dispatches,click,checked:old.checked,
                cloneNull:savedFactories.sourceDocument.cloneNode(false)===null});
        })()"#).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
            serde_json::json!({
                "invalid":"TypeError","nested":"InvalidStateError",
                "dispatches":[[false,0,true],[true,0,true],[true,0,true],[false,0,true],
                    [true,0,true],[true,0,true],[false,0,true]],
                "click":false,"checked":false,"cloneNull":true
            }),
            "{policy:?}"
        );
    }
}

#[test]
fn inert_node_execution_context_survives_creation_realm_and_wrapper_cache_retirement() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://inert-node-realm-retirement.test/");
        vm.set_layout_policy(policy);
        let isolated = vm
            .create_isolated_world("inert-source-realm", false)
            .unwrap();
        vm.eval_in_isolated_context(isolated,
            "globalThis.node=document.implementation.createHTMLDocument('inert').createElement('button');'ready'").unwrap();
        let context_ptr = &vm
            .page_isolated_world_contexts
            .context(isolated)
            .unwrap()
            .context as *const _;
        let handle = vm
            .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
                let global = scope.get_current_context().global(scope);
                let node = global
                    .get(scope, crate::util::v8str(scope, "node").into())
                    .unwrap();
                let node = v8::Local::<v8::Object>::try_from(node).unwrap();
                Ok(
                    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                        scope, node,
                    )?
                    .1,
                )
            })
            .unwrap();
        vm.destroy_isolated_world_context(isolated);
        vm.collect_renderer_document_isolate_garbage().unwrap();
        assert_eq!(
            vm._context_host
                .borrow()
                .native_bridge()
                .isolated_world_wrapper_cache_state_for_test(),
            None
        );
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let node = crate::native_bridge::document::detached_native_object_for_handle(
                scope, host_ptr, handle,
            )
            .unwrap();
            let global = scope.get_current_context().global(scope);
            assert_eq!(
                global.set(
                    scope,
                    crate::util::v8str(scope, "retainedNode").into(),
                    node.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(vm.eval(r#"(()=>{
            const facts=[];const event=new Event('after-realm');
            retainedNode.addEventListener('after-realm',function(e){facts.push(this===retainedNode,e===event,e.target===retainedNode);});
            return JSON.stringify([retainedNode.dispatchEvent(event),facts]);
        })()"#).unwrap(),"[true,[true,true,true]]","the source Window remains live: {policy:?}");
        let host = vm.context_host_weak_for_test();
        let isolate = vm.renderer_document_isolate.clone();
        drop(vm);
        for _ in 0..2 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            host.upgrade().is_none(),
            "native Document lineage must not root the Host"
        );
    }
}

#[test]
fn popup_document_event_authority_follows_its_window_instead_of_parser_provenance() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://popup-node-events.test/");
        vm.set_layout_policy(policy);
        let facts=vm.eval(r#"(()=>{
            const popup=window.open('about:blank');
            const oldDocument=popup.document,node=oldDocument.createElement('input');node.type='checkbox';
            let called=0;node.addEventListener('click',()=>called++);
            const before=node.dispatchEvent(new MouseEvent('click'));
            popup.close();
            const after=node.dispatchEvent(new MouseEvent('click'));
            return JSON.stringify([before,after,called,node.checked,
                oldDocument.cloneNode(false)===null,popup.closed]);
        })()"#).unwrap();
        assert_eq!(facts, "[true,false,1,true,true,true]", "{policy:?}");
    }
}
