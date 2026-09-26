use super::*;

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_locale_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_locale_override/invalid.py.
    for params in [
        json!({}),
        json!({"locale": "fr-FR"}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "locale": false}),
        json!({"contexts": ["TARGET-1"], "locale": 42}),
        json!({"contexts": ["TARGET-1"], "locale": {}}),
        json!({"contexts": ["TARGET-1"], "locale": []}),
        json!({"contexts": [], "locale": "fr-FR"}),
        json!({"contexts": [false], "locale": "fr-FR"}),
        json!({"contexts": [42], "locale": "fr-FR"}),
        json!({"contexts": [{}], "locale": "fr-FR"}),
        json!({"contexts": [[]], "locale": "fr-FR"}),
        json!({"userContexts": [], "locale": "fr-FR"}),
        json!({"userContexts": [false], "locale": "fr-FR"}),
        json!({"userContexts": [42], "locale": "fr-FR"}),
        json!({"userContexts": [{}], "locale": "fr-FR"}),
        json!({"userContexts": [[]], "locale": "fr-FR"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "locale": "fr-FR"
        }),
        json!({"contexts": ["TARGET-1"], "locale": ""}),
        json!({"contexts": ["TARGET-1"], "locale": "en_US"}),
        json!({"contexts": ["TARGET-1"], "locale": "Latn"}),
        json!({"contexts": ["TARGET-1"], "locale": "en--US"}),
        json!({"contexts": ["TARGET-1"], "locale": "en-US-!"}),
        json!({"contexts": ["TARGET-1"], "locale": "x-private"}),
    ] {
        assert_bidi_adapter_invalid("emulation.setLocaleOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_timezone_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_timezone_override/invalid.py.
    for params in [
        json!({}),
        json!({"timezone": "Asia/Tokyo"}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "timezone": false}),
        json!({"contexts": ["TARGET-1"], "timezone": 42}),
        json!({"contexts": ["TARGET-1"], "timezone": {}}),
        json!({"contexts": ["TARGET-1"], "timezone": []}),
        json!({"contexts": [], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [false], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [42], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [{}], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [[]], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [false], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [42], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [{}], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [[]], "timezone": "Asia/Tokyo"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "timezone": "Asia/Tokyo"
        }),
        json!({"contexts": ["TARGET-1"], "timezone": ""}),
        json!({"contexts": ["TARGET-1"], "timezone": "+1:00"}),
        json!({"contexts": ["TARGET-1"], "timezone": "GMT+05:00"}),
        json!({"contexts": ["TARGET-1"], "timezone": "UTC+05:00"}),
    ] {
        assert_bidi_adapter_invalid("emulation.setTimezoneOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_network_conditions_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/invalid.py.
    for params in [
        json!({}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": [], "networkConditions": null}),
        json!({"contexts": [false], "networkConditions": null}),
        json!({"contexts": [42], "networkConditions": null}),
        json!({"contexts": [{}], "networkConditions": null}),
        json!({"contexts": [[]], "networkConditions": null}),
        json!({"userContexts": [], "networkConditions": null}),
        json!({"userContexts": [false], "networkConditions": null}),
        json!({"userContexts": [42], "networkConditions": null}),
        json!({"userContexts": [{}], "networkConditions": null}),
        json!({"userContexts": [[]], "networkConditions": null}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "networkConditions": null
        }),
        json!({"contexts": ["TARGET-1"], "networkConditions": false}),
        json!({"contexts": ["TARGET-1"], "networkConditions": 42}),
        json!({"contexts": ["TARGET-1"], "networkConditions": "offline"}),
        json!({"contexts": ["TARGET-1"], "networkConditions": []}),
        json!({"contexts": ["TARGET-1"], "networkConditions": {}}),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": "SOME_INVALID_TYPE"
            }
        }),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": false
            }
        }),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": "offline",
                "extra": true
            }
        }),
    ] {
        assert_bidi_adapter_invalid("emulation.setNetworkConditions", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_permissions_set_permission_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/external/permissions/set_permission/invalid.py.
    for params in [
        json!({"descriptor": false, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": "SOME_STRING", "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": 42, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": [], "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": 23}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": null, "state": "granted", "origin": "https://example.com"}),
        json!({"state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "unknown"}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": false, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": 42, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": {}, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": [], "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": null, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "UNKNOWN", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "Granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": false}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": 42}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": {}}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": []}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": null}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": false}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": 42}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": {}}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": []}),
    ] {
        assert_bidi_adapter_invalid("permissions.setPermission", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_geolocation_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_geolocation_override/invalid.py.
    for params in [
        json!({"contexts": false, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": 42, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": "foo", "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": {}, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [null], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [false], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [42], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [{}], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [[]], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": false}),
        json!({"contexts": ["TARGET-1"], "coordinates": 42}),
        json!({"contexts": ["TARGET-1"], "coordinates": "foo"}),
        json!({"contexts": ["TARGET-1"], "coordinates": []}),
        json!({"contexts": ["TARGET-1"], "coordinates": {}}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": null, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": false, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": "foo", "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": [], "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": {}, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": -90.1, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 90.1, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": null}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": "foo"}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": []}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": {}}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": -180.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 180.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": "foo"}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": []}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": {}}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": -1}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitude": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitudeAccuracy": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitude": 10, "altitudeAccuracy": -1}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "heading": -0.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "heading": 360}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "speed": -1.5}}),
        json!({"userContexts": true, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"userContexts": [], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"userContexts": [null], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "coordinates": {"latitude": 10, "longitude": 10}
        }),
        json!({
            "contexts": ["TARGET-1"],
            "coordinates": {"latitude": 10, "longitude": 10},
            "error": {"type": "positionUnavailable"}
        }),
        json!({"contexts": ["TARGET-1"], "error": false}),
        json!({"contexts": ["TARGET-1"], "error": 42}),
        json!({"contexts": ["TARGET-1"], "error": "foo"}),
        json!({"contexts": ["TARGET-1"], "error": []}),
        json!({"contexts": ["TARGET-1"], "error": {}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": null}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": false}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": 42}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": {}}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": []}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": "unknownError"}}),
    ] {
        assert_bidi_adapter_invalid("emulation.setGeolocationOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_browsing_context_params() {
    // Mirrors adapter-level invalid.py cases from Chromium's vendored WPT
    // WebDriver BiDi browsing_context/create, close, activate, get_tree,
    // navigate, reload, and traverse_history suites.
    for (method, params) in [
        ("browsingContext.create", json!({"type": null})),
        ("browsingContext.create", json!({"type": false})),
        ("browsingContext.create", json!({"type": 42})),
        ("browsingContext.create", json!({"type": {}})),
        ("browsingContext.create", json!({"type": []})),
        ("browsingContext.create", json!({"type": ""})),
        ("browsingContext.create", json!({"type": "foo"})),
        ("browsingContext.create", json!({"type": "popup"})),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": false
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": []
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": null
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": ""
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": []
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": false
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": []
            }),
        ),
        ("browsingContext.close", json!({"context": null})),
        ("browsingContext.close", json!({"context": false})),
        ("browsingContext.close", json!({"context": 42})),
        ("browsingContext.close", json!({"context": {}})),
        ("browsingContext.close", json!({"context": []})),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": 42
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": ""
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": {}
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": []
            }),
        ),
        ("browsingContext.activate", json!({"context": null})),
        ("browsingContext.activate", json!({"context": false})),
        ("browsingContext.activate", json!({"context": 42})),
        ("browsingContext.activate", json!({"context": {}})),
        ("browsingContext.activate", json!({"context": []})),
        ("browsingContext.getTree", json!({"root": false})),
        ("browsingContext.getTree", json!({"root": 42})),
        ("browsingContext.getTree", json!({"root": {}})),
        ("browsingContext.getTree", json!({"root": []})),
        ("browsingContext.getTree", json!({"maxDepth": false})),
        ("browsingContext.getTree", json!({"maxDepth": "foo"})),
        ("browsingContext.getTree", json!({"maxDepth": {}})),
        ("browsingContext.getTree", json!({"maxDepth": []})),
        ("browsingContext.getTree", json!({"maxDepth": -1})),
        ("browsingContext.getTree", json!({"maxDepth": 1.1})),
        (
            "browsingContext.getTree",
            json!({"maxDepth": 9_007_199_254_740_992_u64}),
        ),
        (
            "browsingContext.navigate",
            json!({
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": null,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": false,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": 42,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": {},
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": [],
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": null
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": false
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": 42
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": {}
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": []
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "http://:invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "http://#invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://:invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://#invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": false
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": 42
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": {}
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": []
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": ""
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": "networkIdle"
            }),
        ),
        (
            "browsingContext.reload",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.reload",
            json!({
                "context": "TARGET-1",
                "ignoreCache": "true"
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": null,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": false,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": 42,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": {},
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": [],
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": null
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": false
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": "foo"
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": {}
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": []
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": 1.5
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": -9_007_199_254_740_992_i64
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": 9_007_199_254_740_992_u64
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": "image/png"
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": {
                    "type": "image/gif"
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": {
                    "type": "image/png",
                    "quality": 2.0
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "origin": "page"
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": false
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "box",
                    "x": "0",
                    "y": 0,
                    "width": 10,
                    "height": 10
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "element",
                    "element": false
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "element",
                    "element": {
                        "sharedId": false
                    }
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "background": "true"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "margin": false
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "margin": {
                    "top": -0.1
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "orientation": "sideways"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "page": []
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "page": {
                    "width": 0.03
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": "1-2"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": ["3-2"]
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": [4.2]
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "scale": 0.09
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "shrinkToFit": "false"
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": false,
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "userContexts": ["default"],
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": 100
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": 100,
                    "height": 42.1
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": -1,
                    "height": 100
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "devicePixelRatio": 0
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "userContexts": [],
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }

    let valid_shape_user_context = super::super::parse_bidi_command(json!({
        "id": 100,
        "method": "browsingContext.setViewport",
        "params": {
            "userContexts": ["somestring"],
            "viewport": {
                "width": 100,
                "height": 200
            }
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    assert!(
        super::super::devtools_command_from_bidi_command(&valid_shape_user_context, &context)
            .is_ok(),
        "user context existence is checked by the execution layer that owns browser contexts"
    );
}

#[test]
fn rejects_chromium_wpt_invalid_script_params() {
    // Covers the Chromium vendored WPT invalid.py interface checks that do
    // not need a live renderer: script/evaluate, call_function, disown,
    // add_preload_script, and remove_preload_script.
    for (method, params) in [
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": false,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": "foo",
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": 42,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {},
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": [],
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": null,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": null
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": false
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": false,
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": false
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": 42
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": {}
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": []
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1",
                    "sandbox": 42
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": null
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": 42
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": {}
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "resultOwnership": "_UNKNOWN_"
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": false
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "maxDomDepth": -1
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "maxObjectDepth": -1
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "includeShadowTree": "foo"
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "userActivation": "foo"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": null,
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": "SOME_STRING"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": 42
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": []
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": {}
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": "SOME_STRING"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": 42
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": {}
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": ["SOME_STRING"]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [42]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [[]]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [false]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [{}]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [
                    {
                        "type": "foo"
                    }
                ]
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": null,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": false,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": "foo",
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": 42,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": {},
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": [false],
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": null
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [{}]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "string",
                        "value": "not-a-channel"
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": 42
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": "foo",
                            "ownership": "_UNKNOWN_"
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": "foo",
                            "serializationOptions": {
                                "includeShadowTree": "_UNKNOWN_"
                            }
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": []
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": [false]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": {}
            }),
        ),
        (
            "script.removePreloadScript",
            json!({
                "script": null
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_get_realms_params() {
    // Mirrors webdriver/tests/bidi/script/get_realms/invalid.py cases
    // that are decidable before a live target lookup.
    for params in [
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"type": false}),
        json!({"type": 42}),
        json!({"type": {}}),
        json!({"type": []}),
        json!({"type": "foo"}),
    ] {
        assert_bidi_adapter_invalid("script.getRealms", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_reference_and_channel_params() {
    // Mirrors additional adapter-level checks from Chromium's
    // script/call_function/invalid.py around remote references and channel
    // serialization options.
    for params in [
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"handle": null}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"handle": false}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"sharedId": []}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "array",
                "value": [{"handle": false}]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "object",
                "value": [[{"type": "string", "value": "not-a-property"}, {"handle": "H"}]]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "map",
                "value": [["key-only"]]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"type": "channel", "value": null}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"type": "channel", "value": {"channel": false}}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "ownership": "_UNKNOWN_"
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": false
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": {
                        "maxObjectDepth": -1
                    }
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": {
                        "includeShadowTree": "_UNKNOWN_"
                    }
                }
            }]
        }),
    ] {
        assert_bidi_adapter_invalid("script.callFunction", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_channel_schema_matrix() {
    // Mirrors the ChannelValue/ChannelProperties invalid.py matrices for
    // script.callFunction and script.addPreloadScript.
    fn assert_invalid_channel_value(channel_value: Value) {
        assert_bidi_adapter_invalid(
            "script.callFunction",
            call_function_with_channel_value(channel_value.clone()),
        );
        assert_bidi_adapter_invalid(
            "script.addPreloadScript",
            add_preload_script_with_channel_value(channel_value),
        );
    }

    for value in [
        Value::Null,
        json!(false),
        json!("_UNKNOWN_"),
        json!(42),
        json!([]),
    ] {
        assert_invalid_channel_value(value);
    }

    for channel in [Value::Null, json!(false), json!(42), json!([]), json!({})] {
        assert_invalid_channel_value(json!({"channel": channel}));
    }

    for ownership in [json!(false), json!(42), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "ownership": ownership
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "ownership": "_UNKNOWN_"
    }));

    for serialization_options in [json!(false), json!("_UNKNOWN_"), json!(42), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": serialization_options
        }));
    }

    for max_dom_depth in [json!(false), json!("_UNKNOWN_"), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"maxDomDepth": max_dom_depth}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"maxDomDepth": -1}
    }));

    for max_object_depth in [json!(false), json!("_UNKNOWN_"), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"maxObjectDepth": max_object_depth}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"maxObjectDepth": -1}
    }));

    for include_shadow_tree in [json!(false), json!(42), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"includeShadowTree": include_shadow_tree}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"includeShadowTree": "_UNKNOWN_"}
    }));
}

