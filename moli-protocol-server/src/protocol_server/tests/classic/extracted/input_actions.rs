use super::*;

#[tokio::test]
async fn webdriver_classic_clear_disabled_form_cases_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/element_clear/disabled.py
    // and element_clear/clear.py non-editable input cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r#"<!doctype html>
        <input id="enabled" value="enabled">
        <fieldset disabled>
          <input id="fieldsetChild" value="blocked">
          <legend><input id="firstLegendInput" value="allowed"></legend>
          <legend><input id="secondLegendInput" value="blocked"></legend>
        </fieldset>
        <select id="disabledSelect" disabled><option id="selectOption">select</option></select>
        <select>
          <option id="disabledOption" disabled>option</option>
          <optgroup id="disabledOptgroup" disabled><option id="optgroupOption">group</option></optgroup>
        </select>
        <input id="checkbox" type="checkbox">
        <input id="hiddenInput" type="hidden" value="hidden">
    "#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn clear_element(
        app: Router,
        session_id: &str,
        selector: &str,
    ) -> (StatusCode, serde_json::Value) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_status_and_json(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/clear"),
        )
        .await
    }

    async fn element_value(app: Router, session_id: &str, selector: &str) -> serde_json::Value {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/property/value"),
        )
        .await
    }

    let (enabled_status, enabled_response) =
        clear_element(app.clone(), session_id, "#enabled").await;
    assert_eq!(enabled_status, StatusCode::OK, "{enabled_response:?}");
    assert_eq!(enabled_response, json!({ "value": null }));
    assert_eq!(
        element_value(app.clone(), session_id, "#enabled").await,
        json!({ "value": "" })
    );

    let (legend_status, legend_response) =
        clear_element(app.clone(), session_id, "#firstLegendInput").await;
    assert_eq!(legend_status, StatusCode::OK, "{legend_response:?}");
    assert_eq!(legend_response, json!({ "value": null }));
    assert_eq!(
        element_value(app.clone(), session_id, "#firstLegendInput").await,
        json!({ "value": "" })
    );

    for selector in [
        "#fieldsetChild",
        "#secondLegendInput",
        "#disabledSelect",
        "#selectOption",
        "#disabledOption",
        "#disabledOptgroup",
        "#optgroupOption",
        "#checkbox",
        "#hiddenInput",
    ] {
        let (status, response) = clear_element(app.clone(), session_id, selector).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{selector}: {response:?}");
        assert_eq!(
            response["value"]["error"],
            json!("invalid element state"),
            "{selector}: {response:?}"
        );
    }
}
#[tokio::test]
async fn webdriver_classic_click_pointer_focus_and_interactability_controls() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"].as_str().expect("session id");
    for (name, target, setup, expected_focus, expected_clicks, expected_error) in [
        ("ordinary", "<input id='target'>", "", "target", 1, None),
        (
            "clipped fixed button",
            "<button id='target' style='position:fixed;left:-150px;top:20px;width:200px;height:50px'>go</button>",
            "",
            "target",
            1,
            None,
        ),
        (
            "entirely outside viewport",
            "<button id='target' style='position:fixed;left:-250px;top:20px;width:200px;height:50px'>go</button>",
            "",
            "origin",
            0,
            Some("element not interactable"),
        ),
        (
            "cancel mousedown",
            "<button id='target'>go</button>",
            "target.onmousedown=e=>e.preventDefault();",
            "origin",
            1,
            None,
        ),
        (
            "cancel pointerdown",
            "<button id='target'>go</button>",
            "target.onpointerdown=e=>e.preventDefault();",
            "origin",
            1,
            None,
        ),
        (
            "label",
            "<label id='target' for='check'>toggle</label><input id='check' type='checkbox'>",
            "",
            "check",
            1,
            None,
        ),
        (
            "disabled",
            "<button id='target' disabled>go</button>",
            "",
            "origin",
            0,
            None,
        ),
        (
            "scroll",
            "<button id='target' style='position:absolute;top:2500px'>go</button>",
            "",
            "target",
            1,
            None,
        ),
        (
            "file",
            "<input id='target' type='file'>",
            "",
            "origin",
            0,
            Some("invalid argument"),
        ),
        (
            "hidden",
            "<button id='target' hidden>go</button>",
            "",
            "origin",
            0,
            Some("element not interactable"),
        ),
        (
            "obscured",
            "<button id='target'>go</button><div style='position:fixed;inset:0;z-index:9'></div>",
            "",
            "origin",
            0,
            Some("element click intercepted"),
        ),
    ] {
        let html = format!(
            "<input id='origin'>{target}<script>window.clicks=0;target.onclick=()=>clicks++;document.getElementById('origin').focus();{setup}</script>"
        );
        let navigated = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({"url":format!("data:text/html,{html}")}),
        )
        .await;
        classic_capture_layout(app.clone(), session_id).await;
        assert_eq!(navigated, json!({"value":null}), "{name}");
        let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
        if name == "scroll" {
            let (status, blocked) = classic_request_status_and_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{element_id}/click"),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
            assert_eq!(blocked["value"]["error"], "element not interactable");
            // Click preparation scrolled live state; explicitly publish the new position.
            classic_capture_layout(app.clone(), session_id).await;
        }
        let (status, clicked) = classic_request_status_and_json(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/click"),
        )
        .await;
        if let Some(error) = expected_error {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {clicked}");
            assert_eq!(clicked["value"]["error"], error, "{name}");
        } else {
            assert_eq!(clicked, json!({"value":null}), "{name}");
        }
        let observed = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script":"return [document.activeElement.id,window.clicks];", "args":[]
            }),
        )
        .await;
        assert_eq!(observed["value"][1], expected_clicks, "{name}");
        // Disabled controls must not be rejected or activated. Their focus
        // result is not the behavior this adapter correction changes.
        if name != "disabled" {
            assert_eq!(observed["value"][0], expected_focus, "{name}");
        }
    }
    // execute/sync remains a JavaScript operation, even with user_gesture set.
    classic_request_json_with_body(app.clone(), Method::POST,
        &format!("/session/{session_id}/url"), json!({
            "url":"data:text/html,<input id='origin'><input id='target'><script>document.getElementById('origin').focus();</script>"
        })).await;
    classic_capture_layout(app.clone(), session_id).await;
    let synthetic = classic_request_json_with_body(app.clone(), Method::POST,
        &format!("/session/{session_id}/execute/sync"), json!({
            "script":"let events=[];for(const type of ['pointerdown','mousedown','focus','mouseup','click'])target.addEventListener(type,e=>events.push([e.type,e.isTrusted]));target.click();return [document.activeElement.id,events];", "args":[]
        })).await;
    assert_eq!(synthetic, json!({"value":["origin",[["click",false]]]}));
    classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}
