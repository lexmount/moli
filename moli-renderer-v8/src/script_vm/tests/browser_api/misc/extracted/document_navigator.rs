use super::*;

#[test]
fn dom_point_accessors_use_private_slots_and_reject_forged_receivers() {
    let mut vm = new_storage_test_vm("https://dompoint-internal-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const stringify = value => value === undefined ? "undefined" : String(value);
              const internalNames = object => Object.getOwnPropertyNames(object)
                .filter(name => name.startsWith("__moliDomPoint"))
                .sort()
                .join(",");
              const descriptorShape = (prototype, receiver, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor.get,
                  descriptor.get && descriptor.get.name,
                  descriptor.get && descriptor.get.length,
                  typeof descriptor.set,
                  descriptor.set && descriptor.set.name,
                  descriptor.set && descriptor.set.length,
                  descriptor.enumerable,
                  descriptor.configurable,
                  Object.prototype.hasOwnProperty.call(receiver, name)
                ].map(stringify).join(":");
              };
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return error.constructor.name;
                }
              };
              const source = new DOMPoint(7, 8, 9, 10);
              const initialOwnSlots = internalNames(source);
              DOMPoint.prototype.__moliDomPointBrand = true;
              DOMPoint.prototype.__moliDomPointX = -100;
              DOMPoint.prototype.__moliDomPointY = -100;
              DOMPoint.prototype.__moliDomPointZ = -100;
              DOMPoint.prototype.__moliDomPointW = -100;
              const before = [
                source.x,
                source.y,
                source.z,
                source.w,
                JSON.stringify(source.toJSON())
              ].join(":");
              source.x = 11;
              source.y = 12;
              source.z = 13;
              source.w = 14;
              const afterSetter = [
                source.x,
                source.y,
                source.z,
                source.w,
                JSON.stringify(source.toJSON()),
                internalNames(source)
              ].join(":");
              source.__moliDomPointBrand = true;
              source.__moliDomPointX = -200;
              source.__moliDomPointY = -200;
              source.__moliDomPointZ = -200;
              source.__moliDomPointW = -200;
              const afterSpoof = [
                source.x,
                source.y,
                source.z,
                source.w,
                JSON.stringify(source.toJSON()),
                internalNames(source)
              ].join(":");
              const receiver = Object.create(source);
              const descriptor = Object.getOwnPropertyDescriptor(DOMPoint.prototype, "x");
              const fake = Object.assign(Object.create(DOMPoint.prototype), {
                __moliDomPointBrand: true,
                __moliDomPointX: 1,
                __moliDomPointY: 2,
                __moliDomPointZ: 3,
                __moliDomPointW: 4
              });
              return JSON.stringify({
                initialOwnSlots,
                before,
                afterSetter,
                afterSpoof,
                fakeResults: [
                  probe(() => descriptor.get.call(receiver)),
                  probe(() => descriptor.set.call(receiver, 5)),
                  probe(() => DOMPoint.prototype.toJSON.call(receiver)),
                  probe(() => descriptor.get.call(fake)),
                  probe(() => descriptor.set.call(fake, 5)),
                  probe(() => DOMPoint.prototype.toJSON.call(fake))
                ].join(","),
                fakeSlots: internalNames(fake),
                descriptors: ["x", "y", "z", "w"]
                  .map(name => descriptorShape(DOMPoint.prototype, source, name))
                  .join(";")
              });
            })()
            "#,
        )
        .expect("DOMPoint inherited-slot probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialOwnSlots":"","before":"7:8:9:10:{\"x\":7,\"y\":8,\"z\":9,\"w\":10}","afterSetter":"11:12:13:14:{\"x\":11,\"y\":12,\"z\":13,\"w\":14}:","afterSpoof":"11:12:13:14:{\"x\":11,\"y\":12,\"z\":13,\"w\":14}:__moliDomPointBrand,__moliDomPointW,__moliDomPointX,__moliDomPointY,__moliDomPointZ","fakeResults":"TypeError,TypeError,TypeError,TypeError,TypeError,TypeError","fakeSlots":"__moliDomPointBrand,__moliDomPointW,__moliDomPointX,__moliDomPointY,__moliDomPointZ","descriptors":"x:function:get x:0:function:set x:1:true:true:false;y:function:get y:0:function:set y:1:true:true:false;z:function:get z:0:function:set z:1:true:true:false;w:function:get w:0:function:set w:1:true:true:false"}"#
    );
}
#[test]
fn stale_popover_element_matches_false_after_document_open_replacement() {
    let mut vm = new_storage_test_vm("https://popover-document-open.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const popover1 = document.createElement("div");
              popover1.id = "popover1";
              popover1.setAttribute("popover", "");
              body.append(popover1);
              popover1.showPopover();
              const before = popover1.matches(":popover-open");
              document.open();
              document.write("<!doctype html><div popover id='popover2'>Popover</div>");
              document.close();
              const popover2 = document.querySelector("#popover2");
              let invalidSelector;
              try {
                popover1.matches("[");
                invalidSelector = "no-throw";
              } catch (error) {
                invalidSelector = error.name;
              }
              popover2.showPopover();
              return [
                before,
                document.querySelector("#popover1") === null,
                !!popover2,
                popover1.matches(":popover-open"),
                invalidSelector,
                popover2.matches(":popover-open")
              ].join("|");
            })()
            "##,
        )
        .expect("stale popover Element.matches probe should evaluate");

    assert_eq!(result, "true|true|true|false|SyntaxError|true");
}
#[test]
fn base_href_reflection_and_document_base_url_follow_first_supported_href() {
    let mut vm = new_storage_test_vm("https://base-url.test/root/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const head = html.appendChild(document.createElement("head"));
              html.appendChild(document.createElement("body"));

              const missing = document.createElement("base");
              const empty = document.createElement("base");
              empty.setAttribute("href", "");
              head.append(missing, empty);

              const blocked = document.createElement("base");
              blocked.href = "javascript:/,ignored";
              head.prepend(blocked);
              const second = document.createElement("base");
              second.href = "https://cdn.example/assets/";
              head.append(second);

              const link = document.createElement("a");
              link.href = "child";
              const missingHref = missing.href;
              const emptyHref = empty.href;
              const blockedHref = link.href;
              blocked.remove();
              const afterRemove = link.href;
              missing.remove();
              empty.remove();
              const afterFallbackBasesRemove = link.href;
              const image = document.createElement("img");
              image.setAttribute("src", "icon.png");
              document.body.append(image);

              return [
                missingHref,
                emptyHref,
                blockedHref,
                afterRemove,
                second.href,
                afterFallbackBasesRemove,
                image.src
              ].join("|");
            })()
            "#,
        )
        .expect("base URL probe should evaluate");

    assert_eq!(
        result,
        "https://base-url.test/root/page.html|https://base-url.test/root/page.html|https://base-url.test/root/child|https://base-url.test/root/child|https://cdn.example/assets/|https://cdn.example/assets/child|https://cdn.example/assets/icon.png"
    );
}
#[test]
fn navigator_runtime_data_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://navigator-private-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(navigator);
              const ua = Object.getOwnPropertyDescriptor(proto, "userAgent");
              const webdriver = Object.getOwnPropertyDescriptor(proto, "webdriver");
              const languages = Object.getOwnPropertyDescriptor(proto, "languages");
              const internalNamesBefore = Object.getOwnPropertyNames(navigator)
                .filter(name => name.startsWith("__moliNavigator"))
                .sort();
              navigator.__moliNavigatorRuntimeData = {
                userAgent: "spoofed",
                webdriver: true,
                languages: ["zz-ZZ"]
              };
              const fake = {
                __moliNavigatorRuntimeData: {
                  userAgent: "fake",
                  webdriver: true,
                  languages: ["fake"]
                }
              };
              const callGetter = getter => {
                try {
                  return String(getter.call(fake));
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              return JSON.stringify({
                internalNamesBefore,
                userAgent: navigator.userAgent,
                webdriver: navigator.webdriver,
                languages: Array.from(navigator.languages).join(","),
                fakeUserAgent: callGetter(ua.get),
                fakeWebdriver: callGetter(webdriver.get),
                fakeLanguages: callGetter(languages.get)
              });
            })()
            "#,
        )
        .expect("navigator private slot spoofing probe should evaluate");

    let expected = format!(
        r#"{{"internalNamesBefore":[],"userAgent":"{}","webdriver":false,"languages":"en-US,en","fakeUserAgent":"throw:TypeError","fakeWebdriver":"throw:TypeError","fakeLanguages":"throw:TypeError"}}"#,
        DEFAULT_USER_AGENT
    );
    assert_eq!(result, expected);
}
#[test]
fn navigator_permissions_background_sync_defaults_granted_and_tracks_overrides() {
    let mut vm = new_storage_test_vm("https://navigator-background-sync-permission.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__backgroundSyncPermissionProbe = "pending";
          navigator.permissions.query({ name: "background-sync" }).then(status => {
            globalThis.__backgroundSyncPermissionProbe = [
              status instanceof PermissionStatus,
              status.name,
              status.state
            ].join("|");
          }, error => {
            globalThis.__backgroundSyncPermissionProbe = `error:${error && error.name}`;
          });
        })()
        "#,
    )
    .expect("background-sync permission default probe should evaluate");
    let result = vm
        .eval("String(globalThis.__backgroundSyncPermissionProbe)")
        .expect("background-sync permission default promise should settle");
    assert_eq!(result, "true|background-sync|granted");

    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("background-sync".to_owned()),
        setting: "denied".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);
    vm.eval(
        r#"
        (() => {
          globalThis.__backgroundSyncPermissionDeniedProbe = "pending";
          navigator.permissions.query({ name: "background-sync" }).then(status => {
            globalThis.__backgroundSyncPermissionDeniedProbe = [
              status instanceof PermissionStatus,
              status.name,
              status.state
            ].join("|");
          }, error => {
            globalThis.__backgroundSyncPermissionDeniedProbe =
              `error:${error && error.name}`;
          });
        })()
        "#,
    )
    .expect("background-sync permission denied probe should evaluate");
    let result = vm
        .eval("String(globalThis.__backgroundSyncPermissionDeniedProbe)")
        .expect("background-sync permission denied promise should settle");
    assert_eq!(result, "true|background-sync|denied");
}

