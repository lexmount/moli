use super::*;

#[test]
fn maps_browsing_context_get_tree_null_root_to_shared_get_frame_trees_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.getTree",
        "params": {
            "root": null,
            "maxDepth": 2
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetFrameTrees(command) = shared else {
        panic!("expected GetFrameTrees command");
    };
    assert_eq!(command.max_depth, Some(2));
    assert_eq!(command.context.target_id, None);
}

#[test]
fn maps_browsing_context_navigate_wait_to_shared_navigation_wait() {
    for (wait, expected) in [
        (
            "none",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::None,
        ),
        (
            "interactive",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::DomContentLoaded,
        ),
        (
            "complete",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::Load,
        ),
    ] {
        let command = super::super::parse_bidi_command(json!({
            "id": 3,
            "method": "browsingContext.navigate",
            "params": {
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": wait
            }
        }))
        .expect("BiDi command");
        let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

        let shared = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect("shared command");

        let moli_protocol::devtools_runtime::DevToolsCommand::Navigate(command) = shared else {
            panic!("expected Navigate command");
        };
        assert_eq!(command.url, "https://example.test/");
        assert_eq!(command.wait, expected);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
            Some("TARGET-1")
        );
    }
}

#[test]
fn maps_browsing_context_reload_to_shared_reload_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.reload",
        "params": {
            "context": "TARGET-1",
            "ignoreCache": true,
            "wait": "complete"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::Reload(command) = shared else {
        panic!("expected Reload command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.ignore_cache);
    assert_eq!(
        command.wait,
        moli_protocol::devtools_runtime::DevToolsNavigationWait::Load
    );
}

#[test]
fn maps_browsing_context_traverse_history_to_shared_delta_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "browsingContext.traverseHistory",
        "params": {
            "context": "TARGET-1",
            "delta": -2
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::TraverseHistory(command) = shared else {
        panic!("expected TraverseHistory command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.destination,
        moli_protocol::devtools_runtime::DevToolsHistoryTraversalDestination::Delta(-2)
    );
    assert_eq!(
        command.wait,
        moli_protocol::devtools_runtime::DevToolsNavigationWait::Load
    );
}

#[test]
fn maps_script_evaluate_context_target_to_shared_evaluate_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "globalThis.answer",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": true,
            "resultOwnership": "root"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(command.expression, "globalThis.answer");
    assert!(command.await_promise);
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::Root
    );
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.realm_id.is_none());
    assert!(command.world_name.is_none());
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(2),
                max_dom_depth: Some(1),
                include_shadow_tree: None,
            }
        )
    );
}

#[test]
fn maps_script_user_activation_to_user_gesture_without_rewriting_source() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let evaluate = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "navigator.userActivation.isActive",
            "target": {
                "context": "TARGET-1"
            },
            "userActivation": true
        }
    }))
    .expect("BiDi evaluate command");

    let shared = super::super::devtools_command_from_bidi_command(&evaluate, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(command.expression, "navigator.userActivation.isActive");
    assert!(command.user_gesture);

    let call_function = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "() => navigator.userActivation.isActive",
            "target": {
                "context": "TARGET-1"
            },
            "userActivation": true
        }
    }))
    .expect("BiDi callFunction command");

    let shared = super::super::devtools_command_from_bidi_command(&call_function, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CallFunction(command) = shared else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command.function_declaration,
        "() => navigator.userActivation.isActive"
    );
    assert!(command.user_gesture);
}

#[test]
fn maps_script_evaluate_await_promise_default_ownership_preserves_metadata() {
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "window",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": true
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::None
    );
    assert!(
        command.preserve_remote_metadata,
        "awaitPromise must still preserve metadata for deep-serialized platform objects"
    );
}

#[test]
fn maps_script_context_sandbox_to_shared_runtime_world() {
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "globalThis.answer",
            "target": {
                "context": "TARGET-1",
                "sandbox": "sandbox"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.realm_id.is_none());
    assert_eq!(command.world_name.as_deref(), Some("sandbox"));
}