#[tokio::test]
async fn webdriver_classic_option_click_updates_select_state_ported_from_selenium_select() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let html = concat!(
        "<select id='single'>",
        "<option id='cheddar' value='cheddar'>Cheddar</option>",
        "<option id='brie' value='brie'>Brie</option>",
        "</select>",
        "<select id='multi' multiple>",
        "<option id='eggs' value='eggs' selected>Eggs</option>",
        "<option id='ham' value='ham'>Ham</option>",
        "</select>",
        "<select id='blocked'>",
        "<option id='safe' value='safe' selected>Safe</option>",
        "<optgroup disabled><option id='blockedOptgroup' value='blocked'>Blocked</option></optgroup>",
        "</select>",
        "<select id='disabledSelectForClick' disabled>",
        "<option id='locked' value='locked' selected>Locked</option>",
        "<option id='disabledSelectTarget' value='target'>Target</option>",
        "</select>",
        "<script>",
        "window.__selectLog=[];",
        "for (const select of document.querySelectorAll('select')) {",
        "select.addEventListener('input', () => window.__selectLog.push(select.id + ':input:' + Array.from(select.selectedOptions).map(o => o.value).join('/')));",
        "select.addEventListener('change', () => window.__selectLog.push(select.id + ':change:' + Array.from(select.selectedOptions).map(o => o.value).join('/')));",
        "}",
        "</script>",
    );

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": format!("data:text/html,{html}") }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let cheddar_id = classic_find_css_element_id(app.clone(), session_id, "#cheddar").await;
    let brie_id = classic_find_css_element_id(app.clone(), session_id, "#brie").await;
    let eggs_id = classic_find_css_element_id(app.clone(), session_id, "#eggs").await;
    let ham_id = classic_find_css_element_id(app.clone(), session_id, "#ham").await;
    let safe_id = classic_find_css_element_id(app.clone(), session_id, "#safe").await;
    let blocked_optgroup_id =
        classic_find_css_element_id(app.clone(), session_id, "#blockedOptgroup").await;
    let locked_id = classic_find_css_element_id(app.clone(), session_id, "#locked").await;
    let disabled_select_target_id =
        classic_find_css_element_id(app.clone(), session_id, "#disabledSelectTarget").await;

    let clicked_brie = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{brie_id}/click"),
    )
    .await;
    assert_eq!(clicked_brie, json!({ "value": null }));
    let focused_select = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({"script":"return document.activeElement.id;", "args":[]}),
    )
    .await;
    assert_eq!(focused_select, json!({"value":"single"}));
    let clicked_ham = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{ham_id}/click"),
    )
    .await;
    assert_eq!(clicked_ham, json!({ "value": null }));
    let clicked_eggs = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{eggs_id}/click"),
    )
    .await;
    assert_eq!(clicked_eggs, json!({ "value": null }));
    let clicked_blocked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{blocked_optgroup_id}/click"),
    )
    .await;
    assert_eq!(clicked_blocked, json!({ "value": null }));
    let clicked_disabled_select = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{disabled_select_target_id}/click"),
    )
    .await;
    assert_eq!(clicked_disabled_select, json!({ "value": null }));

    for (element_id, expected) in [
        (&cheddar_id, false),
        (&brie_id, true),
        (&eggs_id, false),
        (&ham_id, true),
        (&safe_id, true),
        (&blocked_optgroup_id, false),
        (&locked_id, true),
        (&disabled_select_target_id, false),
    ] {
        let selected = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/selected"),
        )
        .await;
        assert_eq!(selected, json!({ "value": expected }), "{element_id}");
    }

    let script_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [document.getElementById('single').value, Array.from(document.getElementById('multi').selectedOptions).map(o => o.value).join('/'), document.getElementById('blocked').value, document.getElementById('disabledSelectForClick').value, window.__selectLog.join(',')].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        script_state,
        json!({
            "value": "brie|ham|safe|locked|single:input:brie,single:change:brie,multi:input:eggs/ham,multi:change:eggs/ham,multi:input:ham,multi:change:ham"
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_option_click_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/element_click/select.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let html = r#"<!doctype html>
        <select id="preselectedSingle">
          <option id="psFirst">first</option>
          <option id="psSecond" selected>second</option>
        </select>
        <select id="singleDeselects">
          <option id="sdFirst">first</option>
          <option id="sdSecond">second</option>
          <option id="sdThird">third</option>
        </select>
        <select id="singleRepeated">
          <option id="srFirst">first</option>
          <option id="srSecond">second</option>
        </select>
        <select id="preselectedMultiple" multiple>
          <option id="pmFirst">first</option>
          <option id="pmSecond" selected>second</option>
        </select>
        <select id="multiKeepsOthers" multiple>
          <option id="mkFirst">first</option>
          <option id="mkSecond">second</option>
          <option id="mkThird">third</option>
        </select>
        <select id="multiToggle" multiple>
          <option id="mtFirst">first</option>
          <option id="mtSecond">second</option>
        </select>
        <select id="outSingle">
          <option id="outSingle1">1</option>
          <option id="outSingle2">2</option>
          <option id="outSingle3">3</option>
          <option id="outSingle4">4</option>
          <option id="outSingle5">5</option>
          <option id="outSingle6">6</option>
          <option id="outSingle7">7</option>
          <option id="outSingle8">8</option>
          <option id="outSingle9">9</option>
          <option id="outSingle10">10</option>
          <option id="outSingle11">11</option>
          <option id="outSingle12">12</option>
          <option id="outSingle13">13</option>
          <option id="outSingle14">14</option>
          <option id="outSingle15">15</option>
          <option id="outSingle16">16</option>
          <option id="outSingle17">17</option>
          <option id="outSingle18">18</option>
          <option id="outSingle19">19</option>
          <option id="outSingle20">20</option>
        </select>
        <select id="outMulti" multiple>
          <option id="outMulti1">1</option>
          <option id="outMulti2">2</option>
          <option id="outMulti3">3</option>
          <option id="outMulti4">4</option>
          <option id="outMulti5">5</option>
          <option id="outMulti6">6</option>
          <option id="outMulti7">7</option>
          <option id="outMulti8">8</option>
          <option id="outMulti9">9</option>
          <option id="outMulti10">10</option>
          <option id="outMulti11">11</option>
          <option id="outMulti12">12</option>
          <option id="outMulti13">13</option>
          <option id="outMulti14">14</option>
          <option id="outMulti15">15</option>
          <option id="outMulti16">16</option>
          <option id="outMulti17">17</option>
          <option id="outMulti18">18</option>
          <option id="outMulti19">19</option>
          <option id="outMulti20">20</option>
        </select>
        <select id="disabledOption">
          <option id="disabledFirst" disabled>foo</option>
          <option id="enabledSecond">bar</option>
        </select>"#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn option_id(app: Router, session_id: &str, selector: &str) -> String {
        classic_find_css_element_id(app, session_id, selector).await
    }

    async fn click_option(app: Router, session_id: &str, element_id: &str) {
        let clicked = classic_request_json(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/click"),
        )
        .await;
        assert_eq!(clicked, json!({ "value": null }));
    }

    async fn assert_selected(
        app: Router,
        session_id: &str,
        element_id: &str,
        label: &str,
        expected: bool,
    ) {
        let selected = classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/selected"),
        )
        .await;
        assert_eq!(
            selected,
            json!({ "value": expected }),
            "{label}: element {element_id}"
        );
    }

    let ps_first = option_id(app.clone(), session_id, "#psFirst").await;
    let ps_second = option_id(app.clone(), session_id, "#psSecond").await;
    assert_selected(app.clone(), session_id, &ps_first, "psFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond initial",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &ps_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond after repeated click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_first,
        "psFirst after repeated click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &ps_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_first,
        "psFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond after psFirst click",
        false,
    )
    .await;

    let sd_first = option_id(app.clone(), session_id, "#sdFirst").await;
    let sd_second = option_id(app.clone(), session_id, "#sdSecond").await;
    let sd_third = option_id(app.clone(), session_id, "#sdThird").await;
    click_option(app.clone(), session_id, &sd_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after click",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &sd_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_second,
        "sdSecond after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after sdSecond click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &sd_third).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_third,
        "sdThird after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_second,
        "sdSecond after sdThird click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &sd_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after second click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_third,
        "sdThird after sdFirst click",
        false,
    )
    .await;

    let sr_second = option_id(app.clone(), session_id, "#srSecond").await;
    click_option(app.clone(), session_id, &sr_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sr_second,
        "srSecond after first click",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &sr_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sr_second,
        "srSecond after repeated click",
        true,
    )
    .await;

    let pm_first = option_id(app.clone(), session_id, "#pmFirst").await;
    let pm_second = option_id(app.clone(), session_id, "#pmSecond").await;
    assert_selected(app.clone(), session_id, &pm_first, "pmFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond initial",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &pm_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond after click",
        false,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_first,
        "pmFirst after pmSecond click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &pm_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_first,
        "pmFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond after pmFirst click",
        false,
    )
    .await;

    let mk_first = option_id(app.clone(), session_id, "#mkFirst").await;
    let mk_second = option_id(app.clone(), session_id, "#mkSecond").await;
    let mk_third = option_id(app.clone(), session_id, "#mkThird").await;
    click_option(app.clone(), session_id, &mk_first).await;
    click_option(app.clone(), session_id, &mk_second).await;
    click_option(app.clone(), session_id, &mk_third).await;
    assert_selected(app.clone(), session_id, &mk_first, "mkFirst final", true).await;
    assert_selected(app.clone(), session_id, &mk_second, "mkSecond final", true).await;
    assert_selected(app.clone(), session_id, &mk_third, "mkThird final", true).await;

    let mt_first = option_id(app.clone(), session_id, "#mtFirst").await;
    let mt_second = option_id(app.clone(), session_id, "#mtSecond").await;
    assert_selected(app.clone(), session_id, &mt_first, "mtFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond initial",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &mt_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_first,
        "mtFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond after mtFirst click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &mt_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_first,
        "mtFirst after repeated click",
        false,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond after repeated click",
        false,
    )
    .await;

    let out_single_15 = option_id(app.clone(), session_id, "#outSingle15").await;
    click_option(app.clone(), session_id, &out_single_15).await;
    assert_selected(
        app.clone(),
        session_id,
        &out_single_15,
        "outSingle15 after click",
        true,
    )
    .await;
    let out_multi_20 = option_id(app.clone(), session_id, "#outMulti20").await;
    click_option(app.clone(), session_id, &out_multi_20).await;
    assert_selected(
        app.clone(),
        session_id,
        &out_multi_20,
        "outMulti20 after click",
        true,
    )
    .await;

    let disabled_first = option_id(app.clone(), session_id, "#disabledFirst").await;
    assert_selected(
        app.clone(),
        session_id,
        &disabled_first,
        "disabledFirst initial",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &disabled_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &disabled_first,
        "disabledFirst after click",
        false,
    )
    .await;

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_enter_inserts_newline_and_submits_through_both_key_routes() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"].as_str().unwrap();
    for key in ["\u{E006}", "\u{E007}"] {
        for use_actions in [false, true] {
            for (target, expected) in [
                ("<textarea id='field'></textarea>", json!(["a\nb", 0])),
                ("<input id='field'>", json!(["ab", 1])),
            ] {
                let html = format!(
                    "<form onsubmit='window.submits++;event.preventDefault()'>{target}<button>submit</button></form><script>window.submits=0;field.focus();</script>"
                );
                let navigated = classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/url"),
                    json!({"url": format!("data:text/html,{html}")}),
                )
                .await;
                classic_capture_layout(app.clone(), session_id).await;
                assert_eq!(navigated, json!({"value": null}));
                let element_id =
                    classic_find_css_element_id(app.clone(), session_id, "#field").await;
                let (route, body) = if use_actions {
                    (
                        format!("/session/{session_id}/actions"),
                        json!({"actions": [{
                            "type": "key", "id": "keyboard", "actions": [
                                {"type":"keyDown", "value":"a"}, {"type":"keyUp", "value":"a"},
                                {"type":"keyDown", "value":key}, {"type":"keyUp", "value":key},
                                {"type":"keyDown", "value":"b"}, {"type":"keyUp", "value":"b"}
                            ]
                        }]}),
                    )
                } else {
                    (
                        format!("/session/{session_id}/element/{element_id}/value"),
                        json!({"text": format!("a{key}b")}),
                    )
                };
                let sent =
                    classic_request_json_with_body(app.clone(), Method::POST, &route, body).await;
                assert_eq!(
                    sent,
                    json!({"value": null}),
                    "{key:?}, actions={use_actions}"
                );
                let observed = classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/execute/sync"),
                    json!({"script": "return [field.value,window.submits];", "args": []}),
                )
                .await;
                assert_eq!(
                    observed["value"], expected,
                    "{target}, {key:?}, actions={use_actions}"
                );
            }
        }
    }
    classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}