#[test]
fn geometry_exposes_svg_point_as_a_legacy_window_alias() {
    let mut vm = new_storage_test_vm("https://geometry-svg-point-alias.test/");
    let result = vm.eval(r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  assert(SVGPoint === DOMPoint, "legacy alias constructor identity");
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "SVGPoint");
  assert(descriptor.value === DOMPoint && descriptor.writable && !descriptor.enumerable && descriptor.configurable, "global alias descriptor");
  const point = new SVGPoint(1, 2);
  assert(Object.getPrototypeOf(point) === DOMPoint.prototype, "prototype identity");
  assert(point.x === 1 && point.y === 2 && point.z === 0 && point.w === 1, "constructor values");
  point.x = 3;
  const serialized = DOMPoint.prototype.toJSON.call(point);
  assert(serialized.x === 3 && serialized.y === 2 && serialized.z === 0 && serialized.w === 1, "alias produces a branded DOMPoint with native slots");
  const copied = SVGPoint.fromPoint(point);
  assert(Object.getPrototypeOf(copied) === DOMPoint.prototype && copied.x === 3 && copied.y === 2, "alias static factory");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M 1 2 L 3 4");
  const created = path.getPointAtLength(0);
  assert(created instanceof SVGPoint && created instanceof DOMPoint, "SVG factory uses the same interface");
  assert(Object.getPrototypeOf(created) === SVGPoint.prototype, "SVG factory prototype");
  return "ok";
})()
"#).expect("SVGPoint should alias the native DOMPoint interface");
    assert_eq!(result, "ok");
}

