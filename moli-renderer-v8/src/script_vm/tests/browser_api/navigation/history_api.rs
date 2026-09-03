use super::*;

#[test]
fn history_navigation_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const probe = (callback) => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              return [
                probe(() => history.pushState()),
                probe(() => history.pushState({ ok: 1 }, Symbol("unused"), "#bad")),
                probe(() => history.pushState({ ok: 1 }, {
                  toString() {
                    throw new RangeError("unused");
                  }
                }, "#bad")),
                probe(() => history.pushState({ ok: 1 }, "", Symbol("url"))),
                probe(() => history.replaceState({ ok: 2 }, "", {
                  toString() {
                    throw new RangeError("url");
                  }
                })),
                probe(() => history.pushState({ ok: 3 }, "", "#three")),
                location.hash,
                history.state.ok,
                probe(() => navigation.navigate()),
                probe(() => navigation.navigate(Symbol("url"))),
                probe(() => navigation.navigate({
                  toString() {
                    throw new RangeError("nav-url");
                  }
                })),
                probe(() => navigation.traverseTo()),
                probe(() => navigation.traverseTo(Symbol("key"))),
                probe(() => navigation.traverseTo({
                  toString() {
                    throw new RangeError("nav-key");
                  }
                }))
              ].join("|");
            })()
            "##,
        )
        .expect("history/navigation WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|RangeError|TypeError|RangeError|undefined|#three|3|TypeError|TypeError|RangeError|TypeError|TypeError|RangeError"
    );
}

#[test]
fn history_operations_reject_a_removed_child_document() {
    let mut vm = new_storage_test_vm("https://example.com/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body || root.appendChild(document.createElement("body"));
              const frame = document.createElement("iframe");
              body.appendChild(frame);
              const childHistory = frame.contentWindow.history;
              const ChildDOMException = frame.contentWindow.DOMException;
              frame.remove();

              const probe = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return `${error.name}:${error instanceof ChildDOMException}`;
                }
              };
              return [
                probe(() => childHistory.length),
                probe(() => childHistory.scrollRestoration),
                probe(() => childHistory.state),
                probe(() => { childHistory.scrollRestoration = "manual"; }),
                probe(() => childHistory.go()),
                probe(() => childHistory.go(0)),
                probe(() => childHistory.go(-1)),
                probe(() => childHistory.go(1)),
                probe(() => childHistory.go(Infinity)),
                probe(() => childHistory.go(-Infinity)),
                probe(() => childHistory.back()),
                probe(() => childHistory.forward()),
                probe(() => childHistory.pushState(1, "", "?x=1")),
                probe(() => childHistory.replaceState(2, "", "?x=2"))
              ].join("|");
            })()
            "#,
        )
        .expect("removed child History operations should be rejected");

    assert_eq!(result, ["SecurityError:true"; 14].join("|"));
}

#[test]
fn history_argument_errors_precede_inactive_document_errors() {
    let mut vm = new_storage_test_vm("https://example.com/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body || root.appendChild(document.createElement("body"));
              const frame = document.createElement("iframe");
              body.appendChild(frame);
              const childHistory = frame.contentWindow.history;
              frame.remove();

              const conversionError = new RangeError("conversion");
              const rejected = {
                [Symbol.toPrimitive]() { throw conversionError; }
              };
              const probe = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return error === conversionError ? "conversion" : error.name;
                }
              };
              return [
                probe(() => childHistory.go(Symbol())),
                probe(() => childHistory.go(1n)),
                probe(() => childHistory.go(rejected)),
                probe(() => { childHistory.scrollRestoration = Symbol(); }),
                probe(() => { childHistory.scrollRestoration = rejected; }),
                probe(() => { childHistory.scrollRestoration = "invalid"; }),
                probe(() => childHistory.pushState()),
                probe(() => childHistory.pushState(null, Symbol())),
                probe(() => childHistory.replaceState(null, "", Symbol())),
                probe(() => childHistory.pushState(null, rejected)),
                probe(() => childHistory.replaceState(null, "", rejected)),
                probe(() => childHistory.pushState(() => {}, "")),
                probe(() => childHistory.replaceState(() => {}, ""))
              ].join("|");
            })()
            "#,
        )
        .expect("inactive History should preserve conversion and serialization error ordering");

    assert_eq!(
        result,
        "TypeError|TypeError|conversion|TypeError|conversion|no throw|TypeError|TypeError|TypeError|conversion|conversion|SecurityError|SecurityError"
    );
}

