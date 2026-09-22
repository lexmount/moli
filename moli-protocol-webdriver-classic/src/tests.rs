use moli_protocol::devtools_runtime::{
    DevToolsCaptureScreenshotClip, DevToolsCommand, DevToolsDomBoxModel,
    DevToolsDomGeometryOperation, DevToolsDomGeometryResult, DevToolsDomNodeReference,
    DevToolsDomQuad, DevToolsError, DevToolsErrorKind, DevToolsGetAttributesResult,
    DevToolsGetCookiesResult, DevToolsGetNavigationHistoryResult, DevToolsGetPropertyResult,
    DevToolsGetTargetsResult, DevToolsGetTextResult, DevToolsHistoryTraversalDestination,
    DevToolsKeyEventType, DevToolsLocateNodesLocator, DevToolsLocateNodesTextMatch,
    DevToolsMouseEventType, DevToolsNavigationWait, DevToolsPointerType,
    DevToolsPrintToPdfTransferMode, DevToolsProtocol, DevToolsRemoteHandleId,
    DevToolsResultOwnership, DevToolsSessionId, DevToolsTargetId, DevToolsTargetKind,
    DevToolsTouchEventType, DevToolsViewportSetting, DevToolsWindowState,
};
use serde_json::json;

use super::*;

const CLASSIC_MODIFIER_CONTROL: u8 = 2;
const CLASSIC_MODIFIER_SHIFT: u8 = 8;

fn box_model_geometry(points: [f64; 8], width: i32, height: i32) -> DevToolsDomGeometryResult {
    let border = DevToolsDomQuad {
        points: points.into(),
    };
    DevToolsDomGeometryResult {
        box_model: Some(DevToolsDomBoxModel {
            content: border.clone(),
            padding: border.clone(),
            border: border.clone(),
            margin: border,
            width,
            height,
        }),
        quads: Vec::new(),
        width: Some(width),
        height: Some(height),
    }
}

#[test]
fn status_response_uses_webdriver_value_envelope() {
    assert_eq!(
        status_response(true, ""),
        json!({
            "value": {
                "ready": true,
                "message": "",
            }
        })
    );
}

#[test]
fn session_registry_allocates_unique_non_reused_ids() {
    let mut registry = ClassicSessionRegistry::new();

    let first = registry.create_session();
    assert_eq!(first.session_id, "classic-session-1");
    assert_eq!(first.timeouts, ClassicTimeouts::default());
    assert_eq!(first.page_load_strategy, ClassicPageLoadStrategy::Normal);
    assert_eq!(
        first.unhandled_prompt_behavior.returned_capability(),
        json!("dismiss and notify")
    );
    assert!(registry.has_session("classic-session-1"));
    assert!(registry.release_session("classic-session-1"));

    let second = registry.create_session();
    assert_eq!(second.session_id, "classic-session-2");
    assert!(!registry.has_session("classic-session-1"));
    assert!(registry.has_session("classic-session-2"));
}

#[test]
fn parses_classic_unhandled_prompt_behavior_capability() {
    // Ported from Chromium/WPT webdriver/tests/classic/new_session/
    // unhandled_prompt_behavior.py.
    let default_behavior =
        unhandled_prompt_behavior_from_new_session_params(&json!({})).expect("default behavior");
    assert_eq!(
        default_behavior.returned_capability(),
        json!("dismiss and notify")
    );
    assert_eq!(
        default_behavior.handler_for_prompt_type("alert"),
        ClassicPromptHandler::Dismiss { notify: true }
    );
    assert_eq!(
        default_behavior.file_prompt_handler_for_bidi_script_commands(),
        None
    );

    let accept = unhandled_prompt_behavior_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "unhandledPromptBehavior": "accept"
            }
        }
    }))
    .expect("string behavior");
    assert_eq!(accept.returned_capability(), json!("accept"));
    assert_eq!(
        accept.handler_for_prompt_type("prompt"),
        ClassicPromptHandler::Accept { notify: false }
    );
    assert_eq!(
        accept.file_prompt_handler_for_bidi_script_commands(),
        Some("accept")
    );

    let object = unhandled_prompt_behavior_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "unhandledPromptBehavior": {
                    "default": "accept",
                    "alert": "ignore",
                    "file": "dismiss",
                    "prompt": "dismiss and notify"
                }
            }
        }
    }))
    .expect("object behavior");
    assert_eq!(
        object.returned_capability(),
        json!({
            "default": "accept",
            "alert": "ignore",
            "file": "dismiss",
            "prompt": "dismiss and notify"
        })
    );
    assert_eq!(
        object.handler_for_prompt_type("confirm"),
        ClassicPromptHandler::Accept { notify: false }
    );
    assert_eq!(
        object.handler_for_prompt_type("alert"),
        ClassicPromptHandler::Ignore
    );
    assert_eq!(
        object.handler_for_prompt_type("prompt"),
        ClassicPromptHandler::Dismiss { notify: true }
    );
    assert_eq!(
        object.file_prompt_handler_for_bidi_script_commands(),
        Some("dismiss")
    );

    let empty_object = unhandled_prompt_behavior_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "unhandledPromptBehavior": {}
            }
        }
    }))
    .expect("empty object behavior");
    assert_eq!(empty_object.returned_capability(), json!({}));
    assert_eq!(
        empty_object.handler_for_prompt_type("alert"),
        ClassicPromptHandler::Dismiss { notify: true }
    );

    for invalid in [
        json!(false),
        json!("ACCEPT"),
        json!("ignore "),
        json!({"foo": "accept"}),
        json!({"beforeunload": "accept"}),
        json!({"alert": null}),
        json!({"prompt": 1}),
    ] {
        let error = unhandled_prompt_behavior_from_new_session_params(&json!({
            "capabilities": {
                "alwaysMatch": {
                    "unhandledPromptBehavior": invalid
                }
            }
        }))
        .expect_err("invalid unhandledPromptBehavior should fail");
        assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    }
}