#[test]
fn request_referrers_use_captured_document_origin() {
    for (document_url, origin, expected_referrer) in [
        (
            "about:blank",
            "https://request-origin.test",
            "https://request-origin.test/referrer",
        ),
        (
            "https://request-origin.test/page.html",
            "null",
            "about:client",
        ),
    ] {
        let mut vm = new_storage_test_vm(document_url);
        {
            // Seed inherited and opaque settings independently of the URL.
            // No requests exist yet, so replacing this fixture's authority
            // cannot discard in-flight loads.
            let mut host = vm._context_host.borrow_mut();
            let loader = host.current_main_document_resource_loader().unwrap();
            let context = loader.fetch_context();
            host.retire_document_resource_loader(context.owner())
                .unwrap();
            host.register_committed_document_resource_loader(
                crate::network::context::DocumentFetchContext::new(
                    context.owner(),
                    context.document_url().clone(),
                    context.base_url().clone(),
                    origin,
                ),
                crate::network::context::DocumentResourceAuthoritySource::Inherited(loader),
            );
        }
        let result = vm
            .eval(
                r#"
(() => {
  globalThis.origin = 'https://other.test';
  const url = 'https://request-origin.test/resource';
  return JSON.stringify([
    document.URL,
    globalThis.origin,
    new Request(url, {referrer: 'https://request-origin.test/referrer'}).referrer,
    new Request(url, {referrer: 'https://other.test/referrer'}).referrer
  ]);
})()
"#,
            )
            .expect("Request referrer origin probe should evaluate");
        let values: Vec<String> = serde_json::from_str(&result).unwrap();
        assert_eq!(
            values,
            [
                document_url,
                "https://other.test",
                expected_referrer,
                "about:client",
            ],
            "document URL {document_url}, captured origin {origin}"
        );
    }
}

#[test]
fn request_relative_urls_follow_live_document_base_urls() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    let result = vm
        .eval(
            r#"
(() => {
  const html = document.createElement('html');
  html.appendChild(document.createElement('head'));
  html.appendChild(document.createElement('body'));
  document.appendChild(html);
  const base = document.createElement('base');
  (document.head || document.documentElement).appendChild(base);
  const checks = [];
  for (const href of ['/first/', 'https://other.test/second/', '/third/']) {
    base.href = href;
    for (const input of ['item', '../item?q#f', '?q', '#f']) {
      checks.push(new Request(input).url === new URL(input, document.baseURI).href);
    }
    checks.push(new Request('https://absolute.test/path').url === 'https://absolute.test/path');
  }
  base.remove();
  checks.push(new Request('item').url === new URL('item', document.baseURI).href);
  return JSON.stringify(checks);
})()
"#,
        )
        .expect("Request base URL probe should evaluate");
    let checks: Vec<bool> = serde_json::from_str(&result).unwrap();
    assert_eq!(checks.len(), 16);
    assert!(checks.iter().all(|check| *check), "{result}");
}

fn install_request_base_parent_document(vm: &mut ScriptVm) {
    vm.exec(
        r#"
const html = document.documentElement || document.appendChild(document.createElement('html'));
const head = document.head || html.appendChild(document.createElement('head'));
if (!document.body) html.appendChild(document.createElement('body'));
const parentBase = head.appendChild(document.createElement('base'));
parentBase.href = '/parent-base/';
globalThis.requestBaseFrame = document.createElement('iframe');
"#,
        None,
    )
    .expect("parent document with its own base should be installed");
}