#[test]
fn history_activity_is_checked_after_argument_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body || root.appendChild(document.createElement("body"));
              const operations = [
                (history, convert) => history.go(convert(0)),
                (history, convert) => history.go(convert(-1)),
                (history, convert) => history.go(convert(Infinity)),
                (history, convert) => { history.scrollRestoration = convert("manual"); },
                (history, convert) => history.pushState(() => {}, convert("")),
                (history, convert) => history.replaceState(() => {}, "", convert("?x=2"))
              ];
              return operations.map(operation => {
                const frame = document.createElement("iframe");
                body.appendChild(frame);
                const childHistory = frame.contentWindow.history;
                let conversions = 0;
                const convert = value => ({
                  [Symbol.toPrimitive]() {
                    conversions++;
                    frame.remove();
                    return value;
                  }
                });
                try {
                  operation(childHistory, convert);
                  return "no throw";
                } catch (error) {
                  return `${error.name}:${conversions}`;
                }
              }).join("|");
            })()
            "#,
        )
        .expect("History should reject a document removed during argument conversion");

    assert_eq!(result, ["SecurityError:1"; 6].join("|"));
}

#[test]
fn history_mutation_empty_url_preserves_current_document_url() {
    let mut vm = new_storage_test_vm("https://example.com/path/page.html?query=value#initial");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const head = document.head ||
                root.insertBefore(document.createElement("head"), root.firstChild);
              const base = document.createElement("base");
              base.href = "https://example.com/different/base/";
              head.appendChild(base);

              const before = location.href;
              history.pushState({ kind: "push" }, "", "");
              const afterPush = location.href;
              history.replaceState({ kind: "replace" }, "", "");

              return JSON.stringify({
                before,
                afterPush,
                afterReplace: location.href,
                baseURI: document.baseURI,
                length: history.length,
                state: history.state.kind,
                entryUrl: navigation.currentEntry.url
              });
            })()
            "#,
        )
        .expect("empty history URL should preserve the current document URL");

    assert_eq!(
        result,
        r#"{"before":"https://example.com/path/page.html?query=value#initial","afterPush":"https://example.com/path/page.html?query=value#initial","afterReplace":"https://example.com/path/page.html?query=value#initial","baseURI":"https://example.com/different/base/","length":2,"state":"replace","entryUrl":"https://example.com/path/page.html?query=value#initial"}"#
    );
}

#[test]
fn history_state_preserves_structured_clone_values_not_representable_as_json() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const sourceBuffer = new Uint8Array([1, 2, 255]).buffer;
              const source = {
                map: new Map([["answer", 42]]),
                set: new Set(["alpha", "beta"]),
                date: new Date("2024-03-04T05:06:07.000Z"),
                buffer: sourceBuffer,
                bigint: 9007199254740993n
              };
              history.pushState(source, "", "#rich");
              source.map.set("later", 9);
              new Uint8Array(sourceBuffer)[0] = 99;
              const stored = history.state;
              return JSON.stringify({
                hash: location.hash,
                brands: [
                  stored.map instanceof Map,
                  stored.set instanceof Set,
                  stored.date instanceof Date,
                  stored.buffer instanceof ArrayBuffer
                ],
                identities: [stored !== source, stored.map !== source.map, stored.buffer !== sourceBuffer],
                map: Array.from(stored.map),
                set: Array.from(stored.set),
                date: stored.date.toISOString(),
                bytes: Array.from(new Uint8Array(stored.buffer)),
                bigint: String(stored.bigint)
              });
            })()
            "##,
        )
        .expect("rich structured-clone history state should evaluate");

    assert_eq!(
        result,
        r##"{"hash":"#rich","brands":[true,true,true,true],"identities":[true,true,true],"map":[["answer",42]],"set":["alpha","beta"],"date":"2024-03-04T05:06:07.000Z","bytes":[1,2,255],"bigint":"9007199254740993"}"##
    );
}

#[test]
fn navigation_navigate_file_url_rejects_without_pending_location_navigation() {
    let mut vm = new_storage_test_vm("https://file-navigation.test/page.html");

    let setup = vm
        .eval(
            r#"
globalThis.__fileNavigationRejectLog = [];
const result = navigation.navigate("file://");
result.committed.then(
  () => globalThis.__fileNavigationRejectLog.push("committed:fulfilled"),
  error => globalThis.__fileNavigationRejectLog.push(`committed:${error.name}`)
);
result.finished.then(
  () => globalThis.__fileNavigationRejectLog.push("finished:fulfilled"),
  error => globalThis.__fileNavigationRejectLog.push(`finished:${error.name}`)
);
undefined;
"#,
        )
        .expect("file navigation setup should evaluate");
    assert_eq!(setup, "undefined");

    let settled = vm
        .eval("globalThis.__fileNavigationRejectLog.join('|')")
        .expect("file navigation rejection log should evaluate");
    assert_eq!(settled, "committed:AbortError|finished:AbortError");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "file URL navigation rejection should not queue a pending location navigation"
    );
}

