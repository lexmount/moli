use super::*;

#[tokio::test]
async fn dom_string_attribute_reflection_preserves_utf16_across_document_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://dom-string-reflection.test/",
        &loader,
    );
    vm.eval(
        r#"
      if (!document.documentElement) document.appendChild(document.createElement('html'));
      if (!document.body) document.documentElement.appendChild(document.createElement('body'));
      const frame=document.body.appendChild(document.createElement('iframe'));
      frame.id='child'; frame.srcdoc='<body></body>'; void frame.contentWindow;
      'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval(include_str!("dom_string_reflection.js")).unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap()
    );
}

#[test]
fn area_hyperlink_metadata_reflection_preserves_reaction_and_adoption_order() {
    let mut vm = new_storage_html_test_vm("https://area-hyperlink-metadata.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
      const assert=(condition,message)=>{if(!condition)throw new Error(message);};
      const changes=[];
      class MetadataArea extends HTMLAreaElement {
        static observedAttributes=['hreflang','type'];
        attributeChangedCallback(name,oldValue,value,namespace) {
          changes.push([this,name,oldValue,value,namespace,this.ownerDocument]);
          if(this.reenter && name==='hreflang') {
            this.reenter=false;
            this.type='inner-\uD800';
          }
        }
      }
      customElements.define('metadata-area',MetadataArea,{extends:'area'});
      const documents=[document,document.implementation.createHTMLDocument(''),
        new DOMParser().parseFromString('<!doctype html><body>','text/html')];
      for(const target of documents) for(const property of ['hreflang','type']) {
        const node=new MetadataArea(),value='metadata-\uD800\0\uDC01';
        const descriptor=Object.getOwnPropertyDescriptor(HTMLAreaElement.prototype,property);
        let conversions=0;changes.length=0;
        descriptor.set.call(node,{toString(){
          conversions++;
          assert(changes.length===0,'no attribute reaction before conversion');
          target.adoptNode(node);
          return value;
        }});
        assert(conversions===1,'convert once');
        assert(node.ownerDocument===target,'owner after conversion adoption');
        assert(descriptor.get.call(node)===value && node.getAttribute(property)===value,
          'native UTF16 attribute after adoption');
        assert(!Object.prototype.hasOwnProperty.call(node,property),'inherited accessor after set');
        assert(changes.length===1,'reaction before setter returns');
        const [receiver,name,oldValue,newValue,namespace,owner]=changes[0];
        assert(receiver===node && name===property && oldValue===null && newValue===value &&
          namespace===null && owner===target,'reaction observes adopted owner and raw value');
      }
      const node=new MetadataArea();changes.length=0;node.reenter=true;
      node.hreflang='outer-\uDC01';
      assert(changes.length===2 && changes[0][1]==='hreflang' && changes[1][1]==='type',
        'nested attribute reaction order');
      assert(node.hreflang==='outer-\uDC01' && node.type==='inner-\uD800',
        'reentrant setters retain independent native UTF16 attributes');
      return 'ok';
    })()"#
        )
        .unwrap(),
        "ok"
    );
}

#[test]
fn form_data_converts_reflected_utf16_names_to_scalar_strings() {
    let mut vm = new_storage_html_test_vm("https://form-reflected-utf16.test/");
    assert_eq!(vm.eval(r#"(() => {
      const form=document.body.appendChild(document.createElement('form'));
      for (const tag of ['input','textarea','select']) {
        const control=form.appendChild(document.createElement(tag));
        const name=tag+'-\uDC01',value='value-\uFFFD';
        control.name=name;
        if (tag==='select') control.appendChild(new Option(value,value));
        else control.value=value;
        if (control.name!==name || control.value!==value || control.getAttribute('name')!==name) return false;
      }
      const entries=Array.from(new FormData(form));
      return entries.length===3 && entries.every(([name,value],index)=>
        name===['input','textarea','select'][index]+'-\uFFFD' && value==='value-\uFFFD');
    })()"#).unwrap(), "true");
}