#[test]
fn maps_script_serialization_options_to_shared_runtime_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "({foo: {bar: 'baz'}})",
            "target": {
                "context": "TARGET-1"
            },
            "serializationOptions": {
                "maxObjectDepth": 1,
                "includeShadowTree": "open"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(1),
                max_dom_depth: None,
                include_shadow_tree: Some("open".to_owned()),
            }
        )
    );
    assert!(
        command.preserve_remote_metadata,
        "deep serialization needs root object metadata to materialize embedded platform objects"
    );
}

#[test]
fn maps_empty_script_serialization_options_to_unbounded_deep_runtime_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "[1, [2]]",
            "target": {
                "context": "TARGET-1"
            },
            "serializationOptions": {}
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: None,
                max_dom_depth: None,
                include_shadow_tree: None,
            }
        )
    );
}

#[test]
fn maps_script_evaluate_realm_target_to_shared_evaluate_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "1 + 1",
            "target": {
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command
            .realm_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsRealmId::as_str),
        Some("REALM-1")
    );
    assert!(command.context.target_id.is_none());
    assert!(command.world_name.is_none());
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::None
    );
}

#[test]
fn maps_script_call_function_to_shared_call_function_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 6,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "(value) => value",
            "target": {
                "context": "TARGET-1"
            },
            "arguments": [
                {"type": "string", "value": "ok"}
            ],
            "this": {
                "handle": "HANDLE-1"
            },
            "awaitPromise": true
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CallFunction(command) = shared else {
        panic!("expected CallFunction command");
    };
    assert_eq!(command.function_declaration, "(value) => value");
    assert_eq!(
        command.arguments,
        vec![json!({"type": "string", "value": "ok"})]
    );
    assert_eq!(command.this_parameter, Some(json!({"handle": "HANDLE-1"})));
    assert!(command.await_promise);
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.world_name.is_none());
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(2),
                max_dom_depth: Some(1),
                include_shadow_tree: None,
            }
        )
    );
    assert!(
        command.preserve_remote_metadata,
        "awaitPromise must still preserve metadata for deep-serialized platform objects"
    );
}