fn assert_request_urls_follow_live_child_base_urls(
    vm: &mut ScriptVm,
    document_url: &str,
    fallback_base_url: &str,
) {
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("iframe should have its own realm");
    let fallback_base_url = Url::parse(fallback_base_url).unwrap();
    let parent_base_url = Url::parse(
        &vm.eval("document.baseURI")
            .expect("parent base should be readable"),
    )
    .unwrap();
    let allowed_referrer = fallback_base_url.join("/allowed-referrer").unwrap();
    let phases = [
        (
            Some("/initial/"),
            fallback_base_url.join("/initial/").unwrap(),
        ),
        (
            Some("../relative/"),
            fallback_base_url.join("../relative/").unwrap(),
        ),
        (
            Some("https://other-base.test/second/"),
            Url::parse("https://other-base.test/second/").unwrap(),
        ),
        (None, fallback_base_url.clone()),
    ];

    for (href, expected_base_url) in phases {
        let mutation = match href {
            Some(href) => format!(
                "const base = childDocument.querySelector('base') || \
                 childDocument.head.appendChild(childDocument.createElement('base')); \
                 base.href = {};",
                serde_json::to_string(href).unwrap()
            ),
            None => "childDocument.querySelector('base').remove();".to_owned(),
        };
        vm.eval(&format!(
            "(() => {{ const childDocument = requestBaseFrame.contentDocument; \
             {mutation} }})()"
        ))
        .expect("child base mutation should evaluate");

        let probe = format!(
            r#"
(() => {{
  const child = CHILD_WINDOW;
  return JSON.stringify({{
    documentURL: child.document.URL,
    baseURI: child.document.baseURI,
    urls: ['item', '../item?q#f', '?q', '#f'].map(input => new child.Request(input).url),
    absoluteURL: new child.Request('https://absolute.test/path').url,
    allowedReferrer: new child.Request('item', {{referrer: {allowed_referrer}}}).referrer,
    relativeReferrer: new child.Request('item', {{referrer: 'referrer'}}).referrer
  }});
}})()
"#,
            allowed_referrer = serde_json::to_string(allowed_referrer.as_str()).unwrap(),
        );
        let parent_result = vm
            .eval(&probe.replace("CHILD_WINDOW", "requestBaseFrame.contentWindow"))
            .expect("parent should construct a Request using the child constructor");
        let child_result = vm
            .eval_in_child_default_context(
                child_context_id,
                &probe.replace("CHILD_WINDOW", "globalThis"),
            )
            .expect("child realm should construct a Request using its own constructor");
        let relative_referrer = expected_base_url.join("referrer").unwrap();
        let expected_referrer = if relative_referrer.origin() == allowed_referrer.origin() {
            relative_referrer.as_str()
        } else {
            "about:client"
        };
        let expected_urls = ["item", "../item?q#f", "?q", "#f"]
            .map(|input| expected_base_url.join(input).unwrap().to_string());
        let expected = serde_json::json!({
            "documentURL": document_url,
            "baseURI": expected_base_url.as_str(),
            "urls": expected_urls,
            "absoluteURL": "https://absolute.test/path",
            "allowedReferrer": allowed_referrer.as_str(),
            "relativeReferrer": expected_referrer,
        });
        for (realm, result) in [("parent", parent_result), ("child", child_result)] {
            let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(
                actual, expected,
                "{realm} call after base mutation {href:?}"
            );
        }
        assert_eq!(
            vm.eval("new Request('parent-item').url")
                .expect("parent Request should retain the parent base"),
            parent_base_url.join("parent-item").unwrap().as_str(),
            "child base mutation {href:?} must not affect the parent's API base"
        );
    }
}

#[test]
fn request_relative_urls_follow_live_initial_about_blank_iframe_base_urls() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.eval("document.body.appendChild(requestBaseFrame); requestBaseFrame.contentDocument")
        .expect("initial about:blank iframe should be created");
    vm.drain_pending_child_frame_work_for_test();
    assert_request_urls_follow_live_child_base_urls(
        &mut vm,
        "about:blank",
        "https://request-base.test/parent-base/",
    );
}

#[test]
fn request_relative_urls_follow_live_srcdoc_iframe_base_urls() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.exec(
        "requestBaseFrame.srcdoc = '<!doctype html><html><head></head><body></body></html>'; \
         document.body.appendChild(requestBaseFrame);",
        None,
    )
    .expect("srcdoc iframe should be created");
    vm.drain_pending_child_frame_work_for_test();
    assert_request_urls_follow_live_child_base_urls(
        &mut vm,
        "about:srcdoc",
        "https://request-base.test/parent-base/",
    );
}

