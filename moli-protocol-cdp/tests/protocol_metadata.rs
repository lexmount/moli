use std::{collections::BTreeSet, io::Read};

use moli_protocol_cdp::{CDP_PROTOCOL_BROTLI, advertised_protocol_json};
use moli_protocol_cdp_defs::{
    FORWARDED_EVENT_DOMAINS, IMPLEMENTED_BROWSER_EVENTS, IMPLEMENTED_COMMANDS, SERVER_COMMANDS,
};
use serde_json::{Value, json};

#[path = "../build/schema.rs"]
mod schema;

fn upstream_protocol_schema() -> Value {
    let mut browser: Value =
        serde_json::from_str(include_str!("../protocol/browser_protocol.json")).unwrap();
    let mut javascript: Value =
        serde_json::from_str(include_str!("../protocol/js_protocol.json")).unwrap();
    browser["domains"]
        .as_array_mut()
        .unwrap()
        .append(javascript["domains"].as_array_mut().unwrap());
    browser
}

fn extensions() -> Value {
    serde_json::from_str(include_str!("../protocol/protocol_extensions.json")).unwrap()
}

#[test]
fn embedded_brotli_protocol_round_trips_byte_for_byte() {
    let compact = include_bytes!(concat!(env!("OUT_DIR"), "/cdp-protocol.json"));
    let mut decoded = Vec::new();
    brotlic::DecompressorReader::new(CDP_PROTOCOL_BROTLI)
        .read_to_end(&mut decoded)
        .expect("decompress embedded protocol");
    assert_eq!(decoded, compact.as_slice());
    assert_eq!(advertised_protocol_json().as_bytes(), compact);
    assert!(CDP_PROTOCOL_BROTLI.len() < compact.len() / 2);
}

#[test]
fn advertised_schema_preserves_domain_metadata_and_all_types() {
    let advertised: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
    let upstream = upstream_protocol_schema();
    let extensions = extensions();
    assert_eq!(advertised["version"], upstream["version"]);
    assert_eq!(
        advertised["domains"].as_array().unwrap().len(),
        upstream["domains"].as_array().unwrap().len()
    );
    for actual in advertised["domains"].as_array().unwrap() {
        let name = actual["domain"].as_str().unwrap();
        let mut expected = domain(&upstream, name).clone();
        for extension in extensions["domains"].as_array().unwrap() {
            if extension["domain"] == name
                && let Some(types) = extension["types"].as_array()
            {
                expected
                    .as_object_mut()
                    .unwrap()
                    .entry("types")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .unwrap()
                    .extend(types.iter().cloned());
            }
        }
        let mut actual = actual.clone();
        for section in ["commands", "events"] {
            expected.as_object_mut().unwrap().remove(section);
            actual.as_object_mut().unwrap().remove(section);
        }
        assert_eq!(actual, expected, "domain metadata and types for {name}");
    }
}

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
    let upstream = upstream_protocol_schema();
    let extensions: Value =
        serde_json::from_str(include_str!("../protocol/protocol_extensions.json")).unwrap();
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
    let upstream = upstream_protocol_schema();
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
    let upstream = upstream_protocol_schema();
    let mut expected = BTreeSet::new();
    for (domain, commands) in IMPLEMENTED_COMMANDS.iter().chain(SERVER_COMMANDS) {
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
            if let Some(entries) = domain.get(section) {
                assert!(entries.is_array(), "{name}.{section} must be an array");
            }
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

#[test]
fn advertised_events_match_the_browser_catalog_and_v8_forwarding() {
    let advertised: Value = serde_json::from_str(advertised_protocol_json()).unwrap();
    let upstream = upstream_protocol_schema();
    let extensions = extensions();
    let mut expected: BTreeSet<_> = IMPLEMENTED_BROWSER_EVENTS
        .iter()
        .flat_map(|(name, events)| events.iter().map(move |event| format!("{name}.{event}")))
        .collect();
    for source in [&upstream, &extensions] {
        for domain in source["domains"].as_array().unwrap() {
            let name = domain["domain"].as_str().unwrap();
            if FORWARDED_EVENT_DOMAINS.contains(&name) {
                expected.extend(
                    domain["events"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|event| format!("{name}.{}", event["name"].as_str().unwrap())),
                );
            }
        }
    }
    let actual: BTreeSet<_> = advertised["domains"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|domain| {
            domain["events"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|event| {
                    format!(
                        "{}.{}",
                        domain["domain"].as_str().unwrap(),
                        event["name"].as_str().unwrap()
                    )
                })
        })
        .collect();
    assert_eq!(actual, expected);
}

fn domain_mut<'a>(protocol: &'a mut Value, name: &str) -> &'a mut Value {
    protocol["domains"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|domain| domain["domain"] == name)
        .unwrap()
}

