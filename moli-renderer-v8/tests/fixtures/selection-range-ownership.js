(() => {
  const failures = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const frame = body.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const make = doc => {
    const box = doc.createElement('section');
    box.innerHTML = '<span>before</span><p><b>middle</b><i>tail</i></p><span>after</span><div></div>';
    const root = box.lastChild.attachShadow({mode:'closed'});
    root.innerHTML = '<p><b>shadow</b></p>';
    return {box, node:box.children[1], root, shadow:root.firstChild};
  };
  const a=make(document), b=make(other.document), detached=make(document);
  body.appendChild(a.box); other.document.body.appendChild(b.box);
  document.createDocumentFragment().appendChild(detached.box);
  const windowless=document.implementation.createHTMLDocument('');
  const inert=make(windowless); windowless.body.appendChild(inert.box);
  const methods=['selectNode','selectNodeContents','setEnd','setEndAfter','setEndBefore','setStart','setStartAfter','setStartBefore'];
  const throws = (fn, name) => { try { fn(); return false; } catch (error) { return error.name === name; } };
  const empty = (selection, label) => {
    check(selection.rangeCount===0 && selection.type==='None' && selection.direction==='none' && selection.isCollapsed, label+': empty state');
    check(selection.anchorNode===null && selection.anchorOffset===0 && selection.focusNode===null && selection.focusOffset===0, label+': empty endpoints');
    check(selection.getComposedRanges().length===0, label+': empty composed ranges');
    check(throws(()=>selection.getRangeAt(0),'IndexSizeError'), label+': getRangeAt rejects');
  };
  for (const owner of [window,other]) for (const caller of [window,other]) for (const creator of [window,other]) {
    const own=owner===window?a:b, foreign=owner===window?b:a;
    const selection=owner.getSelection(), unrelated=(owner===window?other:window).getSelection();
    for (const method of methods) for (const kind of ['same','shadow','foreign','foreign-shadow','fragment','windowless','detached-shadow']) {
      const target=({same:own.node,shadow:own.shadow,foreign:foreign.node,'foreign-shadow':foreign.shadow,fragment:detached.node,windowless:inert.node,'detached-shadow':detached.shadow})[kind];
      const label=[owner===window?'parent':'child',caller===window?'parent':'child',creator===window?'parent':'child',method,kind].join(':');
      selection.removeAllRanges(); unrelated.removeAllRanges();
      const range=new creator.Range(); range.selectNodeContents(own.box); selection.addRange(range);
      unrelated.selectAllChildren(foreign.node);
      const retained=unrelated.getRangeAt(0);
      caller.Range.prototype[method].call(range,target,0);
      const belongs=kind==='same'||kind==='shadow';
      if (belongs) {
        check(selection.type==='Range' && selection.direction==='forward', label+': composed range state');
        check(selection.rangeCount===1 && selection.getRangeAt(0)===range, label+': retain associated range');
        check(selection.anchorNode===range.startContainer && selection.anchorOffset===range.startOffset &&
          selection.focusNode===range.endContainer && selection.focusOffset===range.endOffset, label+': sync projected endpoints');
      } else empty(selection,label);
      check(range.startContainer.getRootNode()===target.getRootNode() && range.endContainer.getRootNode()===target.getRootNode(), label+': native range remains usable');
      caller.Range.prototype.selectNodeContents.call(range,own.box);
      if (!belongs) empty(selection,label+': return does not reattach');
      selection.addRange(range);
      check(selection.getRangeAt(0)===range && selection.anchorNode===own.box && selection.anchorOffset===0 &&
        selection.focusNode===own.box && selection.focusOffset===4, label+': explicit addRange');
      check(unrelated.getRangeAt(0)===retained && unrelated.anchorNode===foreign.node && unrelated.anchorOffset===0 &&
        unrelated.focusNode===foreign.node && unrelated.focusOffset===2, label+': unrelated selection retained');
    }
    for (const method of methods) {
      selection.selectAllChildren(own.node);
      const range=selection.getRangeAt(0), clone=range.cloneRange();
      caller.Range.prototype[method].call(clone,foreign.node,0);
      check(selection.getRangeAt(0)===range && selection.anchorNode===own.node && selection.focusOffset===2, method+': unassociated clone');
      selection.selectAllChildren(own.box);
      const replacement=selection.getRangeAt(0);
      caller.Range.prototype[method].call(range,foreign.node,0);
      check(selection.getRangeAt(0)===replacement && selection.anchorNode===own.box && selection.focusOffset===4, method+': replaced range');
    }
    // DOM boundary validation must finish before changing the associated Range.
    // A foreign node with an invalid offset must preserve the existing selection.
    for (const method of methods) {
      const label=[owner===window?'parent':'child',caller===window?'parent':'child',creator===window?'parent':'child',method].join(':');
      selection.selectAllChildren(own.node);
      const range=selection.getRangeAt(0);
      const target=method==='selectNodeContents'?owner.document.implementation.createDocumentType('html','',''):
        method==='setStart'||method==='setEnd'?foreign.node:owner.document.createElement('div');
      const error=method==='setStart'||method==='setEnd'?'IndexSizeError':'InvalidNodeTypeError';
      check(throws(()=>caller.Range.prototype[method].call(range,target,99),error), label+': invalid boundary');
      check(selection.rangeCount===1 && selection.getRangeAt(0)===range && selection.anchorNode===own.node && selection.anchorOffset===0 &&
        selection.focusNode===own.node && selection.focusOffset===2, label+': error preserves selection');
    }
  }
  // Neither Range association nor Document membership may invoke author getters.
  const selected=other.getSelection();
  let getterCalls=0;
  const poisoned = [[other.document,'defaultView'],[b.node,'ownerDocument'],[b.node,'isConnected']];
  for (const [object,key] of poisoned) Object.defineProperty(object,key,{configurable:true,get(){getterCalls++;throw Error(key);}});
  for (const method of methods) {
    selected.selectAllChildren(b.box);
    const range=selected.getRangeAt(0);
    Range.prototype[method].call(range,b.node,0);
    check(selected.getRangeAt(0)===range, method+': native ownership');
  }
  for (const [object,key] of poisoned) delete object[key];
  check(getterCalls===0,'ownership checks must not run author getters');
  a.box.remove(); frame.remove();
  return failures.join('\n');
})()
