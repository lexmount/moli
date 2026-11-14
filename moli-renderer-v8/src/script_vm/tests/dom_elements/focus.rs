use super::*;

#[test]
fn editing_focus_initializes_selection_before_events_and_preserves_existing_ranges() {
    let mut vm = new_storage_html_test_vm("https://example.com/");
    let result = vm.eval(r#"(() => {
        document.body.innerHTML='<div id=a contenteditable><span>abcd</span></div><div id=b contenteditable>wxyz</div>';
        const a=document.getElementById('a'),b=document.getElementById('b'),s=getSelection();
        const log=[];
        for(const node of [a,b])node.addEventListener('focus',()=>{
            log.push([node.id,s.anchorNode===node.querySelector('span')?.firstChild || s.anchorNode===node.firstChild,s.anchorOffset,s.focusOffset,s.toString()]);
        });
        a.focus();s.collapse(a.firstChild.firstChild,2);b.focus();a.focus();
        s.setBaseAndExtent(b.firstChild,1,b.firstChild,3);b.focus();
        a.addEventListener('focus',()=>s.collapse(a.firstChild.firstChild,3),{once:true});
        a.focus();
        const selectedByAuthor=s.anchorNode===a.firstChild.firstChild && s.anchorOffset===3;
        return JSON.stringify({log,selectedByAuthor});
    })()"#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log":[["a",true,0,0,""],["b",true,0,0,""],["a",true,0,0,""],
                ["b",true,1,3,"xy"],["a",true,0,0,""]],"selectedByAuthor":true,
        })
    );
}

#[test]
fn editing_focus_uses_native_selection_boundaries_without_author_accessors() {
    let mut vm = new_storage_html_test_vm("https://example.com/");
    assert_eq!(vm.eval(r#"(() => {
        document.body.innerHTML='<div id=editor contenteditable>text</div>';
        const e=document.getElementById('editor'),text=e.firstChild,s=getSelection();
        const anchor=Object.getOwnPropertyDescriptor(Selection.prototype,'anchorNode').get;
        let calls=0;const fail=()=>{calls++;throw new Error('author getter');};
        for(const [node,key] of [[e,'ownerDocument'],[text,'ownerDocument'],[text,'nodeType'],[s,'anchorNode'],[s,'focusNode']])
            Object.defineProperty(node,key,{configurable:true,get:fail});
        window.getSelection=fail;window.Range=fail;s.collapse=fail;e.focus();
        return String(calls===0 && anchor.call(s)===text);
    })()"#).unwrap(),"true");
}

#[test]
fn editing_focus_skips_hidden_text_and_empty_markup_but_stops_before_noneditable_content() {
    for (prefix, at_host) in [
        ("<span style='display:none'>hidden</span>", false),
        ("<span style='visibility:hidden'>hidden</span>", false),
        (
            "<span style='display:block;content-visibility:hidden'>hidden</span>",
            false,
        ),
        ("<span inert>hidden</span>", true),
        ("<span></span>", false),
        ("<!--comment-->", false),
        ("<span contenteditable=false>skip</span>", true),
        ("<br>", true),
    ] {
        let mut vm = new_rendered_test_vm(
            "https://example.com/",
            &format!("<!doctype html><div id=editor contenteditable>{prefix}visible</div>"),
        );
        assert_eq!(vm.eval(&format!(r#"(() => {{
            const e=document.getElementById('editor');e.focus();const s=getSelection();
            return String(s.anchorNode===({at_host}?e:e.lastChild) && s.anchorOffset===0 && s.isCollapsed);
        }})()"#)).unwrap(), "true", "{prefix}");
    }
}

#[test]
fn focus_prevent_scroll_controls_real_nested_scroll_container_reveal() {
    let mut vm = new_storage_test_vm("https://focus-prevent-scroll.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = `
    <button id="first">first</button>
    <div id="scroller" style="width:100px;height:100px;overflow:auto">
      <div style="width:500px;height:400px"></div>
      <button id="target" style="margin-left:400px">target</button>
    </div>`;
  return 'installed';
})()
"#,
    )
    .expect("focus scroll fixture should initialize");
    refresh_layout_for_test(&mut vm);

    vm.eval(
        r#"
(() => {
  const first = document.getElementById('first');
  const scroller = document.getElementById('scroller');
  const target = document.getElementById('target');

  target.focus({ preventScroll: true });
  const prevented = scroller.scrollLeft === 0 && scroller.scrollTop === 0;
  const focused = document.activeElement === target;
  first.focus();
  target.focus();
  window.beforeReveal = [prevented, focused];
})()
"#,
    )
    .expect("focus preventScroll probe should evaluate");

    publish_layout_for_test(&mut vm);
    let result = vm
        .eval("beforeReveal.concat(scroller.scrollLeft > 0, scroller.scrollTop > 0).join('|')")
        .unwrap();
    assert_eq!(result, "true|true|true|true");
}

#[test]
fn focusing_contenteditable_in_child_frame_reveals_authored_frame_position() {
    let mut vm = new_storage_test_vm("https://focus-scroll.test/");

    vm.eval(
        r#"
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    iframe { position: absolute; left: 250vw; }
    .spacer { width: 100vw; height: 250vh; }
  `;
  head.appendChild(style);

  const first = document.createElement('div');
  first.contentEditable = 'true';
  const spacer = document.createElement('div');
  spacer.className = 'spacer';
  const frame = document.createElement('iframe');
  body.append(first, spacer, frame);

  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write('<div id="target" contenteditable="true">target</div>');
  childDocument.close();
  const target = childDocument.getElementById('target');
"#,
    )
    .expect("prepare child focus fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"(() => {
  first.focus();
  target.focus();
  const firstX = window.scrollX;
  const firstY = window.scrollY;

  window.scroll(0, 0);
  first.focus();
  target.focus();
  return JSON.stringify({
    beyondViewport: firstX > window.innerWidth && firstY > window.innerHeight,
    repeated: firstX === window.scrollX && firstY === window.scrollY,
    parentRetargeted: document.activeElement === frame,
    childFocused: childDocument.activeElement === target
  });
})()"#,
        )
        .expect("child contenteditable focus scroll probe should evaluate");

    assert_eq!(
        result,
        r#"{"beyondViewport":true,"repeated":true,"parentRetargeted":true,"childFocused":true}"#
    );
}

#[test]
fn focusing_frame_owner_then_input_dispatches_child_window_focus_and_blur() {
    let mut vm = new_storage_test_vm("https://focus-frame-window.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const input = document.createElement('input');
  const frame = document.createElement('iframe');
  body.append(input, frame);
  const log = [];
  window.onblur = () => log.push('top-window-blur');
  window.onfocus = () => log.push('top-window-focus');
  frame.onfocus = () => log.push('frame-focus');
  frame.onblur = () => log.push('frame-blur');
  frame.contentWindow.onfocus = () => log.push('child-window-focus');
  frame.contentWindow.onblur = () => log.push('child-window-blur');
  input.onfocus = () => log.push('input-focus');

  frame.focus();
  input.focus();
  return `${log.join(',')}|${document.activeElement === input}`;
})()
"#,
        )
        .expect("frame window focus transition probe should evaluate");

    assert_eq!(
        result,
        "top-window-blur,frame-focus,child-window-focus,frame-blur,child-window-blur,input-focus,top-window-focus|true"
    );
}