fn take_definition(protocol: &mut Value, name: &str, section: &str, member: &str) -> Value {
    let entries = domain_mut(protocol, name)[section].as_array_mut().unwrap();
    let index = entries
        .iter()
        .position(|entry| entry["name"] == member)
        .unwrap();
    entries.remove(index)
}

#[test]
fn build_generation_merges_supported_event_extensions() {
    let mut protocol = upstream_protocol_schema();
    let event = take_definition(&mut protocol, "Page", "events", "loadEventFired");
    let mut extensions = extensions();
    extensions["domains"].as_array_mut().unwrap().push(json!({
        "domain": "Page", "events": [event.clone()]
    }));
    schema::prepare_protocol(&mut protocol, &extensions);
    assert_eq!(
        definition(domain(&protocol, "Page"), "events", "loadEventFired"),
        &event
    );
}

#[test]
#[should_panic(expected = "missing schema for Page.navigate")]
fn build_generation_rejects_missing_command_definitions() {
    let mut protocol = upstream_protocol_schema();
    take_definition(&mut protocol, "Page", "commands", "navigate");
    schema::prepare_protocol(&mut protocol, &extensions());
}

#[test]
#[should_panic(expected = "missing schema for Browser.close")]
fn build_generation_rejects_missing_server_command_definitions() {
    let mut protocol = upstream_protocol_schema();
    take_definition(&mut protocol, "Browser", "commands", "close");
    schema::prepare_protocol(&mut protocol, &extensions());
}

#[test]
#[should_panic(expected = "missing schema for Tracing.tracingComplete")]
fn build_generation_rejects_missing_event_definitions() {
    let mut protocol = upstream_protocol_schema();
    take_definition(&mut protocol, "Tracing", "events", "tracingComplete");
    schema::prepare_protocol(&mut protocol, &extensions());
}

#[test]
#[should_panic(expected = "Moli extensions must not replace upstream definitions")]
fn build_generation_rejects_event_extension_collisions() {
    let mut protocol = upstream_protocol_schema();
    let event = definition(domain(&protocol, "Page"), "events", "loadEventFired").clone();
    let mut extensions = extensions();
    extensions["domains"].as_array_mut().unwrap().push(json!({
        "domain": "Page", "events": [event]
    }));
    schema::prepare_protocol(&mut protocol, &extensions);
}

#[test]
#[should_panic(expected = "duplicate Page.commands.navigate")]
fn build_generation_rejects_duplicate_definitions() {
    let mut protocol = upstream_protocol_schema();
    let command = definition(domain(&protocol, "Page"), "commands", "navigate").clone();
    domain_mut(&mut protocol, "Page")["commands"]
        .as_array_mut()
        .unwrap()
        .push(command);
    schema::prepare_protocol(&mut protocol, &extensions());
}

#[test]
#[should_panic(expected = "unresolved $ref: Page.MissingForTest")]
fn build_generation_rejects_unresolved_type_references() {
    let mut protocol = upstream_protocol_schema();
    domain_mut(&mut protocol, "Page")["types"]
        .as_array_mut()
        .unwrap()[0]["$ref"] = json!("MissingForTest");
    schema::prepare_protocol(&mut protocol, &extensions());
}

#[test]
#[should_panic(expected = "unresolved domain dependency MissingForTest")]
fn build_generation_rejects_unresolved_domain_dependencies() {
    let mut protocol = upstream_protocol_schema();
    domain_mut(&mut protocol, "Page")
        .as_object_mut()
        .unwrap()
        .entry("dependencies")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap()
        .push(json!("MissingForTest"));
    schema::prepare_protocol(&mut protocol, &extensions());
}