fn assert_retained_request_uses_child_document(vm: &mut ScriptVm, fallback_base: &str) {
    vm.exec(
        r#"
globalThis.retainedChildWindow = requestBaseFrame.contentWindow;
globalThis.retainedChildDocument = retainedChildWindow.document;
globalThis.RetainedChildRequest = retainedChildWindow.Request;
retainedChildDocument.head.innerHTML = '<base href="https://fixture.test/child/">';
"#,
        None,
    )
    .expect("child Document and constructors should be retained");

    let fallback_base_url = Url::parse(fallback_base).unwrap();
    let allowed_referrer = fallback_base_url.join("/allowed-referrer").unwrap();
    let parent_item = fallback_base_url.join("/parent-base/item").unwrap();
    let assert_base = |vm: &mut ScriptVm, base: &str| {
        let result = vm
            .eval(&format!(
                r#"
(() => {{
const controller = new AbortController();
controller.abort('retained-reason');
const request = new RetainedChildRequest('item', {{signal: controller.signal}});
return JSON.stringify({{
  documentIdentity: retainedChildWindow.document === retainedChildDocument,
  urls: ['item', '../item?q#f', '?q', '#f'].map(input => new RetainedChildRequest(input).url),
  referrer: new RetainedChildRequest('item', {{referrer: 'referrer'}}).referrer,
  allowedReferrer: new RetainedChildRequest('item', {{referrer: {allowed_referrer}}}).referrer,
  signalAborted: request.signal.aborted,
  signalReason: request.signal.reason,
  parent: new Request('item').url
}});
}})()
"#,
                allowed_referrer = serde_json::to_string(allowed_referrer.as_str()).unwrap(),
            ))
            .unwrap_or_else(|error| panic!("retained child Request at base {base}: {error}"));
        let base = Url::parse(base).unwrap();
        let relative_referrer = base.join("referrer").unwrap();
        let expected_referrer = if relative_referrer.origin() == allowed_referrer.origin() {
            relative_referrer.as_str()
        } else {
            "about:client"
        };
        let expected_urls =
            ["item", "../item?q#f", "?q", "#f"].map(|input| base.join(input).unwrap().to_string());
        let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({
                "documentIdentity": true,
                "urls": expected_urls,
                "referrer": expected_referrer,
                "allowedReferrer": allowed_referrer.as_str(),
                "signalAborted": true,
                "signalReason": "retained-reason",
                "parent": parent_item.as_str(),
            }),
            "retained Document base {base}"
        );
    };

    assert_base(vm, "https://fixture.test/child/");
    vm.exec("requestBaseFrame.remove();", None)
        .expect("iframe should detach");
    assert_base(vm, "https://fixture.test/child/");
    assert!(vm.live_child_default_runtime_realm_inventory().is_empty());
    assert_base(vm, "https://fixture.test/child/");

    vm.exec(
        "retainedChildDocument.querySelector('base').href = 'https://fixture.test/changed/';",
        None,
    )
    .expect("retained Document base should remain mutable");
    assert_base(vm, "https://fixture.test/changed/");
    vm.exec(
        "retainedChildDocument.querySelector('base').remove();",
        None,
    )
    .expect("retained Document base should be removable");
    assert_base(vm, fallback_base);

    vm.exec(
        "requestBaseFrame.removeAttribute('src'); document.body.appendChild(requestBaseFrame);",
        None,
    )
    .expect("the same iframe element should reattach");
    vm.exec(
        "requestBaseFrame.contentDocument.head.innerHTML = '<base href=\"https://replacement.test/new/\">';",
        None,
    )
    .expect("replacement child Document should get its own base");
    assert_eq!(
        vm.eval("new requestBaseFrame.contentWindow.Request('item').url")
            .unwrap(),
        "https://replacement.test/new/item"
    );
    assert_base(vm, fallback_base);
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const replacement = requestBaseFrame.contentDocument;
  replacement.body.textContent = 'REPLACEMENT';
  retainedChildWindow.document.body.textContent = 'WRITTEN VIA OLD WINDOW';
  return JSON.stringify([
    retainedChildWindow !== requestBaseFrame.contentWindow,
    retainedChildDocument !== replacement,
    retainedChildDocument.body.textContent,
    replacement.body.textContent
  ]);
})()
"#,
        )
        .expect("retained Window writes should remain confined to its original Document"),
        r#"[true,true,"WRITTEN VIA OLD WINDOW","REPLACEMENT"]"#
    );
}

#[test]
fn request_retains_initial_about_blank_document_after_iframe_removal() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.exec("document.body.appendChild(requestBaseFrame);", None)
        .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    assert_retained_request_uses_child_document(&mut vm, "https://request-base.test/parent-base/");
}

#[test]
fn request_retains_srcdoc_document_after_iframe_removal() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.exec(
        "requestBaseFrame.srcdoc = '<!doctype html><html><head></head><body></body></html>'; \
         document.body.appendChild(requestBaseFrame);",
        None,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    assert_retained_request_uses_child_document(&mut vm, "https://request-base.test/parent-base/");
}

#[test]
fn request_retains_child_settings_when_iframe_handle_is_reused_before_cleanup() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.exec(
        r#"
document.body.appendChild(requestBaseFrame);
requestBaseFrame.contentDocument.head.innerHTML = '<base href="https://fixture.test/child/">';
globalThis.RetainedChildRequest = requestBaseFrame.contentWindow.Request;
requestBaseFrame.remove();
document.body.appendChild(requestBaseFrame);
requestBaseFrame.contentDocument.head.innerHTML = '<base href="https://replacement.test/new/">';
"#,
        None,
    )
    .unwrap();
    vm.live_child_default_runtime_realm_inventory();
    assert_eq!(
        vm.eval("new RetainedChildRequest('item').url + '|' + new requestBaseFrame.contentWindow.Request('item').url")
            .unwrap(),
        "https://fixture.test/child/item|https://replacement.test/new/item"
    );
}

#[test]
fn request_retains_child_origin_after_iframe_removal() {
    let mut vm = new_storage_test_vm("https://request-base.test/dir/page.html");
    install_request_base_parent_document(&mut vm);
    vm.exec(
        r#"
document.body.appendChild(requestBaseFrame);
requestBaseFrame.contentDocument.head.innerHTML = '<base href="https://fixture.test/child/">';
globalThis.RetainedChildRequest = requestBaseFrame.contentWindow.Request;
requestBaseFrame.remove();
"#,
        None,
    )
    .unwrap();
    assert!(vm.live_child_default_runtime_realm_inventory().is_empty());
    {
        // Give the top fixture different settings without changing its URL.
        // Only the constructor is retained; no child Document/Window reference
        // or live owner registry is available to supply its original origin.
        let mut host = vm._context_host.borrow_mut();
        let loader = host.current_main_document_resource_loader().unwrap();
        let context = loader.fetch_context();
        host.retire_document_resource_loader(context.owner())
            .unwrap();
        host.register_committed_document_resource_loader(
            crate::network::context::DocumentFetchContext::new(
                context.owner(),
                context.document_url().clone(),
                context.base_url().clone(),
                "https://other-origin.test",
            ),
            crate::network::context::DocumentResourceAuthoritySource::Inherited(loader),
        );
    }
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
new RetainedChildRequest('item').url,
new RetainedChildRequest('item', {referrer: 'https://request-base.test/allowed'}).referrer,
new RetainedChildRequest('item', {referrer: 'https://other-origin.test/rejected'}).referrer
])"#,
        )
        .unwrap(),
        r#"["https://fixture.test/child/item","https://request-base.test/allowed","about:client"]"#
    );
}

