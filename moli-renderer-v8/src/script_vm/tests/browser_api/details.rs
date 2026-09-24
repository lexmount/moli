use super::*;

#[tokio::test]
async fn details_toggle_events_are_queued_coalesced_and_include_parser_changes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://details-toggle-events.test/");

    let before = vm
        .eval(
            r#"
            (() => {
              const details = document.createElement("details");
              const parsed = new DOMParser().parseFromString(
                "<details open></details>",
                "text/html"
              ).querySelector("details");
              globalThis.__lmDetailsToggleEvents = [];
              const record = prefix => event => {
                globalThis.__lmDetailsToggleEvents.push([
                  prefix,
                  `${event.oldState}->${event.newState}`,
                  event.isTrusted,
                  event.bubbles,
                  event.cancelable,
                  event.source === null,
                  Object.getPrototypeOf(event) === ToggleEvent.prototype
                ].join(":"));
              };
              details.addEventListener("toggle", record("live"));
              parsed.addEventListener("toggle", record("parser"));
              details.open = true;
              details.removeAttribute("open");
              details.setAttribute("open", "");
              details.open = true;
              const orderFirst = document.createElement("details");
              const orderSecond = document.createElement("details");
              orderFirst.addEventListener("toggle", record("order-first"));
              orderSecond.addEventListener("toggle", record("order-second"));
              orderFirst.open = true;
              orderSecond.open = true;
              orderFirst.open = false;
              return [details.open, parsed.open, __lmDetailsToggleEvents.length].join("|");
            })()
            "#,
        )
        .expect("details toggle event setup should evaluate");

    assert_eq!(before, "true|true|0");
    assert!(
        !vm.has_ready_timeout(),
        "details toggle events must not acquire PageTimer descriptors"
    );
    for expected in 1..=4 {
        assert!(
            vm.run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::ElementToggle
            )
            .await
            .expect("queued details toggle task should run"),
            "details toggle task {expected} should remain in the production DOM source"
        );
        assert_eq!(
            vm.eval("String(__lmDetailsToggleEvents.length)")
                .expect("details toggle count should evaluate"),
            expected.to_string(),
            "one selected source task must dispatch exactly one toggle event"
        );
    }
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle
        )
        .await
        .expect("drained details toggle source should be observable")
    );

    let after = vm
        .eval("__lmDetailsToggleEvents.join('|')")
        .expect("details toggle events should be observable");
    assert_eq!(
        after,
        "parser:closed->open:true:false:false:true:true|live:closed->open:true:false:false:true:true|order-second:closed->open:true:false:false:true:true|order-first:closed->closed:true:false:false:true:true"
    );
}

#[test]
fn details_name_groups_enforce_exclusivity_within_each_tree_root() {
    let mut vm = new_storage_test_vm("https://details-name-groups.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              const states = elements => elements.map(element => Number(element.open));
              const root = document.createElement("div");
              root.innerHTML = `
                <details id="first" name="a" open></details>
                <details id="second" name="a" open></details>
              `;
              const first = root.querySelector("#first");
              const second = root.querySelector("#second");
              const parserStates = states([first, second]);

              second.open = true;
              const directOpenStates = states([first, second]);

              const inserted = document.createElement("details");
              inserted.name = "a";
              inserted.open = true;
              root.insertBefore(inserted, first);
              const insertionStates = states([inserted, first, second]);

              const renamed = document.createElement("details");
              renamed.name = "b";
              renamed.open = true;
              root.appendChild(renamed);
              renamed.name = "a";
              const renameStates = states([second, renamed]);

              const nestedContainer = document.createElement("section");
              const nested = document.createElement("details");
              nested.name = "a";
              nested.open = true;
              nestedContainer.append(nested);
              root.append(nestedContainer);
              const nestedInsertionStates = states([second, nested]);

              const host = document.createElement("div");
              const shadow = host.attachShadow({mode: "open"});
              shadow.innerHTML = `
                <details name="a" open></details>
                <details name="a" open></details>
              `;
              const shadowDetails = Array.from(shadow.querySelectorAll("details"));
              shadowDetails[1].open = true;
              const treeScopeStates = states([second, ...shadowDetails]);

              const emptyNameA = document.createElement("details");
              const emptyNameB = document.createElement("details");
              root.append(emptyNameA, emptyNameB);
              emptyNameA.open = true;
              emptyNameB.open = true;
              const emptyNameStates = states([emptyNameA, emptyNameB]);

              const parsed = new DOMParser().parseFromString(
                `<details name="a" open></details><details name="a" open></details>`,
                "text/html"
              );
              const detachedParserStates = states(Array.from(parsed.querySelectorAll("details")));

              return JSON.stringify({
                parserStates,
                directOpenStates,
                insertionStates,
                renameStates,
                nestedInsertionStates,
                treeScopeStates,
                emptyNameStates,
                detachedParserStates
              });
            })()
            "##,
        )
        .expect("details name group probe should evaluate");

    assert_eq!(
        result,
        r#"{"parserStates":[1,0],"directOpenStates":[0,1],"insertionStates":[0,0,1],"renameStates":[1,0],"nestedInsertionStates":[1,0],"treeScopeStates":[1,0,1],"emptyNameStates":[1,1],"detachedParserStates":[1,0]}"#
    );
}
