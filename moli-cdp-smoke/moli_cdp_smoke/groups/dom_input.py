from __future__ import annotations

import asyncio
import json
import re
import sys
import urllib.parse

from playwright.async_api import Error as PlaywrightError, expect

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until


def _checkpoint(label: str) -> None:
    # The supervisor can terminate this worker without giving Python a cleanup
    # turn. Flush the replacement/Runtime boundary so a timeout artifact names
    # the exact CDP operation that stopped making progress.
    print(
        f"[moli-cdp-smoke] CHECKPOINT dom-input/{label}",
        file=sys.stderr,
        flush=True,
    )


async def run_dom_input_group(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    temp_dir = state.temp_dir

    _checkpoint("static/set-content")
    await page.set_content('<main id="set-content">set content ok</main>')
    _checkpoint("static/read-text")
    assert_equal(await page.text_content("#set-content"), "set content ok", "setContent text")
    state.record("set_content_static_dom")

    _checkpoint("inline/set-content")
    await page.set_content(
        '<main id="set-content-inline">inline content ok</main><script>window.__moliSetContentInlineRan = (window.__moliSetContentInlineRan || 0) + 1;</script>'
    )
    _checkpoint("inline/read-text")
    assert_equal(await page.text_content("#set-content-inline"), "inline content ok", "setContent inline text")
    _checkpoint("inline/read-marker")
    assert_equal(await page.evaluate("() => window.__moliSetContentInlineRan"), 1, "setContent inline script ran")
    state.record("set_content_inline_script")

    await run_add_script_and_style_tag_workflows(state)

    upload_file = temp_dir / "upload.txt"
    upload_file.write_text("upload contents", encoding="utf-8")
    upload_file_with_spaces = temp_dir / "file to upload.txt"
    upload_file_with_spaces.write_text("contents with spaces", encoding="utf-8")
    await page.set_content(
        """
        <input id="upload" type="file">
        <script>
          window.__uploadEvents = [];
          upload.addEventListener('input', event => window.__uploadEvents.push(event.type + ':' + event.composed + ':' + upload.files.length));
          upload.addEventListener('change', event => window.__uploadEvents.push(event.type + ':' + event.composed + ':' + upload.files.length));
        </script>
        """
    )
    await page.set_input_files("#upload", str(upload_file))
    uploaded = await page.evaluate(
        """() => {
          const file = document.querySelector('#upload')?.files?.[0];
          return file ? { name: file.name, size: file.size } : null;
        }"""
    )
    assert_equal(uploaded.get("name"), "upload.txt", "uploaded file name")
    assert_equal(uploaded.get("size"), len("upload contents"), "uploaded file size")
    upload_text = await page.evaluate(
        """async () => {
          const file = document.querySelector('#upload')?.files?.[0];
          return file ? await new Promise(resolve => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result);
            reader.readAsText(file);
          }) : null;
        }"""
    )
    assert_equal(upload_text, "upload contents", "uploaded file FileReader text")
    assert_equal(
        await page.evaluate("() => window.__uploadEvents.join('|')"),
        "input:true:1|change:false:1",
        "setInputFiles input/change events",
    )
    await page.evaluate("() => window.__uploadEvents = []")
    await page.set_input_files("#upload", str(upload_file_with_spaces))
    replacement = await page.evaluate(
        """async () => {
          const file = document.querySelector('#upload')?.files?.[0];
          if (!file) return null;
          const text = await new Promise(resolve => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result);
            reader.readAsText(file);
          });
          return [file.name, file.size, text, window.__uploadEvents.join('|')].join('||');
        }"""
    )
    assert_equal(
        replacement,
        "file to upload.txt||20||contents with spaces||input:true:1|change:false:1",
        "setInputFiles replaces files and dispatches events again",
    )
    state.record("set_input_files")

    await page.set_content('<input id="chooser" type="file" multiple>')
    surface = await page.locator("#chooser").evaluate(
        """input => [
          input instanceof HTMLInputElement,
          input.constructor && input.constructor.name,
          typeof input.type,
          input.type,
          input.multiple,
        ].join('|')"""
    )
    assert_equal(surface, "true|HTMLInputElement|string|file|true", "file chooser input surface before click")
    async with page.expect_file_chooser(timeout=10_000) as chooser_info:
        await page.locator("#chooser").evaluate("input => input.click()")
    chooser = await chooser_info.value
    await chooser.set_files(str(upload_file))
    chooser_files = await page.evaluate(
        """() => Array.from(document.querySelector('#chooser')?.files || []).map(file => ({ name: file.name, size: file.size }))"""
    )
    assert_equal(len(chooser_files), 1, "file chooser selected file count")
    assert_equal(chooser_files[0].get("name"), "upload.txt", "file chooser selected file name")
    assert_equal(chooser_files[0].get("size"), len("upload contents"), "file chooser selected file size")
    chooser_text = await page.evaluate(
        """async () => {
          const file = document.querySelector('#chooser')?.files?.[0];
          return file ? await new Promise(resolve => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result);
            reader.readAsText(file);
          }) : null;
        }"""
    )
    assert_equal(chooser_text, "upload contents", "file chooser FileReader text")
    state.record("file_chooser_set_files")

    await page.set_content(
        """
        <button id="open-chooser" onclick="document.getElementById('picker-script').showPicker()">open chooser</button>
        <input id="picker-script" type="file" multiple>
        """,
        wait_until="domcontentloaded",
    )
    async with page.expect_file_chooser(timeout=10_000) as scripted_chooser_info:
        await page.locator("#open-chooser").evaluate("button => button.click()")
    scripted_chooser = await scripted_chooser_info.value
    await scripted_chooser.set_files(str(upload_file))
    scripted_files = await page.evaluate(
        """() => Array.from(document.querySelector('#picker-script')?.files || []).map(file => ({ name: file.name, size: file.size }))"""
    )
    assert_equal(len(scripted_files), 1, "scripted file chooser selected file count")
    assert_equal(scripted_files[0].get("name"), "upload.txt", "scripted file chooser selected file name")
    assert_equal(scripted_files[0].get("size"), len("upload contents"), "scripted file chooser selected file size")
    state.record("file_chooser_show_picker")

    await page.set_content(f'<a id="go" href="{fixture}/plain">go</a>')
    assert_equal(await page.get_attribute("#go", "href"), f"{fixture}/plain", "link navigation href")
    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    assert_equal(await page.text_content("main"), "plain ok", "click navigation target text")
    state.record("click_navigation")

    quoted_email = "agent'quoted@example.test"
    await _navigate_until_dom_ready(state, f"{fixture}/auth-email")
    await page.fill("#email", quoted_email)
    await _navigate_until_dom_ready(state, f"{fixture}/auth-password?email={urllib.parse.quote(quoted_email)}")
    assert_equal(
        await page.text_content("main"),
        "password page",
        "auth-style hydrated click redirect target text",
    )
    assert_equal(
        await page.get_attribute("#password", "type"),
        "password",
        "auth-style hydrated click redirect password input",
    )
    assert_equal(
        await page.get_attribute("#password", "data-email"),
        quoted_email,
        "auth fixture password page preserves quoted email attribute value",
    )
    injected_email = "agent' autofocus onfocus='alert(1)@example.test"
    await _navigate_until_dom_ready(state, f"{fixture}/auth-password?email={urllib.parse.quote(injected_email)}")
    assert_equal(
        await page.get_attribute("#password", "data-email"),
        injected_email,
        "auth fixture password page preserves malicious quoted email attribute value",
    )
    assert_equal(
        await page.get_attribute("#password", "autofocus"),
        None,
        "auth fixture password page does not inject autofocus attribute",
    )
    assert_equal(
        await page.get_attribute("#password", "onfocus"),
        None,
        "auth fixture password page does not inject event-handler attribute",
    )
    state.record("auth_style_hydrated_click_redirect")

    await run_locator_input_workflows(state)
    await run_user_facing_locator_workflows(state)
    await run_role_selector_state_workflows(state)
    await run_playwright_expect_matcher_workflows(state)
    await run_locator_composition_workflows(state)
    await run_keyboard_editing_workflows(state)
    await run_cdp_control_key_name_workflow(state)
    await run_cdp_input_navigation_replacement_workflows(state)
    await run_mouse_event_workflows(state)
    await run_fill_input_type_workflows(state)
    await run_check_input_workflows(state)
    await run_select_option_workflows(state)
    await run_selector_eval_and_handle_conversion_workflows(state)
    await run_element_handle_state_workflows(state)
    await run_dom_handle_workflows(state)
    await run_touch_input_workflows(state)


async def run_add_script_and_style_tag_workflows(state: SmokeState) -> None:
    # Reduced from Playwright page-add-script-tag.spec.ts and page-add-style-tag.spec.ts.
    page = state.page
    fixture = state.fixture
    temp_dir = state.temp_dir

    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    script_handle = await page.add_script_tag(content="window.__playwrightInjectedFromContent = 35;")
    assert_equal(await script_handle.evaluate("node => node.tagName"), "SCRIPT", "addScriptTag content handle")
    assert_equal(
        await page.evaluate("() => window.__playwrightInjectedFromContent"),
        35,
        "addScriptTag content executes",
    )

    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    url_script_handle = await page.add_script_tag(url=f"{fixture}/playwright-injected.js")
    assert_equal(await url_script_handle.evaluate("node => node.tagName"), "SCRIPT", "addScriptTag URL handle")
    assert_equal(
        await page.evaluate("() => window.__playwrightInjectedFromUrl"),
        42,
        "addScriptTag URL executes",
    )
    missing_script_url = f"{fixture}/playwright-missing-script.js"
    missing_script_error = await _expect_playwright_error(page.add_script_tag(url=missing_script_url))
    if missing_script_url not in missing_script_error:
        raise SmokeError(f"unexpected addScriptTag missing URL error: {missing_script_error}")

    script_path = temp_dir / "playwright-path-injected.js"
    script_path.write_text("window.__playwrightInjectedFromPath = 51;", encoding="utf-8")
    await page.add_script_tag(path=str(script_path))
    assert_equal(
        await page.evaluate("() => window.__playwrightInjectedFromPath"),
        51,
        "addScriptTag path executes",
    )

    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    style_handle = await page.add_style_tag(content="body { background-color: rgb(0, 128, 0); }")
    assert_equal(await style_handle.evaluate("node => node.tagName"), "STYLE", "addStyleTag content handle")
    assert_equal(
        await page.evaluate("() => getComputedStyle(document.body).backgroundColor"),
        "rgb(0, 128, 0)",
        "addStyleTag content applies",
    )

    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    url_style_handle = await page.add_style_tag(url=f"{fixture}/playwright-injected.css")
    assert_equal(await url_style_handle.evaluate("node => node.tagName"), "LINK", "addStyleTag URL handle")
    assert_equal(
        await page.evaluate("() => getComputedStyle(document.body).backgroundColor"),
        "rgb(255, 0, 0)",
        "addStyleTag URL applies",
    )
    missing_style_url = f"{fixture}/playwright-missing-style.css"
    missing_style_error = await _expect_playwright_error(page.add_style_tag(url=missing_style_url))
    if not missing_style_error:
        raise SmokeError(f"unexpected addStyleTag missing URL error: {missing_style_error}")

    style_path = temp_dir / "playwright-path-injected.css"
    style_path.write_text("body { color: rgb(12, 34, 56); }", encoding="utf-8")
    await page.add_style_tag(path=str(style_path))
    assert_equal(
        await page.evaluate("() => getComputedStyle(document.body).color"),
        "rgb(12, 34, 56)",
        "addStyleTag path applies",
    )

    state.record("playwright_add_script_and_style_tag_workflows")


async def run_locator_input_workflows(state: SmokeState) -> None:
    page = state.page

    await page.evaluate(
        """() => {
          document.body.innerHTML = `
            <style>
              #drag { position: absolute; left: 20px; top: 220px; width: 80px; height: 40px; background: #ddd; }
              #drop { position: absolute; left: 160px; top: 220px; width: 120px; height: 60px; background: #eee; }
              #hover-menu { position: absolute; left: 20px; top: 160px; width: 90px; }
              #hover { width: 90px; height: 36px; }
              #hover-child { display: none; width: 90px; height: 24px; }
              #hover-menu:hover #hover-child { display: block; }
              #editable { min-width: 120px; min-height: 24px; }
            </style>
            <input id="text" value="">
            <textarea id="area"></textarea>
            <div id="editable" contenteditable="true"></div>
            <div id="hover-menu"><button id="hover">hover</button><button id="hover-child">child</button></div>
            <input id="check" type="checkbox">
            <input id="radio-a" name="choice" type="radio" value="a">
            <input id="radio-b" name="choice" type="radio" value="b">
            <select id="select"><option value="one">One</option><option value="two">Two</option></select>
            <div id="drag" draggable="true">drag</div>
            <div id="drop">drop</div>
          `;
          window.__keys = [];
          const textInput = document.getElementById('text');
          const hoverTarget = document.getElementById('hover');
          const hoverChild = document.getElementById('hover-child');
          const dragSource = document.getElementById('drag');
          const dropTarget = document.getElementById('drop');
          textInput.addEventListener('keydown', event => window.__keys.push(`${event.key}:${event.shiftKey}:${event.ctrlKey || event.metaKey}`));
          hoverTarget.addEventListener('mouseenter', () => window.__hovered = true);
          hoverChild.addEventListener('click', () => window.__hoverChildClicked = true);
          dragSource.addEventListener('dragstart', event => event.dataTransfer.setData('text/plain', 'dragged-value'));
          dropTarget.addEventListener('dragover', event => event.preventDefault());
          dropTarget.addEventListener('drop', event => {
            event.preventDefault();
            dropTarget.textContent = event.dataTransfer.getData('text/plain');
          });
        }"""
    )

    await page.locator("#text").fill("alpha")
    await page.locator("#text").press("End")
    await page.locator("#text").type("-beta")
    await page.keyboard.down("Shift")
    await page.locator("#text").press("KeyA")
    await page.keyboard.up("Shift")
    text_value = await page.locator("#text").input_value()
    assert_equal(text_value, "alpha-betaA", "locator fill/type/press text value")

    key_log = await page.evaluate("() => window.__keys")
    if not any(entry.startswith("A:true:") for entry in key_log):
        raise SmokeError(f"keyboard modifier should reach keydown handler, got {key_log}")

    await page.locator("#area").fill("line one")
    await page.locator("#area").press("Enter")
    await page.locator("#area").type("line two")
    assert_equal(await page.locator("#area").input_value(), "line one\nline two", "textarea locator input")

    await page.locator("#editable").fill("draft")
    await page.locator("#editable").press("Control+A")
    await page.locator("#editable").type("final")
    assert_equal(await page.text_content("#editable"), "final", "contenteditable locator input")

    await page.locator("#hover").hover(timeout=1_000)
    assert_equal(
        await page.evaluate("() => window.__hovered === true"),
        True,
        "locator hover is dispatched",
    )
    assert_equal(
        await page.evaluate(
            """() => ({
              target: document.querySelector('#hover').matches(':hover'),
              ancestor: document.querySelector('#hover-menu').matches(':hover'),
              childDisplay: getComputedStyle(document.querySelector('#hover-child')).display,
            })"""
        ),
        {"target": True, "ancestor": True, "childDisplay": "block"},
        "locator hover persists in Stylo and exposes the dropdown",
    )
    await page.locator("#hover-child").click(timeout=1_000)
    assert_equal(
        await page.evaluate("() => window.__hoverChildClicked === true"),
        True,
        "hover dropdown child is clickable without waiting for a screencast frame",
    )

    await page.locator("#check").evaluate("input => input.checked = true")
    await page.locator("#radio-b").evaluate("input => input.checked = true")
    await page.locator("#select").select_option("two")
    form_state = await page.evaluate(
        """() => ({
          checked: document.querySelector('#check').checked,
          radio: document.querySelector('input[name=choice]:checked')?.value,
          select: document.querySelector('#select').value,
        })"""
    )
    assert_equal(form_state, {"checked": True, "radio": "b", "select": "two"}, "checkbox/radio/select state")

    await _expect_drag_interception_boundary(
        page.locator("#drag").drag_to(page.locator("#drop"), timeout=1_000),
        "Playwright locator.drag_to drag interception boundary",
    )
    assert_equal(await page.text_content("#drop"), "drop", "unsupported drag/drop does not dispatch")
    state.record("locator_input_workflows")


async def run_user_facing_locator_workflows(state: SmokeState) -> None:
    # Reduced from Playwright selectors-get-by.spec.ts and locator-query.spec.ts.
    page = state.page

    await page.set_content(
        """
        <section id="selectors">
          <div data-testid="Hello">Hello world</div>
          <div id="loose-text">
ye  </div>
          <label for="first-input">First Name</label>
          <input id="first-input" type="text">
          <label for="last-input">Last <span>Name</span></label>
          <input id="last-input" type="text">
          <label id="launch-label">Launch</label>
          <button id="labelled-button" aria-labelledby="launch-label"><span>Click me</span></button>
          <input id="aria-input" aria-label="Secret Code">
          <input id="placeholder-one" placeholder="Hello">
          <input id="placeholder-two" placeholder="Hello World">
          <img id="alt-target" alt="Hello Alt">
          <input id="title-target" title="Hello Title">
          <button id="quote-button">let's <span>hello</span></button>
          <div class="filter-target">foo <span>hello world</span> bar</div>
          <div class="filter-target">Hello "world"</div>
          <div class="filter-target"><span>nested world</span></div>
        </section>
        """,
        wait_until="domcontentloaded",
    )

    assert_equal(await page.get_by_test_id("Hello").text_content(), "Hello world", "Playwright get_by_test_id text")
    assert_equal(
        await page.locator("section").get_by_test_id("Hello").text_content(),
        "Hello world",
        "Playwright nested get_by_test_id text",
    )
    assert_equal(
        await page.get_by_test_id(re.compile(r"He[l]*o")).text_content(),
        "Hello world",
        "Playwright regex get_by_test_id text",
    )
    assert_equal(
        await page.get_by_text("ye").evaluate("element => element.id"),
        "loose-text",
        "Playwright get_by_text whitespace normalization",
    )
    assert_equal(
        await page.get_by_text("Hello world", exact=True).evaluate("element => element.dataset.testid"),
        "Hello",
        "Playwright exact get_by_text",
    )

    assert_equal(
        await page.get_by_label("First Name").evaluate("element => element.id"),
        "first-input",
        "Playwright get_by_label for= input",
    )
    assert_equal(
        await page.get_by_label("Last Name", exact=True).evaluate("element => element.id"),
        "last-input",
        "Playwright exact get_by_label nested text",
    )
    assert_equal(
        await page.get_by_label(re.compile(r"Last\s+name", re.I)).evaluate("element => element.id"),
        "last-input",
        "Playwright regex get_by_label nested text",
    )
    assert_equal(
        await page.get_by_label("Launch").evaluate("element => element.id"),
        "labelled-button",
        "Playwright get_by_label aria-labelledby button",
    )
    assert_equal(
        await page.get_by_label("Secret Code").evaluate("element => element.id"),
        "aria-input",
        "Playwright get_by_label aria-label input",
    )

    assert_equal(await page.get_by_placeholder("hello").count(), 2, "Playwright get_by_placeholder fuzzy count")
    assert_equal(
        await page.get_by_placeholder("Hello", exact=True).evaluate("element => element.id"),
        "placeholder-one",
        "Playwright exact get_by_placeholder",
    )
    assert_equal(
        await page.get_by_alt_text("Hello Alt", exact=True).evaluate("element => element.id"),
        "alt-target",
        "Playwright get_by_alt_text",
    )
    assert_equal(
        await page.get_by_title(re.compile("title", re.I)).evaluate("element => element.id"),
        "title-target",
        "Playwright get_by_title regex",
    )
    assert_equal(
        await page.get_by_role("button", name="Launch").evaluate("element => element.id"),
        "labelled-button",
        "Playwright get_by_role accessible name",
    )
    assert_equal(
        await page.get_by_role("button", name=re.compile(r"let's", re.I)).locator("span").text_content(),
        "hello",
        "Playwright get_by_role regex with single quote",
    )

    assert_equal(
        await page.locator(".filter-target", has_text="hello world").text_content(),
        "foo hello world bar",
        "Playwright locator has_text descendant",
    )
    assert_equal(
        await page.locator(".filter-target", has_text='Hello "world"').text_content(),
        'Hello "world"',
        "Playwright locator has_text quotes",
    )
    assert_equal(
        await page.locator(".filter-target", has=page.locator("span", has_text="nested")).text_content(),
        "nested world",
        "Playwright locator has child locator",
    )
    assert_equal(
        await page.locator(".filter-target").filter(has_text="hello world").locator("span").text_content(),
        "hello world",
        "Playwright locator.filter has_text",
    )

    state.record("playwright_user_facing_locator_workflows")


async def run_role_selector_state_workflows(state: SmokeState) -> None:
    # Reduced from Playwright selectors-role.spec.ts.
    page = state.page

    await page.set_content(
        """
        <section id="role-states">
          <select id="native-select">
            <option id="native-unselected">Hi</option>
            <option id="native-selected" selected>Hello</option>
          </select>
          <div role="option" id="aria-selected" aria-selected="true">Selected ARIA</div>
          <div role="option" id="aria-unselected" aria-selected="false">Unselected ARIA</div>

          <input id="check-off" type="checkbox">
          <input id="check-on" type="checkbox" checked>
          <input id="check-mixed" type="checkbox">
          <div role="checkbox" id="aria-check-on" aria-checked="true">Hi</div>
          <div role="checkbox" id="aria-check-off" aria-checked="false">Hello</div>

          <button id="plain-button">Hi</button>
          <button id="pressed-on" aria-pressed="true">Hello</button>
          <button id="pressed-off" aria-pressed="false">Bye</button>
          <button id="pressed-mixed" aria-pressed="mixed">Mixed</button>

          <div role="treeitem" id="tree-plain">Plain</div>
          <div role="treeitem" id="tree-open" aria-expanded="true">Open</div>
          <div role="treeitem" id="tree-closed" aria-expanded="false">Closed</div>

          <button id="enabled-button">Enabled</button>
          <button id="disabled-button" disabled>Disabled</button>
          <button id="aria-disabled-button" aria-disabled="true">ARIA Disabled</button>
          <button id="aria-enabled-button" aria-disabled="false">ARIA Enabled</button>
          <fieldset disabled><button id="fieldset-button">Fieldset Disabled</button></fieldset>

          <h1 id="heading-one">Heading One</h1>
          <h3 id="heading-three">Heading Three</h3>
          <div id="heading-five" role="heading" aria-level="5">Heading Five</div>

          <div id="named-button" role="button" aria-label=" Hello "></div>
          <div id="hidden-named-button" role="button" aria-label="Hello" aria-hidden="true"></div>
          <a id="role-link" href="/webdriver/basic">he llo 56</a>
        </section>
        """,
        wait_until="domcontentloaded",
    )
    await page.evaluate("() => { document.getElementById('check-mixed').indeterminate = true; }")

    async def ids(locator) -> list[str]:
        return await locator.evaluate_all("elements => elements.map(element => element.id)")

    assert_equal(
        await ids(page.get_by_role("option", selected=True)),
        ["native-selected", "aria-selected"],
        "Playwright get_by_role selected=true",
    )
    assert_equal(
        await ids(page.get_by_role("option", selected=False)),
        ["native-unselected", "aria-unselected"],
        "Playwright get_by_role selected=false",
    )
    assert_equal(
        await ids(page.get_by_role("checkbox", checked=True)),
        ["check-on", "aria-check-on"],
        "Playwright get_by_role checked=true",
    )
    assert_equal(
        await ids(page.get_by_role("checkbox", checked=False)),
        ["check-off", "aria-check-off"],
        "Playwright get_by_role checked=false",
    )
    assert_equal(
        await ids(page.locator('role=checkbox[checked="mixed"]')),
        ["check-mixed"],
        "Playwright role selector checked=mixed",
    )
    assert_equal(
        await ids(page.get_by_role("button", pressed=True)),
        ["pressed-on"],
        "Playwright get_by_role pressed=true",
    )
    assert_equal(
        await ids(page.get_by_role("button", pressed=False)),
        [
            "plain-button",
            "pressed-off",
            "enabled-button",
            "disabled-button",
            "aria-disabled-button",
            "aria-enabled-button",
            "fieldset-button",
            "named-button",
        ],
        "Playwright get_by_role pressed=false",
    )
    assert_equal(
        await ids(page.locator('role=button[pressed="mixed"]')),
        ["pressed-mixed"],
        "Playwright role selector pressed=mixed",
    )
    assert_equal(
        await ids(page.get_by_role("treeitem", expanded=True)),
        ["tree-open"],
        "Playwright get_by_role expanded=true",
    )
    assert_equal(
        await ids(page.get_by_role("treeitem", expanded=False)),
        ["tree-closed"],
        "Playwright get_by_role expanded=false",
    )
    assert_equal(
        await ids(page.get_by_role("button", disabled=True)),
        ["disabled-button", "aria-disabled-button", "fieldset-button"],
        "Playwright get_by_role disabled=true",
    )
    assert_equal(
        await ids(page.get_by_role("button", disabled=False)),
        ["plain-button", "pressed-on", "pressed-off", "pressed-mixed", "enabled-button", "aria-enabled-button", "named-button"],
        "Playwright get_by_role disabled=false",
    )
    assert_equal(
        await ids(page.get_by_role("heading", level=1)),
        ["heading-one"],
        "Playwright get_by_role heading level=1",
    )
    assert_equal(
        await ids(page.get_by_role("heading", level=3)),
        ["heading-three"],
        "Playwright get_by_role heading level=3",
    )
    assert_equal(
        await ids(page.get_by_role("heading", level=5)),
        ["heading-five"],
        "Playwright get_by_role heading level=5",
    )
    assert_equal(
        await ids(page.get_by_role("button", name="Hello")),
        ["pressed-on", "named-button"],
        "Playwright get_by_role name whitespace normalization",
    )
    assert_equal(
        await ids(page.get_by_role("button", name="Hello", include_hidden=True)),
        ["pressed-on", "named-button", "hidden-named-button"],
        "Playwright get_by_role include_hidden name",
    )
    assert_equal(
        await ids(page.get_by_role("link", name="   he \n llo 56 ", exact=True)),
        ["role-link"],
        "Playwright get_by_role exact name whitespace normalization",
    )

    state.record("playwright_role_selector_state_workflows")


async def run_playwright_expect_matcher_workflows(state: SmokeState) -> None:
    # Reduced from Playwright expect-to-have-text.spec.ts,
    # expect-to-have-value.spec.ts, expect-misc.spec.ts, and expect-boolean.spec.ts.
    page = state.page

    await page.set_content(
        """
        <section id="expect">
          <div id="node"><span></span>Text
            content&nbsp;    </div>
          <ul id="items">
            <li class="item alpha">Alpha    1</li>
            <li class="item beta">Beta
              2</li>
          </ul>
          <input id="name" value="Ada">
          <select id="colors" multiple>
            <option value="R" selected>Red</option>
            <option value="G" selected>Green</option>
            <option value="B">Blue</option>
          </select>
          <div id="status" class="status ready" data-state="ready">Ready</div>
          <button id="enabled">enabled</button>
          <button id="disabled" disabled>disabled</button>
          <input id="checked" type="checkbox" checked>
          <input id="unchecked" type="checkbox">
          <div id="hidden" hidden>hidden</div>
          <div id="visible">visible</div>
        </section>
        """,
        wait_until="domcontentloaded",
    )

    await expect(page.locator("#node")).to_have_text("Text                        content")
    await expect(page.locator("#node")).to_contain_text("ext        cont")
    await expect(page.locator(".item")).to_have_count(2)
    await expect(page.locator(".item")).to_have_text(["Alpha 1", "Beta 2"])
    await expect(page.locator(".item")).to_contain_text([re.compile("Alpha"), "Beta"])
    await expect(page.locator("#name")).to_have_value("Ada")
    await expect(page.locator("#colors")).to_have_values(["R", "G"])
    await expect(page.locator("#status")).to_have_attribute("data-state", "ready")
    await expect(page.locator("#status")).to_have_class("status ready")
    await expect(page.locator("#visible")).to_be_visible()
    await expect(page.locator("#hidden")).to_be_hidden()
    await expect(page.locator("#enabled")).to_be_enabled()
    await expect(page.locator("#disabled")).to_be_disabled()
    await expect(page.locator("#checked")).to_be_checked()
    await expect(page.locator("#unchecked")).not_to_be_checked()

    await page.set_content(
        """
        <ul id="eventual"></ul>
        <script>
          setTimeout(() => {
            document.getElementById('eventual').innerHTML = '<li>One</li><li>Two</li>';
          }, 50);
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await expect(page.locator("#eventual li")).to_have_count(2, timeout=5_000)
    await expect(page.locator("#eventual li")).to_have_text(["One", "Two"])

    try:
        await expect(page.locator("#eventual li").first).to_have_text("Missing", timeout=200)
    except AssertionError as error:
        text_error = str(error)
    else:
        raise SmokeError("expected Playwright expect text mismatch to fail")
    if "Missing" not in text_error or "One" not in text_error:
        raise SmokeError(f"unexpected Playwright expect text mismatch error: {text_error}")

    state.record("playwright_expect_matcher_workflows")


async def run_locator_composition_workflows(state: SmokeState) -> None:
    # Reduced from Playwright locator-query.spec.ts and locator-misc-2.spec.ts.
    page = state.page
    fixture = state.fixture

    await page.set_content(
        """
        <section id="composition">
          <div class="box" data-testid="foo"><p>one</p></div>
          <div class="box" data-testid="bar"><p>two</p><p>second</p></div>
          <div class="box" data-testid="foo"><p>three</p><button>act</button></div>
          <span data-testid="bar">span bar</span>
          <div id="outer-control"><input value="outer"></div>
        </section>
        """,
        wait_until="domcontentloaded",
    )

    assert_equal(await page.locator(".box >> p").count(), 4, "Playwright locator chained selector count")
    assert_equal(await page.locator(".box").locator("p").count(), 4, "Playwright locator.locator count")
    assert_equal(await page.locator(".box").first.locator("p").count(), 1, "Playwright locator.first")
    assert_equal(await page.locator(".box").last.locator("p").count(), 1, "Playwright locator.last")
    assert_equal(await page.locator(".box").nth(1).locator("p").count(), 2, "Playwright locator.nth")
    assert_equal(
        await page.locator(".box").nth(1).locator("p").nth(1).text_content(),
        "second",
        "Playwright nested locator.nth text",
    )

    assert_equal(
        await page.locator(".box").filter(has_text="two").locator("p").count(),
        2,
        "Playwright locator.filter has_text count",
    )
    assert_equal(
        await page.locator(".box").filter(has=page.locator("button")).text_content(),
        "threeact",
        "Playwright locator.filter has locator",
    )
    assert_equal(
        await page.locator(".box").filter(has_not=page.locator("button")).count(),
        2,
        "Playwright locator.filter has_not locator",
    )
    assert_equal(
        await page.locator(".box").filter(has_not_text="two").count(),
        2,
        "Playwright locator.filter has_not_text",
    )

    div_foo = page.locator("div").and_(page.get_by_test_id("foo"))
    assert_equal(await div_foo.count(), 2, "Playwright locator.and_ count")
    assert_equal(await div_foo.nth(1).text_content(), "threeact", "Playwright locator.and_ text")
    assert_equal(
        await page.get_by_test_id("bar").and_(page.locator("span")).text_content(),
        "span bar",
        "Playwright locator.and_ intersect tag",
    )
    assert_equal(
        await page.locator("button").or_(page.locator("span")).count(),
        2,
        "Playwright locator.or_ union count",
    )
    assert_equal(
        await page.locator("article").or_(page.locator("button")).text_content(),
        "act",
        "Playwright locator.or_ fallback text",
    )
    assert_equal(
        await page.locator(".box").locator(page.locator("button").or_(page.locator("span"))).text_content(),
        "act",
        "Playwright locator.locator accepts composite locator",
    )

    input_locator = page.locator("input")
    assert_equal(await input_locator.input_value(), "outer", "Playwright top-level locator input value")
    assert_equal(
        await page.locator("#outer-control").locator(input_locator).input_value(),
        "outer",
        "Playwright Locator.locator accepts Locator",
    )

    await _goto_iframe_for_playwright_frame_model(page, f"{fixture}/iframe")
    frame_input_locator = page.locator("input")
    assert_equal(
        await page.frame_locator("iframe").locator(frame_input_locator).input_value(),
        "inner",
        "Playwright FrameLocator.locator accepts Locator",
    )
    assert_equal(
        await page.frame_locator("iframe").locator("body").locator(frame_input_locator).input_value(),
        "inner",
        "Playwright nested frame locator accepts Locator",
    )

    state.record("playwright_locator_composition_workflows")


async def run_keyboard_editing_workflows(state: SmokeState) -> None:
    # Reduced from Playwright elementhandle-type.spec.ts.
    page = state.page

    await page.set_content("<input id='plain' type='text'>")
    await page.type("#plain", "hello")
    assert_equal(await page.eval_on_selector("#plain", "input => input.value"), "hello", "Playwright page.type basic")

    await page.set_content("<input id='prefilled' type='text' value='hello'>")
    await page.type("#prefilled", "world")
    assert_equal(
        await page.eval_on_selector("#prefilled", "input => input.value"),
        "worldhello",
        "Playwright page.type does not select existing value",
    )

    await page.set_content("<input id='reset' type='text' value='hello'><div id='other' tabindex='2'>other</div>")
    await page.eval_on_selector(
        "#reset",
        """input => {
          input.selectionStart = 2;
          input.selectionEnd = 4;
          document.getElementById('other').focus();
        }""",
    )
    await page.type("#reset", "world")
    assert_equal(
        await page.eval_on_selector("#reset", "input => input.value"),
        "worldhello",
        "Playwright page.type resets stale selection when target is not focused",
    )

    await page.set_content("<input id='focused' type='text' value='hello'>")
    await page.eval_on_selector(
        "#focused",
        """input => {
          input.focus();
          input.selectionStart = 2;
          input.selectionEnd = 4;
        }""",
    )
    await page.type("#focused", "world")
    assert_equal(
        await page.eval_on_selector("#focused", "input => input.value"),
        "heworldo",
        "Playwright page.type preserves active selection",
    )

    await page.set_content("<input id='number' type='number' value='2'>")
    await page.type("#number", "13")
    assert_equal(
        await page.eval_on_selector("#number", "input => input.value"),
        "132",
        "Playwright page.type number input",
    )

    await page.set_content(
        """
        <textarea id="cancel"></textarea>
        <script>
          cancel.addEventListener("keydown", event => {
            if (event.key === "l" || event.key === "o") {
              event.preventDefault();
            }
          });
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.type("#cancel", "Hello World!")
    assert_equal(
        await page.eval_on_selector("#cancel", "textarea => textarea.value"),
        "He Wrd!",
        "Playwright page.type honors canceled keydown",
    )

    await page.set_content(
        """
        <textarea id="guarded">some text</textarea>
        <script>
          guarded.addEventListener("keydown", event => {
            if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
              event.preventDefault();
            }
          });
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.focus("#guarded")
    await page.eval_on_selector(
        "#guarded",
        "textarea => textarea.setSelectionRange(textarea.value.length, textarea.value.length)",
    )
    await page.keyboard.press("Control+A")
    await page.keyboard.press("Backspace")
    assert_equal(
        await page.eval_on_selector("#guarded", "textarea => textarea.value"),
        "some tex",
        "Playwright keyboard honors canceled select-all default",
    )

    await page.set_content(
        """
        <textarea id="repeat"></textarea>
        <script>
          window.__repeatEvents = [];
          repeat.addEventListener("keydown", event => {
            window.__repeatEvents.push(`${event.key}:${event.repeat}`);
          });
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.focus("#repeat")
    await page.keyboard.down("a")
    await page.keyboard.press("a")
    await page.keyboard.down("b")
    await page.keyboard.down("b")
    await page.keyboard.up("a")
    await page.keyboard.down("a")
    assert_equal(
        await page.evaluate("() => window.__repeatEvents"),
        ["a:false", "a:true", "b:false", "b:true", "a:false"],
        "Playwright keyboard repeat property",
    )
    await page.keyboard.up("a")
    await page.keyboard.up("b")

    state.record("playwright_keyboard_editing_workflows")


async def run_cdp_control_key_name_workflow(state: SmokeState) -> None:
    page = await state.context.new_page()
    session = None
    try:
        await page.goto(f"{state.fixture}/plain?cdp-control-key-name")
        await page.evaluate(
            """() => {
              document.body.innerHTML = '<input id="control-key">';
              window.__controlKeyEvents = [];
              const input = document.getElementById("control-key");
              input.focus();
              for (const type of ["keydown", "keyup"]) {
                input.addEventListener(type, event => {
                  window.__controlKeyEvents.push({
                    type: event.type,
                    key: event.key,
                    code: event.code,
                    keyCode: event.keyCode,
                  });
                });
              }
            }"""
        )
        session = await state.context.new_cdp_session(page)

        # Rod encodes Enter as a carriage-return CDP key string. Chromium
        # converts that spelling to the canonical DOM key before dispatching.
        for event_type in ("rawKeyDown", "keyUp"):
            await session.send(
                "Input.dispatchKeyEvent",
                {
                    "type": event_type,
                    "key": "\r",
                    "code": "Enter",
                    "text": "",
                    "unmodifiedText": "",
                    "windowsVirtualKeyCode": 13,
                    "location": 0,
                    "isKeypad": False,
                    "commands": [],
                },
            )

        assert_equal(
            await page.evaluate("() => window.__controlKeyEvents"),
            [
                {"type": "keydown", "key": "Enter", "code": "Enter", "keyCode": 13},
                {"type": "keyup", "key": "Enter", "code": "Enter", "keyCode": 13},
            ],
            "CDP control-key spelling is canonicalized before DOM dispatch",
        )
        state.record("cdp_control_key_name_normalization")
    finally:
        if session is not None:
            await session.detach()
        await page.close()


async def run_cdp_input_navigation_replacement_workflows(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp

    key_destination = f"{state.fixture}/plain?input-navigation=key"
    await page.set_content(
        f"""
        <input id="navigation-field" autofocus>
        <script>
          const navigationField = document.getElementById("navigation-field");
          navigationField.addEventListener("keydown", event => {{
            if (event.key === "Enter") location.href = {json.dumps(key_destination)};
          }});
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.focus("#navigation-field")
    async with page.expect_navigation(
        url=key_destination,
        wait_until="domcontentloaded",
        timeout=10_000,
    ):
        key_result = await asyncio.wait_for(
            cdp.send(
                "Input.dispatchKeyEvent",
                {
                    "type": "keyDown",
                    "key": "Enter",
                    "code": "Enter",
                    "text": "",
                    "unmodifiedText": "",
                    "windowsVirtualKeyCode": 13,
                    "nativeVirtualKeyCode": 13,
                },
            ),
            timeout=5,
        )
    assert_equal(key_result, {}, "CDP key input response across Page replacement")
    assert_equal(
        await page.text_content("main"),
        "plain ok",
        "CDP key input replacement Page remains usable",
    )

    mouse_destination = f"{state.fixture}/plain?input-navigation=mouse"
    await page.set_content(
        f"""
        <style>
          body {{ margin: 0; }}
          #navigation-button {{ position: fixed; left: 0; top: 0; width: 200px; height: 100px; }}
        </style>
        <button id="navigation-button">navigate</button>
        <script>
          document.getElementById("navigation-button").addEventListener("mousedown", () => {{
            location.href = {json.dumps(mouse_destination)};
          }});
        </script>
        """,
        wait_until="domcontentloaded",
    )
    async with page.expect_navigation(
        url=mouse_destination,
        wait_until="domcontentloaded",
        timeout=10_000,
    ):
        mouse_result = await asyncio.wait_for(
            cdp.send(
                "Input.dispatchMouseEvent",
                {
                    "type": "mousePressed",
                    "x": 10,
                    "y": 10,
                    "button": "left",
                    "buttons": 1,
                    "clickCount": 1,
                },
            ),
            timeout=5,
        )
    assert_equal(mouse_result, {}, "CDP mouse input response across Page replacement")
    assert_equal(
        await page.text_content("main"),
        "plain ok",
        "CDP mouse input replacement Page remains usable",
    )
    state.record(
        "cdp_input_navigation_replacement_liveness",
        {"methods": ["Input.dispatchKeyEvent", "Input.dispatchMouseEvent"]},
    )


async def run_mouse_event_workflows(state: SmokeState) -> None:
    page = state.page

    await page.set_content(
        """
        <style>
          body { margin: 0; }
          #target { position: absolute; left: 0; top: 0; width: 400px; height: 400px; }
        </style>
        <button id="target" class="h-screen">target</button>
        <script>
          (() => {
            const target = document.getElementById("target");
            window.__mouseEvents = [];
            window.__resetMouseEvents = () => { window.__mouseEvents = []; };
            const names = [
              "mousemove", "pointermove", "pointerdown", "pointerup",
              "mousedown", "mouseup", "click", "dblclick", "auxclick",
              "contextmenu", "wheel"
            ];
            for (const name of names) {
              target.addEventListener(name, event => {
                window.__mouseEvents.push({
                  type: event.type,
                  detail: event.detail,
                  x: event.clientX,
                  y: event.clientY,
                  trusted: event.isTrusted,
                  button: event.button,
                  buttons: event.buttons,
                  pointerType: "pointerType" in event ? event.pointerType : undefined,
                  deltaX: "deltaX" in event ? event.deltaX : undefined,
                  deltaY: "deltaY" in event ? event.deltaY : undefined,
                  altKey: event.altKey,
                  ctrlKey: event.ctrlKey,
                  metaKey: event.metaKey,
                  shiftKey: event.shiftKey,
                });
              });
            }
          })();
        </script>
        """,
        wait_until="domcontentloaded",
    )

    async def events() -> list[dict[str, object]]:
        return await page.evaluate("() => window.__mouseEvents")

    async def reset() -> None:
        await page.evaluate("() => window.__resetMouseEvents()")

    def require_events(
        log: list[dict[str, object]],
        *required_types: str,
    ) -> None:
        event_types = [event.get("type") for event in log]
        missing = [event_type for event_type in required_types if event_type not in event_types]
        if missing:
            raise SmokeError(f"missing mouse events {missing!r}: {log!r}")

    await reset()
    await page.mouse.click(50, 60)
    click_events = await events()
    require_events(click_events, "pointerdown", "mousedown", "pointerup", "mouseup", "click")
    if not all(event.get("trusted") is True for event in click_events):
        raise SmokeError(f"coordinate click emitted untrusted events: {click_events!r}")

    await reset()
    await page.mouse.dblclick(50, 60)
    double_click_events = await events()
    require_events(double_click_events, "click", "dblclick")
    assert_equal(
        sum(event.get("type") == "click" for event in double_click_events),
        2,
        "Playwright page.mouse.dblclick click count",
    )

    await reset()
    await page.mouse.click(50, 60, button="middle")
    middle_click_events = await events()
    require_events(middle_click_events, "mousedown", "mouseup", "auxclick")
    assert_equal(middle_click_events[-1].get("button"), 1, "middle-click button")

    await reset()
    await page.mouse.click(50, 60, button="right")
    right_click_events = await events()
    require_events(right_click_events, "mousedown", "mouseup", "contextmenu")
    context_menu = next(event for event in right_click_events if event.get("type") == "contextmenu")
    assert_equal(context_menu.get("button"), 2, "right-click button")

    await reset()
    await page.mouse.move(200, 300, steps=5)
    move_events = await events()
    require_events(move_events, "pointermove", "mousemove")
    assert_equal(move_events[-1].get("x"), 200, "mouse move final x")
    assert_equal(move_events[-1].get("y"), 300, "mouse move final y")

    await reset()
    await page.click("#target", modifiers=["Shift"], timeout=1_000)
    shifted_click_events = await events()
    require_events(shifted_click_events, "click")
    if not any(
        event.get("type") == "click" and event.get("shiftKey") is True
        for event in shifted_click_events
    ):
        raise SmokeError(f"page.click did not preserve Shift: {shifted_click_events!r}")

    await reset()
    await page.click("#target", modifiers=[], timeout=1_000)
    require_events(await events(), "click")

    await page.keyboard.down("Control")
    try:
        await reset()
        await page.mouse.wheel(0, -100)
        wheel_events = await events()
        require_events(wheel_events, "wheel")
        wheel = next(event for event in wheel_events if event.get("type") == "wheel")
        assert_equal(wheel.get("deltaY"), -100, "mouse wheel deltaY")
        assert_equal(wheel.get("ctrlKey"), True, "mouse wheel Control modifier")
    finally:
        await page.keyboard.up("Control")

    state.record("playwright_mouse_coordinate_input_workflows")


async def run_fill_input_type_workflows(state: SmokeState) -> None:
    # Reduced from Playwright page-fill.spec.ts.
    page = state.page

    await page.set_content(
        """
        <style>#editor { min-width: 120px; min-height: 24px; }</style>
        <input id="field">
        <textarea id="area"></textarea>
        <div id="editor" contenteditable="true"></div>
        <script>
          window.__fillEvents = [];
          for (const target of [field, area, editor]) {
            for (const eventName of ["input", "change"]) {
              target.addEventListener(eventName, event => {
                window.__fillEvents.push([
                  target.id,
                  event.type,
                  event.composed,
                  "value" in target ? target.value : target.textContent,
                ].join(":"));
              });
            }
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.fill("#field", "some value")
    assert_equal(await page.locator("#field").input_value(), "some value", "Playwright page.fill input")
    await page.fill("#area", "line one\nline two")
    assert_equal(await page.locator("#area").input_value(), "line one\nline two", "Playwright page.fill textarea")
    await page.fill("#editor", "editable value")
    assert_equal(await page.text_content("#editor"), "editable value", "Playwright page.fill contenteditable")
    fill_events = await page.evaluate("() => window.__fillEvents")
    if "field:input:true:some value" not in fill_events:
        raise SmokeError(f"Playwright page.fill should dispatch composed input for text input: {fill_events}")
    if "field:change:false:some value" not in fill_events:
        raise SmokeError(f"Playwright page.fill should dispatch non-composed change for text input: {fill_events}")
    await page.evaluate("() => window.__fillEvents = []")
    await page.locator("#field").clear()
    assert_equal(await page.locator("#field").input_value(), "", "Playwright locator.clear input")
    clear_events = await page.evaluate("() => window.__fillEvents")
    if "field:input:true:" not in clear_events:
        raise SmokeError(f"Playwright locator.clear should dispatch composed input for text input: {clear_events}")

    await page.set_content('<input id="typed">', wait_until="domcontentloaded")
    for input_type in ["password", "search", "tel", "text", "url", "invalid-type"]:
        await page.locator("#typed").evaluate(
            """(input, inputType) => {
              input.type = inputType;
              input.value = "";
            }""",
            input_type,
        )
        await page.fill("#typed", f"text {input_type}")
        assert_equal(
            await page.locator("#typed").input_value(),
            f"text {input_type}",
            f"Playwright page.fill supported input type {input_type}",
        )

    for input_type in ["button", "checkbox", "file", "image", "radio", "reset", "submit"]:
        await page.set_content(
            f'<input id="unsupported" type="{input_type}" style="width:32px;height:24px">'
        )
        error = await _expect_playwright_error(page.fill("#unsupported", ""))
        if f'Input of type "{input_type}" cannot be filled' not in error:
            raise SmokeError(f"Playwright page.fill unsupported {input_type} error mismatch: {error}")

    value_cases = [
        ("color", "#AbCd00", "#abcd00"),
        ("date", "2020-03-02", "2020-03-02"),
        ("time", "13:15", "13:15"),
        ("datetime-local", "2020-03-02T05:15", "2020-03-02T05:15"),
        ("month", "2020-07", "2020-07"),
        ("range", "42", "42"),
        ("week", "2020-W50", "2020-W50"),
        ("number", "-10e5", "-10e5"),
    ]
    for input_type, fill_value, expected in value_cases:
        await page.set_content(f'<input id="typed" type="{input_type}" min="0" max="100" value="">')
        await page.fill("#typed", fill_value)
        assert_equal(
            await page.locator("#typed").input_value(),
            expected,
            f"Playwright page.fill typed input {input_type}",
        )

    invalid_cases = [
        ("color", "badvalue", "Malformed value"),
        ("date", "2020-13-05", "Malformed value"),
        ("time", "25:05", "Malformed value"),
        ("month", "2020-13", "Malformed value"),
        ("week", "2020-123", "Malformed value"),
        ("datetime-local", "abc", "Malformed value"),
        ("number", "abc", "Cannot type text into input[type=number]"),
    ]
    for input_type, fill_value, expected_error in invalid_cases:
        await page.set_content(f'<input id="typed" type="{input_type}">')
        error = await _expect_playwright_error(page.fill("#typed", fill_value))
        if expected_error not in error:
            raise SmokeError(f"Playwright page.fill invalid {input_type} error mismatch: {error}")

    shadow_fill_fixture = """
        <body>
          <div id="host"></div>
          <script>
          (() => {
            const shadowRoot = document.getElementById("host").attachShadow({ mode: "open" });
            shadowRoot.innerHTML = '<input id="shadow-input" type="__INPUT_TYPE__" min="0" max="100">';
            window.__shadowFillEvents = [];
            for (const eventName of ["input", "change"]) {
              document.body.addEventListener(eventName, event => {
                window.__shadowFillEvents.push(`body:${event.type}:${event.composed}`);
              }, { once: true });
              shadowRoot.getElementById("shadow-input").addEventListener(eventName, event => {
                window.__shadowFillEvents.push(`input:${event.type}:${event.composed}`);
              }, { once: false });
            }
          })();
          </script>
        </body>
        """
    for input_type, fill_value, _expected in value_cases[:7]:
        await page.set_content(
            shadow_fill_fixture.replace("__INPUT_TYPE__", input_type),
            wait_until="domcontentloaded",
        )
        await page.locator("input").fill(fill_value)
        assert_equal(
            await page.evaluate("() => window.__shadowFillEvents"),
            ["input:input:true", "body:input:true", "input:change:false"],
            f"Playwright page.fill shadow event composition {input_type}",
        )

    await page.set_content(
        """
        <input id="delayed-disabled" disabled>
        <textarea id="delayed-readonly" readonly></textarea>
        <script>
          setTimeout(() => document.getElementById("delayed-disabled").disabled = false, 50);
          setTimeout(() => document.getElementById("delayed-readonly").readOnly = false, 50);
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.fill("#delayed-disabled", "enabled", timeout=5_000)
    await page.fill("#delayed-readonly", "editable", timeout=5_000)
    assert_equal(await page.locator("#delayed-disabled").input_value(), "enabled", "Playwright page.fill waits for enabled")
    assert_equal(await page.locator("#delayed-readonly").input_value(), "editable", "Playwright page.fill waits for editable")

    await page.set_content("<select><option>value1</option></select>")
    error = await _expect_playwright_error(page.fill("select", ""))
    if "Element is not an <input>, <textarea> or [contenteditable] element" not in error:
        raise SmokeError(f"Playwright page.fill non-fillable error mismatch: {error}")

    state.record("playwright_fill_input_type_workflows")


async def run_check_input_workflows(state: SmokeState) -> None:
    page = state.page

    await page.set_content("<input id='checkbox' type='checkbox'>", wait_until="domcontentloaded")
    assert_equal(await page.locator("input").is_checked(), False, "Playwright is_checked reads unchecked checkbox")
    await page.check("input", timeout=1_000)
    assert_equal(await page.locator("input").is_checked(), True, "Playwright is_checked reads checked checkbox")
    await page.uncheck("input", timeout=1_000)
    assert_equal(await page.locator("input").is_checked(), False, "Playwright page.uncheck toggles checkbox")

    await page.set_content("<div>Check me</div>", wait_until="domcontentloaded")
    check_error = await _expect_playwright_error(page.check("div"))
    if "Not a checkbox or radio button" not in check_error:
        raise SmokeError(f"Playwright page.check non-checkbox error mismatch: {check_error}")
    await page.set_content("<div role='button'>Check me</div>", wait_until="domcontentloaded")
    role_button_error = await _expect_playwright_error(page.check("div"))
    if "Not a checkbox or radio button" not in role_button_error:
        raise SmokeError(f"Playwright page.check role=button error mismatch: {role_button_error}")

    await page.set_content("<input id='trial' type='checkbox'>", wait_until="domcontentloaded")
    await page.check("#trial", trial=True, timeout=1_000)
    assert_equal(await page.locator("#trial").is_checked(), False, "Playwright page.check trial does not check")
    await page.locator("#trial").evaluate("input => input.checked = true")
    await page.uncheck("#trial", trial=True, timeout=1_000)
    assert_equal(await page.locator("#trial").is_checked(), True, "Playwright page.uncheck trial does not uncheck")

    await page.set_content("<input id='set-checked' type='checkbox'>", wait_until="domcontentloaded")
    await page.set_checked("#set-checked", True, timeout=1_000)
    assert_equal(await page.locator("#set-checked").is_checked(), True, "Playwright page.set_checked toggles")

    state.record("playwright_check_input_coordinate_workflows")


async def run_select_option_workflows(state: SmokeState) -> None:
    # Reduced from Playwright page-select-option.spec.ts.
    page = state.page

    await page.set_content(
        """
        <section id="select-wrapper">
          <select id="select">
            <option value="">Choose</option>
            <option value="blue">Blue</option>
            <option value="brown">Brown</option>
            <option value="green">Green</option>
            <option id="white-option" value="white">White</option>
          </select>
        </section>
        <script>
          window.__resetSelectEvents = () => {
            const select = document.getElementById("select");
            select.multiple = false;
            for (const option of select.options)
              option.selected = option.value === "";
            window.__selectResult = {
              onInput: [],
              onChange: [],
              onBubblingInput: [],
              onBubblingChange: [],
            };
          };
          window.__resetSelectEvents();
          const select = document.getElementById("select");
          const wrapper = document.getElementById("select-wrapper");
          select.addEventListener("input", () => window.__selectResult.onInput.push(select.value));
          select.addEventListener("change", () => window.__selectResult.onChange.push(select.value));
          wrapper.addEventListener("input", () => window.__selectResult.onBubblingInput.push(select.value));
          wrapper.addEventListener("change", () => window.__selectResult.onBubblingChange.push(select.value));
        </script>
        """,
        wait_until="domcontentloaded",
    )

    async def reset_select() -> None:
        await page.evaluate("() => window.__resetSelectEvents()")

    async def select_result() -> dict[str, list[str]]:
        return await page.evaluate("() => window.__selectResult")

    await reset_select()
    selected = await page.locator("#select").select_option("blue")
    assert_equal(selected, ["blue"], "Playwright selectOption by value returns selected value")
    assert_equal(
        await select_result(),
        {
            "onInput": ["blue"],
            "onChange": ["blue"],
            "onBubblingInput": ["blue"],
            "onBubblingChange": ["blue"],
        },
        "Playwright selectOption by value dispatches input/change events",
    )

    await reset_select()
    assert_equal(
        await page.locator("#select").select_option(label="Green"),
        ["green"],
        "Playwright selectOption by label",
    )

    await reset_select()
    assert_equal(
        await page.locator("#select").select_option(index=2),
        ["brown"],
        "Playwright selectOption by index",
    )

    await reset_select()
    white = await page.query_selector("#white-option")
    if white is None:
        raise SmokeError("missing white option handle")
    assert_equal(
        await page.locator("#select").select_option(element=white),
        ["white"],
        "Playwright selectOption by ElementHandle",
    )

    await reset_select()
    await page.locator("#select").evaluate("select => select.multiple = true")
    selected = await page.locator("#select").select_option(["blue", "green", "white"])
    assert_equal(selected, ["blue", "green", "white"], "Playwright selectOption multiple values")
    assert_equal(
        await page.locator("#select").evaluate(
            """select => Array.from(select.selectedOptions).map(option => option.value)"""
        ),
        ["blue", "green", "white"],
        "Playwright selectOption multiple DOM state",
    )
    assert_equal(
        await page.locator("#select").select_option([]),
        [],
        "Playwright selectOption empty list returns empty",
    )
    assert_equal(
        await page.locator("#select").evaluate(
            """select => Array.from(select.options).every(option => !option.selected)"""
        ),
        True,
        "Playwright selectOption empty list clears multiple select",
    )

    await reset_select()
    await page.locator("#select").evaluate(
        """select => {
          setTimeout(() => {
            const option = document.createElement("option");
            option.value = "scarlet";
            option.textContent = "Scarlet";
            select.appendChild(option);
          }, 50);
        }"""
    )
    assert_equal(
        await page.locator("#select").select_option("scarlet", timeout=5_000),
        ["scarlet"],
        "Playwright selectOption resolves after option appears",
    )

    select_error = ""
    try:
        await page.locator("body").select_option("blue", timeout=1_000)
    except PlaywrightError as error:
        select_error = str(error)
    if "Element is not a <select> element" not in select_error:
        raise SmokeError(f"Playwright selectOption on non-select should fail, got: {select_error}")

    state.record("playwright_select_option_workflows")


async def _expect_playwright_error(awaitable: object) -> str:
    try:
        await awaitable  # type: ignore[misc]
    except PlaywrightError as error:
        return str(error)
    raise SmokeError("expected Playwright command to fail")


async def _expect_drag_interception_boundary(awaitable: object, label: str) -> str:
    error = await _expect_playwright_error(awaitable)
    explicit_unsupported = "Input.setInterceptDrags" in error and "not supported" in error
    bounded_drag_interception = (
        "Timeout 1000ms exceeded" in error and "performing move and up action" in error
    )
    if not explicit_unsupported and not bounded_drag_interception:
        raise SmokeError(f"{label}: expected drag interception boundary, got: {error}")
    return error


async def run_selector_eval_and_handle_conversion_workflows(state: SmokeState) -> None:
    # Reduced from Playwright eval-on-selector*.spec.ts,
    # elementhandle-content-frame.spec.ts, and jshandle-as-element.spec.ts.
    page = state.page
    fixture = state.fixture

    await page.set_content(
        """
        <section id="testAttribute" data-kind="primary">43543</section>
        <div class="outer"><article><span>Hello</span><button id="target">Next</button></article></div>
        <div class="list">hello</div><div class="list">beautiful</div><div class="list">world!</div>
        """
    )
    assert_equal(
        await page.eval_on_selector("css=section", "element => element.id"),
        "testAttribute",
        "Playwright eval_on_selector css engine",
    )
    assert_equal(
        await page.eval_on_selector("text=43543", "element => element.id"),
        "testAttribute",
        "Playwright eval_on_selector text engine",
    )
    assert_equal(
        await page.eval_on_selector("xpath=/html/body/section", "element => element.dataset.kind"),
        "primary",
        "Playwright eval_on_selector xpath engine",
    )
    assert_equal(
        await page.eval_on_selector(
            "css=div.outer >> css=article >> text=Hello",
            "(element, suffix) => element.textContent + suffix",
            " world",
        ),
        "Hello world",
        "Playwright eval_on_selector chained engines and argument",
    )
    div_handle = await page.query_selector(".list")
    if div_handle is None:
        raise SmokeError("missing selector-eval div handle")
    assert_equal(
        await page.eval_on_selector(
            "section",
            "(element, other) => element.textContent + ':' + other.textContent",
            div_handle,
        ),
        "43543:hello",
        "Playwright eval_on_selector ElementHandle argument",
    )
    assert_equal(
        await page.eval_on_selector_all("css=div.list", "elements => elements.map(element => element.textContent)"),
        ["hello", "beautiful", "world!"],
        "Playwright eval_on_selector_all complex values",
    )
    missing_error = await _expect_playwright_error(page.eval_on_selector("article.missing", "element => element.id"))
    if 'Failed to find element matching selector "article.missing"' not in missing_error:
        raise SmokeError(f"unexpected eval_on_selector missing-element error: {missing_error}")

    body_handle = await page.evaluate_handle("() => document.body")
    if body_handle.as_element() is None:
        raise SmokeError("JSHandle.as_element should return body ElementHandle")
    primitive_handle = await page.evaluate_handle("() => 2")
    assert_equal(primitive_handle.as_element(), None, "JSHandle.as_element returns None for primitives")
    text_handle = await page.evaluate_handle("() => document.querySelector('section').firstChild")
    text_element = text_handle.as_element()
    if text_element is None:
        raise SmokeError("JSHandle.as_element should expose text nodes as ElementHandle")
    assert_equal(
        await page.evaluate("node => node.nodeType === Node.TEXT_NODE", text_element),
        True,
        "JSHandle.as_element text node identity",
    )

    await _goto_iframe_for_playwright_frame_model(page, f"{fixture}/iframe")
    iframe_handle = await page.query_selector("iframe")
    if iframe_handle is None:
        raise SmokeError("missing iframe handle")
    content_frame = await iframe_handle.content_frame()
    child_frame = _child_frame(page)
    if child_frame is None:
        raise SmokeError("missing child frame")
    assert_equal(content_frame, child_frame, "ElementHandle.content_frame for iframe")
    child_body = await child_frame.evaluate_handle("() => document.body")
    child_body_element = child_body.as_element()
    if child_body_element is None:
        raise SmokeError("child body handle should be convertible to ElementHandle")
    assert_equal(await child_body_element.content_frame(), None, "content_frame returns None for non-iframe")
    state.record("playwright_selector_eval_and_handle_conversion_workflows")


async def run_dom_handle_workflows(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture

    await page.evaluate(
        """html => {
          document.body.innerHTML = html;
        }""",
        """
        <main>
          <section id="handle-target" data-kind="primary"><span class="item">one</span><span class="item">two</span></section>
        </main>
        """,
    )
    evaluated = await page.locator("#handle-target").evaluate(
        "node => `${node.tagName}:${node.dataset.kind}:${node.querySelectorAll('.item').length}`"
    )
    assert_equal(evaluated, "SECTION:primary:2", "locator evaluate DOM access")

    handle = await page.query_selector("#handle-target")
    if handle is None:
        raise SmokeError("missing handle target")
    box = await handle.bounding_box()
    if not box or box["width"] <= 0 or box["height"] <= 0:
        raise SmokeError(f"element_handle.bounding_box should expose positive size, got {box}")
    owner_frame = await handle.owner_frame()
    assert_equal(owner_frame, page.main_frame, "main element owner_frame")

    items = await handle.query_selector_all(".item")
    assert_equal([await item.text_content() for item in items], ["one", "two"], "element_handle query_selector_all")

    js_handle = await page.evaluate_handle("() => ({ answer: 42, nested: { value: 'ok' } })")
    answer = await (await js_handle.get_property("answer")).json_value()
    nested = await (await js_handle.get_property("nested")).json_value()
    assert_equal(answer, 42, "JSHandle get_property primitive")
    assert_equal(nested, {"value": "ok"}, "JSHandle get_property object")

    # Reduced from Playwright page-evaluate-handle.spec.ts.
    primitive_handle = await page.evaluate_handle("() => 5")
    is_five = await page.evaluate("(value) => Object.is(value, 5)", primitive_handle)
    assert_equal(is_five, True, "Playwright evaluate accepts primitive JSHandle argument")

    window_handle = await page.evaluate_handle("() => window")
    same_window = await page.evaluate("(value) => value === window", window_handle)
    assert_equal(same_window, True, "Playwright evaluate accepts window handle argument")

    foo_handle = await page.evaluate_handle("() => ({ x: 1, y: 'foo' })")
    bar_handle = await page.evaluate_handle("() => 5")
    baz_handle = await page.evaluate_handle("() => ['baz']")
    nested_handles = await page.evaluate(
        "(value) => JSON.stringify(value)",
        {"a1": {"foo": foo_handle}, "a2": {"bar": bar_handle, "arr": [{"baz": baz_handle}]}},
    )
    assert_equal(
        json.loads(nested_handles),
        {"a1": {"foo": {"x": 1, "y": "foo"}}, "a2": {"bar": 5, "arr": [{"baz": ["baz"]}]}},
        "Playwright evaluate accepts nested JSHandle arguments",
    )
    repeated_handle = await page.evaluate("(value) => value", {"foo": bar_handle, "bar": [bar_handle]})
    assert_equal(
        repeated_handle,
        {"foo": 5, "bar": [5]},
        "Playwright evaluate accepts same JSHandle multiple times",
    )

    # Reduced from Playwright page-event-console.spec.ts.
    async with page.expect_event("console", predicate=lambda msg: "hello" in msg.text, timeout=5_000) as console_info:
        await page.evaluate("() => console.log('hello', 5, { foo: 'bar' })")
    message = await console_info.value
    assert_equal(message.type, "log", "Playwright console event type")
    if "hello" not in message.text or "5" not in message.text:
        raise SmokeError(f"unexpected Playwright console event text: {message.text}")
    assert_equal(await message.args[0].json_value(), "hello", "Playwright console first argument")
    assert_equal(await message.args[1].json_value(), 5, "Playwright console numeric argument")
    assert_equal(await message.args[2].json_value(), {"foo": "bar"}, "Playwright console object argument")

    detached = await page.query_selector("#handle-target")
    await _navigate_until_dom_ready(state, f"{fixture}/plain")
    detached_error = ""
    try:
        await detached.text_content()
    except PlaywrightError as error:
        detached_error = str(error)
    if not detached_error:
        raise SmokeError("detached element handle should fail after navigation")

    child = await _goto_iframe_for_playwright_frame_model(page, f"{fixture}/iframe")
    if child is None:
        raise SmokeError("missing child frame for owner_frame smoke")
    child_body = await child.query_selector("body")
    if child_body is None:
        raise SmokeError("missing child body handle")
    child_owner = await child_body.owner_frame()
    assert_equal(child_owner, child, "child element owner_frame")
    child_text = await child_body.evaluate("node => node.textContent.trim()")
    assert_equal(child_text, "child body text", "child frame element evaluate")
    state.record("dom_handle_workflows")


def _child_frame(page) -> object | None:
    return next((frame for frame in page.frames if "/child" in frame.url), None)


async def _wait_for_child_frame(page) -> object:
    await wait_until(lambda: _child_frame(page) is not None, "iframe child frame", timeout_ms=5_000)
    child = _child_frame(page)
    if child is None:
        raise SmokeError("missing child frame")
    return child


async def _goto_iframe_for_playwright_frame_model(page, url: str) -> object:
    try:
        await page.goto(url, wait_until="domcontentloaded", timeout=10_000)
    except PlaywrightError as error:
        if "Timeout" not in str(error):
            raise
        await wait_until(lambda: page.url == url, f"iframe page URL {url}", timeout_ms=5_000)
    return await _wait_for_child_frame(page)


async def _navigate_until_dom_ready(state: SmokeState, url: str) -> None:
    result = await state.cdp.send("Page.navigate", {"url": url})
    if result.get("errorText"):
        raise SmokeError(f"Page.navigate failed for {url}: {result}")

    async def is_ready() -> bool:
        location = await state.cdp.send(
            "Runtime.evaluate",
            {"expression": "location.href", "returnByValue": True},
        )
        ready_state = await state.cdp.send(
            "Runtime.evaluate",
            {"expression": "document.readyState", "returnByValue": True},
        )
        return (
            location.get("result", {}).get("value") == url
            and ready_state.get("result", {}).get("value") in ["interactive", "complete"]
        )

    await wait_until(is_ready, f"CDP navigation DOM ready for {url}")


async def run_element_handle_state_workflows(state: SmokeState) -> None:
    # Reduced from Playwright elementhandle-wait-for-element-state.spec.ts.
    page = state.page

    await page.set_content(
        """
        <div id="visible-later" style="display:none">content</div>
        <div id="hide-later">hide me</div>
        <button id="enable-later" disabled><span>Target</span></button>
        <input id="editable-later" readonly>
        <script>
          setTimeout(() => { document.getElementById("visible-later").style.display = "block"; }, 50);
          setTimeout(() => { document.getElementById("hide-later").style.display = "none"; }, 75);
          setTimeout(() => { document.getElementById("enable-later").disabled = false; }, 100);
          setTimeout(() => { document.getElementById("editable-later").readOnly = false; }, 125);
        </script>
        """
    )
    visible_later = await page.query_selector("#visible-later")
    hide_later = await page.query_selector("#hide-later")
    enable_later = await page.query_selector("#enable-later span")
    editable_later = await page.query_selector("#editable-later")
    if visible_later is None or hide_later is None or enable_later is None or editable_later is None:
        raise SmokeError("missing element-handle state fixture")
    await visible_later.wait_for_element_state("visible", timeout=5_000)
    await hide_later.wait_for_element_state("hidden", timeout=5_000)
    await enable_later.wait_for_element_state("enabled", timeout=5_000)
    await editable_later.wait_for_element_state("editable", timeout=5_000)

    await page.set_content("<div id='timeout-state' style='display:none'>never visible</div>")
    timeout_handle = await page.query_selector("#timeout-state")
    if timeout_handle is None:
        raise SmokeError("missing element-handle timeout fixture")
    timeout_error = await _expect_playwright_error(timeout_handle.wait_for_element_state("visible", timeout=25))
    if "Timeout 25ms exceeded" not in timeout_error:
        raise SmokeError(f"unexpected element handle state timeout error: {timeout_error}")

    await page.set_content("<button id='detach-state' disabled>Target</button>")
    detach_handle = await page.query_selector("#detach-state")
    if detach_handle is None:
        raise SmokeError("missing element-handle detach fixture")
    detach_wait = detach_handle.wait_for_element_state("enabled", timeout=5_000)
    await detach_handle.evaluate("node => node.remove()")
    detach_error = await _expect_playwright_error(detach_wait)
    if "not attached" not in detach_error and "Element is not attached" not in detach_error:
        raise SmokeError(f"unexpected element handle detached state error: {detach_error}")

    await page.set_content("<div id='detach-hidden'>detach hidden</div>")
    hidden_handle = await page.query_selector("#detach-hidden")
    if hidden_handle is None:
        raise SmokeError("missing element-handle detached-hidden fixture")
    hidden_wait = hidden_handle.wait_for_element_state("hidden", timeout=5_000)
    await hidden_handle.evaluate("node => node.remove()")
    await hidden_wait
    state.record("playwright_element_handle_state_workflows")


async def run_touch_input_workflows(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp

    async def expect_invalid_params(method: str, params: dict[str, object]) -> None:
        error = await _expect_playwright_error(cdp.send(method, params))
        if "InvalidParams" not in error:
            raise SmokeError(f"{method}: expected InvalidParams, got: {error}")

    await page.evaluate(
        """html => {
          document.body.innerHTML = html;
          window.__coordinateEvents = [];
          const tapTarget = document.getElementById('tap-target');
          for (const type of ['touchstart', 'touchend', 'click', 'dragenter', 'dragover', 'drop']) {
            tapTarget.addEventListener(type, () => window.__coordinateEvents.push(type));
          }
        }""",
        """
        <div id="tap-target" draggable="true" style="position:absolute;left:0;top:0;width:120px;height:80px">tap</div>
        """,
    )

    for method in [
        "Input.dispatchMouseEvent",
        "Input.dispatchTouchEvent",
        "Input.emulateTouchFromMouseEvent",
        "Input.synthesizeTapGesture",
        "Input.dispatchDragEvent",
    ]:
        await expect_invalid_params(method, {})

    for method, params in [
        ("Input.dispatchMouseEvent", {"type": "MouseMoved", "x": 20, "y": 20}),
        (
            "Input.dispatchTouchEvent",
            {"type": "TouchStart", "touchPoints": [{"x": 20, "y": 20}]},
        ),
        (
            "Input.emulateTouchFromMouseEvent",
            {"type": "MouseMoved", "x": 20, "y": 20, "button": "Left"},
        ),
        (
            "Input.synthesizeTapGesture",
            {"x": 20, "y": 20, "gestureSourceType": "Touch"},
        ),
        (
            "Input.dispatchDragEvent",
            {
                "type": "DragEnter",
                "x": 20,
                "y": 20,
                "data": {"items": [], "files": [], "dragOperationsMask": 0},
            },
        ),
    ]:
        await expect_invalid_params(method, params)

    await expect_invalid_params(
        "Input.dispatchMouseEvent",
        {"type": "mouseWheel", "x": 20, "y": 20},
    )
    await expect_invalid_params(
        "Input.dispatchMouseEvent",
        {"type": "mouseMoved", "x": 20, "y": 20, "force": 2},
    )
    await expect_invalid_params(
        "Input.dispatchMouseEvent",
        {"type": "mouseMoved", "x": 20, "y": 20, "clickCount": -1, "modifiers": 256},
    )
    await expect_invalid_params(
        "Input.dispatchTouchEvent",
        {"type": "touchStart", "touchPoints": []},
    )
    await expect_invalid_params(
        "Input.dispatchTouchEvent",
        {"type": "touchCancel", "touchPoints": [{"id": 4, "x": 20, "y": 20}]},
    )
    await expect_invalid_params(
        "Input.dispatchTouchEvent",
        {
            "type": "touchStart",
            "touchPoints": [{"id": 4, "x": 20, "y": 20}, {"x": 30, "y": 20}],
        },
    )
    await expect_invalid_params(
        "Input.dispatchTouchEvent",
        {"type": "touchEnd", "touchPoints": [{"id": 4, "x": 20, "y": 20}]},
    )
    await expect_invalid_params(
        "Input.emulateTouchFromMouseEvent",
        {"type": "mouseReleased", "x": 20, "y": 20},
    )
    await expect_invalid_params(
        "Input.emulateTouchFromMouseEvent",
        {"type": "mouseReleased", "x": 20, "y": 20, "button": "middle"},
    )

    valid_commands: list[tuple[str, dict[str, object]]] = [
        (
            "Input.dispatchMouseEvent",
            {"type": "mouseWheel", "x": 20, "y": 20, "deltaX": 0, "deltaY": 13},
        ),
        (
            "Input.dispatchTouchEvent",
            {"type": "touchStart", "touchPoints": [{"x": 20, "y": 20}]},
        ),
        ("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []}),
        (
            "Input.emulateTouchFromMouseEvent",
            {"type": "mousePressed", "x": 20, "y": 20, "button": "left"},
        ),
        (
            "Input.emulateTouchFromMouseEvent",
            {"type": "mouseReleased", "x": 20, "y": 20, "button": "left"},
        ),
        ("Input.synthesizeTapGesture", {"x": 20, "y": 20}),
        (
            "Input.dispatchDragEvent",
            {
                "type": "dragEnter",
                "x": 20,
                "y": 20,
                "data": {
                    "items": [{"mimeType": "text/plain", "data": "drag-text"}],
                    "files": [],
                    "dragOperationsMask": 1,
                },
            },
        ),
    ]
    for method, params in valid_commands:
        await cdp.send(method, params)

    coordinate_events = await page.evaluate("() => window.__coordinateEvents")
    for event_type in ["touchstart", "touchend", "click", "dragenter"]:
        if event_type not in coordinate_events:
            raise SmokeError(
                f"coordinate input did not dispatch {event_type}: {coordinate_events!r}"
            )
    state.record("coordinate_input_layout_hit_test_workflows")