#[tokio::test]
async fn request_relative_urls_follow_live_http_iframe_base_urls() {
    let (server_url, server) = spawn_lightweight_popup_response_html_server(
        "Request iframe base server",
        "Request iframe base",
        "",
        "<!doctype html><html><head></head><body></body></html>",
    )
    .await;
    let server_url = Url::parse(&server_url).unwrap();
    let parent_url = server_url.join("/parent/page.html").unwrap();
    let child_url = server_url.join("/child/page.html").unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    install_request_base_parent_document(&mut vm);
    vm.exec(
        &format!(
            "requestBaseFrame.src = {}; document.body.appendChild(requestBaseFrame);",
            serde_json::to_string(child_url.as_str()).unwrap()
        ),
        None,
    )
    .expect("HTTP iframe navigation should start");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for _ in 0..128 {
            let current_url = vm
                .eval("requestBaseFrame.contentDocument && requestBaseFrame.contentDocument.URL")
                .expect("committed child URL should be readable");
            if current_url == child_url.as_str()
                && !vm.live_child_default_runtime_realm_inventory().is_empty()
            {
                return;
            }
            if !vm
                .run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .expect("iframe navigation Page task should execute")
            {
                assert!(
                    vm.wait_for_task_executor_work_arrival().await,
                    "iframe navigation should publish its next Page task"
                );
            }
        }
        panic!("HTTP iframe navigation exceeded its finite Page task budget");
    })
    .await
    .expect("HTTP iframe should commit and materialize its child realm");
    assert_request_urls_follow_live_child_base_urls(
        &mut vm,
        child_url.as_str(),
        child_url.as_str(),
    );
    assert_retained_request_uses_child_document(&mut vm, child_url.as_str());
    server.await.expect("iframe response server should finish");
}

#[test]
fn dom_point_readonly_constructor_uses_readonly_instances_and_shared_methods() {
    let mut vm = new_storage_test_vm("https://dompoint-readonly-constructor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const point = new DOMPointReadOnly(1, 2, 3, 4);
  point.x = 9;
  return [
    point instanceof DOMPointReadOnly,
    point instanceof DOMPoint,
    [point.x, point.y, point.z, point.w].join(","),
    JSON.stringify(point.toJSON()),
    Object.hasOwn(DOMPointReadOnly.prototype, "x"),
    Object.getOwnPropertyDescriptor(DOMPointReadOnly.prototype, "x").set === undefined,
    Object.hasOwn(DOMPointReadOnly.prototype, "toJSON"),
    Object.hasOwn(DOMPoint.prototype, "toJSON"),
    new DOMPoint() instanceof DOMPointReadOnly
  ].join("|");
})()
"#,
        )
        .expect("DOMPointReadOnly constructor should evaluate");

    assert_eq!(
        result,
        "true|false|1,2,3,4|{\"x\":1,\"y\":2,\"z\":3,\"w\":4}|true|true|true|false|true"
    );
}

#[test]
fn dom_point_readonly_from_point_uses_dictionary_conversion_and_function_realm() {
    let mut vm = new_storage_test_vm("https://dompoint-readonly-from-point.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "dompoint-from-point-realm";
  body.appendChild(frame);
})()
"#,
    )
    .expect("DOMPointReadOnly factory child frame should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "DOMPointReadOnly factory child Realm",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const other = document.getElementById("dompoint-from-point-realm").contentWindow;
  const point = other.DOMPointReadOnly.fromPoint({x: "1", z: 3, w: 4});
  const empty = other.DOMPointReadOnly.fromPoint(null);
  return [
    other.DOMPointReadOnly.fromPoint.length,
    point instanceof other.DOMPointReadOnly,
    point instanceof other.DOMPoint,
    point instanceof DOMPointReadOnly,
    [point.x, point.y, point.z, point.w].join(","),
    [empty.x, empty.y, empty.z, empty.w].join(",")
  ].join("|");
})()
"#,
        )
        .expect("DOMPointReadOnly.fromPoint should evaluate");

    assert_eq!(result, "0|true|false|false|1,0,3,4|0,0,0,1");
}

#[test]
fn dom_point_matrix_transform_validates_matrix_init_and_returns_in_the_function_realm() {
    let mut vm = new_storage_test_vm("https://dompoint-matrix-transform.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "dompoint-matrix-transform-realm";
  body.appendChild(frame);
})()
"#,
    )
    .expect("DOMPoint matrixTransform child frame should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "DOMPoint matrixTransform child Realm",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const other = document.getElementById("dompoint-matrix-transform-realm").contentWindow;
  const source = new DOMPointReadOnly(5, 4, 3, 2);
  const transformed = other.DOMPointReadOnly.prototype.matrixTransform.call(source, {
    a: 2, m11: 2, d: 3, e: 10, f: 20, m33: 4, m44: 5
  });
  const outcome = init => {
    try {
      source.matrixTransform(init);
      return "no throw";
    } catch (error) {
      return error.name;
    }
  };
  return [
    other.DOMPointReadOnly.prototype.matrixTransform.length,
    transformed instanceof other.DOMPoint,
    transformed instanceof DOMPoint,
    [transformed.x, transformed.y, transformed.z, transformed.w].join(","),
    outcome({a: 1, m11: 2}),
    outcome({is2D: true, m33: 1.0000001}),
    outcome({a: NaN, m11: NaN}),
    outcome({a: 0, m11: -0})
  ].join("|");
})()
"#,
        )
        .expect("DOMPoint matrixTransform should evaluate");

    assert_eq!(
        result,
        "0|true|false|30,52,12,10|TypeError|TypeError|no throw|no throw"
    );
}

