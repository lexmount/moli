use super::*;

#[test]
fn document_has_storage_access_is_false_for_insecure_context() {
    let mut vm = new_storage_test_vm("http://example.com/");

    vm.exec(
        r#"
        globalThis.__insecureStorageAccessProbe = "pending";
        document.hasStorageAccess().then(
          value => { globalThis.__insecureStorageAccessProbe = String(value); },
          error => { globalThis.__insecureStorageAccessProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("insecure storage access probe should schedule");

    let result = vm
        .eval("String(globalThis.__insecureStorageAccessProbe)")
        .expect("insecure storage access probe should settle");

    assert_eq!(result, "false");
}

#[test]
fn document_request_storage_access_rejects_insecure_context() {
    let mut vm = new_storage_test_vm("http://example.com/");

    vm.exec(
        r#"
        globalThis.__insecureRequestStorageAccessProbe = "pending";
        document.requestStorageAccess().then(
          () => { globalThis.__insecureRequestStorageAccessProbe = "resolved"; },
          error => {
            globalThis.__insecureRequestStorageAccessProbe =
              `${error && error.name}:${error instanceof DOMException}`;
          }
        );
        "#,
        None,
    )
    .expect("insecure requestStorageAccess probe should schedule");

    let result = vm
        .eval("String(globalThis.__insecureRequestStorageAccessProbe)")
        .expect("insecure requestStorageAccess probe should settle");

    assert_eq!(result, "NotAllowedError:true");
}

#[test]
fn document_storage_access_rejects_non_fully_active_documents() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        globalThis.__inactiveStorageAccessProbe = "pending";
        (async () => {
          const rejectionName = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };
          const xml = document.implementation.createDocument("", null);
          const html = document.implementation.createHTMLDocument("");
          return [
            await rejectionName(xml.hasStorageAccess()),
            await rejectionName(xml.requestStorageAccess()),
            await rejectionName(html.hasStorageAccess()),
            await rejectionName(html.requestStorageAccess())
          ].join("|");
        })().then(
          value => { globalThis.__inactiveStorageAccessProbe = value; },
          error => { globalThis.__inactiveStorageAccessProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("inactive storage access probe should schedule");

    let result = vm
        .eval("String(globalThis.__inactiveStorageAccessProbe)")
        .expect("inactive storage access probe should settle");

    assert_eq!(
        result,
        "InvalidStateError:true|InvalidStateError:true|InvalidStateError:true|InvalidStateError:true"
    );
}

#[test]
fn live_document_cookie_descriptor_matches_chromium_prototype_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(document);
              const proto2 = proto && Object.getPrototypeOf(proto);
              const summarize = (obj, key) => {
                const d = obj && Object.getOwnPropertyDescriptor(obj, key);
                if (!d) return null;
                return {
                  enumerable: !!d.enumerable,
                  configurable: !!d.configurable,
                  writable: Object.prototype.hasOwnProperty.call(d, "writable") ? !!d.writable : null,
                  hasGetter: typeof d.get === "function",
                  hasSetter: typeof d.set === "function",
                  valueType: Object.prototype.hasOwnProperty.call(d, "value") ? typeof d.value : null
                };
              };
              return JSON.stringify({
                ctor: document.constructor && document.constructor.name,
                ownCookie: Object.prototype.hasOwnProperty.call(document, "cookie"),
                documentCookie: summarize(document, "cookie"),
                protoName: proto && proto.constructor && proto.constructor.name,
                protoCookie: summarize(proto, "cookie"),
                proto2Name: proto2 && proto2.constructor && proto2.constructor.name,
                proto2Cookie: summarize(proto2, "cookie"),
                documentProtoCookie: typeof Document !== "undefined" ? summarize(Document.prototype, "cookie") : null,
                htmlDocumentProtoCookie: typeof HTMLDocument !== "undefined" ? summarize(HTMLDocument.prototype, "cookie") : null
              });
            })()
            "#,
        )
        .expect("document cookie descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"HTMLDocument","ownCookie":false,"documentCookie":null,"protoName":"HTMLDocument","protoCookie":null,"proto2Name":"Document","proto2Cookie":{"enumerable":true,"configurable":true,"writable":null,"hasGetter":true,"hasSetter":true,"valueType":null},"documentProtoCookie":{"enumerable":true,"configurable":true,"writable":null,"hasGetter":true,"hasSetter":true,"valueType":null},"htmlDocumentProtoCookie":null}"#
    );
}

