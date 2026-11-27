use super::*;

#[test]
fn focus_callbacks_can_redirect_or_cancel_a_transition() {
    for (listener, action, expected) in [
        (
            "a, 'blur'",
            "c.focus()",
            "a:blur:BODY:b,c:focus:c:-,c:focusin:c:-,a:focusout:c:-,active=c",
        ),
        (
            "a, 'focusout'",
            "c.focus()",
            "a:blur:BODY:b,a:focusout:BODY:b,c:focus:c:-,c:focusin:c:-,active=c",
        ),
        (
            "b, 'focus'",
            "c.focus()",
            concat!(
                "a:blur:BODY:b,a:focusout:BODY:b,b:focus:b:a,",
                "b:blur:BODY:c,b:focusout:BODY:c,c:focus:c:b,c:focusin:c:b,active=c",
            ),
        ),
        (
            "a, 'blur'",
            "b.focus()",
            "a:blur:BODY:b,b:focus:b:-,b:focusin:b:-,a:focusout:b:-,active=b",
        ),
        (
            "a, 'blur'",
            "c.focus(); c.blur()",
            concat!(
                "a:blur:BODY:b,c:focus:c:-,c:focusin:c:-,c:blur:BODY:-,c:focusout:BODY:-,",
                "a:focusout:BODY:b,b:focus:b:a,b:focusin:b:a,active=b",
            ),
        ),
        (
            "a, 'blur'",
            "c.focus(); a.addEventListener('focusout', () => c.blur())",
            concat!(
                "a:blur:BODY:b,c:focus:c:-,c:focusin:c:-,",
                "a:focusout:c:-,c:blur:BODY:-,c:focusout:BODY:-,active=BODY",
            ),
        ),
        (
            "a, 'blur'",
            "b.remove()",
            "a:blur:BODY:b,a:focusout:BODY:b,active=BODY",
        ),
        (
            "a, 'blur'",
            "b.disabled = true",
            "a:blur:BODY:b,a:focusout:BODY:b,active=BODY",
        ),
        (
            "a, 'blur'",
            "b.style.display = 'none'",
            "a:blur:BODY:b,a:focusout:BODY:b,active=BODY",
        ),
        (
            "a, 'focusout'",
            "b.style.visibility = 'hidden'",
            "a:blur:BODY:b,a:focusout:BODY:b,active=BODY",
        ),
        (
            "a, 'blur'",
            "b.parentElement.style.visibility = 'hidden'; b.style.visibility = 'visible'",
            "a:blur:BODY:b,a:focusout:BODY:b,b:focus:b:a,b:focusin:b:a,active=b",
        ),
        (
            "a, 'blur'",
            "b.parentElement.inert = true",
            "a:blur:BODY:b,a:focusout:BODY:b,active=BODY",
        ),
    ] {
        let mut vm = new_storage_html_test_vm("https://example.com/");
        let result = vm
            .eval(&format!(
                r#"(() => {{
  document.body.innerHTML = '<input id=a><button id=b>B</button><button id=c>C</button>';
  const [a, b, c] = document.body.children;
  const log = [];
  const active = () => document.activeElement.id || document.activeElement.tagName;
  a.focus();
  for (const element of [a, b, c]) {{
    for (const type of ['blur', 'focusout', 'focus', 'focusin']) {{
      element.addEventListener(type, event => log.push(
        [element.id, type, active(), event.relatedTarget?.id ?? '-'].join(':')
      ));
    }}
  }}
  const listen = (element, type) => element.addEventListener(type, () => {{ {action}; }});
  listen({listener});
  b.focus();
  log.push('active=' + active());
  return log.join(',');
}})()"#,
            ))
            .unwrap();
        assert_eq!(result, expected, "{listener}: {action}");
    }
}

#[test]
fn blur_callbacks_keep_focus_within_on_common_flat_tree_ancestors() {
    let mut vm = new_storage_html_test_vm("https://example.com/");
    let result = vm
        .eval(
            r#"(() => {
  document.body.innerHTML = `<div id=host>
    <button slot=from id=a>A</button><button slot=to id=b>B</button>
  </div>`;
  const host = document.getElementById('host');
  const [a, b] = host.children;
  const shadow = host.attachShadow({mode: 'open'});
  shadow.innerHTML = `
    <style>
      #common:focus-within { background-color: rgb(0, 128, 0) }
      #leaving:focus-within { background-color: rgb(255, 0, 0) }
    </style>
    <div id=common>
      <div id=leaving><slot name=from></slot></div><slot name=to></slot>
    </div>`;
  const common = shadow.getElementById('common');
  const leaving = shadow.getElementById('leaving');
  const log = [];
  const read = stage => log.push([
    stage, a.matches(':focus'), common.matches(':focus-within'),
    leaving.matches(':focus-within'), getComputedStyle(common).backgroundColor,
    getComputedStyle(leaving).backgroundColor,
  ].join(':'));

  a.focus({preventScroll: true});
  read('focused');
  a.addEventListener('blur', () => read('blur'));
  a.addEventListener('focusout', () => read('focusout'));
  b.focus({preventScroll: true});
  read('switched');

  a.focus({preventScroll: true});
  a.addEventListener('blur', () => { b.style.display = 'none'; }, {once: true});
  b.focus({preventScroll: true});
  read('canceled');
  return log.join('|');
})()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        concat!(
            "focused:true:true:true:rgb(0, 128, 0):rgb(255, 0, 0)|",
            "blur:false:true:false:rgb(0, 128, 0):rgba(0, 0, 0, 0)|",
            "focusout:false:true:false:rgb(0, 128, 0):rgba(0, 0, 0, 0)|",
            "switched:false:true:false:rgb(0, 128, 0):rgba(0, 0, 0, 0)|",
            "blur:false:true:false:rgb(0, 128, 0):rgba(0, 0, 0, 0)|",
            "focusout:false:true:false:rgb(0, 128, 0):rgba(0, 0, 0, 0)|",
            "canceled:false:false:false:rgba(0, 0, 0, 0):rgba(0, 0, 0, 0)"
        )
    );
}

#[test]
fn superseded_focus_request_does_not_scroll_the_target() {
    let mut vm = new_rendered_test_vm(
        "https://example.com/",
        r#"<!doctype html>
<button id="first">first</button>
<div id="scroller" style="width:100px;height:100px;overflow:auto">
  <div style="height:400px"></div>
  <button id="target">target</button>
</div>"#,
    );
    vm.eval(
        r#"
first.focus();
first.addEventListener('blur', () => target.focus({preventScroll: true}));
target.focus();
"#,
    )
    .unwrap();
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(document.activeElement === target && scroller.scrollTop === 0)")
            .unwrap(),
        "true"
    );
}

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