#[test]
fn dom_quad_uses_live_same_object_points_and_geometry_dictionary_factories() {
    let mut vm = new_storage_test_vm("https://domquad-geometry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.name;
    }
  };
  const q = new DOMQuad(
    {x: 1, y: 2, z: 3, w: 4},
    {x: 11, y: 2},
    {x: 11, y: 22},
    {x: 1, y: 22}
  );
  const initial = [q.p1.x, q.p1.y, q.p1.z, q.p1.w,
    q.getBounds().x, q.getBounds().y, q.getBounds().width,
    q.getBounds().height].join(",");
  const copy = DOMQuad.fromQuad(q);
  const copyInitial = [copy.p1.x, copy.p1.y, copy.p1.z, copy.p1.w,
    copy.p4.x, copy.p4.y].join(",");
  const originalP1 = q.p1;
  q.p1.x = -5;
  q.p4.y = 30;
  q.p1 = new DOMPoint(999, 999);
  const live = [q.p1 === originalP1, q.p1.x, q.p4.y,
    q.getBounds().x, q.getBounds().y, q.getBounds().width,
    q.getBounds().height].join(",");
  const copyAfterMutation = [copy.p1.x, copy.p4.y, copy.p1 !== q.p1].join(",");
  const negative = DOMQuad.fromRect({x: 10, y: 20, width: -100, height: -200});
  const negativeValues = [negative.p1.x, negative.p1.y, negative.p2.x,
    negative.p3.y, negative.getBounds().x, negative.getBounds().y,
    negative.getBounds().width, negative.getBounds().height].join(",");
  const nanBounds = new DOMQuad({x: 0, y: 0}, {x: 0, y: 0},
    {x: NaN, y: 0}, {x: 0, y: 0}).getBounds();
  const nanValues = [nanBounds.x, nanBounds.y, nanBounds.width,
    nanBounds.height].join(",");
  const json = q.toJSON();
  const pointDescriptor = Object.getOwnPropertyDescriptor(DOMQuad.prototype, "p1");
  const fake = Object.assign(Object.create(DOMQuad.prototype), {
    __moliDomQuadBrand: true,
    __moliDomQuadP1: new DOMPoint()
  });
  return JSON.stringify({
    interfaceShape: [DOMQuad.length, DOMQuad.fromRect.length,
      DOMQuad.fromQuad.length, q.getBounds.length, q.toJSON.length,
      Object.prototype.toString.call(q), "bounds" in q].join("|"),
    prototypeKeys: Object.keys(DOMQuad.prototype).join(","),
    pointDescriptor: [typeof pointDescriptor.get, pointDescriptor.get.name,
      pointDescriptor.get.length, pointDescriptor.set === undefined,
      pointDescriptor.enumerable, pointDescriptor.configurable].join("|"),
    sameObject: q.p1 === q.p1,
    initial,
    live,
    copyInitial,
    copyAfterMutation,
    negativeValues,
    nanValues,
    jsonShape: [Object.getPrototypeOf(json) === Object.prototype,
      Object.keys(json).join(","), json.p1 === q.p1,
      JSON.stringify(json)].join("|"),
    forged: [probe(() => pointDescriptor.get.call(fake)),
      probe(() => DOMQuad.prototype.getBounds.call(fake)),
      probe(() => DOMQuad.prototype.toJSON.call(fake))].join(","),
    visibleSlots: Object.getOwnPropertyNames(q)
      .filter(name => name.startsWith("__moliDomQuad")).join(",")
  });
})()
"#,
        )
        .expect("DOMQuad geometry probe should evaluate");

    assert_eq!(
        result,
        concat!(
            r#"{"interfaceShape":"0|0|0|0|0|[object DOMQuad]|false","prototypeKeys":"p1,p2,p3,p4,getBounds,toJSON","pointDescriptor":"function|get p1|0|true|true|true","sameObject":true,"initial":"1,2,3,4,1,2,10,20","live":"true,-5,30,-5,2,16,28","copyInitial":"1,2,3,4,1,22","copyAfterMutation":"1,22,true","negativeValues":"10,20,-90,-180,-90,-180,100,200","nanValues":"NaN,0,NaN,0","jsonShape":"true|p1,p2,p3,p4|true|"#,
            r#"{\"p1\":{\"x\":-5,\"y\":2,\"z\":3,\"w\":4},\"p2\":{\"x\":11,\"y\":2,\"z\":0,\"w\":1},\"p3\":{\"x\":11,\"y\":22,\"z\":0,\"w\":1},\"p4\":{\"x\":1,\"y\":30,\"z\":0,\"w\":1}}","forged":"TypeError,TypeError,TypeError","visibleSlots":""}"#
        )
    );
}