#[test]
fn live_document_cookie_getter_returns_string_when_cookie_store_is_empty() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              type: typeof document.cookie,
              value: document.cookie,
              missingMatch: document.cookie.match(/missing_cookie/)
            }))()
            "#,
        )
        .expect("document cookie empty getter probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"string","value":"","missingMatch":null}"#
    );
}

#[test]
fn live_document_cookie_write_uses_prototype_accessor_without_creating_own_property() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.cookie = "probe_cookie=ok; Path=/";
              return JSON.stringify({
                cookieIncludesProbe: document.cookie.includes("probe_cookie=ok"),
                ownCookie: Object.prototype.hasOwnProperty.call(document, "cookie"),
                ownCookieDesc: Object.getOwnPropertyDescriptor(document, "cookie") ?? null,
                protoCookieGetterType: typeof Object.getOwnPropertyDescriptor(Document.prototype, "cookie")?.get,
                protoCookieSetterType: typeof Object.getOwnPropertyDescriptor(Document.prototype, "cookie")?.set
              });
            })()
            "#,
        )
        .expect("document cookie write probe should evaluate");

    assert_eq!(
        result,
        r#"{"cookieIncludesProbe":true,"ownCookie":false,"ownCookieDesc":null,"protoCookieGetterType":"function","protoCookieSetterType":"function"}"#
    );
}

#[test]
fn live_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              type: typeof document.all,
              loose: document.all == undefined,
              strict: document.all === undefined,
              bool: !!document.all,
              tag: Object.prototype.toString.call(document.all),
              string: String(document.all),
              ctorDirect: document.all.constructor && document.all.constructor.name,
              noArgType: typeof document.all(),
              noArgNull: document.all() === null,
              item0: document.all(0)?.tagName ?? null,
              item999Null: document.all(999) === null,
              namedHit: document.all("probe") === document.getElementById("probe"),
              itemMethodNull: document.all.item(999) === null,
              namedMethodNull: document.all.namedItem('missing') === null
            }))()
            "#,
        )
        .expect("live document.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"undefined","loose":true,"strict":false,"bool":false,"tag":"[object HTMLAllCollection]","string":"[object HTMLAllCollection]","ctorDirect":"HTMLAllCollection","noArgType":"object","noArgNull":true,"item0":"HTML","item999Null":true,"namedHit":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}

#[test]
fn live_document_all_obeys_legacy_named_and_indexed_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        r#"<!doctype html><html><body>
          <div id="dupe"></div><div id="dupe"></div>
          <form id="same" name="same"></form>
          <span id="42"></span>
          <span id="043"></span>
          <span id="4294967294"></span>
          <span id="4294967295"></span>
          <span id="undefined"></span>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const all = document.all;
              const collections = [
                all.dupe,
                all.namedItem("dupe"),
                all("dupe", 0),
                all.item("dupe"),
              ];
              const matches = collections[0];
              const first = document.querySelectorAll("#dupe")[0];
              const second = document.querySelectorAll("#dupe")[1];
              const initial = [
                collections.every(collection => collection instanceof HTMLCollection),
                collections.every((collection, index) =>
                  collections.slice(index + 1).every(other => collection !== other)),
                matches.length,
                matches[0] === first,
                matches[1] === second,
                all("same") === document.getElementById("same")
              ];
              const numericNames = [
                all[42] === undefined,
                all[4294967294] === undefined,
                all["043"] === document.getElementById("043"),
                all[4294967295] === document.getElementById("4294967295"),
              ];
              let namedItemMissingThrows = false;
              try {
                all.namedItem();
              } catch (error) {
                namedItemMissingThrows = error instanceof TypeError;
              }
              const explicitUndefined =
                all.namedItem(undefined) === document.getElementById("undefined");
              const namedItemLength = all.namedItem.length;
              const third = document.createElement("div");
              third.id = "dupe";
              document.body.appendChild(third);
              const appended = collections.every(collection =>
                collection.length === 3 && collection[2] === third);
              second.remove();
              const removed = collections.every(collection =>
                collection.length === 2 && collection[0] === first && collection[1] === third);
              return JSON.stringify({
                initial,
                numericNames,
                namedItemMissingThrows,
                explicitUndefined,
                namedItemLength,
                appended,
                removed,
              });
            })()
            "##,
        )
        .expect("document.all multiple named matches should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[true,true,2,true,true,true],"numericNames":[true,true,true,true],"namedItemMissingThrows":true,"explicitUndefined":true,"namedItemLength":1,"appended":true,"removed":true}"#
    );
}