#[test]
fn rejects_chromium_wpt_invalid_preload_target_params() {
    // Mirrors the target and script id validation cases from Chromium's
    // script/add_preload_script/invalid.py and
    // script/remove_preload_script/invalid.py.
    for (method, params) in [
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": false
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": 42
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": "_UNKNOWN_"
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": {}
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": false
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": 42
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": "_UNKNOWN_"
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": []
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "sandbox": []
            }),
        ),
        ("script.removePreloadScript", json!({"script": false})),
        ("script.removePreloadScript", json!({"script": 42})),
        ("script.removePreloadScript", json!({"script": {}})),
        ("script.removePreloadScript", json!({"script": []})),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_storage_cookie_params() {
    // Mirrors the adapter-level invalid.py cases from Chromium's vendored
    // WPT WebDriver BiDi storage get/set/delete cookie suites.
    for (method, params) in [
        ("storage.getCookies", json!({"filter": false})),
        ("storage.getCookies", json!({"filter": 42})),
        ("storage.getCookies", json!({"filter": "foo"})),
        ("storage.getCookies", json!({"filter": []})),
        ("storage.getCookies", json!({"filter": {"domain": false}})),
        ("storage.getCookies", json!({"filter": {"domain": 42}})),
        ("storage.getCookies", json!({"filter": {"domain": {}}})),
        ("storage.getCookies", json!({"filter": {"domain": []}})),
        ("storage.getCookies", json!({"filter": {"expiry": false}})),
        ("storage.getCookies", json!({"filter": {"expiry": "foo"}})),
        ("storage.getCookies", json!({"filter": {"expiry": -1}})),
        ("storage.getCookies", json!({"filter": {"expiry": 0.5}})),
        ("storage.getCookies", json!({"filter": {"expiry": {}}})),
        ("storage.getCookies", json!({"filter": {"expiry": []}})),
        (
            "storage.getCookies",
            json!({"filter": {"httpOnly": "true"}}),
        ),
        ("storage.getCookies", json!({"filter": {"httpOnly": {}}})),
        ("storage.getCookies", json!({"filter": {"httpOnly": []}})),
        ("storage.getCookies", json!({"filter": {"httpOnly": 42}})),
        ("storage.getCookies", json!({"filter": {"name": false}})),
        ("storage.getCookies", json!({"filter": {"name": 42}})),
        ("storage.getCookies", json!({"filter": {"name": {}}})),
        ("storage.getCookies", json!({"filter": {"name": []}})),
        ("storage.getCookies", json!({"filter": {"path": []}})),
        ("storage.getCookies", json!({"filter": {"path": false}})),
        ("storage.getCookies", json!({"filter": {"path": 42}})),
        ("storage.getCookies", json!({"filter": {"path": {}}})),
        ("storage.getCookies", json!({"filter": {"sameSite": ""}})),
        (
            "storage.getCookies",
            json!({"filter": {"sameSite": "INVALID_SAME_SITE_STATE"}}),
        ),
        ("storage.getCookies", json!({"filter": {"secure": 42}})),
        ("storage.getCookies", json!({"filter": {"secure": "foo"}})),
        ("storage.getCookies", json!({"filter": {"secure": {}}})),
        ("storage.getCookies", json!({"filter": {"secure": []}})),
        ("storage.getCookies", json!({"filter": {"size": "6"}})),
        ("storage.getCookies", json!({"filter": {"size": false}})),
        ("storage.getCookies", json!({"filter": {"size": -1}})),
        ("storage.getCookies", json!({"filter": {"size": 0.5}})),
        ("storage.getCookies", json!({"filter": {"value": false}})),
        ("storage.getCookies", json!({"filter": {"value": 42}})),
        ("storage.getCookies", json!({"filter": {"value": "foo"}})),
        ("storage.getCookies", json!({"filter": {"value": []}})),
        (
            "storage.getCookies",
            json!({"filter": {"value": {"type": "foo", "value": "bar"}}}),
        ),
        (
            "storage.getCookies",
            json!({"filter": {"value": {"type": "base64", "value": "%%%"}}}),
        ),
        ("storage.getCookies", json!({"partition": false})),
        ("storage.getCookies", json!({"partition": 42})),
        ("storage.getCookies", json!({"partition": "foo"})),
        ("storage.getCookies", json!({"partition": []})),
        ("storage.getCookies", json!({"partition": {"type": false}})),
        ("storage.getCookies", json!({"partition": {"type": null}})),
        ("storage.getCookies", json!({"partition": {"type": 42}})),
        ("storage.getCookies", json!({"partition": {"type": {}}})),
        ("storage.getCookies", json!({"partition": {"type": []}})),
        (
            "storage.getCookies",
            json!({"partition": {"type": "context", "context": false}}),
        ),
        (
            "storage.getCookies",
            json!({"partition": {"type": "storageKey", "sourceOrigin": false}}),
        ),
        (
            "storage.getCookies",
            json!({"partition": {"type": "storageKey", "userContext": false}}),
        ),
        ("storage.deleteCookies", json!({"filter": "foo"})),
        (
            "storage.deleteCookies",
            json!({"filter": {"sameSite": "invalid"}}),
        ),
        ("storage.deleteCookies", json!({"partition": []})),
        ("storage.setCookie", json!({"cookie": null})),
        ("storage.setCookie", json!({"cookie": false})),
        ("storage.setCookie", json!({"cookie": 42})),
        ("storage.setCookie", json!({"cookie": "foo"})),
        ("storage.setCookie", json!({"cookie": []})),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": false,
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": "SOME_STRING_VALUE",
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": false,
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "base64", "value": "%%%"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": false},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "foo", "value": "abc"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": null
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "path": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "httpOnly": 42
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "secure": "true"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "sameSite": "INVALID_SAME_SITE_STATE"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "expiry": "1"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "expiry": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test"
                },
                "partition": "foo"
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_invalid_bidi_command_adapter_params() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let invalid_wait = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.navigate",
        "params": {
            "context": "TARGET-1",
            "url": "https://example.test/",
            "wait": "networkIdle"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_wait, &context)
            .expect_err("invalid wait should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_wait_type = super::super::parse_bidi_command(json!({
        "id": 2,
        "method": "browsingContext.navigate",
        "params": {
            "context": "TARGET-1",
            "url": "https://example.test/",
            "wait": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_wait_type, &context)
            .expect_err("non-string wait should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_get_tree_root_type = super::super::parse_bidi_command(json!({
        "id": 3,
        "method": "browsingContext.getTree",
        "params": {
            "root": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_get_tree_root_type, &context)
            .expect_err("non-string getTree root should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_reload_ignore_cache_type = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.reload",
        "params": {
            "context": "TARGET-1",
            "ignoreCache": "true"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(
            &invalid_reload_ignore_cache_type,
            &context
        )
        .expect_err("non-boolean ignoreCache should fail")
        .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_await_promise_type = super::super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": "true"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_await_promise_type, &context)
            .expect_err("non-boolean awaitPromise should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_result_ownership_type = super::super::parse_bidi_command(json!({
        "id": 6,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1"
            },
            "resultOwnership": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_result_ownership_type, &context)
            .expect_err("non-string resultOwnership should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_script_target_sandbox_type = super::super::parse_bidi_command(json!({
        "id": 7,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1",
                "sandbox": false
            }
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(
            &invalid_script_target_sandbox_type,
            &context
        )
        .expect_err("non-string script target sandbox should fail")
        .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_call_function_arguments_type = super::super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "(arg) => arg",
            "target": {
                "context": "TARGET-1"
            },
            "arguments": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(
            &invalid_call_function_arguments_type,
            &context
        )
        .expect_err("non-array callFunction arguments should fail")
        .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_sandbox_type = super::super::parse_bidi_command(json!({
        "id": 9,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "sandbox": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_preload_sandbox_type, &context)
            .expect_err("non-string preload sandbox should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let context_and_realm_target = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1",
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let shared =
        super::super::devtools_command_from_bidi_command(&context_and_realm_target, &context)
            .expect("context target should ignore realm");
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

    let invalid_realm_type = super::super::parse_bidi_command(json!({
        "id": 11,
        "method": "script.getRealms",
        "params": {
            "type": "document"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_realm_type, &context)
            .expect_err("invalid realm type should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let conflicting_preload_targets = super::super::parse_bidi_command(json!({
        "id": 12,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "contexts": ["TARGET-1"],
            "userContexts": ["BID-1"]
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&conflicting_preload_targets, &context)
            .expect_err("conflicting preload targets should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let empty_preload_contexts = super::super::parse_bidi_command(json!({
        "id": 13,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "contexts": []
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&empty_preload_contexts, &context)
            .expect_err("empty preload contexts should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_arguments_type = super::super::parse_bidi_command(json!({
        "id": 14,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "arguments": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_preload_arguments_type, &context)
            .expect_err("non-array preload arguments should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_argument_entry = super::super::parse_bidi_command(json!({
        "id": 15,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "arguments": [false]
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::super::devtools_command_from_bidi_command(&invalid_preload_argument_entry, &context)
            .expect_err("non-object preload argument entries should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );
}

#[test]
fn network_continue_request_preserves_header_bytes() {
    let command = super::super::parse_bidi_command(json!({
        "id": 100,
        "method": "network.continueRequest",
        "params": {"request": "REQ-raw", "headers": [
            {"name": "X-Raw", "value": {"type": "base64", "value": "6f8="}},
            {"name": "x-raw", "value": {"type": "string", "value": "é"}}
        ]}
    }))
    .unwrap();
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueInterceptedRequest(command) =
        super::super::devtools_command_from_bidi_command(&command, &context).unwrap()
    else {
        panic!("expected request continuation");
    };
    assert_eq!(
        command.headers.unwrap(),
        moli_header_field::HeaderFields::from_bytes(vec![
            ("X-Raw".to_owned(), vec![0xe9, 0xff]),
            ("x-raw".to_owned(), vec![0xc3, 0xa9]),
        ])
    );
}