#[tokio::test]
async fn webdriver_classic_send_keys_form_control_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // element_send_keys/form_controls.py input, textarea, append,
    // focused-selection insertion, and date cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page = classic_data_url(
        r#"<!doctype html>
        <input id="input-empty">
        <textarea id="textarea-empty"></textarea>
        <input id="input-append" value="a">
        <textarea id="textarea-append">a</textarea>
        <input id="input-insert" value="a">
        <textarea id="textarea-insert">a</textarea>
        <input id="disabled-text" disabled>
        <input id="date" type="date">
        "#,
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn send_keys(app: Router, session_id: &str, selector: &str, text: &str) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let sent = classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/value"),
            json!({ "text": text }),
        )
        .await;
        assert_eq!(sent, json!({ "value": null }), "{selector} send keys");
    }

    async fn send_keys_status(
        app: Router,
        session_id: &str,
        selector: &str,
        text: &str,
    ) -> (StatusCode, serde_json::Value) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_status_and_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/value"),
            json!({ "text": text }),
        )
        .await
    }

    async fn property_value(app: Router, session_id: &str, selector: &str) -> serde_json::Value {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/property/value"),
        )
        .await
    }

    async fn active_element_id(app: Router, session_id: &str) -> serde_json::Value {
        classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.activeElement && document.activeElement.id;",
                "args": []
            }),
        )
        .await
    }

    send_keys(app.clone(), session_id, "#input-empty", "foo").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-empty").await,
        json!({ "value": "foo" })
    );
    assert_eq!(
        active_element_id(app.clone(), session_id).await,
        json!({ "value": "input-empty" })
    );

    send_keys(app.clone(), session_id, "#textarea-empty", "foo").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-empty").await,
        json!({ "value": "foo" })
    );
    assert_eq!(
        active_element_id(app.clone(), session_id).await,
        json!({ "value": "textarea-empty" })
    );

    send_keys(app.clone(), session_id, "#input-append", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-append").await,
        json!({ "value": "ab" })
    );
    send_keys(app.clone(), session_id, "#input-append", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-append").await,
        json!({ "value": "abc" })
    );

    send_keys(app.clone(), session_id, "#textarea-append", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-append").await,
        json!({ "value": "ab" })
    );
    send_keys(app.clone(), session_id, "#textarea-append", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-append").await,
        json!({ "value": "abc" })
    );

    let prepared_input = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const elem = document.getElementById('input-insert'); elem.focus(); elem.setSelectionRange(0, 0); return [elem.selectionStart, elem.selectionEnd].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(prepared_input, json!({ "value": "0|0" }));
    send_keys(app.clone(), session_id, "#input-insert", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-insert").await,
        json!({ "value": "ba" })
    );
    send_keys(app.clone(), session_id, "#input-insert", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-insert").await,
        json!({ "value": "bca" })
    );

    let prepared_textarea = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const elem = document.getElementById('textarea-insert'); elem.focus(); elem.setSelectionRange(0, 0); return [elem.selectionStart, elem.selectionEnd].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(prepared_textarea, json!({ "value": "0|0" }));
    send_keys(app.clone(), session_id, "#textarea-insert", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-insert").await,
        json!({ "value": "ba" })
    );
    send_keys(app.clone(), session_id, "#textarea-insert", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-insert").await,
        json!({ "value": "bca" })
    );

    send_keys(app.clone(), session_id, "#date", "2000-01-01").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#date").await,
        json!({ "value": "2000-01-01" })
    );

    let (disabled_status, disabled_response) =
        send_keys_status(app.clone(), session_id, "#disabled-text", "blocked").await;
    assert_eq!(disabled_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        disabled_response["value"]["error"],
        json!("element not interactable")
    );
}
#[tokio::test]
async fn webdriver_classic_file_input_send_keys_sets_selected_files() {
    let first_file = TempPath::new("classic-file-upload-first");
    let second_file = TempPath::new("classic-file-upload-second");
    let third_file = TempPath::new("classic-file-upload-third");
    fs::write(&first_file.path, b"alpha").expect("write first upload file");
    fs::write(&second_file.path, b"bravo!").expect("write second upload file");
    fs::write(&third_file.path, b"charlie").expect("write third upload file");
    let first_name = first_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("first file should have a filename")
        .to_owned();
    let second_name = second_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("second file should have a filename")
        .to_owned();
    let third_name = third_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("third file should have a filename")
        .to_owned();
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page = classic_data_url(
        "<input id='multi' type='file' multiple style='display:none'>\
         <input id='single' type='file'>\
         <script>\
         window.__events=[];\
         for (const id of ['multi','single']) {\
           const el = document.getElementById(id);\
           el.addEventListener('input', () => window.__events.push(id + ':input:' + el.files.length));\
           el.addEventListener('change', () => window.__events.push(id + ':change:' + el.files.length));\
         }\
         </script>",
    );

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let multi = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#multi"
        }),
    )
    .await;
    let multi_id = multi["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("multi file input id");
    let uploaded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{multi_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(uploaded, json!({ "value": null }));

    let summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('multi'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), sizes: Array.from(input.files).map(file => file.size).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let summary: serde_json::Value = serde_json::from_str(
        summary["value"]
            .as_str()
            .expect("summary should be JSON string"),
    )
    .expect("summary JSON");
    assert_eq!(summary["length"], json!(2));
    assert_eq!(
        summary["names"],
        json!(format!("{first_name}|{second_name}"))
    );
    assert_eq!(summary["sizes"], json!("5|6"));
    assert_eq!(
        summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(summary["events"], json!("multi:input:2,multi:change:2"));

    let appended = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{multi_id}/value"),
        json!({
            "text": third_file.path.to_string_lossy().to_string()
        }),
    )
    .await;
    assert_eq!(appended, json!({ "value": null }));

    let appended_summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('multi'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), sizes: Array.from(input.files).map(file => file.size).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let appended_summary: serde_json::Value = serde_json::from_str(
        appended_summary["value"]
            .as_str()
            .expect("appended summary should be JSON string"),
    )
    .expect("appended summary JSON");
    assert_eq!(appended_summary["length"], json!(3));
    assert_eq!(
        appended_summary["names"],
        json!(format!("{first_name}|{second_name}|{third_name}"))
    );
    assert_eq!(appended_summary["sizes"], json!("5|6|7"));
    assert_eq!(
        appended_summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        appended_summary["events"],
        json!("multi:input:2,multi:change:2,multi:input:3,multi:change:3")
    );

    let single = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#single"
        }),
    )
    .await;
    let single_id = single["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("single file input id");
    let (non_multiple_status, non_multiple) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(non_multiple_status, StatusCode::BAD_REQUEST);
    assert_eq!(non_multiple["value"]["error"], json!("invalid argument"));

    let trailing_newlines = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": format!("{}\n\n", first_file.path.to_string_lossy())
        }),
    )
    .await;
    assert_eq!(trailing_newlines, json!({ "value": null }));

    let single_summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('single'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let single_summary: serde_json::Value = serde_json::from_str(
        single_summary["value"]
            .as_str()
            .expect("single summary should be JSON string"),
    )
    .expect("single summary JSON");
    assert_eq!(single_summary["length"], json!(1));
    assert_eq!(single_summary["names"], json!(first_name));
    assert_eq!(
        single_summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        single_summary["events"],
        json!(
            "multi:input:2,multi:change:2,multi:input:3,multi:change:3,single:input:1,single:change:1"
        )
    );

    let (empty_paths_status, empty_paths) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": "\n \n"
        }),
    )
    .await;
    assert_eq!(empty_paths_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_paths["value"]["error"], json!("invalid argument"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_key_source_uses_shared_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<input id='field' value='abc'>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let actions = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyUp", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "\u{E003}" },
                    { "type": "keyUp", "value": "\u{E003}" },
                    { "type": "keyDown", "value": "x" },
                    { "type": "keyUp", "value": "x" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions, json!({ "value": null }));

    let value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/value"),
    )
    .await;
    assert_eq!(value, json!({ "value": "x" }));

    let cleared = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/clear"),
    )
    .await;
    assert_eq!(cleared, json!({ "value": null }));
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let shifted_actions = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "f" },
                    { "type": "keyUp", "value": "f" },
                    { "type": "keyDown", "value": "\u{E008}" },
                    { "type": "keyDown", "value": "o" },
                    { "type": "keyUp", "value": "o" },
                    { "type": "keyDown", "value": "b" },
                    { "type": "keyUp", "value": "b" },
                    { "type": "keyUp", "value": "\u{E008}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyDown", "value": "r" },
                    { "type": "keyUp", "value": "r" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(shifted_actions, json!({ "value": null }));

    let shifted_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/value"),
    )
    .await;
    assert_eq!(shifted_value, json!({ "value": "fOBar" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_wheel_source_dispatches_with_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicWheel=null;document.addEventListener('wheel',function(event){window.__classicWheel={type:event.type,deltaX:event.deltaX,deltaY:event.deltaY,clientX:event.clientX,clientY:event.clientY};});</script><div style='width:200px;height:200px'>wheel-target</div>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": "viewport",
                    "x": 10,
                    "y": 11,
                    "deltaX": 7,
                    "deltaY": 13
                }]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let wheel = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicWheel;",
            "args": []
        }),
    )
    .await;
    assert_eq!(wheel["value"]["type"], json!("wheel"));
    assert_eq!(wheel["value"]["deltaX"], json!(7));
    assert_eq!(wheel["value"]["deltaY"], json!(13));
    assert_eq!(wheel["value"]["clientX"], json!(10));
    assert_eq!(wheel["value"]["clientY"], json!(11));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_touch_pointer_dispatches_with_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicTouch=[];['touchstart','touchmove','touchend'].forEach(function(type){document.addEventListener(type,function(event){var point=event.changedTouches[0];window.__classicTouch.push(type+':' +(event instanceof TouchEvent)+':' + point.clientX + ':' + point.clientY);});});</script><main style='width:200px;height:200px'>touch-target</main>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown" },
                    { "type": "pointerMove", "origin": "pointer", "x": 3, "y": 4 },
                    { "type": "pointerUp" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let touch_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicTouch;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        touch_events,
        json!({
            "value": [
                "touchstart:true:10:11",
                "touchmove:true:13:15",
                "touchend:true:13:15"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_touch_pointer_capture_uses_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicTouchCapture=[];button.addEventListener('pointerdown',event=>{window.__classicTouchCapture.push('pointerdown@button:'+event.pointerType);target0.setPointerCapture(event.pointerId);window.__classicTouchCapture.push('has:'+target0.hasPointerCapture(event.pointerId));});button.addEventListener('pointermove',()=>window.__classicTouchCapture.push('pointermove@button'));target0.addEventListener('gotpointercapture',event=>window.__classicTouchCapture.push('gotpointercapture@target0:'+event.pointerType));target0.addEventListener('pointermove',event=>window.__classicTouchCapture.push('pointermove@target0:'+event.pointerType));target0.addEventListener('pointerup',event=>window.__classicTouchCapture.push('pointerup@target0:'+event.pointerType));target0.addEventListener('lostpointercapture',event=>window.__classicTouchCapture.push('lostpointercapture@target0:'+event.pointerType));</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown" },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerUp" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicTouchCapture;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button:touch",
                "has:true",
                "gotpointercapture@target0:touch",
                "pointermove@target0:touch",
                "pointerup@target0:touch",
                "lostpointercapture@target0:touch"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_pen_pointer_preserves_pointer_properties() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<main id='target' style='width:200px;height:200px'>pen-target</main><script>window.__classicPen=[];function v(value){if(value===undefined)return '';return value;}const target=document.getElementById('target');['pointerover','pointerenter','pointermove','pointerdown','pointerup','mouseover','mouseenter','mousemove','mousedown','mouseup','click'].forEach(function(type){target.addEventListener(type,function(event){window.__classicPen.push([type,event.pointerType||'',v(event.pressure),v(event.tangentialPressure),v(event.tiltX),v(event.tiltY),v(event.twist),event.clientX,event.clientY,event.button,event.buttons].join(':'));});});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "pen",
                "parameters": { "pointerType": "pen" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
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
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let pen_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicPen;",
            "args": []
        }),
    )
    .await;
    let pen_events = pen_events["value"]
        .as_array()
        .expect("pen event log should be an array");
    assert!(
        pen_events.iter().any(|event| {
            event.as_str().is_some_and(|event| {
                event.starts_with("pointerdown:pen:0.75:-0.25:12:-8:45:10:11:0:1")
            })
        }),
        "pen events: {pen_events:?}"
    );
    assert!(
        pen_events.iter().any(|event| {
            event
                .as_str()
                .is_some_and(|event| event.starts_with("pointerup:pen:0:0:0:0:0:10:11:0:0"))
        }),
        "pen events: {pen_events:?}"
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_cancelled_pointerdown_suppresses_compat_mouse_events() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='target0'>first</div><div id='target1'>second</div><script>window.__classicCompat=[];for(const id of ['target0','target1']){const target=document.getElementById(id);for(const type of ['pointerdown','pointerup','mousedown','mouseup','click']){target.addEventListener(type,function(event){window.__classicCompat.push(type+'@'+id);if(id==='target0'&&type==='pointerdown')event.preventDefault();});}}</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let compat_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCompat;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        compat_events,
        json!({
            "value": [
                "pointerdown@target0",
                "pointerup@target0",
                "click@target0",
                "pointerdown@target1",
                "mousedown@target1",
                "pointerup@target1",
                "mouseup@target1",
                "click@target1"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_pointer_capture_routes_real_coordinate_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='target0'>first</div><div id='target1'>second</div><script>window.__captureStarted=false;window.__classicCapture=[];for(const id of ['target0','target1']){const target=document.getElementById(id);for(const type of ['pointerdown','gotpointercapture','pointermove','pointerup','lostpointercapture']){target.addEventListener(type,function(event){if(type==='pointermove'&&!window.__captureStarted)return;window.__classicCapture.push(type+'@'+id);if(id==='target0'&&type==='pointerdown'){window.__captureStarted=true;target.setPointerCapture(event.pointerId);window.__classicCapture.push('has:'+target.hasPointerCapture(event.pointerId));}});}}</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCapture;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@target0",
                "has:true",
                "gotpointercapture@target0",
                "pointermove@target0",
                "pointerup@target0",
                "lostpointercapture@target0"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_removed_capture_target_retargets_to_document() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicCaptureRemoval=[];button.addEventListener('pointerdown',event=>{window.__classicCaptureRemoval.push('pointerdown@button');target0.setPointerCapture(event.pointerId);});button.addEventListener('pointerup',()=>window.__classicCaptureRemoval.push('pointerup@button'));target0.addEventListener('gotpointercapture',()=>{window.__classicCaptureRemoval.push('gotpointercapture@target0');target0.remove();});target0.addEventListener('lostpointercapture',()=>window.__classicCaptureRemoval.push('lostpointercapture@target0'));target0.addEventListener('pointerup',()=>window.__classicCaptureRemoval.push('pointerup@target0'));document.addEventListener('lostpointercapture',event=>{if(event.target===document)window.__classicCaptureRemoval.push('lostpointercapture@document');});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCaptureRemoval;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button",
                "gotpointercapture@target0",
                "lostpointercapture@document",
                "pointerup@button"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_active_capture_move_handles_removed_target() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicActiveRemoval=[];button.addEventListener('pointerdown',event=>{window.__classicActiveRemoval.push('pointerdown@button');target0.setPointerCapture(event.pointerId);});button.addEventListener('pointerup',()=>window.__classicActiveRemoval.push('pointerup@button'));target0.addEventListener('gotpointercapture',()=>window.__classicActiveRemoval.push('gotpointercapture@target0'));target0.addEventListener('pointermove',event=>{window.__classicActiveRemoval.push('pointermove@target0');target0.remove();window.__classicActiveRemoval.push('has:'+target0.hasPointerCapture(event.pointerId));});target0.addEventListener('pointerup',()=>window.__classicActiveRemoval.push('pointerup@target0'));document.addEventListener('lostpointercapture',event=>{if(event.target===document)window.__classicActiveRemoval.push('lostpointercapture@document');});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicActiveRemoval;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button",
                "gotpointercapture@target0",
                "pointermove@target0",
                "has:false",
                "lostpointercapture@document",
                "pointerup@button"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_coordinate_actions_dispatch_after_tick_delay() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicEvents=[];document.addEventListener('mousemove',function(){window.__classicEvents.push({type:'move',t:performance.now()});});document.addEventListener('mousedown',function(){window.__classicEvents.push({type:'down',t:performance.now()});});</script><main style='width:200px;height:200px'>actions</main>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11, "duration": 100 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents.map(event => event.type).join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(events, json!({ "value": "move|down" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_release_actions_clear_pressed_sources() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicEvents=[];document.addEventListener('mouseup',function(event){window.__classicEvents.push('mouseup:'+event.button+':'+event.clientX+':'+event.clientY);});document.addEventListener('keyup',function(event){window.__classicEvents.push('keyup:'+event.key);});</script><input id='field' value=''>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let reset_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "window.__classicEvents=[]; return true;",
            "args": []
        }),
    )
    .await;
    assert_eq!(reset_events, json!({ "value": true }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [
                {
                    "type": "pointer",
                    "id": "mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        { "type": "pointerMove", "origin": "viewport", "x": 12, "y": 13 },
                        { "type": "pointerDown", "button": 0 }
                    ]
                },
                {
                    "type": "key",
                    "id": "keyboard",
                    "actions": [{ "type": "keyDown", "value": "a" }]
                }
            ]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let (released_status, released) = classic_request_status_and_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;
    assert_eq!(released_status, StatusCode::OK, "response: {released:?}");
    assert_eq!(released, json!({ "value": null }));

    let events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents;",
            "args": []
        }),
    )
    .await;
    assert_eq!(events, json!({ "value": ["mouseup:0:12:13", "keyup:a"] }));

    let released_again = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;
    assert_eq!(released_again, json!({ "value": null }));

    let events_after_second_release = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents;",
            "args": []
        }),
    )
    .await;
    assert_eq!(events_after_second_release, events);

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_actions_reject_move_target_out_of_bounds() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<main>bounds</main>" }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(navigated, json!({ "value": null }));

    let (status, response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 5000, "y": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response["value"]["error"],
        json!("move target out of bounds")
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_active_element_uses_current_browsing_context_after_child_autofocus() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = "data:text/html,<body id='top-body'><input id='top-input'><iframe id='child' srcdoc=\"<body id='child-body'><input id='child-input' autofocus></body>\"></iframe></body>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    // Navigation completion may precede autofocus's rendering opportunity.
    let rendered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const done = arguments[arguments.length - 1]; requestAnimationFrame(() => done(null));",
            "args": []
        }),
    )
    .await;
    assert_eq!(rendered, json!({ "value": null }));

    let (active_status, active) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active"),
    )
    .await;
    assert_eq!(active_status, StatusCode::OK, "{active:?}");
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("active element should return the focused iframe: {active:?}"));
    let active_tag = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/name"),
    )
    .await;
    assert_eq!(active_tag, json!({ "value": "iframe" }));
    let active_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/property/id"),
    )
    .await;
    assert_eq!(active_property, json!({ "value": "child" }));

    let focused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.getElementById('top-input').focus(); return document.activeElement.id;",
            "args": []
        }),
    )
    .await;
    assert_eq!(focused, json!({ "value": "top-input" }));

    let active = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active/"),
    )
    .await;
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("focused input should be active: {active:?}"));
    let active_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/property/id"),
    )
    .await;
    assert_eq!(active_property, json!({ "value": "top-input" }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let child_focused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.getElementById('child-input').focus(); return document.activeElement.id;",
            "args": []
        }),
    )
    .await;
    assert_eq!(child_focused, json!({ "value": "child-input" }));

    let (active_status, active) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active"),
    )
    .await;
    assert_eq!(active_status, StatusCode::OK, "{active:?}");
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("child frame active element should return input: {active:?}"));
    let active_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/property/id"),
    )
    .await;
    assert_eq!(active_property, json!({ "value": "child-input" }));
}