#[test]
fn legacy_named_access_filters_name_candidates_by_consumer() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        r#"<!doctype html><html><body>
            <applet id="appletId" name="appletOnly"></applet>
            <applet name="shared"></applet>
            <form id="form" name="shared"></form>
            <div id="divId" name="divOnly"></div>
            <a id="anchor" name="allOnly"></a>
            <img id="image" name="windowAllowed">
            <svg><a id="svgId" name="svgOnly"></a></svg>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const detached = new DOMParser().parseFromString(
                '<html><body><applet id="detachedAppletId" name="detachedApplet"></applet>' +
                '<form id="detachedFormId" name="detachedForm"></form></body></html>',
                'text/html'
              );
              return JSON.stringify({
                windowAppletNameAbsent: window.appletOnly === undefined,
                allAppletNameAbsent: document.all.appletOnly === undefined,
                windowSharedUsesEligibleForm: window.shared === document.getElementById("form"),
                allSharedUsesEligibleForm: document.all.shared === document.getElementById("form"),
                windowAnchorNameAbsent: window.allOnly === undefined,
                allAnchorNamePresent: document.all.allOnly === document.getElementById("anchor"),
                windowImageNamePresent:
                  window.windowAllowed === document.getElementById("image"),
                allImageNamePresent:
                  document.all.windowAllowed === document.getElementById("image"),
                windowDivNameAbsent: window.divOnly === undefined,
                allDivNameAbsent: document.all.divOnly === undefined,
                windowSvgNameAbsent: window.svgOnly === undefined,
                allSvgNameAbsent: document.all.svgOnly === undefined,
                windowAppletIdPresent: window.appletId === document.getElementById("appletId"),
                allAppletIdPresent: document.all.appletId === document.getElementById("appletId"),
                windowSvgIdPresent: window.svgId === document.getElementById("svgId"),
                allSvgIdPresent: document.all.svgId === document.getElementById("svgId"),
                detachedAppletNameAbsent: detached.all.detachedApplet === undefined,
                detachedAppletIdPresent:
                  detached.all.detachedAppletId === detached.getElementById("detachedAppletId"),
                detachedFormNamePresent:
                  detached.all.detachedForm === detached.getElementById("detachedFormId")
              });
            })()
            "#,
        )
        .expect("legacy named access candidate filtering should evaluate");

    assert_eq!(
        result,
        r#"{"windowAppletNameAbsent":true,"allAppletNameAbsent":true,"windowSharedUsesEligibleForm":true,"allSharedUsesEligibleForm":true,"windowAnchorNameAbsent":true,"allAnchorNamePresent":true,"windowImageNamePresent":true,"allImageNamePresent":true,"windowDivNameAbsent":true,"allDivNameAbsent":true,"windowSvgNameAbsent":true,"allSvgNameAbsent":true,"windowAppletIdPresent":true,"allAppletIdPresent":true,"windowSvgIdPresent":true,"allSvgIdPresent":true,"detachedAppletNameAbsent":true,"detachedAppletIdPresent":true,"detachedFormNamePresent":true}"#
    );
}

