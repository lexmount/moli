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
