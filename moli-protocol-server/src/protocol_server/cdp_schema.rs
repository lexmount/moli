use std::sync::OnceLock;

use moli_protocol_cdp::CDP_PROTOCOL_JSON;
use serde_json::Value;

// Browser events emitted by the protocol owner and renderer agents. V8 events
// are forwarded using the matching JS protocol schema below.
const IMPLEMENTED_BROWSER_EVENTS: &[(&str, &[&str])] = &[
    ("Audits", &["issueAdded"]),
    ("Browser", &["downloadProgress", "downloadWillBegin"]),
    ("CSS", &["styleSheetAdded", "styleSheetRemoved"]),
    (
        "DOM",
        &[
            "attributeModified",
            "attributeRemoved",
            "characterDataModified",
            "childNodeCountUpdated",
            "childNodeInserted",
            "childNodeRemoved",
            "documentUpdated",
            "setChildNodes",
        ],
    ),
    (
        "DOMStorage",
        &[
            "domStorageItemAdded",
            "domStorageItemRemoved",
            "domStorageItemUpdated",
            "domStorageItemsCleared",
        ],
    ),
    ("Fetch", &["authRequired", "requestPaused"]),
    ("Input", &["dragIntercepted"]),
    (
        "Inspector",
        &[
            "detached",
            "targetCrashed",
            "targetReloadedAfterCrash",
            "workerScriptLoaded",
        ],
    ),
    ("Log", &["entryAdded"]),
    (
        "Network",
        &[
            "dataReceived",
            "eventSourceMessageReceived",
            "loadingFailed",
            "loadingFinished",
            "requestServedFromCache",
            "requestWillBeSent",
            "requestWillBeSentExtraInfo",
            "responseReceived",
            "responseReceivedExtraInfo",
            "webSocketClosed",
            "webSocketCreated",
            "webSocketFrameError",
            "webSocketFrameReceived",
            "webSocketFrameSent",
            "webSocketHandshakeResponseReceived",
            "webSocketWillSendHandshakeRequest",
        ],
    ),
    (
        "Page",
        &[
            "documentOpened",
            "domContentEventFired",
            "downloadProgress",
            "downloadWillBegin",
            "fileChooserOpened",
            "frameAttached",
            "frameClearedScheduledNavigation",
            "frameDetached",
            "frameNavigated",
            "frameRequestedNavigation",
            "frameScheduledNavigation",
            "frameStartedLoading",
            "frameStartedNavigating",
            "frameStoppedLoading",
            "javascriptDialogClosed",
            "javascriptDialogOpening",
            "lifecycleEvent",
            "loadEventFired",
            "navigatedWithinDocument",
            "screencastFrame",
            "screencastVisibilityChanged",
            "windowOpen",
        ],
    ),
    (
        "ServiceWorker",
        &[
            "workerErrorReported",
            "workerRegistrationUpdated",
            "workerVersionUpdated",
        ],
    ),
    (
        "Target",
        &[
            "attachedToTarget",
            "detachedFromTarget",
            "receivedMessageFromTarget",
            "targetCrashed",
            "targetCreated",
            "targetDestroyed",
            "targetInfoChanged",
        ],
    ),
    ("Tracing", &["dataCollected", "tracingComplete"]),
    (
        "WebMCP",
        &["toolInvoked", "toolResponded", "toolsAdded", "toolsRemoved"],
    ),
];

