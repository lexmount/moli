use std::collections::BTreeSet;

use moli_protocol_cdp_defs::{
    DomainMembers, FORWARDED_COMMAND_DOMAINS, FORWARDED_EVENT_DOMAINS, IMPLEMENTED_BROWSER_EVENTS,
    IMPLEMENTED_COMMANDS, SERVER_COMMANDS,
};
use serde_json::Value;

pub fn prepare_protocol(protocol: &mut Value, extensions: &Value) {
    validate_protocol(protocol);
    let domains = protocol["domains"].as_array_mut().expect("CDP domains");
    for extension in extensions["domains"].as_array().expect("extension domains") {
        let domain = domains
            .iter_mut()
            .find(|domain| domain["domain"] == extension["domain"])
            .expect("extension has an upstream domain");
        for section in ["commands", "events", "types"] {
            if let Some(entries) = extension.get(section) {
                let target = domain
                    .as_object_mut()
                    .expect("CDP domain")
                    .entry(section)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("CDP definitions");
                let key = if section == "types" { "id" } else { "name" };
                for entry in entries.as_array().expect("extension definitions") {
                    assert!(
                        !target.iter().any(|item| item[key] == entry[key]),
                        "Moli extensions must not replace upstream definitions"
                    );
                    target.push(entry.clone());
                }
            }
        }
    }

    for commands in [IMPLEMENTED_COMMANDS, SERVER_COMMANDS] {
        validate_supported_definitions(domains, "commands", commands);
    }
    validate_supported_definitions(domains, "events", IMPLEMENTED_BROWSER_EVENTS);
    for (section, forwarded) in [
        ("commands", FORWARDED_COMMAND_DOMAINS),
        ("events", FORWARDED_EVENT_DOMAINS),
    ] {
        for name in forwarded {
            let domain = required_domain(domains, name);
            domain[section]
                .as_array()
                .expect("forwarded domain definitions");
        }
    }

    for domain in domains {
        let name = domain["domain"].as_str().expect("domain name").to_owned();
        if let Some(entries) = domain.get_mut("commands").and_then(Value::as_array_mut) {
            entries.retain(|entry| {
                let command = entry["name"].as_str().expect("command name");
                FORWARDED_COMMAND_DOMAINS.contains(&name.as_str())
                    || supports(IMPLEMENTED_COMMANDS, &name, command)
                    || supports(SERVER_COMMANDS, &name, command)
            });
        }
        if let Some(entries) = domain.get_mut("events").and_then(Value::as_array_mut) {
            entries.retain(|entry| {
                let event = entry["name"].as_str().expect("event name");
                FORWARDED_EVENT_DOMAINS.contains(&name.as_str())
                    || supports(IMPLEMENTED_BROWSER_EVENTS, &name, event)
            });
        }
    }
    validate_protocol(protocol);
}

fn supports(catalog: &[DomainMembers], domain: &str, member: &str) -> bool {
    catalog
        .iter()
        .any(|(name, supported)| *name == domain && supported.contains(&member))
}

fn required_domain<'a>(domains: &'a [Value], name: &str) -> &'a Value {
    domains
        .iter()
        .find(|domain| domain["domain"] == name)
        .unwrap_or_else(|| panic!("missing schema for domain {name}"))
}

fn validate_supported_definitions(domains: &[Value], section: &str, catalog: &[DomainMembers]) {
    for (name, supported) in catalog {
        let domain = required_domain(domains, name);
        for member in *supported {
            assert!(
                definitions(domain, section)
                    .iter()
                    .any(|entry| entry["name"] == *member),
                "missing schema for {name}.{member}"
            );
        }
    }
}

fn definitions<'a>(domain: &'a Value, section: &str) -> &'a [Value] {
    domain
        .get(section)
        .map(|value| value.as_array().expect("CDP definitions").as_slice())
        .unwrap_or(&[])
}

fn validate_protocol(protocol: &Value) {
    let domains = protocol["domains"].as_array().expect("CDP domains");
    let mut names = BTreeSet::new();
    let mut types = BTreeSet::new();
    for domain in domains {
        let name = domain["domain"].as_str().expect("domain name");
        assert!(names.insert(name), "duplicate domain {name}");
        for (section, key) in [("types", "id"), ("commands", "name"), ("events", "name")] {
            let mut members = BTreeSet::new();
            for entry in definitions(domain, section) {
                let id = entry[key].as_str().expect("definition name");
                assert!(members.insert(id), "duplicate {name}.{section}.{id}");
                if section == "types" {
                    types.insert(format!("{name}.{id}"));
                }
            }
        }
    }
    for domain in domains {
        let name = domain["domain"].as_str().expect("domain name");
        check_refs(domain, name, &types);
        for dependency in definitions(domain, "dependencies") {
            let dependency = dependency.as_str().expect("domain dependency");
            assert!(
                names.contains(dependency),
                "unresolved domain dependency {dependency}"
            );
        }
    }
}

fn check_refs(value: &Value, domain: &str, types: &BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let reference = reference.as_str().expect("type reference");
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