#[test]
fn parses_classic_page_load_strategy_capability() {
    assert_eq!(
        page_load_strategy_from_new_session_params(&json!({})).expect("default strategy"),
        ClassicPageLoadStrategy::Normal
    );
    assert_eq!(
        page_load_strategy_from_new_session_params(&json!({
            "capabilities": {
                "alwaysMatch": {
                    "pageLoadStrategy": "eager"
                }
            }
        }))
        .expect("alwaysMatch strategy"),
        ClassicPageLoadStrategy::Eager
    );
    assert_eq!(
        page_load_strategy_from_new_session_params(&json!({
            "capabilities": {
                "firstMatch": [
                    {
                        "pageLoadStrategy": "none"
                    }
                ]
            }
        }))
        .expect("firstMatch strategy"),
        ClassicPageLoadStrategy::None
    );

    let invalid = page_load_strategy_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "pageLoadStrategy": "fast"
            }
        }
    }))
    .expect_err("unknown strategy should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
    let invalid = page_load_strategy_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "pageLoadStrategy": false
            }
        }
    }))
    .expect_err("non-string strategy should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn matches_classic_new_session_capabilities_by_browser_name() {
    let mismatch = matched_capabilities_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "browserName": "not-moli"
            }
        }
    }))
    .expect_err("unsupported browserName must not create a session");
    assert_eq!(mismatch.code, ClassicErrorCode::SessionNotCreated);

    let selected = matched_capabilities_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": {
                "timeouts": {}
            },
            "firstMatch": [
                {
                    "browserName": "firefox",
                    "unhandledPromptBehavior": "accept"
                },
                {
                    "browserName": "moli",
                    "pageLoadStrategy": "eager"
                }
            ]
        }
    }))
    .expect("the second matching capability set should be selected");
    assert_eq!(selected["browserName"], json!("moli"));
    assert_eq!(selected["pageLoadStrategy"], json!("eager"));
    assert_eq!(selected["timeouts"], json!({}));
    assert!(!selected.contains_key("unhandledPromptBehavior"));

    let duplicate = matched_capabilities_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": { "browserName": "moli" },
            "firstMatch": [{ "browserName": "moli" }]
        }
    }))
    .expect_err("duplicate capability names must fail merging");
    assert_eq!(duplicate.code, ClassicErrorCode::InvalidArgument);

    let invalid_type = matched_capabilities_from_new_session_params(&json!({
        "capabilities": {
            "alwaysMatch": { "browserName": 7 }
        }
    }))
    .expect_err("non-string browserName must be invalid");
    assert_eq!(invalid_type.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn registry_tracks_current_target_id_for_future_command_adapters() {
    let mut registry = ClassicSessionRegistry::new();
    let session = registry.create_session();

    assert_eq!(registry.current_target_id(&session.session_id), None);
    assert!(registry.set_current_target_id(&session.session_id, "TID-1"));
    assert_eq!(
        registry.current_target_id(&session.session_id),
        Some("TID-1")
    );
    assert!(!registry.set_current_target_id("missing", "TID-2"));
}

#[test]
fn registry_tracks_current_frame_and_resets_it_on_window_switch() {
    let mut registry = ClassicSessionRegistry::new();
    let session = registry.create_session();

    assert_eq!(registry.current_frame_id(&session.session_id), Some(None));
    assert!(registry.set_current_frame_id(&session.session_id, Some("FRAME-1".to_owned())));
    assert_eq!(
        registry.current_frame_id(&session.session_id),
        Some(Some("FRAME-1"))
    );
    assert!(registry.set_current_target_id(&session.session_id, "TID-2"));
    assert_eq!(registry.current_frame_id(&session.session_id), Some(None));
    assert!(!registry.set_current_frame_id("missing", None));
}

#[test]
fn registry_tracks_classic_timeouts() {
    let mut registry = ClassicSessionRegistry::new();
    let session = registry.create_session();
    assert_eq!(
        registry.timeouts(&session.session_id),
        Some(ClassicTimeouts::default())
    );

    let updated = parse_timeouts(
        &json!({ "script": 25, "pageLoad": 50, "implicit": 3 }),
        registry.timeouts(&session.session_id).unwrap(),
    )
    .expect("timeouts");
    assert_eq!(
        timeouts_value(updated),
        json!({ "script": 25, "pageLoad": 50, "implicit": 3 })
    );
    assert!(registry.set_timeouts(&session.session_id, updated));
    assert_eq!(registry.timeouts(&session.session_id), Some(updated));

    let partial = parse_timeouts(&json!({ "script": 7 }), updated).expect("partial update");
    assert_eq!(
        timeouts_value(partial),
        json!({ "script": 7, "pageLoad": 50, "implicit": 3 })
    );

    let nullable = parse_timeouts(
        &json!({ "script": null, "pageLoad": null, "implicit": null }),
        partial,
    )
    .expect("nullable timeouts");
    assert_eq!(
        timeouts_value(nullable),
        json!({ "script": null, "pageLoad": null, "implicit": null })
    );

    let integer_float =
        parse_timeouts(&json!({ "script": 2.0 }), nullable).expect("integer-valued float");
    assert_eq!(timeouts_value(integer_float)["script"], json!(2));

    let safe_integer = 9_007_199_254_740_991_u64;
    let safe = parse_timeouts(&json!({ "script": safe_integer }), integer_float)
        .expect("max safe integer");
    assert_eq!(timeouts_value(safe)["script"], json!(safe_integer));

    let invalid = parse_timeouts(&json!({ "script": -1 }), updated)
        .expect_err("negative timeouts should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
    let invalid = parse_timeouts(&json!({ "implicit": 1.5 }), updated)
        .expect_err("fractional timeouts should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
    let invalid = parse_timeouts(&json!({ "implicit": 9_007_199_254_740_992_u64 }), updated)
        .expect_err("unsafe integer timeouts should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn new_delete_and_error_responses_use_classic_shapes() {
    assert_eq!(
        new_session_response("classic-session-1", json!({"browserName": "moli"})),
        json!({
            "value": {
                "sessionId": "classic-session-1",
                "capabilities": {
                    "browserName": "moli"
                }
            }
        })
    );
    assert_eq!(delete_session_response(), json!({ "value": null }));
    assert_eq!(
        error_response(ClassicErrorCode::InvalidSessionId, "session not found"),
        json!({
            "value": {
                "error": "invalid session id",
                "message": "session not found",
                "stacktrace": "",
            }
        })
    );
    assert_eq!(
        error_response(ClassicErrorCode::ScriptTimeout, "script timed out")["value"]["error"],
        json!("script timeout")
    );
    assert_eq!(
        error_response(ClassicErrorCode::Timeout, "page load timed out")["value"]["error"],
        json!("timeout")
    );
    assert_eq!(
        error_response(ClassicErrorCode::InvalidSelector, "bad selector")["value"]["error"],
        json!("invalid selector")
    );
    assert_eq!(
        error_response(ClassicErrorCode::NoSuchShadowRoot, "missing shadow")["value"]["error"],
        json!("no such shadow root")
    );
    assert_eq!(
        error_response(ClassicErrorCode::DetachedShadowRoot, "detached shadow")["value"]["error"],
        json!("detached shadow root")
    );
}

#[test]
fn creates_initial_target_command_for_webdriver_classic_session() {
    let context = ClassicDevToolsCommandContext::new("classic-session-1");

    let command = create_initial_target_command(&context);

    let DevToolsCommand::CreateTarget(command) = command else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(command.url, "about:blank");
    assert_eq!(command.context.protocol, DevToolsProtocol::WebDriverClassic);
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(DevToolsSessionId::as_str),
        Some("classic-session-1")
    );
    assert!(command.context.target_id.is_none());
}

#[test]
fn maps_url_commands_to_shared_devtools_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let navigate = navigate_command(
        &context,
        &json!({"url": "https://example.test/"}),
        DevToolsNavigationWait::Load,
    )
    .expect("navigate command");
    let DevToolsCommand::Navigate(navigate) = navigate else {
        panic!("expected Navigate command");
    };
    assert_eq!(navigate.url, "https://example.test/");
    assert_eq!(navigate.wait, DevToolsNavigationWait::Load);
    assert_eq!(
        navigate
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let get_url = current_url_command(&context);
    let DevToolsCommand::GetTargets(get_url) = get_url else {
        panic!("expected GetTargets command");
    };
    assert_eq!(
        get_url.root.as_ref().map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_title_and_source_to_shared_devtools_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let title = title_command(&context);
    let DevToolsCommand::EvaluateScript(title) = title else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(title.expression, "document.title");
    assert!(title.await_promise);
    assert_eq!(title.result_ownership, DevToolsResultOwnership::None);
    assert_eq!(
        title
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let source = page_source_command(&context);
    let DevToolsCommand::GetOuterHtml(source) = source else {
        panic!("expected GetOuterHtml command");
    };
    assert_eq!(source.reference, None);
    assert!(!source.include_shadow_dom);
    assert_eq!(
        source
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_find_element_to_shared_dom_query_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let single = find_element_command(
        &context,
        &json!({
            "using": "css selector",
            "value": "main.item"
        }),
        false,
    )
    .expect("find element command");
    let DevToolsCommand::QuerySelector(single) = single else {
        panic!("expected QuerySelector command");
    };
    assert_eq!(single.root, None);
    assert_eq!(single.selector, "main.item");
    assert!(!single.multiple);
    assert_eq!(
        single
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let multiple = find_element_command(
        &context,
        &json!({
            "using": "css selector",
            "value": ".item"
        }),
        true,
    )
    .expect("find elements command");
    let DevToolsCommand::QuerySelector(multiple) = multiple else {
        panic!("expected QuerySelector command");
    };
    assert!(multiple.multiple);

    for (using, value, expected_selector) in [
        ("id", "source", r#"[id="source"]"#),
        ("name", "field", r#"[name="field"]"#),
        ("class name", "primary", r#"[class~="primary"]"#),
        ("id", r#"quote"backslash\"#, r#"[id="quote\"backslash\\"]"#),
        ("id", "form\x0cfeed", r#"[id="form\c feed"]"#),
    ] {
        let command = find_element_command(
            &context,
            &json!({
                "using": using,
                "value": value
            }),
            false,
        )
        .unwrap_or_else(|error| panic!("{using} locator should map: {error:?}"));
        let DevToolsCommand::QuerySelector(command) = command else {
            panic!("expected QuerySelector command for {using}");
        };
        assert_eq!(command.selector, expected_selector, "locator {using}");
    }

    let compound_class = find_element_command(
        &context,
        &json!({
            "using": "class name",
            "value": "one two"
        }),
        false,
    )
    .expect_err("compound class name should fail");
    assert_eq!(compound_class.code, ClassicErrorCode::InvalidSelector);

    let tag_name = find_element_command(
        &context,
        &json!({
            "using": "tag name",
            "value": "main"
        }),
        false,
    )
    .expect("tag name locator should map");
    let DevToolsCommand::LocateNodes(tag_name) = tag_name else {
        panic!("expected LocateNodes command for tag name");
    };
    assert_eq!(
        tag_name.locator,
        DevToolsLocateNodesLocator::TagName("main".to_owned())
    );
    assert_eq!(tag_name.max_node_count, Some(1));

    let tag_name_all = find_element_command(
        &context,
        &json!({
            "using": "tag name",
            "value": "main"
        }),
        true,
    )
    .expect("tag name locator should map for find elements");
    let DevToolsCommand::LocateNodes(tag_name_all) = tag_name_all else {
        panic!("expected LocateNodes command for tag name");
    };
    assert_eq!(tag_name_all.max_node_count, None);

    let empty_tag_name = find_element_command(
        &context,
        &json!({
            "using": "tag name",
            "value": ""
        }),
        false,
    )
    .expect_err("empty tag name should fail");
    assert_eq!(empty_tag_name.code, ClassicErrorCode::InvalidSelector);

    let empty_css = find_element_command(
        &context,
        &json!({
            "using": "css selector",
            "value": ""
        }),
        false,
    )
    .expect_err("empty CSS selector should fail");
    assert_eq!(empty_css.code, ClassicErrorCode::InvalidSelector);

    let xpath = find_element_command(
        &context,
        &json!({
            "using": "xpath",
            "value": "//main"
        }),
        false,
    )
    .expect("xpath locator should map");
    let DevToolsCommand::LocateNodes(xpath) = xpath else {
        panic!("expected LocateNodes command for xpath");
    };
    assert_eq!(
        xpath.locator,
        DevToolsLocateNodesLocator::XPath("//main".to_owned())
    );
    assert_eq!(xpath.max_node_count, Some(1));

    let xpath_all = find_element_command(
        &context,
        &json!({
            "using": "xpath",
            "value": "//main"
        }),
        true,
    )
    .expect("xpath locator should map for find elements");
    let DevToolsCommand::LocateNodes(xpath_all) = xpath_all else {
        panic!("expected LocateNodes command for xpath");
    };
    assert_eq!(xpath_all.max_node_count, None);

    let empty_xpath = find_element_command(
        &context,
        &json!({
            "using": "xpath",
            "value": ""
        }),
        false,
    )
    .expect_err("empty xpath should fail");
    assert_eq!(empty_xpath.code, ClassicErrorCode::InvalidSelector);

    let link_text = find_element_command(
        &context,
        &json!({
            "using": "link text",
            "value": "Docs"
        }),
        false,
    )
    .expect("link text locator should map");
    let DevToolsCommand::LocateNodes(link_text) = link_text else {
        panic!("expected LocateNodes command for link text");
    };
    assert_eq!(
        link_text.locator,
        DevToolsLocateNodesLocator::LinkText {
            value: "Docs".to_owned(),
            match_type: DevToolsLocateNodesTextMatch::Full,
        }
    );
    assert_eq!(link_text.max_node_count, Some(1));

    let partial_link_text = find_element_command(
        &context,
        &json!({
            "using": "partial link text",
            "value": "Doc"
        }),
        true,
    )
    .expect("partial link text locator should map");
    let DevToolsCommand::LocateNodes(partial_link_text) = partial_link_text else {
        panic!("expected LocateNodes command for partial link text");
    };
    assert_eq!(
        partial_link_text.locator,
        DevToolsLocateNodesLocator::LinkText {
            value: "Doc".to_owned(),
            match_type: DevToolsLocateNodesTextMatch::Partial,
        }
    );
    assert_eq!(partial_link_text.max_node_count, None);

    let empty_link_text = find_element_command(
        &context,
        &json!({
            "using": "link text",
            "value": ""
        }),
        false,
    )
    .expect_err("empty link text should fail");
    assert_eq!(empty_link_text.code, ClassicErrorCode::InvalidSelector);

    let unsupported = find_element_command(
        &context,
        &json!({
            "using": "relative",
            "value": "Docs"
        }),
        false,
    )
    .expect_err("unsupported locator should fail");
    assert_eq!(unsupported.code, ClassicErrorCode::InvalidArgument);

    let rooted = find_element_command_with_root(
        &context,
        &json!({
            "using": "css selector",
            "value": ".child"
        }),
        false,
        Some(DevToolsDomNodeReference::FrontendNodeId(7)),
    )
    .expect("root-scoped CSS locator should map");
    let DevToolsCommand::QuerySelector(rooted) = rooted else {
        panic!("expected QuerySelector command for root-scoped CSS");
    };
    assert_eq!(
        rooted.root,
        Some(DevToolsDomNodeReference::FrontendNodeId(7))
    );

    assert_eq!(
        classic_element_reference(42),
        json!({
            CLASSIC_ELEMENT_REFERENCE_KEY: "moli-node-42"
        })
    );
    assert_eq!(
        classic_shadow_root_reference("moli-shadow-42-shadow-9"),
        json!({
            CLASSIC_SHADOW_ROOT_REFERENCE_KEY: "moli-shadow-42-shadow-9"
        })
    );
    assert_eq!(
        cdp_node_id_from_classic_element_id("moli-node-42-element-7").expect("owner id"),
        42
    );
    assert_eq!(
        cdp_node_id_from_classic_shadow_root_id("moli-shadow-42-shadow-7").expect("owner id"),
        42
    );
    let high_id_attribute =
        get_element_attributes_command(&context, "moli-node-2000000042-element-7")
            .expect("legacy element id should parse");
    let DevToolsCommand::GetAttributes(high_id_attribute) = high_id_attribute else {
        panic!("expected GetAttributes command for high legacy element id");
    };
    assert_eq!(
        high_id_attribute.reference,
        DevToolsDomNodeReference::FrontendNodeId(2_000_000_042)
    );
    let high_id_shadow_root = resolve_shadow_root_command(
        &context,
        "moli-shadow-2000000042-shadow-7",
        "classic-shadow",
    )
    .expect("legacy shadow root id should parse");
    let DevToolsCommand::ResolveNode(high_id_shadow_root) = high_id_shadow_root else {
        panic!("expected ResolveNode command for high legacy shadow root id");
    };
    assert_eq!(
        high_id_shadow_root.reference,
        DevToolsDomNodeReference::FrontendNodeId(2_000_000_042)
    );
    for invalid in [
        "moli-node-42-element-",
        "moli-node-42-element-owner",
        "moli-node-42-suffix",
    ] {
        assert_eq!(
            cdp_node_id_from_classic_element_id(invalid)
                .expect_err("malformed element id should fail")
                .code,
            ClassicErrorCode::NoSuchElement,
            "{invalid}"
        );
    }
    for invalid in [
        "moli-shadow-42-shadow-",
        "moli-shadow-42-shadow-owner",
        "moli-shadow-42-suffix",
    ] {
        assert_eq!(
            cdp_node_id_from_classic_shadow_root_id(invalid)
                .expect_err("malformed shadow root id should fail")
                .code,
            ClassicErrorCode::NoSuchShadowRoot,
            "{invalid}"
        );
    }
}

#[test]
fn maps_screenshot_endpoints_to_shared_page_capture_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let page = screenshot_command(&context);
    let DevToolsCommand::CaptureScreenshot(page) = page else {
        panic!("expected CaptureScreenshot command");
    };
    assert_eq!(page.format.as_deref(), Some("png"));
    assert!(page.clip.is_none());
    assert_eq!(
        page.context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let element = element_screenshot_command(&context, "remote-element-1");
    let DevToolsCommand::CaptureScreenshot(element) = element else {
        panic!("expected CaptureScreenshot command");
    };
    assert_eq!(element.format.as_deref(), Some("png"));
    let Some(DevToolsCaptureScreenshotClip::Element(clip)) = element.clip else {
        panic!("expected element clip");
    };
    assert_eq!(clip.shared_id.as_str(), "remote-element-1");
}

#[test]
fn maps_print_page_to_shared_print_to_pdf_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = print_page_command(
        &context,
        &json!({
            "orientation": "landscape",
            "scale": 1.25,
            "background": true,
            "shrinkToFit": false,
            "pageRanges": ["1-2", 4],
            "page": {
                "width": 30.0,
                "height": 20.0
            },
            "margin": {
                "top": 0.0,
                "bottom": 1.0,
                "left": 2.0,
                "right": 3.0
            }
        }),
    )
    .expect("classic print page command should map");
    let DevToolsCommand::PrintToPdf(command) = command else {
        panic!("expected PrintToPdf command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
    assert_eq!(command.landscape, Some(true));
    assert_eq!(command.print_background, Some(true));
    assert_eq!(command.scale, Some(1.25));
    assert_eq!(command.page_ranges.as_deref(), Some("1-2,4"));
    assert_eq!(command.shrink_to_fit, Some(false));
    assert_eq!(
        command.transfer_mode,
        Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64)
    );
    assert!((command.paper_width.expect("paper width") - 30.0 / 2.54).abs() < f64::EPSILON);
    assert!((command.paper_height.expect("paper height") - 20.0 / 2.54).abs() < f64::EPSILON);
    assert!((command.margin_top.expect("top margin") - 0.0).abs() < f64::EPSILON);
    assert!((command.margin_bottom.expect("bottom margin") - 1.0 / 2.54).abs() < f64::EPSILON);
    assert!((command.margin_left.expect("left margin") - 2.0 / 2.54).abs() < f64::EPSILON);
    assert!((command.margin_right.expect("right margin") - 3.0 / 2.54).abs() < f64::EPSILON);

    for params in [
        json!({"orientation": "sideways"}),
        json!({"scale": 3.0}),
        json!({"pageRanges": ["3-2"]}),
        json!({"page": {"width": 0.03}}),
        json!({"margin": {"left": -1.0}}),
    ] {
        let error = print_page_command(&context, &params).expect_err("params should fail");
        assert_eq!(
            error.code,
            ClassicErrorCode::InvalidArgument,
            "params should be invalid: {params}"
        );
    }
}

#[test]
fn maps_element_attribute_to_shared_dom_attributes_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = get_element_attributes_command(&context, "moli-node-42").expect("command");
    let DevToolsCommand::GetAttributes(command) = command else {
        panic!("expected GetAttributes command");
    };
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(42)
    );
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = get_element_attributes_command(&context, "not-a-moli-node").expect_err("error");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);

    let value = classic_attribute_value(
        DevToolsGetAttributesResult {
            attributes: vec![moli_protocol::devtools_runtime::DevToolsDomAttribute {
                name: "data-kind".to_owned(),
                value: "primary".to_owned(),
            }],
        },
        "data-kind",
    );
    assert_eq!(value.as_deref(), Some("primary"));
    let boolean_value = classic_attribute_value(
        DevToolsGetAttributesResult {
            attributes: vec![moli_protocol::devtools_runtime::DevToolsDomAttribute {
                name: "disabled".to_owned(),
                value: String::new(),
            }],
        },
        "disabled",
    );
    assert_eq!(boolean_value.as_deref(), Some("true"));
    let boolean_with_false_value = classic_attribute_value(
        DevToolsGetAttributesResult {
            attributes: vec![moli_protocol::devtools_runtime::DevToolsDomAttribute {
                name: "checked".to_owned(),
                value: "false".to_owned(),
            }],
        },
        "checked",
    );
    assert_eq!(boolean_with_false_value.as_deref(), Some("true"));
    let missing_boolean = classic_attribute_value(
        DevToolsGetAttributesResult {
            attributes: Vec::new(),
        },
        "checked",
    );
    assert_eq!(missing_boolean, None);

    let stale = classic_error_from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::NoSuchHandle,
        "Could not find node with given id",
    ));
    assert_eq!(stale.code, ClassicErrorCode::StaleElementReference);
    let invalid_selector = classic_error_from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::InvalidSelector,
        "bad selector",
    ));
    assert_eq!(invalid_selector.code, ClassicErrorCode::InvalidSelector);
    let timeout = classic_error_from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::Timeout,
        "navigation wait timed out",
    ));
    assert_eq!(timeout.code, ClassicErrorCode::Timeout);
}

#[test]
fn maps_internal_navigation_failure_to_classic_unknown_error() {
    let navigation_policy = classic_error_from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::Internal,
        "Navigation to a local file URL requires an explicitly granted browser capability.",
    ));

    assert_eq!(navigation_policy.code, ClassicErrorCode::UnknownError);
    assert_eq!(
        error_response(navigation_policy.code, navigation_policy.message),
        json!({
            "value": {
                "error": "unknown error",
                "message": "Navigation to a local file URL requires an explicitly granted browser capability.",
                "stacktrace": "",
            }
        })
    );
}

#[test]
fn maps_element_text_to_shared_dom_text_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = get_element_text_command(&context, "moli-node-7").expect("command");
    let DevToolsCommand::GetText(command) = command else {
        panic!("expected GetText command");
    };
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(7)
    );
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = get_element_text_command(&context, "not-a-moli-node")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);

    assert_eq!(
        classic_text_value(DevToolsGetTextResult {
            text: "visible text".to_owned(),
        }),
        "visible text"
    );
    assert_eq!(
        classic_text_value(DevToolsGetTextResult {
            text: "  visible\n\ttext  with   spaces  ".to_owned(),
        }),
        "visible text with spaces"
    );
}