#[test]
fn maps_browsing_context_locate_nodes_to_shared_runtime_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 17,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "TARGET-1",
            "locator": {
                "type": "innerText",
                "value": "Foo",
                "ignoreCase": true,
                "matchType": "partial",
                "maxDepth": 2
            },
            "maxNodeCount": 3,
            "serializationOptions": {
                "maxDomDepth": 1
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::LocateNodes(command) = shared else {
        panic!("expected LocateNodes command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.max_node_count, Some(3));
    assert_eq!(
        command
            .serialization_options
            .as_ref()
            .and_then(|options| options.max_dom_depth),
        Some(1)
    );
    assert!(matches!(
        command.locator,
        moli_protocol::devtools_runtime::DevToolsLocateNodesLocator::InnerText {
            ref value,
            ignore_case: true,
            match_type: moli_protocol::devtools_runtime::DevToolsLocateNodesTextMatch::Partial,
            max_depth: 2,
        } if value == "Foo"
    ));
}

#[test]
fn maps_browsing_context_locate_nodes_context_locator_to_shared_runtime_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 18,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "PARENT-1",
            "locator": {
                "type": "context",
                "value": {
                    "context": "CHILD-1"
                }
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::LocateNodes(command) = shared else {
        panic!("expected LocateNodes command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("PARENT-1")
    );
    assert!(matches!(
        command.locator,
        moli_protocol::devtools_runtime::DevToolsLocateNodesLocator::Context(ref context)
            if context.as_str() == "CHILD-1"
    ));
}

#[test]
fn rejects_browsing_context_locate_nodes_context_locator_start_nodes() {
    let command = super::super::parse_bidi_command(json!({
        "id": 19,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "PARENT-1",
            "locator": {
                "type": "context",
                "value": {
                    "context": "CHILD-1"
                }
            },
            "startNodes": [{
                "type": "node",
                "sharedId": "NODE-1"
            }]
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("context locator startNodes should fail validation");

    assert_eq!(error.code, super::super::BidiErrorCode::InvalidArgument);
}

#[test]
fn maps_script_get_realms_to_shared_get_realms_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 7,
        "method": "script.getRealms",
        "params": {
            "context": "TARGET-1",
            "type": "window"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetRealms(command) = shared else {
        panic!("expected GetRealms command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.realm_type.as_deref(), Some("window"));
}

#[test]
fn maps_script_get_realms_to_service_worker_target_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 71,
        "method": "script.getRealms",
        "params": {
            "context": "TID-service-worker",
            "type": "service-worker"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetRealms(command) = shared else {
        panic!("expected GetRealms command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TID-service-worker")
    );
    assert_eq!(command.realm_type.as_deref(), Some("service-worker"));
}

#[test]
fn maps_script_disown_to_shared_release_objects_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.disown",
        "params": {
            "handles": ["HANDLE-1", "HANDLE-2"],
            "target": {
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::ReleaseObjects(command) = shared else {
        panic!("expected ReleaseObjects command");
    };
    assert_eq!(
        command
            .realm_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsRealmId::as_str),
        Some("REALM-1")
    );
    assert_eq!(
        command
            .handles
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsRemoteHandleId::as_str)
            .collect::<Vec<_>>(),
        vec!["HANDLE-1", "HANDLE-2"]
    );
    assert!(command.world_name.is_none());
}

#[test]
fn maps_storage_cookie_commands_to_shared_storage_commands() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let set = super::super::parse_bidi_command(json!({
        "id": 20,
        "method": "storage.setCookie",
        "params": {
            "cookie": {
                "name": "sid",
                "value": {
                    "type": "string",
                    "value": "abc"
                },
                "domain": "example.test",
                "path": "/",
                "httpOnly": true,
                "secure": true,
                "sameSite": "lax",
                "expiry": 1_800_000_000_u64
            },
            "partition": {
                "type": "context",
                "context": "TARGET-1"
            }
        }
    }))
    .expect("BiDi storage.setCookie command");
    let shared =
        super::super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCookies(set) = shared else {
        panic!("expected SetCookies command");
    };
    assert_eq!(
        set.context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(set.cookies.len(), 1);
    assert_eq!(set.cookies[0].name, "sid");
    assert_eq!(set.cookies[0].value, "abc");
    assert_eq!(set.cookies[0].domain.as_deref(), Some("example.test"));
    assert_eq!(set.cookies[0].path.as_deref(), Some("/"));
    assert_eq!(set.cookies[0].secure, Some(true));
    assert!(set.cookies[0].http_only);
    assert_eq!(set.cookies[0].same_site.as_deref(), Some("Lax"));
    assert_eq!(set.cookies[0].expires, Some(1_800_000_000.0));

    let get = super::super::parse_bidi_command(json!({
        "id": 21,
        "method": "storage.getCookies",
        "params": {
            "filter": {
                "name": "sid",
                "value": {
                    "type": "base64",
                    "value": "YWJj"
                },
                "domain": "example.test",
                "path": "/",
                "httpOnly": true,
                "secure": true,
                "sameSite": "lax",
                "size": 6,
                "expiry": 1_800_000_000_u64
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared =
        super::super::devtools_command_from_bidi_command(&get, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(get) = shared else {
        panic!("expected GetCookies command");
    };
    let filter = get.filter.expect("cookie filter");
    assert_eq!(filter.name.as_deref(), Some("sid"));
    assert_eq!(filter.value.as_deref(), Some("abc"));
    assert_eq!(filter.domain.as_deref(), Some("example.test"));
    assert_eq!(filter.path.as_deref(), Some("/"));
    assert_eq!(filter.http_only, Some(true));
    assert_eq!(filter.secure, Some(true));
    assert_eq!(filter.same_site.as_deref(), Some("lax"));
    assert_eq!(filter.size, Some(6));
    assert_eq!(filter.expires, Some(1_800_000_000));

    let delete = super::super::parse_bidi_command(json!({
        "id": 22,
        "method": "storage.deleteCookies",
        "params": {
            "filter": {
                "name": "sid",
                "domain": "example.test",
                "path": "/"
            }
        }
    }))
    .expect("BiDi storage.deleteCookies command");
    let shared = super::super::devtools_command_from_bidi_command(&delete, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::DeleteCookies(delete) = shared else {
        panic!("expected DeleteCookies command");
    };
    assert_eq!(delete.name.as_deref(), Some("sid"));
    assert_eq!(delete.domain.as_deref(), Some("example.test"));
    assert_eq!(delete.path.as_deref(), Some("/"));
    assert!(delete.filter.is_some());
}

#[test]
fn maps_chromium_wpt_storage_base64_and_partition_descriptors() {
    // Covers adapter-level shapes from Chromium's storage set_cookie,
    // get_cookies, and delete_cookies partition/value WPT suites.
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let set = super::super::parse_bidi_command(json!({
        "id": 23,
        "method": "storage.setCookie",
        "params": {
            "cookie": {
                "name": "sid",
                "value": {
                    "type": "base64",
                    "value": "YWJj"
                },
                "domain": "example.test",
                "sameSite": "default"
            },
            "partition": {
                "type": "storageKey",
                "userContext": "BID-2",
                "sourceOrigin": "https://example.test"
            }
        }
    }))
    .expect("BiDi storage.setCookie command");
    let shared =
        super::super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCookies(set) = shared else {
        panic!("expected SetCookies command");
    };
    assert_eq!(
        set.browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-2")
    );
    assert_eq!(
        set.context
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-2")
    );
    assert_eq!(set.cookies[0].value, "abc");
    assert_eq!(set.cookies[0].same_site, None);

    let default_partition = super::super::parse_bidi_command(json!({
        "id": 231,
        "method": "storage.getCookies",
        "params": {
            "partition": {
                "type": "storageKey",
                "userContext": "default"
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared = super::super::devtools_command_from_bidi_command(&default_partition, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(default_partition) = shared
    else {
        panic!("expected GetCookies command");
    };
    assert_eq!(default_partition.browser_context_id, None);
    assert_eq!(default_partition.context.browser_context_id, None);

    let get = super::super::parse_bidi_command(json!({
        "id": 24,
        "method": "storage.getCookies",
        "params": {
            "partition": {
                "type": "context",
                "context": "TARGET-2"
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared =
        super::super::devtools_command_from_bidi_command(&get, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(get) = shared else {
        panic!("expected GetCookies command");
    };
    assert_eq!(
        get.context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-2")
    );

    let delete = super::super::parse_bidi_command(json!({
        "id": 25,
        "method": "storage.deleteCookies",
        "params": {
            "filter": {
                "value": {
                    "type": "base64",
                    "value": "YmFy"
                },
                "size": 6
            },
            "partition": {
                "type": "storageKey",
                "userContext": "BID-3"
            }
        }
    }))
    .expect("BiDi storage.deleteCookies command");
    let shared = super::super::devtools_command_from_bidi_command(&delete, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::DeleteCookies(delete) = shared else {
        panic!("expected DeleteCookies command");
    };
    assert_eq!(
        delete
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-3")
    );
    let filter = delete.filter.expect("delete filter");
    assert_eq!(filter.value.as_deref(), Some("bar"));
    assert_eq!(filter.size, Some(6));
}

#[test]
fn rejects_script_disown_non_string_handles() {
    let command = super::super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.disown",
        "params": {
            "handles": ["HANDLE-1", false],
            "target": {
                "context": "TARGET-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("non-string handles should be rejected");

    assert_eq!(error.code, super::super::BidiErrorCode::InvalidArgument);
    assert_eq!(error.message, "handles entries must be strings");
}

#[test]
fn serializes_get_realms_result_to_bidi_realm_list() {
    let response = super::super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![RuntimeExecutionContextEvent {
                    target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                        "TARGET-1",
                    )),
                    context_id: Some(3),
                    realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                        "REALM-1",
                    )),
                    frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                        "TARGET-1",
                    )),
                    origin: Some("https://example.test".to_owned()),
                    name: Some(String::new()),
                    is_default: Some(true),
                    context_type: Some("default".to_owned()),
                    grant_universal_access: None,
                }],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 9,
            "result": {
                "realms": [{
                    "realm": "REALM-1",
                    "origin": "https://example.test",
                    "type": "window",
                    "context": "TARGET-1",
                }]
            }
        })
    );
}

#[test]
fn serializes_get_realms_result_to_service_worker_bidi_realm() {
    let response = super::super::bidi_response_from_devtools_result(
        92,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![RuntimeExecutionContextEvent {
                    target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                        "TID-service-worker",
                    )),
                    context_id: Some(20_000_007),
                    realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                        "service-worker-TID-service-worker",
                    )),
                    frame_id: None,
                    origin: Some("https://example.test".to_owned()),
                    name: Some(String::new()),
                    is_default: Some(true),
                    context_type: Some("service-worker".to_owned()),
                    grant_universal_access: None,
                }],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 92,
            "result": {
                "realms": [{
                    "realm": "service-worker-TID-service-worker",
                    "origin": "https://example.test",
                    "type": "service-worker",
                }]
            }
        })
    );
}

#[test]
fn serializes_get_realms_default_window_realm_before_sandbox_realm() {
    let response = super::super::bidi_response_from_devtools_result(
        91,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![
                    RuntimeExecutionContextEvent {
                        target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        context_id: Some(5),
                        realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                            "REALM-SANDBOX",
                        )),
                        frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                            "child-browsing-context-1",
                        )),
                        origin: Some("https://not-web-platform.test:8443".to_owned()),
                        name: Some("sandbox".to_owned()),
                        is_default: Some(false),
                        context_type: Some("isolated".to_owned()),
                        grant_universal_access: None,
                    },
                    RuntimeExecutionContextEvent {
                        target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        context_id: Some(4),
                        realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                            "REALM-DEFAULT",
                        )),
                        frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                            "child-browsing-context-1",
                        )),
                        origin: Some("https://not-web-platform.test:8443".to_owned()),
                        name: Some(String::new()),
                        is_default: Some(true),
                        context_type: Some("default".to_owned()),
                        grant_universal_access: None,
                    },
                ],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 91,
            "result": {
                "realms": [
                    {
                        "realm": "REALM-DEFAULT",
                        "origin": "https://not-web-platform.test:8443",
                        "type": "window",
                        "context": "child-browsing-context-1",
                    },
                    {
                        "realm": "REALM-SANDBOX",
                        "origin": "https://not-web-platform.test:8443",
                        "type": "window",
                        "context": "child-browsing-context-1",
                        "sandbox": "sandbox",
                    }
                ]
            }
        })
    );
}

#[test]
fn maps_script_add_preload_script_to_shared_preload_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => { globalThis.ready = true; }",
            "contexts": ["TARGET-1"],
            "sandbox": "utility",
            "arguments": [
                {
                    "type": "channel",
                    "value": {
                        "channel": "preload"
                    }
                }
            ]
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::AddPreloadScript(command) = shared else {
        panic!("expected AddPreloadScript command");
    };
    let moli_protocol::devtools_runtime::DevToolsPreloadScriptSource::FunctionDeclaration {
        function_declaration,
        arguments,
    } = command.source
    else {
        panic!("expected function declaration preload source");
    };
    assert_eq!(function_declaration, "() => { globalThis.ready = true; }");
    assert_eq!(
        arguments,
        vec![json!({"type": "channel", "value": {"channel": "preload"}})]
    );
    assert_eq!(command.world_name.as_deref(), Some("utility"));
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.target_ids.as_ref().map(|target_ids| {
            target_ids
                .iter()
                .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
                .collect::<Vec<_>>()
        }),
        Some(vec!["TARGET-1"])
    );
}

#[test]
fn maps_browsing_context_handle_user_prompt_to_shared_page_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "browsingContext.handleUserPrompt",
        "params": {
            "context": "TARGET-1",
            "accept": true,
            "userText": "Test"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::HandleJavaScriptDialog(command) = shared
    else {
        panic!("expected HandleJavaScriptDialog command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.accept);
    assert_eq!(command.prompt_text, "Test");
}

#[test]
fn maps_browsing_context_capture_screenshot_to_shared_page_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.captureScreenshot",
        "params": {
            "context": "TARGET-1",
            "format": {
                "type": "image/jpeg", "quality": 0.6
            },
            "origin": "viewport",
            "clip": {
                "type": "box",
                "x": 1,
                "y": 2,
                "width": 30,
                "height": 40
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CaptureScreenshot(command) = shared
    else {
        panic!("expected CaptureScreenshot command");
    };
    assert_eq!(command.quality, Some(60));
    assert!(!command.capture_beyond_viewport);
    assert_eq!(command.format.as_deref(), Some("jpeg"));
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    let moli_protocol::devtools_runtime::DevToolsCaptureScreenshotClip::Box(clip) =
        command.clip.expect("box clip should map")
    else {
        panic!("expected box clip");
    };
    assert_eq!(clip.x, 1.0);
    assert_eq!(clip.y, 2.0);
    assert_eq!(clip.width, 30.0);
    assert_eq!(clip.height, 40.0);
    assert_eq!(clip.scale, 1.0);
}

#[test]
fn maps_browsing_context_capture_screenshot_element_clip_to_shared_page_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.captureScreenshot",
        "params": {
            "context": "TARGET-1",
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": "ELEMENT-1"
                }
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CaptureScreenshot(command) = shared
    else {
        panic!("expected CaptureScreenshot command");
    };
    let moli_protocol::devtools_runtime::DevToolsCaptureScreenshotClip::Element(clip) =
        command.clip.expect("element clip should map")
    else {
        panic!("expected element clip");
    };
    assert_eq!(clip.shared_id.as_str(), "ELEMENT-1");
}

#[test]
fn maps_browsing_context_print_to_shared_page_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.print",
        "params": {
            "context": "TARGET-1",
            "background": true,
            "margin": {
                "top": 1.0,
                "bottom": 2.0,
                "left": 3.0,
                "right": 4.0
            },
            "orientation": "landscape",
            "page": {
                "width": 21.59,
                "height": 27.94
            },
            "pageRanges": ["1-2", 4, "9-"],
            "scale": 1.5,
            "shrinkToFit": false
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::PrintToPdf(command) = shared else {
        panic!("expected PrintToPdf command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.landscape, Some(true));
    assert_eq!(command.print_background, Some(true));
    assert_eq!(command.scale, Some(1.5));
    assert_eq!(command.page_ranges.as_deref(), Some("1-2,4,9-"));
    assert_eq!(command.shrink_to_fit, Some(false));
    assert_eq!(
        command.transfer_mode,
        Some(moli_protocol::devtools_runtime::DevToolsPrintToPdfTransferMode::ReturnAsBase64)
    );
    assert!((command.margin_top.unwrap() - (1.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_bottom.unwrap() - (2.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_left.unwrap() - (3.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_right.unwrap() - (4.0 / 2.54)).abs() < 1e-12);
    assert!((command.paper_width.unwrap() - (21.59 / 2.54)).abs() < 1e-12);
    assert!((command.paper_height.unwrap() - (27.94 / 2.54)).abs() < 1e-12);
}

#[test]
fn serializes_network_data_result_to_bidi_bytes_payload() {
    let response = super::super::bidi_response_from_devtools_result(
        10,
        moli_protocol::devtools_runtime::DevToolsCommandResult::NetworkData(
            moli_protocol::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: moli_protocol::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "body text".to_owned(),
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 10,
            "result": {
                "bytes": {
                    "type": "string",
                    "value": "body text",
                }
            }
        })
    );

    let response = super::super::bidi_response_from_devtools_result(
        11,
        moli_protocol::devtools_runtime::DevToolsCommandResult::NetworkData(
            moli_protocol::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: moli_protocol::devtools_runtime::DevToolsNetworkDataBytesType::Base64,
                value: "AP8=".to_owned(),
            },
        ),
    );

    assert_eq!(
        response["result"]["bytes"],
        json!({
            "type": "base64",
            "value": "AP8=",
        })
    );
}

#[test]
fn maps_browsing_context_set_viewport_to_shared_emulation_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "context": "TARGET-1",
            "viewport": {
                "width": 800,
                "height": 600
            },
            "devicePixelRatio": 2.0
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.viewport,
        moli_protocol::devtools_runtime::DevToolsViewportSetting::Dimensions {
            width: 800,
            height: 600,
        }
    );
    assert_eq!(
        command.device_pixel_ratio,
        moli_protocol::devtools_runtime::DevToolsDevicePixelRatioSetting::Scale(2.0)
    );
}

#[test]
fn maps_browsing_context_set_viewport_user_contexts_without_id_format_guessing() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "userContexts": ["custom-user-context"],
            "viewport": {
                "width": 800,
                "height": 600
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(command.context.target_id, None);
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["custom-user-context"]
    );
}

#[test]
fn maps_browsing_context_set_viewport_nulls_to_default_settings() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "context": "TARGET-1",
            "viewport": null,
            "devicePixelRatio": null
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(
        command.viewport,
        moli_protocol::devtools_runtime::DevToolsViewportSetting::Default
    );
    assert_eq!(
        command.device_pixel_ratio,
        moli_protocol::devtools_runtime::DevToolsDevicePixelRatioSetting::Default
    );
}

#[test]
fn maps_emulation_set_user_agent_override_global_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userAgent": "Moli-BiDi-UA/1.0"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.user_agent.as_deref(), Some("Moli-BiDi-UA/1.0"));
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
}

#[test]
fn maps_emulation_set_user_agent_override_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "userAgent": "Moli-Context-UA/1.0"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.user_agent.as_deref(), Some("Moli-Context-UA/1.0"));
}

