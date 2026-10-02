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
globalThis.retainedChildDocument = requestBaseFrame.contentDocument;
globalThis.RetainedChildRequest = requestBaseFrame.contentWindow.Request;
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