#[test]
fn maps_element_property_to_shared_dom_property_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = get_element_property_command(&context, "moli-node-11", "value").expect("command");
    let DevToolsCommand::GetProperty(command) = command else {
        panic!("expected GetProperty command");
    };
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(11)
    );
    assert_eq!(command.name, "value");
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = get_element_property_command(&context, "not-a-moli-node", "value")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);

    assert_eq!(
        classic_property_value(DevToolsGetPropertyResult {
            value: json!("property"),
        }),
        json!("property")
    );
}

#[test]
fn maps_element_css_value_to_current_context_runtime_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "FRAME-1");

    let command = get_element_css_value_command(&context, "remote-42", "display");
    let DevToolsCommand::CallFunction(command) = command else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("FRAME-1")
    );
    assert_eq!(
        command
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("remote-42")
    );
    assert_eq!(command.arguments, vec![json!("display")]);
    assert!(
        command
            .function_declaration
            .contains("getComputedStyle(this)")
    );
    assert_eq!(command.result_ownership, DevToolsResultOwnership::None);
}

#[test]
fn maps_element_displayed_to_current_context_runtime_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "FRAME-1");

    let command = get_element_displayed_command(&context, "remote-17");
    let DevToolsCommand::CallFunction(command) = command else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("FRAME-1")
    );
    assert_eq!(
        command
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("remote-17")
    );
    assert!(command.arguments.is_empty());
    assert!(command.function_declaration.contains("getComputedStyle"));
    assert!(command.function_declaration.contains("getClientRects"));
    assert_eq!(command.result_ownership, DevToolsResultOwnership::None);
}

#[test]
fn maps_element_rendered_text_to_current_context_runtime_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "FRAME-1");

    let command = get_element_rendered_text_command(&context, "remote-18");
    let DevToolsCommand::CallFunction(command) = command else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("FRAME-1")
    );
    assert_eq!(
        command
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("remote-18")
    );
    assert!(command.arguments.is_empty());
    assert!(command.function_declaration.contains("blockTags"));
    assert!(command.function_declaration.contains("getComputedStyle"));
    assert!(command.function_declaration.contains("text-transform"));
    assert!(command.function_declaration.contains("capitalizeText"));
    assert!(command.function_declaration.contains("assignedNodes"));
    assert!(command.function_declaration.contains("shadowRoot"));
    assert!(command.function_declaration.contains("skippedTags"));
    assert_eq!(command.result_ownership, DevToolsResultOwnership::None);
}

#[test]
fn maps_element_enabled_to_current_context_runtime_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "FRAME-1");

    let command = get_element_enabled_command(&context, "remote-19");
    let DevToolsCommand::CallFunction(command) = command else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("FRAME-1")
    );
    assert_eq!(
        command
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("remote-19")
    );
    assert!(command.arguments.is_empty());
    assert!(
        command
            .function_declaration
            .contains("ancestorName === 'optgroup' || ancestorName === 'select'")
    );
    assert!(command.function_declaration.contains("isActuallyDisabled"));
    assert_eq!(command.result_ownership, DevToolsResultOwnership::None);
}

#[test]
fn maps_shadow_root_commands_to_current_context_runtime_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "FRAME-1");

    let get = get_element_shadow_root_command(&context, "host-remote-1");
    let DevToolsCommand::CallFunction(get) = get else {
        panic!("expected get shadow root CallFunction command");
    };
    assert_eq!(
        get.context.target_id.as_ref().map(DevToolsTargetId::as_str),
        Some("FRAME-1")
    );
    assert_eq!(
        get.object_id.as_ref().map(DevToolsRemoteHandleId::as_str),
        Some("host-remote-1")
    );
    assert!(get.function_declaration.contains("this.shadowRoot"));
    assert!(get.preserve_remote_metadata);

    let element_attached = verify_element_attached_command(&context, "host-remote-1");
    let DevToolsCommand::CallFunction(element_attached) = element_attached else {
        panic!("expected element attachment CallFunction command");
    };
    assert_eq!(
        element_attached
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("host-remote-1")
    );
    assert!(
        element_attached
            .function_declaration
            .contains("isConnected")
    );
    assert!(
        !element_attached
            .function_declaration
            .contains("this.shadowRoot")
    );
    assert!(!element_attached.preserve_remote_metadata);

    let describe = describe_node_command(&context, 42, 1, true);
    let DevToolsCommand::DescribeNode(describe) = describe else {
        panic!("expected DescribeNode command");
    };
    assert_eq!(
        describe.reference,
        Some(DevToolsDomNodeReference::FrontendNodeId(42))
    );
    assert_eq!(describe.depth, 1);
    assert!(describe.pierce);

    let resolve = resolve_shadow_root_command(
        &context,
        "moli-shadow-42-shadow-3",
        "webdriver-classic-shadow-root",
    )
    .expect("resolve command");
    let DevToolsCommand::ResolveNode(resolve) = resolve else {
        panic!("expected ResolveNode command");
    };
    assert_eq!(
        resolve.reference,
        DevToolsDomNodeReference::FrontendNodeId(42)
    );
    assert_eq!(
        resolve.object_group.as_deref(),
        Some("webdriver-classic-shadow-root")
    );

    let attached = shadow_root_attached_command(&context, "shadow-remote-1");
    let DevToolsCommand::CallFunction(attached) = attached else {
        panic!("expected shadow root attachment CallFunction command");
    };
    assert_eq!(
        attached
            .object_id
            .as_ref()
            .map(DevToolsRemoteHandleId::as_str),
        Some("shadow-remote-1")
    );
    assert!(
        attached
            .function_declaration
            .contains("this.nodeType === Node.DOCUMENT_FRAGMENT_NODE")
    );
    assert!(!attached.preserve_remote_metadata);
}