#[test]
fn maps_emulation_set_user_agent_override_user_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "userAgent": null
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.user_agent, None);
}

#[test]
fn maps_emulation_set_locale_override_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setLocaleOverride",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "locale": "de-DE"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetLocaleOverride(command) = shared
    else {
        panic!("expected SetLocaleOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.locale.as_deref(), Some("de-DE"));
}

#[test]
fn maps_emulation_set_locale_override_user_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setLocaleOverride",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "locale": null
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetLocaleOverride(command) = shared
    else {
        panic!("expected SetLocaleOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.locale, None);
}

#[test]
fn maps_emulation_set_timezone_override_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setTimezoneOverride",
        "params": {
            "contexts": ["TARGET-1"],
            "timezone": "+10:00"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
    else {
        panic!("expected SetTimezoneOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.timezone.as_deref(), Some("GMT+10:00"));
}

#[test]
fn maps_emulation_set_timezone_override_user_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setTimezoneOverride",
        "params": {
            "userContexts": ["custom-user-context"],
            "timezone": null
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
    else {
        panic!("expected SetTimezoneOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["custom-user-context"]
    );
    assert_eq!(command.timezone, None);
}

#[test]
fn maps_emulation_set_network_conditions_global_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "networkConditions": {
                "type": "offline"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(
        command.network_conditions,
        Some(moli_protocol::devtools_runtime::DevToolsNetworkConditions::offline())
    );
}

#[test]
fn maps_emulation_set_network_conditions_contexts_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "networkConditions": {
                "type": "offline"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(
        command.network_conditions,
        Some(moli_protocol::devtools_runtime::DevToolsNetworkConditions::offline())
    );
}

#[test]
fn maps_emulation_set_network_conditions_user_contexts_reset_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "networkConditions": null
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.network_conditions, None);
}

#[test]
fn maps_permissions_set_permission_to_shared_browser_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "permissions.setPermission",
        "params": {
            "descriptor": { "name": "storage-access" },
            "state": "granted",
            "origin": "https://top.example",
            "embeddedOrigin": "https://frame.example",
            "userContext": "default"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetPermission(command) = shared else {
        panic!("expected SetPermission command");
    };
    assert_eq!(command.permission, json!({ "name": "storage-access" }));
    assert_eq!(command.setting, "granted");
    assert_eq!(command.origin, "https://top.example");
    assert_eq!(
        command.embedded_origin.as_deref(),
        Some("https://frame.example")
    );
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-default")
    );
}

#[test]
fn maps_emulation_set_geolocation_override_coordinates_to_shared_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setGeolocationOverride",
        "params": {
            "contexts": ["TARGET-1"],
            "coordinates": {
                "latitude": 4,
                "longitude": 2,
                "altitude": 8,
                "altitudeAccuracy": 3,
                "heading": 12,
                "speed": 5
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetGeolocationOverride(command) = shared
    else {
        panic!("expected SetGeolocationOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );
    assert!(command.browser_context_ids.is_empty());
    let override_state = command.override_state.expect("coordinates override");
    let moli_protocol::devtools_runtime::DevToolsGeolocationOverrideState::Position(override_state) =
        override_state
    else {
        panic!("expected coordinates override");
    };
    assert_eq!(override_state.latitude, 4.0);
    assert_eq!(override_state.longitude, 2.0);
    assert_eq!(override_state.accuracy, 1.0);
    assert_eq!(override_state.altitude, Some(8.0));
    assert_eq!(override_state.altitude_accuracy, Some(3.0));
    assert_eq!(override_state.heading, Some(12.0));
    assert_eq!(override_state.speed, Some(5.0));
}

#[test]
fn maps_emulation_set_geolocation_override_reset_and_error_to_distinct_shared_states() {
    for (params, expected) in [
        (
            json!({
                "userContexts": ["default", "custom-user-context"],
                "coordinates": null
            }),
            None,
        ),
        (
            json!({
                "contexts": ["TARGET-1"],
                "error": { "type": "positionUnavailable" }
            }),
            Some(
                moli_protocol::devtools_runtime::DevToolsGeolocationOverrideState::PositionUnavailable,
            ),
        ),
    ] {
        let command = super::super::parse_bidi_command(json!({
            "id": 10,
            "method": "emulation.setGeolocationOverride",
            "params": params
        }))
        .expect("BiDi command");
        let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

        let shared =
            super::super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

        let moli_protocol::devtools_runtime::DevToolsCommand::SetGeolocationOverride(command) =
            shared
        else {
            panic!("expected SetGeolocationOverride command");
        };
        assert_eq!(command.override_state, expected);
    }
}

#[test]
fn timezone_adapter_defers_name_validation_to_native_environment_controller() {
    for timezone in [
        "Africa/Cairo",
        "Pacific/Auckland",
        "Australia/Sydney",
        "Indian/Kolkata",
        "Atlantic/Reykjavik",
        "Arctic/Longyearbyen",
        "Antarctica/McMurdo",
        "Etc/GMT+5",
        "CET",
        "Japan",
        "Europe/Bielefeld",
        "America/Not_A_Zone",
        "Z",
    ] {
        let command = super::super::parse_bidi_command(json!({
            "id": 29,
            "method": "emulation.setTimezoneOverride",
            "params": {
                "contexts": ["TARGET-1"],
                "timezone": timezone
            }
        }))
        .expect("BiDi command");
        let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
        let shared = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect("shared command");
        let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
        else {
            panic!("expected SetTimezoneOverride command");
        };
        assert_eq!(command.timezone.as_deref(), Some(timezone));
    }
}

#[test]
fn maps_script_remove_preload_script_to_shared_preload_command() {
    let command = super::super::parse_bidi_command(json!({
        "id": 11,
        "method": "script.removePreloadScript",
        "params": {
            "script": "SCRIPT-1"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::RemovePreloadScript(command) = shared
    else {
        panic!("expected RemovePreloadScript command");
    };
    assert_eq!(command.script_id.as_str(), "SCRIPT-1");
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_user_agent_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/invalid.py.
    for params in [
        json!({}),
        json!({"userAgent": false}),
        json!({"userAgent": 42}),
        json!({"userAgent": {}}),
        json!({"userAgent": []}),
        json!({"contexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "userAgent": "Moli-UA/1.0"
        }),
    ] {
        assert_bidi_adapter_invalid("emulation.setUserAgentOverride", params);
    }

    let command = super::super::parse_bidi_command(json!({
        "id": 99,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userAgent": ""
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let error = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("empty userAgent should fail validation");
    assert_eq!(
        error.code,
        super::super::BidiErrorCode::UnsupportedOperation
    );
}