/// Full upstream definitions, filtered to the methods Moli implements. Keep all
/// types, including type-only domains, so cross-domain references stay valid.
pub(super) fn advertised_protocol_json() -> &'static str {
    static JSON: OnceLock<String> = OnceLock::new();
    JSON.get_or_init(|| {
        let mut protocol: Value =
            serde_json::from_str(CDP_PROTOCOL_JSON).expect("embedded CDP protocol is valid JSON");
        let extensions: Value = serde_json::from_str(include_str!("protocol_extensions.json"))
            .expect("Moli protocol extensions are valid JSON");
        let domains = protocol["domains"].as_array_mut().expect("CDP domains");
        for extension in extensions["domains"].as_array().expect("extension domains") {
            let domain = domains
                .iter_mut()
                .find(|domain| domain["domain"] == extension["domain"])
                .expect("extension has an upstream domain");
            for section in ["commands", "types"] {
                if let Some(entries) = extension[section].as_array() {
                    let target = domain
                        .as_object_mut()
                        .expect("CDP domain")
                        .entry(section)
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()
                        .expect("CDP definitions");
                    let key = if section == "types" { "id" } else { "name" };
                    for entry in entries {
                        assert!(
                            !target.iter().any(|item| item[key] == entry[key]),
                            "Moli extensions must not replace upstream definitions"
                        );
                        target.push(entry.clone());
                    }
                }
            }
        }
        let commands = moli_protocol::implemented_cdp_commands();
        for (name, supported) in &commands {
            let domain = domains
                .iter()
                .find(|domain| domain["domain"] == *name)
                .expect("implemented domain has a schema");
            for command in *supported {
                assert!(
                    domain["commands"]
                        .as_array()
                        .expect("domain commands")
                        .iter()
                        .any(|entry| entry["name"] == *command),
                    "missing schema for {name}.{command}"
                );
            }
        }
        for (name, supported) in IMPLEMENTED_BROWSER_EVENTS {
            let domain = domains
                .iter()
                .find(|domain| domain["domain"] == *name)
                .expect("event domain has a schema");
            for event in *supported {
                assert!(
                    domain["events"]
                        .as_array()
                        .expect("domain events")
                        .iter()
                        .any(|entry| entry["name"] == *event),
                    "missing schema for {name}.{event}"
                );
            }
        }
        for domain in domains {
            let name = domain["domain"].as_str().expect("domain name").to_owned();
            if let Some(entries) = domain["commands"].as_array_mut() {
                entries.retain(|entry| {
                    let command = entry["name"].as_str().expect("command name");
                    // Debugger is forwarded directly to V8. Browser.close is
                    // handled by the server, outside BrowserAction.
                    name == "Debugger"
                        || (name == "Browser" && command == "close")
                        || commands.iter().any(|(domain, supported)| {
                            *domain == name && supported.contains(&command)
                        })
                });
            }
            if let Some(entries) = domain["events"].as_array_mut() {
                entries.retain(|entry| {
                    // These events come from V8's Inspector session.
                    matches!(
                        name.as_str(),
                        "Console" | "Debugger" | "HeapProfiler" | "Profiler" | "Runtime"
                    ) || IMPLEMENTED_BROWSER_EVENTS
                        .iter()
                        .any(|(domain, supported)| {
                            *domain == name
                                && supported.contains(&entry["name"].as_str().expect("event name"))
                        })
                });
            }
        }
        serde_json::to_string(&protocol).expect("advertised protocol serializes")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn domain<'a>(protocol: &'a Value, name: &str) -> &'a Value {
        protocol["domains"]
            .as_array()
            .unwrap()
            .iter()
            .find(|domain| domain["domain"] == name)
            .unwrap_or_else(|| panic!("missing domain {name}"))
    }

    fn definition<'a>(domain: &'a Value, section: &str, name: &str) -> &'a Value {
        domain[section]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("missing {}.{name}", domain["domain"]))
    }

    #[test]
    fn advertised_commands_and_events_keep_their_complete_definitions() {
        let advertised: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
        let upstream: Value = serde_json::from_str(CDP_PROTOCOL_JSON).unwrap();
        let extensions: Value =
            serde_json::from_str(include_str!("protocol_extensions.json")).unwrap();
        for advertised_domain in advertised["domains"].as_array().unwrap() {
            let name = advertised_domain["domain"].as_str().unwrap();
            for section in ["commands", "events"] {
                for entry in advertised_domain[section].as_array().into_iter().flatten() {
                    let original = [&upstream, &extensions]
                        .into_iter()
                        .flat_map(|source| source["domains"].as_array().unwrap())
                        .filter(|domain| domain["domain"] == name)
                        .flat_map(|domain| domain[section].as_array().into_iter().flatten())
                        .find(|original| original["name"] == entry["name"])
                        .unwrap();
                    assert_eq!(entry, original, "{name}.{section} must not lose its schema");
                }
            }
        }
        let navigate = definition(domain(&advertised, "Page"), "commands", "navigate");
        assert!(
            navigate["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| { p["name"] == "url" && p["type"] == "string" && p["optional"] != true })
        );
        assert!(
            navigate["returns"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| { p["name"] == "frameId" && p["$ref"] == "FrameId" })
        );
        for (name, event, parameter) in [
            ("Page", "loadEventFired", "timestamp"),
            ("Network", "requestWillBeSent", "request"),
            ("Target", "attachedToTarget", "sessionId"),
        ] {
            assert!(
                definition(domain(&advertised, name), "events", event)["parameters"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["name"] == parameter)
            );
        }
    }

    #[test]
    fn advertised_tracing_events_cover_both_transfer_modes() {
        let advertised: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
        let upstream: Value = serde_json::from_str(CDP_PROTOCOL_JSON).unwrap();
        for event in ["dataCollected", "tracingComplete"] {
            assert_eq!(
                definition(domain(&advertised, "Tracing"), "events", event),
                definition(domain(&upstream, "Tracing"), "events", event),
            );
        }
    }

    #[test]
    fn advertised_commands_match_the_dispatcher_and_server() {
        let advertised: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
        let upstream: Value = serde_json::from_str(CDP_PROTOCOL_JSON).unwrap();
        let mut expected = BTreeSet::from(["Browser.close".to_owned()]);
        for (domain, commands) in moli_protocol::implemented_cdp_commands() {
            expected.extend(commands.iter().map(|command| format!("{domain}.{command}")));
        }
        expected.extend(
            domain(&upstream, "Debugger")["commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|command| format!("Debugger.{}", command["name"].as_str().unwrap())),
        );
        let actual: BTreeSet<_> = advertised["domains"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|domain| {
                domain["commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|command| {
                        format!(
                            "{}.{}",
                            domain["domain"].as_str().unwrap(),
                            command["name"].as_str().unwrap()
                        )
                    })
            })
            .collect();
        assert_eq!(actual, expected);
        // A type-only domain must not expose unsupported upstream operations.
        assert!(
            domain(&advertised, "Animation")["commands"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            domain(&advertised, "Animation")["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn advertised_schema_has_unique_definitions_and_resolvable_references() {
        fn check_refs(value: &Value, domain: &str, types: &BTreeSet<String>) {
            match value {
                Value::Object(object) => {
                    if let Some(reference) = object.get("$ref") {
                        let reference = reference.as_str().unwrap();
                        let qualified = if reference.contains('.') {
                            reference.to_owned()
                        } else {
                            format!("{domain}.{reference}")
                        };
                        assert!(types.contains(&qualified), "unresolved $ref: {qualified}");
                    }
                    for child in object.values() {
                        check_refs(child, domain, types);
                    }
                }
                Value::Array(array) => {
                    for child in array {
                        check_refs(child, domain, types);
                    }
                }
                _ => {}
            }
        }
        let protocol: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
        let domains = protocol["domains"].as_array().unwrap();
        let mut names = BTreeSet::new();
        let mut types = BTreeSet::new();
        for domain in domains {
            let name = domain["domain"].as_str().unwrap();
            assert!(names.insert(name));
            for (section, key) in [("types", "id"), ("commands", "name"), ("events", "name")] {
                let mut definitions = BTreeSet::new();
                for entry in domain[section].as_array().into_iter().flatten() {
                    let id = entry[key].as_str().unwrap();
                    assert!(definitions.insert(id), "duplicate {name}.{section}.{id}");
                    if section == "types" {
                        assert!(types.insert(format!("{name}.{id}")));
                    }
                }
            }
        }
        for domain in domains {
            check_refs(domain, domain["domain"].as_str().unwrap(), &types);
            for dependency in domain["dependencies"].as_array().into_iter().flatten() {
                assert!(names.contains(dependency.as_str().unwrap()));
            }
        }
    }
}