#[test]
fn maps_element_tag_name_to_shared_dom_local_name_property_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = get_element_tag_name_command(&context, "moli-node-13").expect("command");
    let DevToolsCommand::GetProperty(command) = command else {
        panic!("expected GetProperty command");
    };
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(13)
    );
    assert_eq!(command.name, "localName");
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = get_element_tag_name_command(&context, "not-a-moli-node")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);
}

#[test]
fn maps_active_element_to_shared_runtime_node_remote_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = active_element_command(&context);
    let DevToolsCommand::EvaluateScript(command) = command else {
        panic!("expected EvaluateScript command");
    };
    assert!(command.expression.contains("document.activeElement"));
    assert!(command.expression.contains("document.body"));
    assert!(command.await_promise);
    assert_eq!(command.result_ownership, DevToolsResultOwnership::None);
    assert!(command.preserve_remote_metadata);
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_get_element_rect_to_shared_dom_geometry_command_and_response() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let command = get_element_rect_command(&context, "moli-node-13").expect("command");
    let DevToolsCommand::DomGeometry(command) = command else {
        panic!("expected DOM geometry command");
    };
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(13)
    );
    assert_eq!(command.operation, DevToolsDomGeometryOperation::GetBoxModel);
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let rect = classic_rect_from_geometry(&box_model_geometry(
        [10.0, 20.0, 40.0, 20.0, 40.0, 60.0, 10.0, 60.0],
        30,
        40,
    ))
    .expect("rect");
    assert_eq!(
        rect,
        json!({
            "x": 10.0,
            "y": 20.0,
            "width": 30.0,
            "height": 40.0,
        })
    );

    let invalid = get_element_rect_command(&context, "not-a-moli-node")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);
}

#[test]
fn maps_element_clear_to_shared_dom_resolve_and_runtime_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let resolve = resolve_element_command(&context, "moli-node-31", "webdriver-classic-clear")
        .expect("resolve command");
    let DevToolsCommand::ResolveNode(resolve) = resolve else {
        panic!("expected ResolveNode command");
    };
    assert_eq!(
        resolve.reference,
        DevToolsDomNodeReference::FrontendNodeId(31)
    );
    assert_eq!(
        resolve
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
    assert_eq!(
        resolve.object_group.as_deref(),
        Some("webdriver-classic-clear")
    );

    let clear = clear_element_command(&context, "remote-object-1");
    let DevToolsCommand::CallFunction(clear) = clear else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        clear.object_id.as_ref().map(|object_id| object_id.as_str()),
        Some("remote-object-1")
    );
    assert_eq!(clear.result_ownership, DevToolsResultOwnership::None);
    assert!(clear.await_promise);
    assert!(clear.function_declaration.contains("isContentEditable"));
    assert!(clear.function_declaration.contains("isActuallyDisabled"));
    assert!(clear.function_declaration.contains("isInFirstLegend"));
    assert!(clear.function_declaration.contains("invalid element state"));

    let release = release_remote_object_command(&context, "remote-object-1");
    let DevToolsCommand::ReleaseObjects(release) = release else {
        panic!("expected ReleaseObjects command");
    };
    assert_eq!(release.realm_id, None);
    assert_eq!(release.world_name, None);
    assert_eq!(
        release
            .handles
            .iter()
            .map(|handle| handle.as_str())
            .collect::<Vec<_>>(),
        vec!["remote-object-1"]
    );
    assert_eq!(
        release
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = resolve_element_command(&context, "not-a-moli-node", "clear")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);
}

#[test]
fn maps_element_click_to_shared_dom_geometry_and_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = element_click_prepare_commands(&context, "moli-node-23").expect("commands");
    assert_eq!(commands.len(), 2);
    let DevToolsCommand::ScrollIntoViewIfNeeded(scroll) = &commands[0] else {
        panic!("expected scroll command");
    };
    assert_eq!(
        scroll.reference,
        Some(DevToolsDomNodeReference::FrontendNodeId(23))
    );
    assert_eq!(
        scroll
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
    let DevToolsCommand::DomGeometry(geometry) = &commands[1] else {
        panic!("expected DOM geometry command");
    };
    assert_eq!(
        geometry.reference,
        DevToolsDomNodeReference::FrontendNodeId(23)
    );
    assert_eq!(
        geometry.operation,
        DevToolsDomGeometryOperation::GetBoxModel
    );

    let input = element_click_input_commands(
        &context,
        &box_model_geometry([10.0, 20.0, 30.0, 20.0, 30.0, 40.0, 10.0, 40.0], 20, 20),
    )
    .expect("input commands");
    assert_eq!(input.len(), 2);
    for (command, (event_type, buttons)) in input.into_iter().zip([
        (DevToolsMouseEventType::Pressed, Some(1)),
        (DevToolsMouseEventType::Released, Some(0)),
    ]) {
        let DevToolsCommand::DispatchMouseEvent(command) = command else {
            panic!("expected mouse dispatch");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.x, 20.0);
        assert_eq!(command.y, 30.0);
        assert_eq!(command.button, 0);
        assert_eq!(command.buttons, buttons);
    }

    let invalid = element_click_prepare_commands(&context, "not-a-moli-node")
        .expect_err("invalid element id should fail");
    assert_eq!(invalid.code, ClassicErrorCode::NoSuchElement);
}

#[test]
fn maps_element_send_keys_to_shared_input_key_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    assert_eq!(
        element_send_keys_text(&json!({ "text": "ab" })).expect("text"),
        "ab"
    );
    assert_eq!(
        element_send_keys_text(&json!({ "value": ["a", "b"] })).expect("legacy value"),
        "ab"
    );

    let commands = element_send_keys_input_commands(&context, "ab");
    assert_eq!(commands.len(), 4);
    for (command, (event_type, key, text, should_insert_text)) in commands.into_iter().zip([
        (DevToolsKeyEventType::KeyDown, "a", "a", true),
        (DevToolsKeyEventType::KeyUp, "a", "", false),
        (DevToolsKeyEventType::KeyDown, "b", "b", true),
        (DevToolsKeyEventType::KeyUp, "b", "", false),
    ]) {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.text, text);
        assert_eq!(command.should_insert_text, should_insert_text);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(DevToolsTargetId::as_str),
            Some("TID-1")
        );
    }

    let commands = element_send_keys_input_commands(&context, "a\u{E012}b\u{E017}\u{E003}");
    let expected = [
        (DevToolsKeyEventType::KeyDown, "a", "", "a", true),
        (DevToolsKeyEventType::KeyUp, "a", "", "", false),
        (
            DevToolsKeyEventType::KeyDown,
            "ArrowLeft",
            "ArrowLeft",
            "",
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "ArrowLeft",
            "ArrowLeft",
            "",
            false,
        ),
        (DevToolsKeyEventType::KeyDown, "b", "", "b", true),
        (DevToolsKeyEventType::KeyUp, "b", "", "", false),
        (DevToolsKeyEventType::KeyDown, "Delete", "Delete", "", false),
        (DevToolsKeyEventType::KeyUp, "Delete", "Delete", "", false),
        (
            DevToolsKeyEventType::KeyDown,
            "Backspace",
            "Backspace",
            "",
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Backspace",
            "Backspace",
            "",
            false,
        ),
    ];
    assert_eq!(commands.len(), expected.len());
    for (command, (event_type, key, code, text, should_insert_text)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.code, code);
        assert_eq!(command.text, text);
        assert_eq!(command.should_insert_text, should_insert_text);
    }

    let commands =
        element_send_keys_input_commands(&context, "\u{E008}\u{E012}\u{E012}\u{E012}\u{E017}");
    let expected = [
        (
            DevToolsKeyEventType::KeyDown,
            "Shift",
            "ShiftLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "ArrowLeft",
            "ArrowLeft",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "Delete",
            "Delete",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Delete",
            "Delete",
            "",
            8,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Shift",
            "ShiftLeft",
            "",
            0,
            false,
        ),
    ];
    assert_eq!(commands.len(), expected.len());
    for (command, (event_type, key, code, text, modifiers, should_insert_text)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.code, code);
        assert_eq!(command.text, text);
        assert_eq!(command.modifiers, modifiers);
        assert_eq!(command.should_insert_text, should_insert_text);
    }

    let commands = element_send_keys_input_commands(
        &context,
        "\u{E00E}\u{E00F}\u{E024}\u{E027}\u{E025}\u{E028}\u{E026}\u{E01A}\u{E023}\u{E029}\u{E01D}\u{E032}",
    );
    let expected = [
        ("PageUp", "PageUp", "", false),
        ("PageDown", "PageDown", "", false),
        ("*", "NumpadMultiply", "*", true),
        ("-", "NumpadSubtract", "-", true),
        ("+", "NumpadAdd", "+", true),
        (".", "NumpadDecimal", ".", true),
        (",", "NumpadComma", ",", true),
        ("0", "Numpad0", "0", true),
        ("9", "Numpad9", "9", true),
        ("/", "NumpadDivide", "/", true),
        ("3", "Numpad3", "3", true),
        ("F2", "F2", "", false),
    ];
    assert_eq!(commands.len(), expected.len() * 2);
    for (pair, (key, code, text, should_insert_text)) in commands.chunks_exact(2).zip(expected) {
        let DevToolsCommand::DispatchKeyEvent(key_down) = &pair[0] else {
            panic!("expected keyDown dispatch");
        };
        assert_eq!(key_down.event_type, DevToolsKeyEventType::KeyDown);
        assert_eq!(key_down.key, key);
        assert_eq!(key_down.code, code);
        assert_eq!(key_down.text, text);
        assert_eq!(key_down.should_insert_text, should_insert_text);

        let DevToolsCommand::DispatchKeyEvent(key_up) = &pair[1] else {
            panic!("expected keyUp dispatch");
        };
        assert_eq!(key_up.event_type, DevToolsKeyEventType::KeyUp);
        assert_eq!(key_up.key, key);
        assert_eq!(key_up.code, code);
        assert_eq!(key_up.text, "");
        assert!(!key_up.should_insert_text);
    }

    let commands = element_send_keys_input_commands(&context, "\u{E008}a\u{E000}a");
    let expected = [
        (
            DevToolsKeyEventType::KeyDown,
            "Shift",
            "ShiftLeft",
            "",
            8,
            false,
        ),
        (DevToolsKeyEventType::KeyDown, "A", "", "A", 8, true),
        (DevToolsKeyEventType::KeyUp, "A", "", "", 8, false),
        (
            DevToolsKeyEventType::KeyUp,
            "Shift",
            "ShiftLeft",
            "",
            0,
            false,
        ),
        (DevToolsKeyEventType::KeyDown, "a", "", "a", 0, true),
        (DevToolsKeyEventType::KeyUp, "a", "", "", 0, false),
    ];
    assert_eq!(commands.len(), expected.len());
    for (command, (event_type, key, code, text, modifiers, should_insert_text)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.code, code);
        assert_eq!(command.text, text);
        assert_eq!(command.modifiers, modifiers);
        assert_eq!(command.should_insert_text, should_insert_text);
    }

    let invalid =
        element_send_keys_text(&json!({ "text": false })).expect_err("non-string text should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
    let invalid = element_send_keys_text(&json!({ "value": ["a", false] }))
        .expect_err("non-string value entry should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn maps_pointer_actions_to_shared_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 20, "y": 21 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .expect("pointer actions should map");

    assert_eq!(commands.len(), 5);
    let expected = [
        (DevToolsMouseEventType::Moved, 20.0, 21.0, 0, Some(0), 0),
        (DevToolsMouseEventType::Pressed, 20.0, 21.0, 0, Some(1), 1),
        (DevToolsMouseEventType::Released, 20.0, 21.0, 0, Some(0), 1),
        (DevToolsMouseEventType::Pressed, 20.0, 21.0, 0, Some(1), 2),
        (DevToolsMouseEventType::Released, 20.0, 21.0, 0, Some(0), 2),
    ];
    for (command, (event_type, x, y, button, buttons, click_count)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchMouseEvent(command) = command else {
            panic!("expected mouse dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.x, x);
        assert_eq!(command.y, y);
        assert_eq!(command.button, button);
        assert_eq!(command.buttons, buttons);
        assert_eq!(command.click_count, click_count);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(DevToolsTargetId::as_str),
            Some("TID-1")
        );
    }

    let invalid = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "touch",
                "id": "touch",
                "actions": []
            }]
        }),
    )
    .expect_err("unsupported source type should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn maps_touch_pointer_actions_to_shared_touch_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();

    let commands = perform_actions_commands_with_state(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 20, "y": 21 },
                    { "type": "pointerDown" },
                    { "type": "pointerMove", "origin": "pointer", "x": 2, "y": 3 },
                    { "type": "pointerUp" }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        &mut state,
    )
    .expect("touch pointer actions should map");

    assert_eq!(commands.len(), 3);
    for (command, (event_type, x, y)) in commands.into_iter().zip([
        (DevToolsTouchEventType::Start, 20.0, 21.0),
        (DevToolsTouchEventType::Move, 22.0, 24.0),
        (DevToolsTouchEventType::End, 22.0, 24.0),
    ]) {
        let DevToolsCommand::DispatchTouchEvent(command) = command else {
            panic!("expected touch dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.touch_points.len(), 1);
        assert_eq!(command.touch_points[0].x, x);
        assert_eq!(command.touch_points[0].y, y);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(DevToolsTargetId::as_str),
            Some("TID-1")
        );
    }

    assert!(release_actions_commands(&context, &mut state).is_empty());
}