#[test]
fn navigation_navigate_seed_uses_current_document_referrer_policy() {
    let mut vm = new_storage_test_vm("https://navigation-policy.test/start.html");
    vm.set_response_referrer_policy(Some("no-referrer".to_owned()));

    let result = vm
        .eval(
            r#"
const result = navigation.navigate("/next.html");
result.committed.catch(() => {});
result.finished.catch(() => {});
"queued"
"#,
        )
        .expect("navigation policy setup should evaluate");
    assert_eq!(result, "queued");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("navigation.navigate should queue pending location navigation");
    assert_eq!(
        pending.url.as_str(),
        "https://navigation-policy.test/next.html"
    );
    let seed = pending
        .entry_seed
        .expect("cross-document navigation should carry history entry seed");
    let current_entry = seed
        .entries
        .iter()
        .find(|entry| entry.url == "https://navigation-policy.test/start.html")
        .expect("current document entry should be serialized into navigation seed");
    assert_eq!(
        current_entry.referrer_policy.as_deref(),
        Some("no-referrer"),
        "navigation seed must read the current document policy container"
    );
    let activation_from = seed
        .activation
        .as_ref()
        .and_then(|activation| activation.from.as_ref())
        .expect("same-origin navigation activation should expose from entry");
    assert_eq!(
        activation_from.referrer_policy.as_deref(),
        Some("no-referrer"),
        "activation.from should preserve the current document policy snapshot"
    );
}

#[test]
fn cross_document_pending_navigation_slot_is_not_script_writable() {
    let mut vm = new_storage_test_vm("https://cross-document-pending-slot.test/start.html");

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__lmCrossDocumentPendingSlotLog = [];
  const first = navigation.navigate("/first.html");
  first.committed.then(
    () => __lmCrossDocumentPendingSlotLog.push("firstCommitted"),
    error => __lmCrossDocumentPendingSlotLog.push(`firstCommittedRejected:${error.name}`)
  );
  first.finished.then(
    () => __lmCrossDocumentPendingSlotLog.push("firstFinished"),
    error => __lmCrossDocumentPendingSlotLog.push(`firstFinishedRejected:${error.name}`)
  );
  const exposedBefore = "__lmNavigationActiveCrossDocumentPending" in navigation;
  navigation.__lmNavigationActiveCrossDocumentPending = null;
  const second = navigation.navigate("/second.html");
  second.committed.catch(error => __lmCrossDocumentPendingSlotLog.push(`secondCommittedRejected:${error.name}`));
  second.finished.catch(error => __lmCrossDocumentPendingSlotLog.push(`secondFinishedRejected:${error.name}`));
  return JSON.stringify({
    exposedBefore,
    ownCrossDocumentSlots: Object.getOwnPropertyNames(navigation)
      .filter(name => name.startsWith("__lmCrossDocumentPending"))
      .join(","),
    publicSpoof: Object.hasOwn(navigation, "__lmNavigationActiveCrossDocumentPending"),
    log: __lmCrossDocumentPendingSlotLog.join("|")
  });
})()
"#,
        )
        .expect("cross-document pending slot setup should evaluate");

    assert_eq!(
        setup,
        r#"{"exposedBefore":false,"ownCrossDocumentSlots":"","publicSpoof":true,"log":""}"#
    );

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("second cross-document navigation should remain pending");
    assert_eq!(
        pending.url.as_str(),
        "https://cross-document-pending-slot.test/second.html"
    );

    let settled = vm
        .eval("globalThis.__lmCrossDocumentPendingSlotLog.join('|')")
        .expect("cross-document pending slot log should evaluate");
    assert_eq!(
        settled,
        "firstCommittedRejected:AbortError|firstFinishedRejected:AbortError"
    );
}

#[test]
fn location_hash_empty_fragment_serializes_empty_and_clears_target() {
    let mut vm = new_parsed_test_vm(
        "https://location-empty-fragment.test/#target",
        r##"
        <!doctype html>
        <html>
          <body>
            <a id="clear" href="#"></a>
            <div id="target"></div>
          </body>
        </html>
        "##,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const target = document.getElementById("target");
  const initial = [location.hash, target.matches(":target")].join("/");
  document.getElementById("clear").click();
  return [
    initial,
    location.href.endsWith("#"),
    location.hash,
    target.matches(":target"),
    document.querySelector(":target") === null
  ].join("|");
})()
"##,
        )
        .expect("empty fragment hash probe should evaluate");

    assert_eq!(result, "#target/true|true||false|true");
}