#[test]
fn window_named_access_returns_a_live_deduplicated_collection_for_multiple_matches() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const first = document.createElement("img");
              first.name = "multi";
              const duplicate = document.createElement("form");
              duplicate.id = "multi";
              duplicate.name = "multi";
              const rejected = document.createElement("span");
              rejected.setAttribute("name", "multi");
              const idOnly = document.createElement("div");
              idOnly.id = "multi";
              document.body.append(first, duplicate, rejected, idOnly);

              const collection = window.multi;
              const initial = {
                isCollection: collection instanceof HTMLCollection,
                cachedIdentity: window.multi === collection,
                deduplicatedAndFiltered:
                  collection.length === 3 &&
                  collection[0] === first &&
                  collection[1] === duplicate &&
                  collection[2] === idOnly
              };

              const inserted = document.createElement("object");
              inserted.name = "multi";
              document.body.insertBefore(inserted, first);
              const liveAfterInsert =
                collection.length === 4 &&
                collection[0] === inserted &&
                collection[1] === first &&
                collection[2] === duplicate &&
                collection[3] === idOnly;

              first.remove();
              idOnly.id = "";
              inserted.name = "";
              return JSON.stringify({
                ...initial,
                liveAfterInsert,
                liveAfterRemoval:
                  collection.length === 1 && collection[0] === duplicate,
                getterCollapsesToElement: window.multi === duplicate
              });
            })()
            "#,
        )
        .expect("window named multi-match collection should evaluate");

    assert_eq!(
        result,
        r#"{"isCollection":true,"cachedIdentity":true,"deduplicatedAndFiltered":true,"liveAfterInsert":true,"liveAfterRemoval":true,"getterCollapsesToElement":true}"#
    );
}

#[test]
fn live_html_collection_enforces_brand_and_legacy_named_property_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><p id=\"named\"></p></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const collection = document.getElementsByTagName("p");
              const element = document.getElementById("named");
              const derived = Object.create(collection);
              const detachedCollection = new DOMParser()
                .parseFromString("<html><body><span></span></body></html>", "text/html")
                .getElementsByTagName("span");
              const lengthGetter = Object.getOwnPropertyDescriptor(
                HTMLCollection.prototype,
                "length"
              ).get;
              let derivedLengthError = null;
              try {
                lengthGetter.call(derived);
              } catch (error) {
                derivedLengthError = error.name;
              }
              derived.named = "derived expando";
              collection.named = "ignored";
              let strictSetError = null;
              try {
                (() => {
                  "use strict";
                  collection.named = "ignored";
                })();
              } catch (error) {
                strictSetError = error.name;
              }
              collection.unsupported = "collection expando";
              const namedDescriptor = Object.getOwnPropertyDescriptor(collection, "named");
              return JSON.stringify({
                inheritedNamed: Object.getPrototypeOf(derived).named === element,
                derivedOwnNamed: Object.hasOwn(derived, "named"),
                derivedNamed: derived.named,
                collectionNamedPreserved: collection.named === element,
                strictSetError,
                unsupportedExpando: collection.unsupported,
                namedDescriptor: {
                  writable: namedDescriptor.writable,
                  enumerable: namedDescriptor.enumerable,
                  configurable: namedDescriptor.configurable
                },
                collectionLength: lengthGetter.call(collection),
                detachedCollectionLength: lengthGetter.call(detachedCollection),
                derivedLengthError
              });
            })()
            "#,
        )
        .expect("HTMLCollection legacy platform object semantics should evaluate");

    assert_eq!(
        result,
        r#"{"inheritedNamed":true,"derivedOwnNamed":true,"derivedNamed":"derived expando","collectionNamedPreserved":true,"strictSetError":"TypeError","unsupportedExpando":"collection expando","namedDescriptor":{"writable":false,"enumerable":false,"configurable":true},"collectionLength":1,"detachedCollectionLength":1,"derivedLengthError":"TypeError"}"#
    );
}

#[test]
fn html_collection_prototype_members_follow_webidl_enumeration() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><body><form id=\"first\"></form><form id=\"second\"></form></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const forms = document.forms;
              const descriptorEnumerability = ["length", "item", "namedItem"].map(
                name => Object.getOwnPropertyDescriptor(
                  HTMLCollection.prototype,
                  name
                ).enumerable
              );
              const iteratorEnumerable = Object.getOwnPropertyDescriptor(
                HTMLCollection.prototype,
                Symbol.iterator
              ).enumerable;
              const enumerated = [];
              for (const name in forms) {
                enumerated.push(name);
              }
              return JSON.stringify({
                descriptorEnumerability,
                iteratorEnumerable,
                indices: enumerated.splice(0, 2),
                prototypeMembers: enumerated.sort()
              });
            })()
            "#,
        )
        .expect("HTMLCollection enumeration checks should evaluate");

    assert_eq!(
        result,
        r#"{"descriptorEnumerability":[true,true,true],"iteratorEnumerable":false,"indices":["0","1"],"prototypeMembers":["item","length","namedItem"]}"#
    );
}