#[test]
fn coalesces_same_tick_touch_sources_into_multi_point_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();

    let commands = perform_actions_commands_with_state(
        &context,
        &json!({
            "actions": [
                {
                    "type": "pointer",
                    "id": "finger-1",
                    "parameters": { "pointerType": "touch" },
                    "actions": [
                        { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                        { "type": "pointerDown" },
                        { "type": "pointerMove", "origin": "pointer", "x": 1, "y": 1 },
                        { "type": "pointerUp" }
                    ]
                },
                {
                    "type": "pointer",
                    "id": "finger-2",
                    "parameters": { "pointerType": "touch" },
                    "actions": [
                        { "type": "pointerMove", "origin": "viewport", "x": 30, "y": 31 },
                        { "type": "pointerDown" },
                        { "type": "pointerMove", "origin": "pointer", "x": 1, "y": 1 },
                        { "type": "pointerUp" }
                    ]
                }
            ]
        }),
        &ClassicElementOriginViewportPoints::new(),
        &mut state,
    )
    .expect("multi-touch pointer actions should map");

    assert_eq!(commands.len(), 3);
    for (command, (event_type, points)) in commands.into_iter().zip([
        (
            DevToolsTouchEventType::Start,
            [(0, 10.0, 11.0), (1, 30.0, 31.0)],
        ),
        (
            DevToolsTouchEventType::Move,
            [(0, 11.0, 12.0), (1, 31.0, 32.0)],
        ),
        (
            DevToolsTouchEventType::End,
            [(0, 11.0, 12.0), (1, 31.0, 32.0)],
        ),
    ]) {
        let DevToolsCommand::DispatchTouchEvent(command) = command else {
            panic!("expected touch dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.touch_points.len(), 2);
        for (point, (id, x, y)) in command.touch_points.iter().zip(points) {
            assert_eq!(point.id, id);
            assert_eq!(point.x, x);
            assert_eq!(point.y, y);
        }
    }

    assert!(release_actions_commands(&context, &mut state).is_empty());
}

#[test]
fn releases_pressed_touch_pointer_source() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();

    let press = perform_actions_commands_with_state(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 7, "y": 8 },
                    { "type": "pointerDown" }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        &mut state,
    )
    .expect("touch press should map");
    assert_eq!(press.len(), 1);

    let release = release_actions_commands(&context, &mut state);
    assert_eq!(release.len(), 1);
    let DevToolsCommand::DispatchTouchEvent(command) = &release[0] else {
        panic!("expected touch release command");
    };
    assert_eq!(command.event_type, DevToolsTouchEventType::End);
    assert_eq!(command.touch_points[0].x, 7.0);
    assert_eq!(command.touch_points[0].y, 8.0);
    assert!(release_actions_commands(&context, &mut state).is_empty());
}

#[test]
fn maps_pen_pointer_actions_to_shared_mouse_commands_with_pointer_type() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "pen",
                "parameters": { "pointerType": "pen" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 20, "y": 21 },
                    {
                        "type": "pointerDown",
                        "button": 0,
                        "pressure": 0.75,
                        "tangentialPressure": -0.25,
                        "tiltX": 12,
                        "tiltY": -8,
                        "twist": 45
                    },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .expect("pen pointer actions should map");

    assert_eq!(commands.len(), 3);
    for (command, event_type) in commands.into_iter().zip([
        DevToolsMouseEventType::Moved,
        DevToolsMouseEventType::Pressed,
        DevToolsMouseEventType::Released,
    ]) {
        let DevToolsCommand::DispatchMouseEvent(command) = command else {
            panic!("expected mouse dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.pointer_type, DevToolsPointerType::Pen);
        assert_eq!(command.x, 20.0);
        assert_eq!(command.y, 21.0);
        match event_type {
            DevToolsMouseEventType::Moved => {
                assert_eq!(command.force, 0.0);
                assert_eq!(command.tangential_pressure, 0.0);
                assert_eq!(command.tilt_x, 0.0);
                assert_eq!(command.tilt_y, 0.0);
                assert_eq!(command.twist, 0.0);
            }
            DevToolsMouseEventType::Pressed => {
                assert_eq!(command.force, 0.75);
                assert_eq!(command.tangential_pressure, -0.25);
                assert_eq!(command.tilt_x, 12.0);
                assert_eq!(command.tilt_y, -8.0);
                assert_eq!(command.twist, 45.0);
            }
            DevToolsMouseEventType::Released => {
                assert_eq!(command.force, 0.0);
                assert_eq!(command.tangential_pressure, 0.0);
                assert_eq!(command.tilt_x, 0.0);
                assert_eq!(command.tilt_y, 0.0);
                assert_eq!(command.twist, 0.0);
            }
            DevToolsMouseEventType::Wheel => unreachable!("pen sequence has no wheel"),
        }
    }

    let invalid = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "eraser",
                "parameters": { "pointerType": "eraser" },
                "actions": []
            }]
        }),
    )
    .expect_err("unsupported pointer types should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn rejects_invalid_pointer_action_properties() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    for action in [
        json!({ "type": "pointerDown", "button": 0, "pressure": 1.25 }),
        json!({ "type": "pointerDown", "button": 0, "tangentialPressure": -1.25 }),
        json!({ "type": "pointerDown", "button": 0, "tiltX": 91 }),
        json!({ "type": "pointerDown", "button": 0, "tiltY": -91 }),
        json!({ "type": "pointerDown", "button": 0, "twist": 360 }),
        json!({ "type": "pointerDown", "button": 0, "width": -1 }),
        json!({ "type": "pointerDown", "button": 0, "height": -1 }),
    ] {
        let invalid = perform_actions_commands(
            &context,
            &json!({
                "actions": [{
                    "type": "pointer",
                    "id": "pen",
                    "parameters": { "pointerType": "pen" },
                    "actions": [action]
                }]
            }),
        )
        .expect_err("invalid pointer action property should fail");
        assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
    }
}

#[test]
fn groups_action_commands_by_tick_duration() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();

    let ticks = perform_actions_ticks_with_state_and_viewport(
            &context,
            &json!({
                "actions": [
                    {
                        "type": "pointer",
                        "id": "mouse",
                        "parameters": { "pointerType": "mouse" },
                        "actions": [
                            { "type": "pointerMove", "origin": "viewport", "x": 20, "y": 21, "duration": 30 },
                            { "type": "pointerDown", "button": 0 }
                        ]
                    },
                    {
                        "type": "key",
                        "id": "keyboard",
                        "actions": [
                            { "type": "pause", "duration": 5 },
                            { "type": "keyDown", "value": "a" }
                        ]
                    },
                    {
                        "type": "wheel",
                        "id": "wheel",
                        "actions": [
                            {
                                "type": "scroll",
                                "origin": "viewport",
                                "x": 10,
                                "y": 11,
                                "deltaX": 1,
                                "deltaY": 2,
                                "duration": 45
                            }
                        ]
                    },
                    {
                        "type": "none",
                        "id": "timer",
                        "actions": [{ "type": "pause", "duration": 40 }]
                    }
                ]
            }),
            &ClassicElementOriginViewportPoints::new(),
            None,
            &mut state,
        )
        .expect("actions should group into ticks");

    assert_eq!(ticks.len(), 2);
    assert_eq!(ticks[0].duration_ms, 45);
    assert_eq!(ticks[0].commands.len(), 2);
    assert_eq!(ticks[1].duration_ms, 0);
    assert_eq!(ticks[1].commands.len(), 2);

    let DevToolsCommand::DispatchMouseEvent(pointer_move) = &ticks[0].commands[0] else {
        panic!("expected pointer move in first tick");
    };
    assert_eq!(pointer_move.event_type, DevToolsMouseEventType::Moved);
    assert_eq!(pointer_move.x, 20.0);
    assert_eq!(pointer_move.y, 21.0);

    let DevToolsCommand::DispatchMouseEvent(wheel_scroll) = &ticks[0].commands[1] else {
        panic!("expected wheel scroll in first tick");
    };
    assert_eq!(wheel_scroll.event_type, DevToolsMouseEventType::Wheel);
    assert_eq!(wheel_scroll.delta_x, 1.0);
    assert_eq!(wheel_scroll.delta_y, 2.0);

    let DevToolsCommand::DispatchMouseEvent(pointer_down) = &ticks[1].commands[0] else {
        panic!("expected pointer down in second tick");
    };
    assert_eq!(pointer_down.event_type, DevToolsMouseEventType::Pressed);

    let DevToolsCommand::DispatchKeyEvent(key_down) = &ticks[1].commands[1] else {
        panic!("expected key down in second tick");
    };
    assert_eq!(key_down.event_type, DevToolsKeyEventType::KeyDown);
    assert_eq!(key_down.key, "a");
}

