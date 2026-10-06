use super::*;

#[test]
fn svg_value_lists_preserve_items_and_validate_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-value-lists.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_value_lists.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "6864");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed).slice(0, 20))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "6864");
}

#[test]
fn svg_value_list_registered_proxies_share_native_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-value-list-proxies.test/");
    vm.eval(r#"
      document.body.innerHTML='<iframe></iframe>';
      const child=document.querySelector('iframe').contentWindow;
      const detached=child.document.implementation.createHTMLDocument('');
      const ns='http://www.w3.org/2000/svg', root=detached.createElementNS(ns,'svg');
      const entries={}, proxies={};
      for(const [kind,tag,attr] of [['Length','text','x'],['Number','text','rotate'],['Point','polygon','points']]){
        const owner=detached.createElementNS(ns,tag);owner.setAttribute(attr,'10 20 30 40');
        const animated=kind==='Point'?null:owner[attr];
        const base=animated?animated.baseVal:owner.points, anim=animated?animated.animVal:owner.animatedPoints;
        entries[kind+'Base']=base;entries[kind+'Anim']=anim;entries[kind+'Item']=base[0];entries[kind+'Readonly']=anim[0];
        if(animated)entries[kind+'Animated']=animated;
      }
      const individual=detached.createElementNS(ns,'rect');individual.setAttribute('x','9px');
      entries.individual=individual.x.baseVal;
      globalThis.__valueListEntries=entries;globalThis.__valueListProxies=proxies;
    "#).unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let entries_key = crate::util::v8str(scope, "__valueListEntries");
        let entries = global.get(scope, entries_key.into()).unwrap();
        let entries = v8::Local::<v8::Object>::try_from(entries).unwrap();
        let proxies_key = crate::util::v8str(scope, "__valueListProxies");
        let proxies = global.get(scope, proxies_key.into()).unwrap();
        let proxies = v8::Local::<v8::Object>::try_from(proxies).unwrap();
        for name in [
            "LengthBase",
            "LengthAnim",
            "LengthItem",
            "LengthReadonly",
            "LengthAnimated",
            "NumberBase",
            "NumberAnim",
            "NumberItem",
            "NumberReadonly",
            "NumberAnimated",
            "PointBase",
            "PointAnim",
            "PointItem",
            "PointReadonly",
            "individual",
        ] {
            let key = crate::util::v8str(scope, name);
            let target = entries.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                proxies.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const p=__valueListProxies, t=__valueListEntries;
      for(const kind of ['Length','Number','Point']){
        const base=p[kind+'Base'], anim=p[kind+'Anim'], item=p[kind+'Item'], readonly=p[kind+'Readonly'];
        const prop=kind==='Point'?'x':kind==='Length'?'valueInSpecifiedUnits':'value';
        if(base[0]!==t[kind+'Item'])return kind+' index';
        if(kind!=='Point'&&(p[kind+'Animated'].baseVal!==t[kind+'Base']||p[kind+'Animated'].animVal!==t[kind+'Anim']))return kind+' animated';
        item[prop]=7;
        if(t[kind+'Item'][prop]!==7||anim[0][prop]!==7)return kind+' value';
        const copy=base.appendItem(item);
        if(copy===t[kind+'Item']||Object.getPrototypeOf(copy)!==child[kind==='Point'?'DOMPoint':'SVG'+kind].prototype)return kind+' copy';
        item[prop]=8;
        if(copy[prop]!==7||base.length!==(kind==='Point'?3:5))return kind+' isolation';
        if(kind==='Point'&&(item.toJSON().x!==8||item.matrixTransform().x!==8))return 'Point methods';
        if(kind==='Length'){
          item.newValueSpecifiedUnits(5,.1);
          if(item.valueInSpecifiedUnits!==Math.fround(.1))return 'Length units';
        }
        let rejected=0,conversions=0,traps=0;
        const index={valueOf(){conversions++;return 0;}};
        for(const receiver of [new Proxy(base,{get(){traps++;throw Error('trap');}}),Object.create(base)]){
          try{child['SVG'+kind+'List'].prototype.removeItem.call(receiver,index);}
          catch(e){if(e instanceof child.TypeError)rejected++;}
        }
        try{base.insertItemBefore(new Proxy(item,{}),index);}
        catch(e){if(e instanceof child.TypeError)rejected++;}
        try{readonly[prop]=9;}
        catch(e){if(e instanceof child.DOMException&&e.name==='NoModificationAllowedError')rejected++;}
        if(rejected!==4||conversions!==0||traps!==0)return JSON.stringify({kind,rejected,conversions,traps});
      }
      const clone=p.LengthBase.appendItem(p.individual);
      p.individual.value=6;
      return clone!==t.individual&&clone.value===9&&individual.x.baseVal.value===6?'ok':'individual attribute copy';
    })()"#).unwrap(),"ok");
}