#[test]
fn live_html_collection_iterator_observes_mutations_between_steps() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><body><div id=\"host\"><span id=\"initial\"></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.getElementById("host");
              const collection = host.getElementsByTagName("span");
              const iterator = collection[Symbol.iterator]();

              const replacement = document.createElement("span");
              replacement.id = "replacement";
              document.getElementById("initial").replaceWith(replacement);
              const first = iterator.next();

              const appended = document.createElement("span");
              appended.id = "appended";
              host.appendChild(appended);
              const second = iterator.next();

              appended.remove();
              const third = iterator.next();

              return JSON.stringify({
                first: [first.done, first.value && first.value.id],
                second: [second.done, second.value && second.value.id],
                thirdDone: third.done,
                finalIds: Array.from(collection, element => element.id)
              });
            })()
            "#,
        )
        .expect("live HTMLCollection iterator mutation checks should evaluate");

    assert_eq!(
        result,
        r#"{"first":[false,"replacement"],"second":[false,"appended"],"thirdDone":true,"finalIds":["replacement"]}"#
    );
}

#[test]
fn live_document_all_declared_members_ignore_public_data_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const all = document.all;
              const probe = document.getElementById("probe");
              const summarize = name => {
                const descriptor = Object.getOwnPropertyDescriptor(all, name);
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const summarizePrototype = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  HTMLAllCollection.prototype,
                  name
                );
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const beforeNames = Object.getOwnPropertyNames(all).includes("data");
              all.data = {
                items: [],
                named: { probe: null }
              };
              return [
                summarize("length"),
                summarize("item"),
                summarize("namedItem"),
                summarizePrototype(Symbol.iterator),
                Object.prototype.hasOwnProperty.call(all, Symbol.iterator),
                beforeNames,
                Object.prototype.hasOwnProperty.call(all, "data"),
                all.data && Array.isArray(all.data.items),
                all.item(0) && all.item(0).tagName,
                all.namedItem("probe") === probe,
                typeof all[Symbol.iterator]
              ].join("|");
            })()
            "#,
        )
        .expect("document.all declared surface spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:false:number|true:false:true:true:function|true:false:true:true:function|true:false:true:true:function|false|false|true|true|HTML|true|function"
    );
}

#[test]
fn document_write_parses_variadic_webidl_strings() {
    let mut vm = new_storage_test_vm("https://document-write-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              document.write(
                "<main id='",
                { toString() { return "written"; } },
                "'>value:",
                null,
                "</main>"
              );
              document.close();
              return [
                document.getElementById("written")?.textContent,
                probe(() => document.write(Symbol("chunk"))),
                document.getElementById("written")?.textContent
              ].join("|");
            })()
            "#,
        )
        .expect("Document.write WebIDL variadic argument probe should evaluate");

    assert_eq!(result, "value:null|TypeError|value:null");
}

#[test]
fn class_name_collections_follow_the_owner_document_quirks_mode() {
    for (doctype, mode, lowercase_matches) in [
        ("", "BackCompat", 2),
        ("<!doctype html>", "CSS1Compat", 1),
        (
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#,
            "CSS1Compat",
            1,
        ),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://class-name-queries.test/",
            &format!(
                "{doctype}<main id='root'><i class='FOO BAR'></i><i class='foo bar'></i><i class='É'></i></main>"
            ),
        );
        assert_eq!(vm.eval(r#"(() => {
            const root = document.getElementById('root');
            const live = root.getElementsByClassName('foo bar');
            const before = live.length;
            root.firstElementChild.setAttribute('class', 'other');
            const xml = new DOMParser().parseFromString('<root><item class="FOO BAR"/></root>', 'application/xml');
            return [document.compatMode, document.getElementsByClassName('foo bar').length,
                before, live.length, root.getElementsByClassName('é').length,
                xml.getElementsByClassName('foo bar').length,
                xml.getElementsByClassName('FOO BAR').length].join('|');
        })()"#).unwrap(), format!("{mode}|1|{lowercase_matches}|1|0|0|1"), "doctype {doctype}");
    }
}