#[test]
fn rejects_invalid_action_durations() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let mut state = ClassicActionState::default();
    let negative = perform_actions_ticks_with_state_and_viewport(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 0, "y": 0, "duration": -1 }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        None,
        &mut state,
    )
    .expect_err("negative pointerMove duration should fail");
    assert_eq!(negative.code, ClassicErrorCode::InvalidArgument);

    let mut state = ClassicActionState::default();
    let fractional = perform_actions_ticks_with_state_and_viewport(
        &context,
        &json!({
            "actions": [{
                "type": "none",
                "id": "timer",
                "actions": [{ "type": "pause", "duration": 1.5 }]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        None,
        &mut state,
    )
    .expect_err("fractional pause duration should fail");
    assert_eq!(fractional.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn collects_action_element_origins_for_pointer_and_wheel_sources() {
    let origins = action_element_origin_ids(&json!({
        "actions": [
            {
                "type": "pointer",
                "id": "mouse",
                "actions": [{
                    "type": "pointerMove",
                    "origin": { "element-6066-11e4-a52e-4f735466cecf": "moli-node-7" },
                    "x": 0,
                    "y": 0
                }]
            },
            {
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": { "element-6066-11e4-a52e-4f735466cecf": "moli-node-9" },
                    "x": 0,
                    "y": 0,
                    "deltaX": 0,
                    "deltaY": 1
                }]
            },
            {
                "type": "key",
                "id": "keyboard",
                "actions": [{ "type": "pause" }]
            }
        ]
    }))
    .expect("element origins should collect");
    assert_eq!(
        origins,
        vec!["moli-node-7".to_owned(), "moli-node-9".to_owned()]
    );

    let invalid = action_element_origin_ids(&json!({
        "actions": [{
            "type": "pointer",
            "id": "mouse",
            "actions": [{
                "type": "pointerMove",
                "origin": {},
                "x": 0,
                "y": 0
            }]
        }]
    }))
    .expect_err("element origin object without element reference should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn maps_pointer_actions_with_element_origin_to_shared_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut origins = ClassicElementOriginViewportPoints::new();
    origins.insert(
        "moli-node-23".to_owned(),
        ClassicViewportPoint::new(20.0, 30.0).expect("origin point"),
    );

    let commands = perform_actions_commands_with_element_origins(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    {
                        "type": "pointerMove",
                        "origin": { "element-6066-11e4-a52e-4f735466cecf": "moli-node-23" },
                        "x": 3,
                        "y": -4
                    },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
        &origins,
    )
    .expect("pointer element origin should map");

    let expected = [
        (DevToolsMouseEventType::Moved, 23.0, 26.0, 0, Some(0)),
        (DevToolsMouseEventType::Pressed, 23.0, 26.0, 0, Some(1)),
        (DevToolsMouseEventType::Released, 23.0, 26.0, 0, Some(0)),
    ];
    for (command, (event_type, x, y, button, buttons)) in commands.into_iter().zip(expected) {
        let DevToolsCommand::DispatchMouseEvent(command) = command else {
            panic!("expected mouse dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.x, x);
        assert_eq!(command.y, y);
        assert_eq!(command.button, button);
        assert_eq!(command.buttons, buttons);
    }

    let unresolved = perform_actions_commands_with_element_origins(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "actions": [{
                    "type": "pointerMove",
                    "origin": { "element-6066-11e4-a52e-4f735466cecf": "moli-node-99" },
                    "x": 0,
                    "y": 0
                }]
            }]
        }),
        &origins,
    )
    .expect_err("unresolved element origin should fail");
    assert_eq!(unresolved.code, ClassicErrorCode::UnknownError);
}

#[test]
fn maintains_action_state_across_commands_and_releases_pressed_sources() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();

    let press_commands = perform_actions_commands_with_state(
        &context,
        &json!({
            "actions": [
                {
                    "type": "pointer",
                    "id": "mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        { "type": "pointerMove", "origin": "viewport", "x": 20, "y": 21 },
                        { "type": "pointerDown", "button": 0 }
                    ]
                },
                {
                    "type": "key",
                    "id": "keyboard",
                    "actions": [{ "type": "keyDown", "value": "\u{E009}" }]
                }
            ]
        }),
        &ClassicElementOriginViewportPoints::new(),
        &mut state,
    )
    .expect("press actions should map");
    assert_eq!(press_commands.len(), 3);

    let moved = perform_actions_commands_with_state(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "pointer", "x": 2, "y": -1 }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        &mut state,
    )
    .expect("pointer origin should use persisted pointer position");
    assert_eq!(moved.len(), 1);
    let DevToolsCommand::DispatchMouseEvent(command) = &moved[0] else {
        panic!("expected mouse dispatch command");
    };
    assert_eq!(command.event_type, DevToolsMouseEventType::Moved);
    assert_eq!(command.x, 22.0);
    assert_eq!(command.y, 20.0);
    assert_eq!(command.buttons, Some(1));

    let release = release_actions_commands(&context, &mut state);
    assert_eq!(release.len(), 2);
    let DevToolsCommand::DispatchMouseEvent(mouse_up) = &release[0] else {
        panic!("expected mouse release command first");
    };
    assert_eq!(mouse_up.event_type, DevToolsMouseEventType::Released);
    assert_eq!(mouse_up.x, 22.0);
    assert_eq!(mouse_up.y, 20.0);
    assert_eq!(mouse_up.button, 0);
    assert_eq!(mouse_up.buttons, Some(0));

    let DevToolsCommand::DispatchKeyEvent(key_up) = &release[1] else {
        panic!("expected key release command second");
    };
    assert_eq!(key_up.event_type, DevToolsKeyEventType::KeyUp);
    assert_eq!(key_up.key, "Control");
    assert_eq!(key_up.modifiers, 0);
    assert!(release_actions_commands(&context, &mut state).is_empty());
}

#[test]
fn rejects_action_targets_outside_viewport_bounds() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut state = ClassicActionState::default();
    let bounds = ClassicViewportBounds::new(100, 50);

    let edge = perform_actions_commands_with_state_and_viewport(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 100, "y": 50 }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        Some(bounds),
        &mut state,
    )
    .expect("viewport edge should be accepted");
    assert_eq!(edge.len(), 1);

    let out_of_bounds = perform_actions_commands_with_state_and_viewport(
        &context,
        &json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 101, "y": 50 }
                ]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        Some(bounds),
        &mut state,
    )
    .expect_err("pointer move outside viewport should fail");
    assert_eq!(out_of_bounds.code, ClassicErrorCode::MoveTargetOutOfBounds);

    let wheel_out_of_bounds = perform_actions_commands_with_state_and_viewport(
        &context,
        &json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": "viewport",
                    "x": -1,
                    "y": 0,
                    "deltaX": 0,
                    "deltaY": 1
                }]
            }]
        }),
        &ClassicElementOriginViewportPoints::new(),
        Some(bounds),
        &mut state,
    )
    .expect_err("wheel scroll outside viewport should fail");
    assert_eq!(
        wheel_out_of_bounds.code,
        ClassicErrorCode::MoveTargetOutOfBounds
    );
}

#[test]
fn maps_wheel_actions_to_shared_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [
                    { "type": "pause" },
                    {
                        "type": "scroll",
                        "origin": "viewport",
                        "x": 20,
                        "y": 21,
                        "deltaX": 5,
                        "deltaY": -10
                    }
                ]
            }]
        }),
    )
    .expect("wheel actions should map");

    assert_eq!(commands.len(), 1);
    let DevToolsCommand::DispatchMouseEvent(command) = commands.into_iter().next().unwrap() else {
        panic!("expected mouse dispatch command");
    };
    assert_eq!(command.event_type, DevToolsMouseEventType::Wheel);
    assert_eq!(command.x, 20.0);
    assert_eq!(command.y, 21.0);
    assert_eq!(command.button, 0);
    assert_eq!(command.buttons, Some(0));
    assert_eq!(command.delta_x, 5.0);
    assert_eq!(command.delta_y, -10.0);
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let invalid = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{ "type": "scroll", "x": 1.5, "y": 0, "deltaX": 0, "deltaY": 0 }]
            }]
        }),
    )
    .expect_err("fractional wheel coordinates should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);

    let invalid = perform_actions_commands(
            &context,
            &json!({
                "actions": [{
                    "type": "wheel",
                    "id": "wheel",
                    "actions": [{ "type": "scroll", "x": 0, "y": 0, "deltaX": 0, "deltaY": 0, "origin": "pointer" }]
                }]
            }),
        )
        .expect_err("pointer origin is not supported for wheel actions");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn maps_wheel_actions_with_element_origin_to_shared_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let mut origins = ClassicElementOriginViewportPoints::new();
    origins.insert(
        "moli-node-42".to_owned(),
        ClassicViewportPoint::new(10.0, 20.0).expect("origin point"),
    );

    let commands = perform_actions_commands_with_element_origins(
        &context,
        &json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": { "element-6066-11e4-a52e-4f735466cecf": "moli-node-42" },
                    "x": 3,
                    "y": 4,
                    "deltaX": 5,
                    "deltaY": -6
                }]
            }]
        }),
        &origins,
    )
    .expect("wheel element origin should map");

    assert_eq!(commands.len(), 1);
    let DevToolsCommand::DispatchMouseEvent(command) = commands.into_iter().next().unwrap() else {
        panic!("expected mouse dispatch command");
    };
    assert_eq!(command.event_type, DevToolsMouseEventType::Wheel);
    assert_eq!(command.x, 13.0);
    assert_eq!(command.y, 24.0);
    assert_eq!(command.delta_x, 5.0);
    assert_eq!(command.delta_y, -6.0);
}

#[test]
fn maps_key_actions_to_shared_input_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyUp", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "\u{E003}" },
                    { "type": "keyUp", "value": "\u{E003}" }
                ]
            }]
        }),
    )
    .expect("key actions should map");

    assert_eq!(commands.len(), 6);
    let expected = [
        (
            DevToolsKeyEventType::KeyDown,
            "Control",
            "ControlLeft",
            "",
            CLASSIC_MODIFIER_CONTROL,
            false,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "a",
            "",
            "",
            CLASSIC_MODIFIER_CONTROL,
            false,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "a",
            "",
            "",
            CLASSIC_MODIFIER_CONTROL,
            false,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Control",
            "ControlLeft",
            "",
            0,
            false,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "Backspace",
            "Backspace",
            "",
            0,
            false,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Backspace",
            "Backspace",
            "",
            0,
            false,
            false,
        ),
    ];
    for (command, (event_type, key, code, text, modifiers, auto_repeat, should_insert_text)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.code, code);
        assert_eq!(command.text, text);
        assert_eq!(command.modifiers, modifiers);
        assert_eq!(command.auto_repeat, auto_repeat);
        assert_eq!(command.should_insert_text, should_insert_text);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(DevToolsTargetId::as_str),
            Some("TID-1")
        );
    }

    let invalid = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [{ "type": "keyDown", "value": "ab" }]
            }]
        }),
    )
    .expect_err("multi-character key action value should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn maps_repeated_keydown_action_to_auto_repeat_key_event() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyDown", "value": "a" }
                ]
            }]
        }),
    )
    .expect("repeated key actions should map");

    assert_eq!(commands.len(), 4);
    let repeats = commands
        .iter()
        .map(|command| {
            let DevToolsCommand::DispatchKeyEvent(command) = command else {
                panic!("expected key dispatch command");
            };
            command.auto_repeat
        })
        .collect::<Vec<_>>();
    assert_eq!(repeats, vec![false, true, false, false]);
}