#[test]
fn dom_quad_factories_and_default_to_json_use_the_function_realm() {
    let mut vm = new_storage_test_vm("https://domquad-function-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "domquad-function-realm";
  body.appendChild(frame);
})()
"#,
    )
    .expect("DOMQuad factory child frame should be created");
    materialize_single_child_default_realm_for_test(&mut vm, "DOMQuad factory child Realm");

    let result = vm
        .eval(
            r#"
(() => {
  const other = document.getElementById("domquad-function-realm").contentWindow;
  const quad = other.DOMQuad.fromRect({x: 1, y: 2, width: 3, height: 4});
  const source = new DOMQuad({x: 7, y: 8});
  const json = other.DOMQuad.prototype.toJSON.call(source);
  return [
    quad instanceof other.DOMQuad,
    quad instanceof DOMQuad,
    quad.p1 instanceof other.DOMPoint,
    quad.p1 instanceof DOMPoint,
    quad.getBounds() instanceof other.DOMRect,
    quad.getBounds() instanceof DOMRect,
    Object.getPrototypeOf(json) === other.Object.prototype,
    json.p1 === source.p1,
    json.p1 instanceof DOMPoint,
    json.p1 instanceof other.DOMPoint
  ].join("|");
})()
"#,
        )
        .expect("DOMQuad function realm probe should evaluate");

    assert_eq!(
        result,
        "true|false|true|false|true|false|true|true|true|false"
    );
}

#[test]
fn geometry_exposes_legacy_window_aliases_without_replacing_native_svg_rect() {
    let mut vm = new_storage_test_vm("https://geometry-legacy-window-aliases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const interfaces = ["SVGPoint", "SVGRect", "SVGMatrix", "WebKitCSSMatrix"];
  const descriptorShape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    return [
      descriptor.value === globalThis[name],
      descriptor.writable,
      descriptor.enumerable,
      descriptor.configurable
    ].join(",");
  };
  const point = new SVGPoint(1, 2);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const rect = svg.createSVGRect();
  rect.x = 3;
  rect.y = 4;
  rect.width = 5;
  rect.height = 6;
  const matrix = svg.createSVGMatrix();
  const multiplied = matrix.multiply({a: 3, d: 2});
  matrix.a = 2;
  return JSON.stringify({
    identities: [
      SVGPoint === DOMPoint,
      SVGRect === DOMRect,
      SVGMatrix === DOMMatrix,
      WebKitCSSMatrix === DOMMatrix
    ].join(","),
    point: [point instanceof DOMPoint, point.x, point.y, String(point)].join(","),
    rect: [rect instanceof SVGRect, rect instanceof DOMRect, rect.x, rect.height, String(rect)].join(","),
    matrix: [
      matrix instanceof DOMMatrix,
      matrix instanceof SVGMatrix,
      Object.getPrototypeOf(matrix) === DOMMatrix.prototype,
      !Object.hasOwn(matrix, "multiply"),
      matrix.a,
      multiplied.a,
      multiplied.d,
      Object.prototype.toString.call(matrix)
    ].join(","),
    descriptors: interfaces.map(descriptorShape).join("|")
  });
})()
"#,
        )
        .expect("Geometry legacy Window aliases should evaluate");

    assert_eq!(
        result,
        concat!(
            r#"{"identities":"true,true,true,true","point":"true,1,2,[object DOMPoint]","rect":"true,true,3,6,[object DOMRect]","matrix":"true,true,true,true,2,3,2,[object DOMMatrix]","descriptors":""#,
            "true,true,false,true|true,true,false,true|",
            "true,true,false,true|true,true,false,true\"}"
        )
    );
}

#[test]
fn svg_transform_factories_convert_optional_dom_matrix_2d_init() {
    let mut vm = new_storage_test_vm("https://svg-transform-dom-matrix-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const SVG_NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(SVG_NS, "svg");
  const group = document.createElementNS(SVG_NS, "g");
  const fromElement = svg.createSVGTransformFromMatrix({a: 2, e: 5});
  const defaultElement = svg.createSVGTransformFromMatrix();
  const fromFactory = svg.createSVGTransformFromMatrix(svg.createSVGMatrix().translate(9, 10));
  const transform = svg.createSVGTransform();
  transform.setMatrix({b: 4, f: 6});
  const fromList = group.transform.baseVal.createSVGTransformFromMatrix({c: 7, d: 8});
  let mismatch;
  try {
    transform.setMatrix({a: 1, m11: 2});
    mismatch = "none";
  } catch (error) {
    mismatch = error.name;
  }
  return JSON.stringify({
    fromElement: [fromElement.matrix.a, fromElement.matrix.e].join(","),
    defaultElement: [defaultElement.matrix.a, defaultElement.matrix.d].join(","),
    fromFactory: [fromFactory.matrix.e, fromFactory.matrix.f].join(","),
    setMatrix: [transform.matrix.b, transform.matrix.f].join(","),
    fromList: [fromList.matrix.c, fromList.matrix.d].join(","),
    mismatch,
    lengths: [
      SVGSVGElement.prototype.createSVGTransformFromMatrix.length,
      SVGTransform.prototype.setMatrix.length,
      SVGTransformList.prototype.createSVGTransformFromMatrix.length
    ].join(",")
  });
})()
"#,
        )
        .expect("SVG transform DOMMatrix2DInit conversion should evaluate");

    assert_eq!(
        result,
        r#"{"fromElement":"2,5","defaultElement":"1,1","fromFactory":"9,10","setMatrix":"4,6","fromList":"7,8","mismatch":"TypeError","lengths":"0,0,0"}"#
    );
}