#[test]
fn maps_shift_modified_key_actions_to_shifted_key_and_text() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let commands = perform_actions_commands(
        &context,
        &json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "\u{E008}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyDown", "value": "A" },
                    { "type": "keyUp", "value": "A" },
                    { "type": "keyDown", "value": "1" },
                    { "type": "keyUp", "value": "1" },
                    { "type": "keyDown", "value": "-" },
                    { "type": "keyUp", "value": "-" },
                    { "type": "keyUp", "value": "\u{E008}" }
                ]
            }]
        }),
    )
    .expect("shift key actions should map");

    assert_eq!(commands.len(), 10);
    let expected = [
        (
            DevToolsKeyEventType::KeyDown,
            "Shift",
            "ShiftLeft",
            "",
            CLASSIC_MODIFIER_SHIFT,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "A",
            "",
            "A",
            CLASSIC_MODIFIER_SHIFT,
            true,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "A",
            "",
            "",
            CLASSIC_MODIFIER_SHIFT,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "A",
            "",
            "A",
            CLASSIC_MODIFIER_SHIFT,
            true,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "A",
            "",
            "",
            CLASSIC_MODIFIER_SHIFT,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "!",
            "",
            "!",
            CLASSIC_MODIFIER_SHIFT,
            true,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "!",
            "",
            "",
            CLASSIC_MODIFIER_SHIFT,
            false,
        ),
        (
            DevToolsKeyEventType::KeyDown,
            "_",
            "",
            "_",
            CLASSIC_MODIFIER_SHIFT,
            true,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "_",
            "",
            "",
            CLASSIC_MODIFIER_SHIFT,
            false,
        ),
        (
            DevToolsKeyEventType::KeyUp,
            "Shift",
            "ShiftLeft",
            "",
            0,
            false,
        ),
    ];
    for (command, (event_type, key, code, text, modifiers, should_insert_text)) in
        commands.into_iter().zip(expected)
    {
        let DevToolsCommand::DispatchKeyEvent(command) = command else {
            panic!("expected key dispatch command");
        };
        assert_eq!(command.event_type, event_type);
        assert_eq!(command.key, key);
        assert_eq!(command.code, code);
        assert_eq!(command.text, text);
        assert_eq!(command.modifiers, modifiers);
        assert_eq!(command.should_insert_text, should_insert_text);
    }
}

#[test]
fn maps_window_commands_to_shared_target_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let handles = window_handles_command(&context);
    let DevToolsCommand::GetTargets(handles) = handles else {
        panic!("expected GetTargets command");
    };
    assert!(handles.root.is_none());
    assert_eq!(
        handles
            .context
            .session_id
            .as_ref()
            .map(DevToolsSessionId::as_str),
        Some("classic-session-1")
    );

    let new_window = new_window_command(&context);
    let DevToolsCommand::CreateTarget(new_window) = new_window else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(new_window.url, "about:blank");
    assert!(
        !new_window.activate,
        "WebDriver New Window must not switch the current top-level browsing context"
    );

    let switch =
        switch_window_command(&context, &json!({"handle": "TID-2"})).expect("switch command");
    let DevToolsCommand::ActivateTarget(switch) = switch else {
        panic!("expected ActivateTarget command");
    };
    assert_eq!(switch.target_id.as_str(), "TID-2");
    assert_eq!(
        switch
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-2")
    );

    let close = close_window_command(&context).expect("close command");
    let DevToolsCommand::CloseTarget(close) = close else {
        panic!("expected CloseTarget command");
    };
    assert_eq!(close.target_id.as_str(), "TID-1");

    let set_rect = set_window_rect_command(&context, 640, 480);
    let DevToolsCommand::SetViewport(set_rect) = set_rect else {
        panic!("expected SetViewport command");
    };
    assert_eq!(
        set_rect.viewport,
        DevToolsViewportSetting::Dimensions {
            width: 640,
            height: 480
        }
    );
    assert_eq!(
        set_rect
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_classic_window_state_to_headless_viewport_contract() {
    let context = ClassicDevToolsCommandContext::with_target_id("SID-1", "TID-1");
    let current = ClassicWindowRect {
        x: 10,
        y: 20,
        width: 640,
        height: 480,
    };

    assert_eq!(
        classic_window_rect_for_state(current, ClassicWindowState::Minimized),
        current,
        "minimize preserves the restore rect in the lightweight headless model"
    );
    assert!(
        set_window_state_command(&context, ClassicWindowState::Minimized).is_none(),
        "minimize has no viewport command because Moli has no OS window"
    );
    let DevToolsCommand::SetWindowState(minimize_surface) =
        set_window_surface_state_command(&context, ClassicWindowState::Minimized)
    else {
        panic!("minimize should map to SetWindowState");
    };
    assert_eq!(minimize_surface.state, DevToolsWindowState::Minimized);
    assert_eq!(
        minimize_surface
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let maximized = classic_window_rect_for_state(current, ClassicWindowState::Maximized);
    assert_eq!(
        maximized,
        ClassicWindowRect {
            x: 0,
            y: 0,
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
        }
    );
    let Some(DevToolsCommand::SetViewport(maximize_command)) =
        set_window_state_command(&context, ClassicWindowState::Maximized)
    else {
        panic!("maximize should map to SetViewport");
    };
    assert_eq!(
        maximize_command.viewport,
        DevToolsViewportSetting::Dimensions {
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
        }
    );
    assert_eq!(
        (
            maximize_command.screen_width,
            maximize_command.screen_height
        ),
        (
            Some(CLASSIC_HEADLESS_SCREEN_WIDTH),
            Some(CLASSIC_HEADLESS_SCREEN_HEIGHT)
        )
    );
    let DevToolsCommand::SetWindowState(maximize_surface) =
        set_window_surface_state_command(&context, ClassicWindowState::Maximized)
    else {
        panic!("maximize should map to SetWindowState");
    };
    assert_eq!(maximize_surface.state, DevToolsWindowState::Maximized);

    let fullscreen = classic_window_rect_for_state(current, ClassicWindowState::Fullscreen);
    assert_eq!(
        fullscreen,
        ClassicWindowRect {
            x: 0,
            y: 0,
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_SCREEN_HEIGHT,
        }
    );
    let Some(DevToolsCommand::SetViewport(fullscreen_command)) =
        set_window_state_command(&context, ClassicWindowState::Fullscreen)
    else {
        panic!("fullscreen should map to SetViewport");
    };
    assert_eq!(
        fullscreen_command.viewport,
        DevToolsViewportSetting::Dimensions {
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_SCREEN_HEIGHT,
        }
    );
    assert_eq!(
        (
            fullscreen_command.screen_width,
            fullscreen_command.screen_height
        ),
        (
            Some(CLASSIC_HEADLESS_SCREEN_WIDTH),
            Some(CLASSIC_HEADLESS_SCREEN_HEIGHT)
        )
    );
    let DevToolsCommand::SetWindowState(fullscreen_surface) =
        set_window_surface_state_command(&context, ClassicWindowState::Fullscreen)
    else {
        panic!("fullscreen should map to SetWindowState");
    };
    assert_eq!(fullscreen_surface.state, DevToolsWindowState::Fullscreen);
    let DevToolsCommand::SetWindowState(normal_surface) =
        set_window_normal_surface_state_command(&context)
    else {
        panic!("set rect should map to normal SetWindowState");
    };
    assert_eq!(normal_surface.state, DevToolsWindowState::Normal);
}

#[test]
fn parses_classic_window_rect_params_like_webdriver() {
    let update = set_window_rect_update(&json!({
        "x": 10.9,
        "y": -8.9,
        "width": 650.5,
        "height": 420
    }))
    .expect("valid window rect update");
    assert_eq!(update.x, Some(10));
    assert_eq!(update.y, Some(-8));
    assert_eq!(update.width, Some(650));
    assert_eq!(update.height, Some(420));

    let unchanged = set_window_rect_update(&json!({
        "x": null,
        "y": null,
        "width": null,
        "height": null
    }))
    .expect("null fields should be treated as unchanged");
    assert_eq!(unchanged, ClassicWindowRectUpdate::default());

    let original = ClassicWindowRect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    };
    assert_eq!(
        original.with_update(update),
        ClassicWindowRect {
            x: 10,
            y: -8,
            width: 650,
            height: 420,
        }
    );
    assert_eq!(
        original.value(),
        json!({
            "x": 0,
            "y": 0,
            "width": 800,
            "height": 600,
        })
    );

    for invalid in [
        json!({"width": "650"}),
        json!({"height": false}),
        json!({"x": []}),
        json!({"y": {}}),
        json!({"width": -1}),
        json!({"height": 0}),
        json!(null),
    ] {
        let error = set_window_rect_update(&invalid).expect_err("invalid rect should fail");
        assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    }
}

#[test]
fn maps_alert_commands_to_shared_page_dialog_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("SID-1", "TID-1");

    let DevToolsCommand::GetJavaScriptDialog(get) = alert_text_command(&context) else {
        panic!("alert text should map to get dialog command");
    };
    assert_eq!(get.context.session_id.as_ref().unwrap().as_str(), "SID-1");
    assert_eq!(get.context.target_id.as_ref().unwrap().as_str(), "TID-1");

    let DevToolsCommand::HandleJavaScriptDialog(accept) = alert_handle_command(&context, true)
    else {
        panic!("accept alert should map to handle dialog command");
    };
    assert!(accept.accept);
    assert_eq!(accept.prompt_text, "");

    let DevToolsCommand::HandleJavaScriptDialog(dismiss) = alert_handle_command(&context, false)
    else {
        panic!("dismiss alert should map to handle dialog command");
    };
    assert!(!dismiss.accept);

    let DevToolsCommand::SetJavaScriptDialogPromptText(send_text) =
        alert_send_text_command(&context, &json!({"text": "cheese"})).expect("send text command")
    else {
        panic!("send alert text should map to set dialog prompt text command");
    };
    assert_eq!(send_text.prompt_text, "cheese");

    let invalid = alert_send_text_command(&context, &json!({"text": false}))
        .expect_err("non-string text should fail");
    assert_eq!(invalid.code, ClassicErrorCode::InvalidArgument);
}

#[test]
fn serializes_element_not_interactable_error_code() {
    assert_eq!(
        error_response(
            ClassicErrorCode::ElementNotInteractable,
            "current user prompt is not a prompt"
        )["value"]["error"],
        json!("element not interactable")
    );
}

#[test]
fn extracts_classic_window_handles_from_page_targets() {
    let handles = window_handles_from_targets(DevToolsGetTargetsResult {
        targets: vec![
            moli_protocol::devtools_runtime::DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("PAGE-1")),
                kind: DevToolsTargetKind::Page,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: false,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
            moli_protocol::devtools_runtime::DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("TAB-PAGE-1")),
                kind: DevToolsTargetKind::Tab,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: false,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
            moli_protocol::devtools_runtime::DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("WORKER-1")),
                kind: DevToolsTargetKind::Worker,
                title: String::new(),
                url: String::new(),
                attached: false,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
            moli_protocol::devtools_runtime::DevToolsTargetInfo {
                target_id: None,
                kind: DevToolsTargetKind::Browser,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: true,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
        ],
    });

    assert_eq!(handles, vec!["PAGE-1"]);
}

#[test]
fn rejects_invalid_classic_window_params() {
    assert_eq!(new_window_type(&json!({})), Ok("tab".to_owned()));
    assert_eq!(
        new_window_type(&json!({"type": null})),
        Ok("tab".to_owned())
    );
    assert_eq!(
        new_window_type(&json!({"type": "window"})),
        Ok("window".to_owned())
    );
    assert_eq!(
        new_window_type(&json!({"type": "popup"})),
        Ok("tab".to_owned())
    );

    let error = new_window_type(&json!({"type": false}))
        .expect_err("non-string new window type should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "type must be a string");

    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let error = switch_window_command(&context, &json!({"handle": false}))
        .expect_err("non-string handle should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "handle must be a string");
}

#[test]
fn maps_refresh_and_history_traversal_to_shared_devtools_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let refresh = refresh_command(&context, DevToolsNavigationWait::Load);
    let DevToolsCommand::Reload(refresh) = refresh else {
        panic!("expected Reload command");
    };
    assert_eq!(refresh.wait, DevToolsNavigationWait::Load);
    assert_eq!(
        refresh
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let history_command = navigation_history_command(&context);
    let DevToolsCommand::GetNavigationHistory(history_command) = history_command else {
        panic!("expected GetNavigationHistory command");
    };
    assert_eq!(
        history_command
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let history = DevToolsGetNavigationHistoryResult {
        current_index: 1,
        entries: vec![
            moli_protocol::devtools_runtime::DevToolsNavigationHistoryEntry {
                id: 7,
                url: "https://example.test/first".to_owned(),
                user_typed_url: "https://example.test/first".to_owned(),
                title: "first".to_owned(),
                transition_type: "typed".to_owned(),
            },
            moli_protocol::devtools_runtime::DevToolsNavigationHistoryEntry {
                id: 8,
                url: "https://example.test/second".to_owned(),
                user_typed_url: "https://example.test/second".to_owned(),
                title: "second".to_owned(),
                transition_type: "typed".to_owned(),
            },
        ],
    };
    assert_eq!(
        history_traversal_entry(&history, -1),
        Some((7, "https://example.test/first".to_owned()))
    );
    assert_eq!(history_traversal_entry(&history, 1), None);

    let traverse = traverse_history_command(
        &context,
        7,
        "https://example.test/first",
        DevToolsNavigationWait::Load,
    );
    let DevToolsCommand::TraverseHistory(traverse) = traverse else {
        panic!("expected TraverseHistory command");
    };
    assert_eq!(
        traverse.destination,
        DevToolsHistoryTraversalDestination::Entry {
            entry_id: 7,
            url: "https://example.test/first".to_owned(),
        }
    );
    assert_eq!(traverse.wait, DevToolsNavigationWait::Load);
    assert_eq!(
        traverse
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_execute_sync_to_shared_call_function_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let execute = execute_sync_command(
        &context,
        &json!({
            "script": "return arguments[0].nested + arguments[1];",
            "args": [
                { "nested": 4 },
                3
            ]
        }),
    )
    .expect("execute command");

    let DevToolsCommand::CallFunction(execute) = execute else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        execute.function_declaration,
        "async function() {\nreturn arguments[0].nested + arguments[1];\n}"
    );
    assert_eq!(
        execute.arguments,
        vec![json!({"value": {"nested": 4}}), json!({"value": 3})]
    );
    assert!(execute.await_promise);
    assert_eq!(execute.result_ownership, DevToolsResultOwnership::None);
    assert_eq!(
        execute
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_execute_async_to_shared_callback_call_function_command() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let execute = execute_async_command(
        &context,
        &json!({
            "script": "arguments[arguments.length - 1](arguments[0].nested + arguments[1]);",
            "args": [
                { "nested": 4 },
                3
            ]
        }),
    )
    .expect("execute async command");

    let DevToolsCommand::CallFunction(execute) = execute else {
        panic!("expected CallFunction command");
    };
    assert!(
        execute
            .function_declaration
            .contains("const __moliUserFunction = async function()")
    );
    assert!(
        execute
            .function_declaration
            .contains("Promise.resolve(__moliScriptResult).catch")
    );
    assert!(
        execute
            .function_declaration
            .contains("arguments[arguments.length - 1](arguments[0].nested + arguments[1]);")
    );
    assert!(execute.function_declaration.contains("__moliArgs.push"));
    assert_eq!(
        execute.arguments,
        vec![json!({"value": {"nested": 4}}), json!({"value": 3})]
    );
    assert!(execute.await_promise);
    assert_eq!(execute.result_ownership, DevToolsResultOwnership::None);
    assert_eq!(
        execute
            .context
            .target_id
            .as_ref()
            .map(DevToolsTargetId::as_str),
        Some("TID-1")
    );
}

#[test]
fn maps_cookie_commands_to_shared_storage_commands() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");
    let current_url = "https://example.test/path";

    let get = get_cookies_command(&context, current_url);
    let DevToolsCommand::GetCookies(get) = get else {
        panic!("expected GetCookies command");
    };
    assert_eq!(get.urls, Some(vec![current_url.to_owned()]));
    assert_eq!(
        get.context.target_id.as_ref().map(DevToolsTargetId::as_str),
        Some("TID-1")
    );

    let add = add_cookie_command(
        &context,
        &json!({
            "cookie": {
                "name": "sid",
                "value": "abc",
                "path": "/",
                "secure": true,
                "httpOnly": true,
                "sameSite": "Lax",
                "expiry": 1_800_000_000
            }
        }),
        current_url,
    )
    .expect("add cookie command");
    let DevToolsCommand::SetCookies(add) = add else {
        panic!("expected SetCookies command");
    };
    assert_eq!(add.cookies.len(), 1);
    assert_eq!(add.cookies[0].name, "sid");
    assert_eq!(add.cookies[0].value, "abc");
    assert_eq!(add.cookies[0].url.as_deref(), Some(current_url));
    assert_eq!(add.cookies[0].path.as_deref(), Some("/"));
    assert_eq!(add.cookies[0].secure, Some(true));
    assert!(add.cookies[0].http_only);
    assert_eq!(add.cookies[0].same_site.as_deref(), Some("Lax"));
    assert_eq!(add.cookies[0].expires, Some(1_800_000_000.0));

    for invalid_expiry in [json!(-1), json!(0.5), json!(9_007_199_254_740_992_u64)] {
        let error = add_cookie_command(
            &context,
            &json!({
                "cookie": {
                    "name": "bad",
                    "value": "expiry",
                    "expiry": invalid_expiry
                }
            }),
            current_url,
        )
        .expect_err("invalid expiry should fail before SetCookies");
        assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    }

    let default_add = add_cookie_command(
        &context,
        &json!({
            "cookie": {
                "name": "default",
                "value": "value"
            }
        }),
        current_url,
    )
    .expect("default add cookie command");
    let DevToolsCommand::SetCookies(default_add) = default_add else {
        panic!("expected SetCookies command");
    };
    assert_eq!(default_add.cookies[0].secure, Some(false));
    assert!(default_add.cookies[0].same_site.is_none());

    let delete = delete_cookie_command(&context, "sid", current_url);
    let DevToolsCommand::DeleteCookies(delete) = delete else {
        panic!("expected DeleteCookies command");
    };
    assert_eq!(delete.name.as_deref(), Some("sid"));
    assert_eq!(delete.url.as_deref(), Some(current_url));

    let delete_all = delete_all_cookies_command(&context, current_url);
    let DevToolsCommand::DeleteCookies(delete_all) = delete_all else {
        panic!("expected DeleteCookies command");
    };
    assert!(delete_all.name.is_none());
    assert_eq!(delete_all.url.as_deref(), Some(current_url));
}

#[test]
fn maps_devtools_cookies_to_classic_cookie_shape() {
    let result = DevToolsGetCookiesResult {
        cookies: vec![json!({
            "name": "sid",
            "value": "abc",
            "domain": "example.test",
            "path": "/",
            "secure": true,
            "httpOnly": true,
            "sameSite": "Strict",
            "expires": 1_800_000_000.9
        })],
    };

    let cookies = classic_cookies_from_devtools(result);

    assert_eq!(
        cookies,
        vec![json!({
            "name": "sid",
            "value": "abc",
            "domain": "example.test",
            "path": "/",
            "secure": true,
            "httpOnly": true,
            "sameSite": "Strict",
            "expiry": 1_800_000_000_i64
        })]
    );

    let result = DevToolsGetCookiesResult {
        cookies: vec![json!({
            "name": "default",
            "value": "abc",
            "domain": "example.test",
            "path": "/",
            "secure": false,
            "httpOnly": false,
            "expires": -1.0
        })],
    };
    assert_eq!(
        classic_cookies_from_devtools(result),
        vec![json!({
            "name": "default",
            "value": "abc",
            "domain": "example.test",
            "path": "/",
            "secure": false,
            "httpOnly": false,
            "sameSite": "None"
        })]
    );
}

#[test]
fn rejects_invalid_classic_cookie_params() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    let error = add_cookie_command(
        &context,
        &json!({
            "cookie": {
                "name": false,
                "value": "abc"
            }
        }),
        "https://example.test/",
    )
    .expect_err("non-string cookie name should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "name must be a string");

    let error = add_cookie_command(
        &context,
        &json!({
            "cookie": {
                "name": "sid",
                "value": "abc",
                "sameSite": "Default"
            }
        }),
        "https://example.test/",
    )
    .expect_err("invalid sameSite should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "sameSite must be None, Lax, or Strict");
}

#[test]
fn rejects_classic_command_invalid_params() {
    let context = ClassicDevToolsCommandContext::with_target_id("classic-session-1", "TID-1");

    // Mirrors Chromium's vendored WPT Classic navigate_to/navigate.py body
    // validation and the executable ChromeDriver invalid-URL matrix.
    for (name, params, expected_message) in [
        ("null body", json!(null), "url must be a string"),
        ("missing url", json!({}), "url must be a string"),
        ("null url", json!({"url": null}), "url must be a string"),
        ("boolean url", json!({"url": false}), "url must be a string"),
        ("number url", json!({"url": 42}), "url must be a string"),
        ("object url", json!({"url": {}}), "url must be a string"),
        ("array url", json!({"url": []}), "url must be a string"),
        (
            "relative url",
            json!({"url": "relative/path"}),
            "url must be a valid absolute URL",
        ),
        (
            "invalid http host",
            json!({"url": "http://:invalid"}),
            "url must be a valid absolute URL",
        ),
        (
            "invalid https host",
            json!({"url": "https://#invalid"}),
            "url must be a valid absolute URL",
        ),
    ] {
        let error = navigate_command(&context, &params, DevToolsNavigationWait::Load)
            .expect_err("invalid navigation params should fail");
        assert_eq!(error.code, ClassicErrorCode::InvalidArgument, "{name}");
        assert_eq!(error.message, expected_message, "{name}");
    }

    let error = execute_sync_command(&context, &json!({"script": "return 1;", "args": false}))
        .expect_err("non-array args should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "args must be an array");

    let error = execute_async_command(&context, &json!({"script": "return 1;", "args": false}))
        .expect_err("non-array args should fail");
    assert_eq!(error.code, ClassicErrorCode::InvalidArgument);
    assert_eq!(error.message, "args must be an array");
}
