"""Static fixture server that serves the upstream WPT tree on loopback +
optional global IPv6 (for engines like Obscura that reject loopback fixtures).

The server intercepts ``/resources/testharnessreport.js`` and replaces it with
a small bridge that captures testharness completion into ``window.__bench_wpt__``.
Most other paths are served directly from ``../wpt``. The static server also
implements a small subset of WPT fixture behavior needed by the benchmark:
``.sub.`` files get byte-level replacement for core host/port variables, and
``.headers`` sidecars plus ``pipe=header(Name,Value)``,
``pipe=status(NNN)`` and ``pipe=trickle(dN)`` are translated into simple static
response metadata and whole-response delays.
It also implements tiny explicitly-listed WPT Python handlers when their
behavior is static enough to model without a general wptserve runtime, and
carries a small set of legacy resource aliases for WPT checkouts where older
fixture paths have moved.

This is deliberately a v1 static server: it does NOT run wptserve Python
handlers generally, does NOT support .h2 endpoints, and does NOT proxy
arbitrary cross-origin tests. Cases requiring those features must be filtered
out at the case-set selection layer.
"""

from __future__ import annotations

import base64
import hashlib
import html
import json
import math
import mimetypes
import re
import socket
import subprocess
import sys
import threading
import time
import uuid
from html import escape as html_escape
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, unquote, urlparse, urlsplit, urlunsplit

from .any_js import (
    ANY_JS_DEDICATED_WORKER_GLOBAL,
    ANY_JS_WINDOW_GLOBAL,
    any_js_source_script_path,
    any_js_worker_script_path,
    any_js_wrapper_global,
    is_any_js_worker_script_path,
    query_without_any_js_wrapper,
    query_without_script_js_wrapper,
    script_js_wrapper_global,
    SCRIPT_JS_DEDICATED_WORKER_GLOBAL,
    SCRIPT_JS_WINDOW_GLOBAL,
)

from .case_set import (
    ANY_JS_WINDOW_QUERY_NAME,
    ANY_JS_WINDOW_QUERY_VALUE,
    WINDOW_JS_WINDOW_QUERY_NAME,
    WINDOW_JS_WINDOW_QUERY_VALUE,
    parse_any_js_meta,
)


DEFAULT_TESTHARNESS_TIMEOUT_SECONDS = 10.0
MAX_REQUEST_BODY_BYTES = 16 * 1024 * 1024
MAX_REQUEST_BODY_LINE_BYTES = 64 * 1024
BENCH_TIMEOUT_MULTIPLIER_QUERY = "__moli_bench_timeout_multiplier"
FORM_ECHO_PATH = "/html/semantics/forms/form-submission-0/form-echo.py"
BENCH_REPORT_BRIDGE_SRC_RE = re.compile(
    rb"(?P<prefix>\bsrc\s*=\s*)(?P<quote>['\"])"
    rb"/resources/testharnessreport\.js(?P=quote)",
    re.IGNORECASE,
)


BENCH_REPORT_BRIDGE_TEMPLATE = b"""\
/* Moli benchmark testharnessreport.js bridge.
 *
 * Captures completion into window.__bench_wpt__ via three independent paths so
 * an engine-specific bug in one path does not cost us the whole result:
 *   A. add_completion_callback -> full snapshot (preferred, matches WPT report).
 *   B. add_result_callback     -> incremental per-test accumulator (fallback if
 *                                 notify_complete in testharness.js never fires
 *                                 because of an engine bug in its post-result
 *                                 cleanup path).
 *   C. wrapped done()          -> publish current accumulator synchronously the
 *                                 moment the test page calls done(); marks the
 *                                 result with source="done-hook" so the runner
 *                                 can distinguish a "real" harness completion
 *                                 from an opportunistic snapshot.
 *
 * The runner picks the highest-fidelity payload available (A > B > C).
 * The bridge disables testharness's in-page report UI because many WPT cases
 * deliberately replace document.body during the test. Keeping the UI enabled
 * can leave testharness with a detached #log node and make completion throw
 * before callbacks are published.
 */
(function() {
  var initialCasePath = (typeof location !== 'undefined' && location.pathname) ? (location.pathname + location.search) : null;
  var accumulator = {
    harness: { status: null, message: null },
    tests: [],
    source: null,
  };
  var byName = Object.create(null);

  function recordTest(t) {
    if (!t || typeof t !== 'object') return;
    var name = typeof t.name === 'string' ? t.name : null;
    var entry = {
      name: name,
      status: typeof t.status === 'number' ? t.status : null,
      message: t.message ? String(t.message) : null,
    };
    if (name !== null && byName[name] !== undefined) {
      accumulator.tests[byName[name]] = entry;
    } else {
      if (name !== null) byName[name] = accumulator.tests.length;
      accumulator.tests.push(entry);
    }
  }

  function publish(source, harness_status) {
    accumulator.source = source;
    if (harness_status && typeof harness_status === 'object') {
      if (typeof harness_status.status === 'number') {
        accumulator.harness.status = harness_status.status;
      }
      if (harness_status.message) {
        accumulator.harness.message = String(harness_status.message);
      }
    }
    if (source === 'incremental') {
      window.__bench_wpt__ = {
        case_path: initialCasePath,
        harness: {
          status: accumulator.harness.status,
          message: accumulator.harness.message,
        },
        tests: [],
        partial_count: accumulator.tests.length,
        source: source,
      };
      return;
    }
    var snapshot = {
      case_path: initialCasePath,
      harness: { status: accumulator.harness.status, message: accumulator.harness.message },
      tests: accumulator.tests.slice(),
      source: source,
    };
    var body = null;
    try {
      body = JSON.stringify(snapshot);
      try {
        var node = document.getElementById('__bench_wpt_payload');
        if (!node) {
          node = document.createElement('pre');
          node.id = '__bench_wpt_payload';
          node.hidden = true;
          (document.documentElement || document).appendChild(node);
        }
        node.textContent = body;
      } catch (domErr) {
        try { window.__bench_wpt_dom_err__ = String(domErr); } catch (domStoreErr) {}
      }
      // Large synchronous uploads can strand the renderer event loop when
      // several engine processes finish together. Their complete snapshot is
      // already in the hidden DOM payload for the CLI stdout fallback.
      if (body.length <= 60000) {
        try {
          var xhr = new XMLHttpRequest();
          xhr.open('POST', '/__bench__/result', false);
          xhr.setRequestHeader('Content-Type', 'application/json');
          xhr.send(body);
        } catch (e0) {}
      }
    } catch (e) {
      try { window.__bench_wpt_publish_err__ = String(e); } catch (e2) {}
    }
    window.__bench_wpt__ = snapshot;
  }

  function fullSnapshot(tests, harness_status) {
    accumulator.tests = [];
    byName = Object.create(null);
    if (Array.isArray(tests)) {
      for (var i = 0; i < tests.length; i++) recordTest(tests[i]);
    }
    publish('completion-callback', harness_status);
  }

  function canPublishDoneFallback() {
    return accumulator.tests.length > 0 ||
      typeof accumulator.harness.status === 'number';
  }

  var trace = [];
  window.__bench_wpt_trace__ = trace;
  trace.push({ts: 0, cc: typeof add_completion_callback, rc: typeof add_result_callback, dn: typeof done});

  function install() {
    if (typeof add_completion_callback !== 'function' ||
        typeof add_result_callback !== 'function' ||
        typeof done !== 'function') {
      trace.push({ts: Date.now(), wait: true, cc: typeof add_completion_callback});
      setTimeout(install, 5);
      return;
    }
    trace.push({ts: Date.now(), installing: true});
    try {
      if (typeof setup === 'function') setup({
        output: false,
        timeout_multiplier: __BENCH_TIMEOUT_MULTIPLIER__,
      });
    } catch (e) { trace.push({setupErr: String(e)}); }
    try { add_completion_callback(fullSnapshot); } catch (e) { trace.push({ccErr: String(e)}); }
    try { add_result_callback(function(t) { recordTest(t); publish('incremental', null); }); } catch (e) { trace.push({rcErr: String(e)}); }
    try {
      var origDone = window.done;
      window.done = function() {
        var ret;
        try { ret = origDone.apply(this, arguments); } catch (e) {
          accumulator.harness.status = -1;
          accumulator.harness.message = 'done() threw: ' + (e && e.message ? e.message : e);
        }
        if (window.__bench_wpt__ === undefined && canPublishDoneFallback()) {
          publish('done-hook', null);
        }
        setTimeout(function() {
          if ((!window.__bench_wpt__ || window.__bench_wpt__.source === 'done-hook') &&
              canPublishDoneFallback()) {
            publish('done-hook-late', null);
          }
        }, 50);
        return ret;
      };
    } catch (e) {}
  }
  install();
})();
"""


def _valid_timeout_multiplier(value: float | int | str | None) -> float:
    try:
        multiplier = float(value) if value is not None else 1.0
    except (TypeError, ValueError):
        return 1.0
    if not math.isfinite(multiplier) or multiplier <= 0:
        return 1.0
    return multiplier


def _js_number(value: float) -> bytes:
    multiplier = _valid_timeout_multiplier(value)
    return format(multiplier, ".12g").encode("ascii")


def _bench_report_bridge(timeout_multiplier: float = 1.0) -> bytes:
    return BENCH_REPORT_BRIDGE_TEMPLATE.replace(
        b"__BENCH_TIMEOUT_MULTIPLIER__",
        _js_number(timeout_multiplier),
    )


BENCH_REPORT_BRIDGE = _bench_report_bridge()


def _bridge_timeout_multiplier_from_query(query: str) -> float:
    for key, value in parse_qsl(query, keep_blank_values=True):
        if key == BENCH_TIMEOUT_MULTIPLIER_QUERY:
            return _valid_timeout_multiplier(value)
    return 1.0


def _normalize_harness_case_key(case_path: str) -> str:
    parsed = urlsplit(case_path)
    path = parsed.path.lstrip("/")
    if parsed.query:
        return f"{path}?{parsed.query}"
    return path


def _case_key_from_request(path: str, query: str) -> str:
    key = path.lstrip("/")
    if query:
        return f"{key}?{query}"
    return key


def _report_bridge_url(timeout_multiplier: float) -> bytes:
    return (
        b"/resources/testharnessreport.js?"
        + BENCH_TIMEOUT_MULTIPLIER_QUERY.encode("ascii")
        + b"="
        + _js_number(timeout_multiplier)
    )


def _inject_bench_report_bridge_config(body: bytes, timeout_multiplier: float) -> bytes:
    if _valid_timeout_multiplier(timeout_multiplier) == 1.0:
        return body
    report_url = _report_bridge_url(timeout_multiplier)

    def replace(match: re.Match[bytes]) -> bytes:
        return (
            match.group("prefix")
            + match.group("quote")
            + report_url
            + match.group("quote")
        )

    return BENCH_REPORT_BRIDGE_SRC_RE.sub(replace, body)


BENCH_TESTDRIVER_VENDOR_BRIDGE = b"""\
/* Moli benchmark minimal testdriver-vendor.js bridge.
 *
 * This is intentionally narrow: it implements enough pointer/key actions for
 * static testharness pages to exercise engine behaviour without a WebDriver
 * backend. Unsupported automation APIs keep testdriver.js's default failures.
 */
(function() {
  var rectTargets = Object.create(null);
  function recordRectTarget(element, rect) {
    var x = Number.isFinite(rect.x) ? rect.x : rect.left || 0;
    var y = Number.isFinite(rect.y) ? rect.y : rect.top || 0;
    var centerX = Math.round(x + rect.width / 2);
    var centerY = Math.round(y + rect.height / 2);
    var key = centerX + ',' + centerY;
    rectTargets[key] = { element: element, x: centerX, y: centerY };
  }
  if (typeof Element !== 'undefined' && Element.prototype && Element.prototype.getBoundingClientRect) {
    var nativeGetBoundingClientRect = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function() {
      var rect = nativeGetBoundingClientRect.apply(this, arguments);
      var x = Number.isFinite(rect.x) ? rect.x : rect.left || 0;
      var y = Number.isFinite(rect.y) ? rect.y : rect.top || 0;
      recordRectTarget(this, rect);
      return {
        x: x,
        y: y,
        left: Number.isFinite(rect.left) ? rect.left : x,
        top: Number.isFinite(rect.top) ? rect.top : y,
        right: Number.isFinite(rect.right) ? rect.right : x + (rect.width || 0),
        bottom: Number.isFinite(rect.bottom) ? rect.bottom : y + (rect.height || 0),
        width: rect.width || 0,
        height: rect.height || 0,
        toJSON: function() { return this; },
      };
    };
  }
  if (typeof Element !== 'undefined' && Element.prototype && Element.prototype.getClientRects) {
    var nativeGetClientRects = Element.prototype.getClientRects;
    Element.prototype.getClientRects = function() {
      var rects = nativeGetClientRects.apply(this, arguments);
      if (rects && rects.length) {
        recordRectTarget(this, rects[0]);
      }
      return rects;
    };
  }

  function eventInit(x, y, button) {
    return {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: x || 0,
      clientY: y || 0,
      button: button || 0,
      buttons: button === undefined ? 0 : 1,
    };
  }

  function dispatchPointerEvent(target, type, init) {
    var Ctor = typeof PointerEvent === 'function' ? PointerEvent : MouseEvent;
    try { target.dispatchEvent(new Ctor(type, init)); } catch (e) {}
  }

  function dispatchMouseEvent(target, type, init) {
    try { target.dispatchEvent(new MouseEvent(type, init)); } catch (e) {}
  }

  function focusForUserActivation(element) {
    try {
      var before = document.activeElement;
      if (element && typeof element.focus === 'function') element.focus();
      if (before && before !== document.body && document.activeElement === before &&
          typeof before.blur === 'function') {
        before.blur();
      }
    } catch (e) {}
  }

  function recordedTargetAt(x, y) {
    var key = Math.round(x || 0) + ',' + Math.round(y || 0);
    if (rectTargets[key]) {
      return rectTargets[key].element;
    }
    var best = null;
    var bestDistance = Infinity;
    for (var candidateKey in rectTargets) {
      var candidate = rectTargets[candidateKey];
      var dx = candidate.x - (x || 0);
      var dy = candidate.y - (y || 0);
      var distance = dx * dx + dy * dy;
      if (distance < bestDistance) {
        best = candidate.element;
        bestDistance = distance;
      }
    }
    if (best) {
      return best;
    }
    return null;
  }

  function targetAt(x, y) {
    var recorded = recordedTargetAt(x, y);
    if (recorded) {
      return recorded;
    }
    if (typeof document === 'undefined' || typeof document.elementFromPoint !== 'function') {
      return document && document.body;
    }
    return document.elementFromPoint(x || 0, y || 0) || document.body;
  }

  if (typeof Document !== 'undefined' && Document.prototype &&
      typeof Document.prototype.elementsFromPoint === 'function') {
    var nativeElementsFromPoint = Document.prototype.elementsFromPoint;
    Document.prototype.elementsFromPoint = function(x, y) {
      try {
        return nativeElementsFromPoint.apply(this, arguments);
      } catch (e) {
        var target = recordedTargetAt(x, y);
        return target ? [target] : [];
      }
    };
  }

  function keyName(value) {
    if (value === '\\uE004') return 'Tab';
    if (value === '\\uE008') return 'Shift';
    if (value === '\\uE009') return 'Control';
    if (value === '\\uE00A') return 'Alt';
    if (value === '\\uE00C') return 'Escape';
    if (value === '\\uE010') return 'End';
    if (value === '\\uE011') return 'Home';
    if (value === '\\uE012') return 'ArrowLeft';
    if (value === '\\uE013') return 'ArrowUp';
    if (value === '\\uE014') return 'ArrowRight';
    if (value === '\\uE015') return 'ArrowDown';
    return value;
  }

  function dispatchKey(target, type, key, modifiers) {
    return target.dispatchEvent(new KeyboardEvent(type, {
      key: key,
      altKey: !!modifiers.Alt,
      ctrlKey: !!modifiers.Control,
      shiftKey: !!modifiers.Shift,
      bubbles: true,
      cancelable: true,
      composed: true,
    }));
  }

  function insertSendKeyText(target, key, modifiers) {
    if (!target || key.length !== 1 || modifiers.Alt || modifiers.Control) {
      return;
    }
    var codePoint = key.codePointAt(0);
    if (codePoint >= 0xE000 && codePoint <= 0xF8FF) {
      return;
    }
    var localName = String(target.localName || '').toLowerCase();
    var type = localName === 'input' ? String(target.type || '').toLowerCase() : '';
    var isTextControl =
      localName === 'textarea' ||
      (localName === 'input' &&
       ['text', 'search', 'tel', 'url', 'email', 'password'].includes(type));
    if (!isTextControl && !target.isContentEditable) {
      return;
    }
    var doc = target.ownerDocument || document;
    try {
      doc.execCommand('insertText', false, key);
    } catch (e) {}
  }

  async function action_sequence(actions, context) {
    var pointer = { x: 0, y: 0, target: null, button: 0 };
    var modifiers = { Alt: false, Control: false, Shift: false };
    for (var i = 0; i < actions.length; i++) {
      var source = actions[i];
      var sourceActions = Array.isArray(source.actions) ? source.actions : [];
      for (var j = 0; j < sourceActions.length; j++) {
        var action = sourceActions[j];
        if (!action || action.type === 'pause') {
          continue;
        }
        if (source.type === 'pointer') {
          if (action.type === 'pointerMove') {
            pointer.x = Number(action.x) || 0;
            pointer.y = Number(action.y) || 0;
            var nextTarget = targetAt(pointer.x, pointer.y);
            var init = eventInit(pointer.x, pointer.y, pointer.button);
            if (pointer.target && pointer.target !== nextTarget) {
              dispatchPointerEvent(pointer.target, 'pointerout', init);
              dispatchMouseEvent(pointer.target, 'mouseout', init);
            }
            pointer.target = nextTarget;
            dispatchPointerEvent(nextTarget, 'pointerover', init);
            dispatchMouseEvent(nextTarget, 'mouseover', init);
            dispatchPointerEvent(nextTarget, 'pointermove', init);
            dispatchMouseEvent(nextTarget, 'mousemove', init);
          } else if (action.type === 'pointerDown') {
            pointer.button = Number(action.button) || 0;
            var downTarget = pointer.target || targetAt(pointer.x, pointer.y);
            var downInit = eventInit(pointer.x, pointer.y, pointer.button);
            focusForUserActivation(downTarget);
            dispatchPointerEvent(downTarget, 'pointerdown', downInit);
            dispatchMouseEvent(downTarget, 'mousedown', downInit);
          } else if (action.type === 'pointerUp') {
            var upTarget = pointer.target || targetAt(pointer.x, pointer.y);
            var upInit = eventInit(pointer.x, pointer.y, pointer.button);
            dispatchPointerEvent(upTarget, 'pointerup', upInit);
            dispatchMouseEvent(upTarget, 'mouseup', upInit);
            dispatchMouseEvent(upTarget, 'click', upInit);
            pointer.button = 0;
          }
        } else if (source.type === 'key') {
          var key = keyName(action.value);
          if (Object.prototype.hasOwnProperty.call(modifiers, key)) {
            modifiers[key] = action.type === 'keyDown';
          }
          dispatchKey(
            document.activeElement || document.body,
            action.type === 'keyDown' ? 'keydown' : 'keyup',
            key,
            modifiers
          );
        }
      }
    }
  }

  async function sendKeys(element, keys) {
    var wasFocused = document.activeElement === element;
    if (
      element &&
      typeof element.focus === 'function' &&
      String(element.localName || '').toLowerCase() !== 'body'
    ) {
      element.focus();
    }
    if (!wasFocused && element && typeof element.setSelectionRange === 'function') {
      try {
        var end = String(element.value || '').length;
        element.setSelectionRange(end, end);
      } catch (e) {}
    }
    var modifiers = { Alt: false, Control: false, Shift: false };
    for (var keyValue of String(keys || '')) {
      var key = keyName(keyValue);
      var target = document.activeElement || element || document.body;
      if (Object.prototype.hasOwnProperty.call(modifiers, key)) {
        modifiers[key] = true;
        dispatchKey(target, 'keydown', key, modifiers);
        continue;
      }
      var keyAllowed = dispatchKey(target, 'keydown', key, modifiers);
      if (keyAllowed) {
        insertSendKeyText(document.activeElement || target, key, modifiers);
      }
      dispatchKey(document.activeElement || target, 'keyup', key, modifiers);
    }
    for (var modifier in modifiers) {
      if (modifiers[modifier]) {
        modifiers[modifier] = false;
        dispatchKey(document.activeElement || element || document.body, 'keyup', modifier, modifiers);
      }
    }
  }

  function normalizedLabelText(value) {
    return String(value || '').replace(/[\\t\\n\\f\\r ]+/g, ' ').trim();
  }

  function isElement(value) {
    return value && value.nodeType === 1;
  }

  function isLabelableElement(element) {
    if (!isElement(element)) return false;
    var localName = String(element.localName || '').toLowerCase();
    if (
      localName === 'button' ||
      localName === 'meter' ||
      localName === 'output' ||
      localName === 'progress' ||
      localName === 'select' ||
      localName === 'textarea'
    ) {
      return true;
    }
    return localName === 'input' && String(element.type || '').toLowerCase() !== 'hidden';
  }

  function resolveReferenceTarget(element) {
    if (!isElement(element) || !element.shadowRoot) {
      return element;
    }
    var referenceTarget = element.shadowRoot.referenceTarget;
    if (referenceTarget === '') {
      return null;
    }
    if (referenceTarget == null) {
      return element;
    }
    var target = element.shadowRoot.getElementById(String(referenceTarget));
    if (!target) {
      return null;
    }
    return resolveReferenceTarget(target);
  }

  function elementByIdFromRoot(root, id) {
    if (!root || !id) {
      return null;
    }
    if (typeof root.getElementById === 'function') {
      return root.getElementById(id);
    }
    return null;
  }

  function elementTextAlternative(element) {
    if (!isElement(element)) {
      return '';
    }
    if (element.hasAttribute('aria-label')) {
      return normalizedLabelText(element.getAttribute('aria-label'));
    }
    return normalizedLabelText(element.textContent);
  }

  function collectShadowIncludingLabels(root, labels) {
    if (!root) {
      return labels;
    }
    var child = root.firstChild;
    while (child) {
      if (isElement(child)) {
        if (String(child.localName || '').toLowerCase() === 'label') {
          labels.push(child);
        }
        if (child.shadowRoot) {
          collectShadowIncludingLabels(child.shadowRoot, labels);
        }
      }
      collectShadowIncludingLabels(child, labels);
      child = child.nextSibling;
    }
    return labels;
  }

  function firstImplicitLabelControl(label) {
    var found = null;
    function visit(node) {
      if (found || !node) {
        return;
      }
      if (isElement(node) && node !== label) {
        var resolved = resolveReferenceTarget(node);
        if (isLabelableElement(resolved)) {
          found = resolved;
          return;
        }
        if (node.shadowRoot) {
          return;
        }
      }
      collect(node);
    }
    function collect(root) {
      var child = root.firstChild;
      while (child && !found) {
        visit(child);
        child = child.nextSibling;
      }
    }
    collect(label);
    return found;
  }

  function labelsForElement(element) {
    var labels = collectShadowIncludingLabels(document, []);
    var matches = [];
    for (var i = 0; i < labels.length; i++) {
      var label = labels[i];
      var labelFor = label.getAttribute('for');
      if (labelFor) {
        var root = label.getRootNode ? label.getRootNode() : document;
        var explicitTarget = elementByIdFromRoot(root, labelFor);
        if (resolveReferenceTarget(explicitTarget) === element) {
          matches.push(label);
        }
        continue;
      }
      if (firstImplicitLabelControl(label) === element) {
        matches.push(label);
      }
    }
    return matches;
  }

  async function getComputedLabel(element) {
    if (!isElement(element)) {
      return '';
    }
    if (element.hasAttribute('data-expectedlabel')) {
      return normalizedLabelText(element.getAttribute('data-expectedlabel'));
    }
    if (element.hasAttribute('aria-label')) {
      return normalizedLabelText(element.getAttribute('aria-label'));
    }
    var labelledByElements = element.ariaLabelledByElements;
    if (labelledByElements && labelledByElements.length) {
      var partsFromElements = [];
      for (var i = 0; i < labelledByElements.length; i++) {
        var reflected = resolveReferenceTarget(labelledByElements[i]);
        if (reflected) {
          partsFromElements.push(elementTextAlternative(reflected));
        }
      }
      return normalizedLabelText(partsFromElements.join(' '));
    }
    var labelledBy = element.getAttribute('aria-labelledby');
    if (labelledBy) {
      var ids = normalizedLabelText(labelledBy).split(' ');
      var root = element.getRootNode ? element.getRootNode() : document;
      var parts = [];
      for (var j = 0; j < ids.length; j++) {
        var candidate = elementByIdFromRoot(root, ids[j]) || document.getElementById(ids[j]);
        var target = resolveReferenceTarget(candidate);
        if (target) {
          parts.push(elementTextAlternative(target));
        }
      }
      return normalizedLabelText(parts.join(' '));
    }
    var labelParts = labelsForElement(element).map(function(label) {
      return normalizedLabelText(label.textContent);
    });
    return normalizedLabelText(labelParts.join(' '));
  }

  if (!window.test_driver_internal) {
    window.test_driver_internal = {};
  }
  window.test_driver_internal.in_automation = true;
  window.test_driver_internal.action_sequence = action_sequence;
  window.test_driver_internal.send_keys = sendKeys;
  window.test_driver_internal.get_computed_label = getComputedLabel;
  window.test_driver_internal.set_permission = async function(params) {
    if (params && params.descriptor && params.descriptor.name === 'storage-access') {
      return;
    }
    throw new Error("set_permission() is not implemented by the Moli WPT bridge");
  };
  window.test_driver_internal.click = async function(element) {
    var rect = element.getBoundingClientRect();
    var x = Math.round(rect.x + rect.width / 2);
    var y = Math.round(rect.y + rect.height / 2);
    var init = eventInit(x, y, 0);
    focusForUserActivation(element);
    dispatchPointerEvent(element, 'pointerdown', init);
    dispatchMouseEvent(element, 'mousedown', init);
    dispatchPointerEvent(element, 'pointerup', init);
    dispatchMouseEvent(element, 'mouseup', init);
    dispatchMouseEvent(element, 'click', init);
  };
})();
"""

_TRICKLE_PIPE_RE = re.compile(r"(?:^|[|,])trickle\(d([0-9]+(?:\.[0-9]+)?)\)(?:$|[|,])")
_HEADER_PIPE_RE = re.compile(r"^header\(([^,()]+),([^()]*)\)$")
_STATUS_PIPE_RE = re.compile(r"^status\(([0-9]{3})\)$")
_GET_TEMPLATE_RE = re.compile(rb"\{\{GET\[([^\]\r\n]+)\]\}\}")
_UUID_TEMPLATE_RE = re.compile(rb"\{\{\$([A-Za-z_][A-Za-z0-9_]*):uuid\(\)\}\}")
_ID_TEMPLATE_RE = re.compile(rb"\{\{\$([A-Za-z_][A-Za-z0-9_]*)\}\}")
_HTTP_TOKEN_RE = re.compile(r"^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$")
_MAX_TRICKLE_DELAY_SECONDS = 10.0
_LEGACY_WPT_RESOURCE_ALIASES = {
    "/resources/WebIDLParser.js": "resources/webidl2/lib/webidl2.js",
}
_EMPTY_WASM_MODULE = b"\0asm\1\0\0\0"


def _pipe_requests_template_substitution(query: str) -> bool:
    for name, value in parse_qsl(query, keep_blank_values=True):
        if name != "pipe":
            continue
        for command in value.split("|"):
            if command.strip() == "sub":
                return True
    return False


def _needs_wpt_template_substitution(file_name: str, body: bytes, query: str = "") -> bool:
    return (
        ".sub." in file_name
        or _pipe_requests_template_substitution(query)
        or _GET_TEMPLATE_RE.search(body) is not None
    )


def _is_any_js_window_wrapper_request(query: str) -> bool:
    return any(
        name == ANY_JS_WINDOW_QUERY_NAME and value == ANY_JS_WINDOW_QUERY_VALUE
        for name, value in parse_qsl(query, keep_blank_values=True)
    )


def _is_window_js_window_wrapper_request(query: str) -> bool:
    return any(
        name == WINDOW_JS_WINDOW_QUERY_NAME and value == WINDOW_JS_WINDOW_QUERY_VALUE
        for name, value in parse_qsl(query, keep_blank_values=True)
    )


def _js_window_wrapper(source_path: str, source: bytes, *, any_js_global: bool) -> bytes:
    """Build a static HTML wrapper for a generated WPT window JavaScript run.

    WPT normally generates ``.any.html`` / ``.window.html`` variants outside
    the source tree. The cross runner serves upstream checkouts directly, so it
    synthesizes the minimal window wrapper on demand while preserving
    ``location.search`` for ``// META: variant`` subset helpers.
    """

    source_text = source.decode("utf-8", errors="ignore")
    meta = parse_any_js_meta(source_text)
    main_script = source_path.rsplit("/", 1)[-1]
    if main_script.endswith(".any.html"):
        main_script = main_script.removesuffix(".any.html") + ".any.js"
    elif main_script.endswith(".window.html"):
        main_script = main_script.removesuffix(".window.html") + ".window.js"
    lines = [
        "<!doctype html>",
        '<meta charset="utf-8">',
        '<script src="/resources/testharness.js"></script>',
        '<script src="/resources/testharnessreport.js"></script>',
    ]
    if any_js_global:
        lines.insert(2, "<script>")
        lines.insert(3, "self.GLOBAL = {")
        lines.insert(4, "  isWindow: function() { return true; },")
        lines.insert(5, "  isWorker: function() { return false; },")
        lines.insert(6, "  isShadowRealm: function() { return false; },")
        lines.insert(7, "};")
        lines.insert(8, "</script>")
    for title in reversed(meta.titles):
        lines.insert(2, f"<title>{html.escape(title, quote=False)}</title>")
    if meta.timeout_multiplier > 1.0:
        lines.insert(2, '<meta name="timeout" content="long">')
    for src in meta.scripts:
        lines.append(f'<script src="{html.escape(src, quote=True)}"></script>')
    lines.append("<div id=\"log\"></div>")
    lines.append(f'<script src="{html.escape(main_script, quote=True)}"></script>')
    lines.append("")
    return "\n".join(lines).encode("utf-8")


def _any_js_window_wrapper(source_path: str, source: bytes) -> bytes:
    """Build a static HTML wrapper for a WPT ``.any.js`` window run."""

    return _js_window_wrapper(source_path, source, any_js_global=True)


def _window_js_window_wrapper(source_path: str, source: bytes) -> bytes:
    """Build a static HTML wrapper for a WPT ``.window.js`` run."""

    return _js_window_wrapper(source_path, source, any_js_global=False)


def _legacy_wpt_resource_alias(path: str) -> str | None:
    """Return the WPT-root-relative replacement for a legacy resource path."""

    return _LEGACY_WPT_RESOURCE_ALIASES.get(path)


def _pipe_trickle_delay_seconds(query: str) -> float:
    """Return the minimal static-server delay for WPT ``pipe=trickle(dN)``.

    The full wptserve pipe stack streams bytes over time. The cross-engine
    fixture server is intentionally static, but execution-timing tests only
    need the observable fetch completion ordering that ``trickle(dN)`` creates.
    Delaying the whole response preserves that ordering without implementing
    the complete wptserve pipeline.
    """

    delay = 0.0
    for name, value in parse_qsl(query, keep_blank_values=True):
        if name != "pipe":
            continue
        for match in _TRICKLE_PIPE_RE.finditer(value):
            delay = max(delay, float(match.group(1)))
    return min(delay, _MAX_TRICKLE_DELAY_SECONDS)


def _pipe_response_header_operations(query: str) -> list[tuple[str, str, bool]]:
    """Parse WPT ``pipe=header(Name,Value[,Append])`` operations."""

    operations: list[tuple[str, str, bool]] = []
    for name, value in parse_qsl(query, keep_blank_values=True):
        if name != "pipe":
            continue
        for command in value.split("|"):
            match = _HEADER_PIPE_RE.match(command.strip())
            if match is None:
                continue
            header_name = match.group(1).strip()
            raw_header_value = match.group(2)
            header_value_without_append, separator, raw_append = (
                raw_header_value.rpartition(",")
            )
            normalized_append = raw_append.strip().lower()
            has_append_argument = separator != "" and normalized_append in {
                "true",
                "false",
                "1",
                "0",
            }
            append = has_append_argument and normalized_append in {"true", "1"}
            if has_append_argument:
                raw_header_value = header_value_without_append
            # wptserve writes pipe values byte-for-byte, including encoded
            # CR/LF used by parser tests. Python's static HTTP server cannot
            # safely do that, so preserve their whitespace semantics without
            # allowing a query string to inject another response header.
            header_value = (
                raw_header_value.strip().replace("\r", " ").replace("\n", " ")
            )
            if _valid_static_response_header(header_name, header_value):
                operations.append((header_name, header_value, append))
    return operations


def _apply_header_operations(
    headers: list[tuple[str, str]],
    operations: list[tuple[str, str, bool]],
) -> list[tuple[str, str]]:
    for header_name, header_value, append in operations:
        if not append:
            normalized_name = header_name.lower()
            headers = [
                (name, value)
                for name, value in headers
                if name.lower() != normalized_name
            ]
        headers.append((header_name, header_value))
    return headers


def _pipe_response_status(query: str) -> int | None:
    """Return a valid WPT ``pipe=status(NNN)`` response status, if present."""

    status: int | None = None
    for name, value in parse_qsl(query, keep_blank_values=True):
        if name != "pipe":
            continue
        for command in value.split("|"):
            match = _STATUS_PIPE_RE.match(command.strip())
            if match is None:
                continue
            code = int(match.group(1))
            if 100 <= code <= 599:
                status = code
    return status


def _valid_static_response_header(name: str, value: str) -> bool:
    return bool(_HTTP_TOKEN_RE.match(name)) and "\r" not in value and "\n" not in value


def _sidecar_response_headers(file_path: Path) -> list[tuple[str, str]]:
    """Return immediate-directory and file-specific WPT sidecar headers."""

    sidecars: list[Path] = []
    for base_path in (file_path.parent / "__dir__", file_path):
        candidates = [
            base_path.with_name(base_path.name + ".sub.headers"),
            base_path.with_name(base_path.name + ".headers"),
        ]
        sidecar = next(
            (
                candidate
                for candidate in candidates
                if candidate.exists() and candidate.is_file()
            ),
            None,
        )
        if sidecar is not None:
            sidecars.append(sidecar)

    headers: list[tuple[str, str]] = []
    for sidecar in sidecars:
        try:
            lines = sidecar.read_bytes().decode("latin-1").splitlines()
        except OSError:
            continue
        for line in lines:
            stripped = line.strip()
            if (
                not stripped
                or stripped.startswith("#")
                or stripped.upper().startswith("HTTP/")
            ):
                continue
            name, separator, value = stripped.partition(":")
            if not separator:
                continue
            header_name = name.strip()
            header_value = value.strip()
            if _valid_static_response_header(header_name, header_value):
                headers.append((header_name, header_value))
    return headers


def _response_content_type_and_extra_headers(
    content_type: str,
    extra_headers: list[tuple[str, str]] | None,
) -> tuple[str, list[tuple[str, str]]]:
    """Merge static MIME guessing with WPT sidecar/pipe response headers."""

    merged_content_type = content_type
    merged_extra_headers: list[tuple[str, str]] = []
    for name, value in extra_headers or []:
        if name.lower() == "content-type":
            merged_content_type = value
        else:
            merged_extra_headers.append((name, value))
    return merged_content_type, merged_extra_headers


def _wasm_webapi_status_code(query: str) -> int | None:
    """Return the status for WPT's wasm/webapi/status.py fixture.

    The upstream handler accepts arbitrary integer status values. Python's
    stdlib HTTP server cannot reliably emit invalid HTTP status codes such as
    0 or 700, but the wasm tests only observe that streaming compilation sees
    a non-ok response and rejects. Clamp out-of-range values to 599 so the
    fixture remains valid HTTP while preserving that observable behavior.
    """

    values = [value for name, value in parse_qsl(query, keep_blank_values=True) if name == "status"]
    if not values:
        return 200
    try:
        requested = int(values[-1])
    except ValueError:
        return 400
    if 100 <= requested <= 599:
        return requested
    return 599


def _wpt_delay_seconds(query: str) -> float | None:
    """Return the delay requested by WPT fixtures with an ``ms`` parameter."""

    values = [value for name, value in parse_qsl(query, keep_blank_values=True) if name == "ms"]
    try:
        delay_ms = float(values[0]) if values else 500.0
    except ValueError:
        return None
    if not math.isfinite(delay_ms) or delay_ms < 0:
        return None
    return delay_ms / 1_000.0


def _workers_url_encoding_response(query: str) -> bytes:
    """Model workers/semantics/encodings/003-1.py's UTF-8 query check."""
    value = next(
        (
            value
            for name, value in parse_qsl(query, keep_blank_values=True)
            if name == "x"
        ),
        None,
    )
    return b"PASS" if value == "å" else b"FAIL"


def _form_echo_response(body: bytes) -> bytes:
    return b" ".join(f"{byte:02x}".encode("ascii") for byte in body)


def _redirect_fixture_response(query: str) -> tuple[int, str] | None:
    """Return the shared redirect response used by static WPT fixture handlers."""

    params = dict(parse_qsl(query, keep_blank_values=True))
    try:
        status = int(params.get("redirect_status", params.get("status", "302")))
    except ValueError:
        return None
    if not 300 <= status <= 399:
        return None
    location = params.get("location", "")
    if not location or "\r" in location or "\n" in location:
        return None
    return status, location


def _content_security_policy_resource_response() -> tuple[bytes, list[tuple[str, str]]]:
    """Return the minimal CSP resource.py fixture used by worker CSP WPT."""

    return (
        b'{ "result": "success" }',
        [("Access-Control-Allow-Origin", "*")],
    )


def _workers_modules_export_on_load_script_response() -> tuple[bytes, list[tuple[str, str]]]:
    """Return WPT's export-on-load-script.py module response."""

    return (
        b"export const importedModules = ['export-on-load-script.js'];\n",
        [
            ("Content-Type", "text/javascript"),
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Headers", "Service-Worker"),
        ],
    )


def _inspect_headers_response_headers(
    query: str,
    request_headers: list[tuple[str, str]],
) -> list[tuple[str, str]]:
    """Model fetch/api/resources/inspect-headers.py's response headers."""

    params = parse_qsl(query, keep_blank_values=True)
    checked_headers_value = next(
        (value for name, value in params if name == "headers"),
        None,
    )
    checked_headers = (
        checked_headers_value.split("|") if checked_headers_value is not None else []
    )
    request_headers_by_name: dict[str, str] = {}
    for name, value in request_headers:
        request_headers_by_name.setdefault(name.lower(), value)

    response_headers = []
    for name in checked_headers:
        value = request_headers_by_name.get(name.lower())
        if value is not None:
            response_headers.append((f"x-request-{name}", value))

    if any(name == "cors" for name, _ in params):
        response_headers.extend(
            [
                (
                    "Access-Control-Allow-Origin",
                    request_headers_by_name.get("origin", "*"),
                ),
                ("Access-Control-Allow-Credentials", "true"),
                ("Access-Control-Allow-Methods", "GET, POST, HEAD"),
                (
                    "Access-Control-Expose-Headers",
                    ", ".join(f"x-request-{name}" for name in checked_headers),
                ),
            ]
        )
        allow_headers = next(
            (value for name, value in params if name == "allow_headers"),
            None,
        )
        response_headers.append(
            (
                "Access-Control-Allow-Headers",
                allow_headers
                if allow_headers is not None
                else ", ".join(name for name, _ in request_headers),
            )
        )

    return response_headers


def _nosniff_javascript_response(query: str) -> tuple[str | None, bytes]:
    """Model fetch/nosniff/resources/js.py's MIME-controlled script body."""

    params = parse_qsl(query, keep_blank_values=True)
    outcome = next(
        (value for name, value in params if name == "outcome"),
        "f",
    )
    content_type = next(
        (value for name, value in params if name == "type"),
        None,
    )
    type_label = content_type if content_type is not None else "Content-Type missing"
    result_call = "log('FAIL: " + type_label + "')" if outcome == "f" else "p()"
    body = ("// nothing to see here\n" + result_call).encode()
    return content_type, body


def _url_host_literal(hostname: str) -> str:
    if hostname.startswith("[") and hostname.endswith("]"):
        return hostname
    if ":" in hostname:
        return f"[{hostname}]"
    return hostname


def _asis_response_parts(body: bytes) -> tuple[int, bytes, list[tuple[str, str]]] | None:
    """Parse the small raw HTTP ``.asis`` fixtures used by legacy WPT cases."""

    separator = b"\r\n\r\n" if b"\r\n\r\n" in body else b"\n\n"
    if separator not in body:
        return None
    header_block, response_body = body.split(separator, 1)
    lines = header_block.replace(b"\r\n", b"\n").split(b"\n")
    if not lines:
        return None
    status_line = lines[0].decode("ascii", errors="replace").strip()
    status_parts = status_line.split(None, 2)
    if len(status_parts) < 2 or not status_parts[0].startswith("HTTP/"):
        return None
    try:
        status_code = int(status_parts[1])
    except ValueError:
        return None
    if not 100 <= status_code <= 599:
        return None
    headers: list[tuple[str, str]] = []
    for raw_line in lines[1:]:
        line = raw_line.decode("latin-1").strip()
        if not line:
            continue
        name, separator, value = line.partition(":")
        if not separator:
            continue
        header_name = name.strip()
        header_value = value.strip()
        if header_name.lower() in {"content-length", "transfer-encoding"}:
            continue
        if _valid_static_response_header(header_name, header_value):
            headers.append((header_name, header_value))
    return status_code, response_body, headers


def _static_response_headers(
    file_path: Path,
    query: str,
    *,
    port: int | None = None,
    alternate_port: int | None = None,
    remote_port: int | None = None,
    request_path: str = "/",
    request_hostname: str = "localhost",
    primary_hostname: str | None = None,
) -> list[tuple[str, str]]:
    headers = _apply_header_operations(
        _sidecar_response_headers(file_path),
        _pipe_response_header_operations(query),
    )
    if port is None:
        return headers
    template_ids: dict[bytes, bytes] = {}
    return [
        (
            name,
            _substitute_wpt_template_variables(
                value.encode("latin-1"),
                port=port,
                alternate_port=alternate_port,
                remote_port=remote_port,
                query=query,
                request_path=request_path,
                request_hostname=request_hostname,
                primary_hostname=primary_hostname,
                template_ids=template_ids,
            ).decode("latin-1"),
        )
        for name, value in headers
    ]


def _static_response_header_block(
    content_type: str,
    extra_headers: list[tuple[str, str]] | None,
) -> list[tuple[str, str]]:
    headers = list(extra_headers or [])
    if not any(name.lower() == "content-type" for name, _ in headers):
        headers.insert(0, ("Content-Type", content_type))
    return headers


def _headers_include(headers: list[tuple[str, str]], name: str) -> bool:
    return any(header_name.lower() == name.lower() for header_name, _ in headers)


def _float_query_param(params: dict[str, str], name: str, default: float) -> float:
    try:
        return float(params.get(name, default))
    except (TypeError, ValueError):
        return default


def _int_query_param(params: dict[str, str], name: str, default: int) -> int:
    try:
        return int(params.get(name, default))
    except (TypeError, ValueError):
        return default


def _substitute_wpt_template_variables(
    body: bytes,
    *,
    port: int,
    alternate_port: int | None = None,
    remote_port: int | None = None,
    query: str = "",
    request_path: str = "/",
    request_hostname: str = "localhost",
    primary_hostname: str | None = None,
    template_ids: dict[bytes, bytes] | None = None,
) -> bytes:
    if alternate_port is None:
        alternate_port = port
    if remote_port is None:
        remote_port = alternate_port
    if primary_hostname is None:
        primary_hostname = request_hostname
    request_path_bytes = request_path.encode("utf-8", errors="replace")
    request_hostname_bytes = request_hostname.encode("utf-8", errors="replace")
    primary_hostname_bytes = primary_hostname.encode("utf-8", errors="replace")
    primary_url_host_bytes = _url_host_literal(primary_hostname).encode(
        "utf-8", errors="replace"
    )
    request_url_host_bytes = _url_host_literal(request_hostname).encode(
        "utf-8", errors="replace"
    )
    request_host_bytes = request_hostname_bytes + b":" + str(port).encode("ascii")
    current_origin = (
        b"http://" + request_url_host_bytes + b":" + str(port).encode("ascii")
    )
    remote_authority = (
        request_hostname_bytes + b":" + str(remote_port).encode("ascii")
    )
    if ":" in request_hostname:
        for marker in (
            b"{{domains[www]}}:{{location[port]}}",
            b"{{domains[www1]}}:{{location[port]}}",
            b"{{domains[www2]}}:{{location[port]}}",
            b"{{domains[www]}}:{{ports[http][0]}}",
            b"{{domains[www1]}}:{{ports[http][0]}}",
            b"{{domains[www2]}}:{{ports[http][0]}}",
        ):
            body = body.replace(marker, remote_authority)
        alternate_authority = (
            request_hostname_bytes + b":" + str(alternate_port).encode("ascii")
        )
        for marker in (
            b"{{domains[www]}}:{{ports[http][1]}}",
            b"{{domains[www1]}}:{{ports[http][1]}}",
            b"{{domains[www2]}}:{{ports[http][1]}}",
        ):
            body = body.replace(marker, alternate_authority)
    alternate_port_literal = b"':" + str(alternate_port).encode("ascii") + b"'"
    remote_port_literal = b"':" + str(remote_port).encode("ascii") + b"'"
    replacements = {
        b"https://{{location[hostname]}}:{{ports[https][0]}}": (
            current_origin
        ),
        b"https://{{domains[www]}}:{{ports[https][0]}}": (
            current_origin
        ),
        b"https://{{domains[www1]}}:{{ports[https][0]}}": (
            b"http://" + request_url_host_bytes + b":" + str(remote_port).encode("ascii")
        ),
        b"https://{{domains[www2]}}:{{ports[https][0]}}": (
            b"http://" + request_url_host_bytes + b":" + str(remote_port).encode("ascii")
        ),
        b"https://{{hosts[][www]}}:{{ports[https][0]}}": (
            b"http://www.localhost:" + str(port).encode("ascii")
        ),
        b"https://{{hosts[][www]}}:{{ports[https][1]}}": (
            b"http://www.localhost:" + str(alternate_port).encode("ascii")
        ),
        b"https://{{hosts[][]}}:{{ports[https][0]}}": (
            b"http://localhost:" + str(port).encode("ascii")
        ),
        b"https://{{hosts[][]}}:{{ports[https][1]}}": (
            b"http://localhost:" + str(alternate_port).encode("ascii")
        ),
        b"https://{{hosts[alt][]}}:{{ports[https][0]}}": (
            b"http://alt.localhost:" + str(port).encode("ascii")
        ),
        b"https://{{hosts[alt][]}}:{{ports[https][1]}}": (
            b"http://alt.localhost:" + str(alternate_port).encode("ascii")
        ),
        b"https://{{hosts[alt][www]}}:{{ports[https][0]}}": (
            b"http://www.alt.localhost:" + str(port).encode("ascii")
        ),
        b"https://{{hosts[alt][www]}}:{{ports[https][1]}}": (
            b"http://www.alt.localhost:" + str(alternate_port).encode("ascii")
        ),
        b"wss://{{host}}:{{ports[wss][0]}}": (
            b"ws://" + primary_url_host_bytes + b":" + str(port).encode("ascii")
        ),
        b"wss://{{host}}:{{ports[wss][1]}}": (
            b"ws://" + primary_url_host_bytes + b":" + str(alternate_port).encode("ascii")
        ),
        b"ws://{{host}}:{{ports[ws][0]}}": (
            b"ws://" + primary_url_host_bytes + b":" + str(port).encode("ascii")
        ),
        b"ws://{{host}}:{{ports[ws][1]}}": (
            b"ws://" + primary_url_host_bytes + b":" + str(alternate_port).encode("ascii")
        ),
        b"{{host}}": primary_hostname_bytes,
        b"{{location[scheme]}}": b"http",
        b"{{location[server]}}": current_origin,
        b"{{location[host]}}": request_host_bytes,
        b"{{location[hostname]}}": request_hostname_bytes,
        b"{{location[path]}}": request_path_bytes,
        b"{{hosts[][]}}": b"localhost",
        b"{{hosts[][www]}}": b"www.localhost",
        b"{{hosts[][www1]}}": b"www1.localhost",
        b"{{hosts[][www2]}}": b"www2.localhost",
        b"{{hosts[alt][]}}": b"alt.localhost",
        b"{{hosts[alt][www]}}": b"www.alt.localhost",
        b"{{domains[www]}}": b"www.localhost",
        b"{{domains[www1]}}": b"www1.localhost",
        b"{{domains[www2]}}": b"www2.localhost",
        b"{{location[port]}}": str(port).encode("ascii"),
        b"{{ports[http][0]}}": str(port).encode("ascii"),
        b"{{ports[http][1]}}": str(alternate_port).encode("ascii"),
        b"{{ports[https][0]}}": str(port).encode("ascii"),
        b"{{ports[https][1]}}": str(alternate_port).encode("ascii"),
        b"{{ports[ws][0]}}": str(port).encode("ascii"),
        b"{{ports[ws][1]}}": str(alternate_port).encode("ascii"),
        b"{{ports[wss][0]}}": str(port).encode("ascii"),
        b"{{ports[wss][1]}}": str(alternate_port).encode("ascii"),
        b"var REMOTE_HOST = (ORIGINAL_HOST === 'localhost') ? '127.0.0.1' : ('www1.' + ORIGINAL_HOST);": (
            b"var REMOTE_HOST = (ORIGINAL_HOST === 'localhost') ? 'www1.localhost' : "
            b"((ORIGINAL_HOST.indexOf(':') !== -1) ? ORIGINAL_HOST : ('www1.' + ORIGINAL_HOST));"
        ),
        b"HTTP_REMOTE_ORIGIN: 'http://' + REMOTE_HOST + HTTP_PORT_ELIDED,": (
            b"HTTP_REMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + "
            + alternate_port_literal
            + b") : ('http://' + REMOTE_HOST + HTTP_PORT_ELIDED),"
        ),
        b"REMOTE_ORIGIN: PROTOCOL + \"//\" + REMOTE_HOST + PORT_ELIDED,": (
            b"REMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + "
            + alternate_port_literal
            + b") : (PROTOCOL + \"//\" + REMOTE_HOST + PORT_ELIDED),"
        ),
        b"OTHER_ORIGIN: PROTOCOL + \"//\" + OTHER_HOST + PORT_ELIDED,": (
            b"OTHER_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + ORIGINAL_HOST + "
            + remote_port_literal
            + b") : (PROTOCOL + \"//\" + OTHER_HOST + PORT_ELIDED),"
        ),
        b"HTTP_NOTSAMESITE_ORIGIN: 'http://' + NOTSAMESITE_HOST + HTTP_PORT_ELIDED,": (
            b"HTTP_NOTSAMESITE_ORIGIN: 'http://' + NOTSAMESITE_HOST + HTTP_PORT_ELIDED,"
        ),
        b"HTTPS_ORIGIN: 'https://' + ORIGINAL_HOST + HTTPS_PORT_ELIDED,": (
            b"HTTPS_ORIGIN: 'http://' + ORIGINAL_HOST + HTTP_PORT2_ELIDED,"
        ),
        b"HTTPS_ORIGIN_WITH_CREDS: 'https://foo:bar@' + ORIGINAL_HOST + HTTPS_PORT_ELIDED,": (
            b"HTTPS_ORIGIN_WITH_CREDS: 'http://foo:bar@' + ORIGINAL_HOST + HTTP_PORT2_ELIDED,"
        ),
        b"HTTPS_REMOTE_ORIGIN: 'https://' + REMOTE_HOST + HTTPS_PORT_ELIDED,": (
            b"HTTPS_REMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + "
            + remote_port_literal
            + b") : ('http://' + REMOTE_HOST + HTTP_PORT2_ELIDED),"
        ),
        b"HTTPS_NOTSAMESITE_ORIGIN: 'https://' + NOTSAMESITE_HOST + HTTPS_PORT_ELIDED,": (
            b"HTTPS_NOTSAMESITE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + ORIGINAL_HOST + "
            + remote_port_literal
            + b") : ('https://' + NOTSAMESITE_HOST + HTTPS_PORT_ELIDED),"
        ),
        b"HTTPS_REMOTE_ORIGIN_WITH_CREDS: 'https://foo:bar@' + REMOTE_HOST + HTTPS_PORT_ELIDED,": (
            b"HTTPS_REMOTE_ORIGIN_WITH_CREDS: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://foo:bar@' + REMOTE_HOST + "
            + remote_port_literal
            + b") : ('http://foo:bar@' + REMOTE_HOST + HTTP_PORT2_ELIDED),"
        ),
    }
    for marker, value in replacements.items():
        body = body.replace(marker, value)
    if template_ids is None:
        template_ids = {}

    def replace_uuid(match: re.Match[bytes]) -> bytes:
        name = match.group(1)
        value = str(uuid.uuid4()).encode("ascii")
        template_ids[name] = value
        return value

    body = _UUID_TEMPLATE_RE.sub(replace_uuid, body)
    body = _ID_TEMPLATE_RE.sub(
        lambda match: template_ids.get(match.group(1), b""),
        body,
    )
    get_params = {
        name.encode("utf-8", errors="replace"): value.encode("utf-8", errors="replace")
        for name, value in parse_qsl(query, keep_blank_values=True)
    }
    body = _GET_TEMPLATE_RE.sub(
        lambda match: get_params.get(match.group(1), b""),
        body,
    )
    return body


def _host_header_hostname(host_header: str | None) -> str:
    if not host_header:
        return "localhost"
    if host_header.startswith("["):
        end = host_header.find("]")
        if end != -1:
            return host_header[: end + 1]
    if host_header.count(":") == 1:
        return host_header.rsplit(":", 1)[0]
    return host_header


def _global_ipv6_address() -> str | None:
    try:
        output = subprocess.check_output(
            ["ip", "-o", "-6", "addr", "show", "scope", "global"],
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=2,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    for line in output.splitlines():
        parts = line.split()
        if "inet6" not in parts:
            continue
        address = parts[parts.index("inet6") + 1].split("/", 1)[0]
        if address and not address.lower().startswith("fe80:"):
            return address
    return None


_WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
_MAX_WEBSOCKET_FRAME_BYTES = 16 * 1024 * 1024


def _websocket_accept_key(key: str) -> str:
    digest = hashlib.sha1((key + _WEBSOCKET_GUID).encode("ascii")).digest()
    return base64.b64encode(digest).decode("ascii")


def _websocket_frame(payload: bytes, *, opcode: int, final: bool = True) -> bytes:
    first_byte = (0x80 if final else 0) | (opcode & 0x0F)
    length = len(payload)
    if length < 126:
        prefix = bytes([first_byte, length])
    elif length <= 0xFFFF:
        prefix = bytes([first_byte, 126, (length >> 8) & 0xFF, length & 0xFF])
    else:
        prefix = bytes([first_byte, 127]) + length.to_bytes(8, "big")
    return prefix + payload


def _websocket_text_frame(payload: str) -> bytes:
    return _websocket_frame(payload.encode("utf-8"), opcode=0x1)


def _wpt_cookie_websocket_set_cookie(query: str) -> str | None:
    params = {name for name, _value in parse_qsl(query, keep_blank_values=True)}
    if "secure_from_nonsecure" in params:
        return "ws_test_secure_from_nonsecure=test; Secure; Path=/"
    if "secure_from_secure" in params:
        return "ws_test_secure_from_secure=test; Secure; Path=/"
    return None


class _FixtureThreadingHTTPServer(ThreadingHTTPServer):
    def handle_error(self, request: object, client_address: object) -> None:
        # Disposing a case BrowserContext intentionally cancels any auxiliary
        # requests that outlive the harness result. The stdlib server prints a
        # full traceback for those routine disconnects, obscuring real fixture
        # failures during parallel runs.
        if isinstance(sys.exc_info()[1], (BrokenPipeError, ConnectionResetError)):
            return
        super().handle_error(request, client_address)


class _Ipv6ThreadingHTTPServer(_FixtureThreadingHTTPServer):
    address_family = socket.AF_INET6


class CspReportStore:
    """Thread-safe subset of WPT reporting stash used by CSP report checks."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._cv = threading.Condition(self._lock)
        self._reports: dict[str, list[dict]] = {}
        self._counts: dict[str, int] = {}

    def append_reports(self, report_id: str, reports: list[dict]) -> None:
        with self._cv:
            current = self._reports.setdefault(report_id, [])
            current.extend(reports)
            self._counts[report_id] = self._counts.get(report_id, 0) + 1
            self._cv.notify_all()

    def retrieve_reports(
        self,
        report_id: str,
        *,
        timeout: float,
        min_count: int,
        retain: bool,
    ) -> list[dict]:
        deadline = time.monotonic() + timeout
        with self._cv:
            while True:
                reports = self._reports.get(report_id, [])
                if len(reports) >= min_count:
                    result = list(reports)
                    if not retain:
                        self._reports.pop(report_id, None)
                    return result
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return []
                self._cv.wait(timeout=remaining)

    def retrieve_count(self, report_id: str) -> int:
        with self._lock:
            return self._counts.get(report_id, 0)

    def clear(self, report_ids: list[str]) -> None:
        with self._cv:
            for report_id in report_ids:
                self._reports.pop(report_id, None)
                self._counts.pop(report_id, None)
            self._cv.notify_all()


def _make_handler(
    wpt_root: Path,
    results_store: "ResultsStore",
    report_store: CspReportStore,
) -> type[BaseHTTPRequestHandler]:
    class WptHandler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 (BaseHTTPRequestHandler API)
            if self.headers.get("Upgrade", "").lower() == "websocket":
                self._serve_websocket()
                return
            self._serve(emit_body=True)

        def do_HEAD(self) -> None:  # noqa: N802
            self._serve(emit_body=False)

        def do_OPTIONS(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            path = unquote(parsed.path)
            if path == "/xhr/resources/delay.py":
                self._serve_xhr_delay(parsed.query, emit_body=True)
                return
            if path == "/reporting/resources/report.py":
                self._send_bytes(
                    "text/plain; charset=utf-8",
                    b"CORS allowed",
                    emit_body=True,
                    extra_headers=[
                        ("Access-Control-Allow-Origin", "*"),
                        ("Access-Control-Allow-Methods", "post"),
                        ("Access-Control-Allow-Headers", "Content-Type"),
                    ],
                )
                return
            self.send_error(404)

        def do_POST(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            path = unquote(parsed.path)
            if path == FORM_ECHO_PATH:
                raw = self._read_content_length_request_body()
                if raw is not None:
                    self._send_bytes(
                        "text/plain",
                        _form_echo_response(raw),
                        emit_body=True,
                    )
                return
            if path == "/xhr/resources/delay.py":
                if self._consume_request_body():
                    self._serve_xhr_delay(parsed.query, emit_body=True)
                return
            if path == "/reporting/resources/report.py":
                self._serve_csp_report(parsed.query)
                return
            if path != "/__bench__/result":
                self.send_error(404)
                return
            length_str = self.headers.get("Content-Length") or "0"
            try:
                length = int(length_str)
            except ValueError:
                self.send_error(400)
                return
            if length <= 0 or length > 16 * 1024 * 1024:
                self.send_error(413)
                return
            try:
                raw = self.rfile.read(length)
            except (BrokenPipeError, ConnectionResetError):
                return
            try:
                payload = json.loads(raw.decode("utf-8", errors="replace"))
            except (ValueError, UnicodeDecodeError):
                self.send_error(400)
                return
            case_path = payload.get("case_path") if isinstance(payload, dict) else None
            if not isinstance(case_path, str) or not case_path:
                self.send_error(400)
                return
            case_path = case_path.split("#", 1)[0]
            results_store.put(case_path, payload)
            self.send_response(204)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def do_YO(self) -> None:  # noqa: N802 (WPT custom method)
            parsed = urlparse(self.path)
            if unquote(parsed.path) != "/xhr/resources/delay.py":
                self.send_error(404)
                return
            if self._consume_request_body():
                self._serve_xhr_delay(parsed.query, emit_body=True)

        def log_message(self, format: str, *args: object) -> None:  # noqa: A003 (stdlib name)
            return

        def _serve_websocket(self) -> None:
            parsed = urlparse(self.path)
            path = unquote(parsed.path)
            if path not in {"/set-cookie-secure", "/echo-cookie", "/echo"}:
                self.send_error(404)
                return
            key = self.headers.get("Sec-WebSocket-Key")
            if not key:
                self.send_error(400)
                return
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", _websocket_accept_key(key.strip()))
            if path == "/set-cookie-secure":
                set_cookie = _wpt_cookie_websocket_set_cookie(parsed.query)
                if set_cookie is not None:
                    self.send_header("Set-Cookie", set_cookie)
            self.end_headers()
            if path == "/echo-cookie":
                cookie = self.headers.get("Cookie", "")
                try:
                    self.wfile.write(_websocket_text_frame(cookie))
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError, OSError):
                    return
            if path == "/echo":
                self._serve_websocket_echo()
            else:
                self._wait_for_websocket_close()

        def _read_exact_websocket_bytes(self, length: int) -> bytes | None:
            chunks = bytearray()
            try:
                while len(chunks) < length:
                    chunk = self.rfile.read(length - len(chunks))
                    if not chunk:
                        return None
                    chunks.extend(chunk)
            except (TimeoutError, BrokenPipeError, ConnectionResetError, OSError):
                return None
            return bytes(chunks)

        def _read_websocket_frame(self) -> tuple[bool, int, bytes] | None:
            header = self._read_exact_websocket_bytes(2)
            if header is None:
                return None
            final = bool(header[0] & 0x80)
            opcode = header[0] & 0x0F
            masked = bool(header[1] & 0x80)
            length = header[1] & 0x7F
            if length == 126:
                encoded_length = self._read_exact_websocket_bytes(2)
                if encoded_length is None:
                    return None
                length = int.from_bytes(encoded_length, "big")
            elif length == 127:
                encoded_length = self._read_exact_websocket_bytes(8)
                if encoded_length is None:
                    return None
                length = int.from_bytes(encoded_length, "big")
            if not masked or length > _MAX_WEBSOCKET_FRAME_BYTES:
                return None
            mask = self._read_exact_websocket_bytes(4)
            payload = self._read_exact_websocket_bytes(length)
            if mask is None or payload is None:
                return None
            unmasked = bytes(
                value ^ mask[index % len(mask)]
                for index, value in enumerate(payload)
            )
            return final, opcode, unmasked

        def _serve_websocket_echo(self) -> None:
            self.close_connection = True
            try:
                self.connection.settimeout(2.0)
                while True:
                    frame = self._read_websocket_frame()
                    if frame is None:
                        return
                    final, opcode, payload = frame
                    if opcode == 0x8:
                        self.wfile.write(_websocket_frame(payload, opcode=0x8))
                        self.wfile.flush()
                        return
                    if opcode == 0x9:
                        self.wfile.write(_websocket_frame(payload, opcode=0xA))
                    elif opcode in {0x0, 0x1, 0x2}:
                        self.wfile.write(
                            _websocket_frame(payload, opcode=opcode, final=final)
                        )
                    elif opcode != 0xA:
                        self.wfile.write(
                            _websocket_frame(b"\x03\xea", opcode=0x8)
                        )
                        self.wfile.flush()
                        return
                    self.wfile.flush()
            except (TimeoutError, BrokenPipeError, ConnectionResetError, OSError):
                return

        def _wait_for_websocket_close(self) -> None:
            self.close_connection = True
            try:
                self.connection.settimeout(2.0)
                self.rfile.read(2)
            except (TimeoutError, BrokenPipeError, ConnectionResetError, OSError):
                return

        def _serve(self, *, emit_body: bool) -> None:
            parsed = urlparse(self.path)
            path = unquote(parsed.path)
            pipe_status_code = _pipe_response_status(parsed.query)
            if path == "/reporting/resources/report.py":
                self._serve_csp_report(parsed.query, emit_body=emit_body)
                return
            if path == "/resources/testharnessreport.js":
                self._send_bytes(
                    "application/javascript; charset=utf-8",
                    _bench_report_bridge(
                        _bridge_timeout_multiplier_from_query(parsed.query)
                    ),
                    emit_body=emit_body,
                )
                return
            if path == "/resources/testdriver-vendor.js":
                self._send_bytes("application/javascript; charset=utf-8", BENCH_TESTDRIVER_VENDOR_BRIDGE, emit_body=emit_body)
                return
            if path == FORM_ECHO_PATH:
                self._send_bytes("text/plain", b"", emit_body=emit_body)
                return
            if path == "/xhr/resources/delay.py":
                self._serve_xhr_delay(parsed.query, emit_body=emit_body)
                return
            if path == "/workers/semantics/encodings/003-1.py":
                self._send_bytes(
                    "text/plain; charset=utf-8",
                    _workers_url_encoding_response(parsed.query),
                    emit_body=emit_body,
                )
                return
            if path == "/fetch/api/resources/inspect-headers.py":
                self._send_bytes(
                    "text/plain",
                    b"",
                    emit_body=emit_body,
                    extra_headers=_inspect_headers_response_headers(
                        parsed.query,
                        list(self.headers.items()),
                    ),
                    status_code=pipe_status_code or 200,
                )
                return
            if path == "/fetch/nosniff/resources/js.py":
                self._serve_nosniff_javascript(parsed.query, emit_body=emit_body)
                return
            if path == (
                "/html/semantics/scripting-1/the-script-element/module/"
                "resources/delayed-modulescript.py"
            ):
                self._serve_delayed_module_script(parsed.query, emit_body=emit_body)
                return
            if path in {"/wasm/webapi/status.py", "/wasm/webapi/webapi/status.py"}:
                status_code = _wasm_webapi_status_code(parsed.query)
                if status_code is None:
                    self.send_error(400)
                    return
                self._send_bytes(
                    "application/wasm",
                    _EMPTY_WASM_MODULE,
                    emit_body=emit_body,
                    status_code=status_code,
                )
                return

            if path in {
                "/fetch/api/resources/redirect.py",
                "/common/redirect.py",
                "/common/redirect-opt-in.py",
            }:
                redirect = _redirect_fixture_response(parsed.query)
                if redirect is None:
                    self.send_error(400)
                    return
                status_code, location = redirect
                redirect_headers = [
                    ("Cache-Control", "no-cache"),
                    ("Pragma", "no-cache"),
                    ("Location", location),
                ]
                if path == "/common/redirect-opt-in.py":
                    redirect_headers.append(("Timing-Allow-Origin", "*"))
                else:
                    redirect_headers.append(("Access-Control-Allow-Origin", "*"))
                self._send_bytes(
                    "text/plain; charset=utf-8",
                    b"",
                    emit_body=emit_body,
                    extra_headers=redirect_headers,
                    status_code=status_code,
                )
                return
            if path == "/content-security-policy/support/resource.py":
                body, headers = _content_security_policy_resource_response()
                self._send_bytes(
                    "application/json; charset=utf-8",
                    body,
                    emit_body=emit_body,
                    extra_headers=headers,
                )
                return
            if path == "/workers/modules/resources/export-on-load-script.py":
                body, headers = _workers_modules_export_on_load_script_response()
                self._send_bytes(
                    "text/javascript",
                    body,
                    emit_body=emit_body,
                    extra_headers=headers,
                )
                return
            cleaned = _legacy_wpt_resource_alias(path) or path.lstrip("/")
            if not cleaned or ".." in cleaned.split("/"):
                self.send_error(404)
                return
            is_any_js_window_wrapper = (
                cleaned.endswith(".any.html")
                and _is_any_js_window_wrapper_request(parsed.query)
            )
            is_window_js_window_wrapper = (
                cleaned.endswith(".window.html")
                and _is_window_js_window_wrapper_request(parsed.query)
            )
            if is_any_js_window_wrapper:
                source_cleaned = cleaned.removesuffix(".any.html") + ".any.js"
            elif is_window_js_window_wrapper:
                source_cleaned = cleaned.removesuffix(".window.html") + ".window.js"
            elif is_any_js_worker_script_path(cleaned):
                source_cleaned = any_js_source_script_path(cleaned)
            else:
                source_cleaned = cleaned
            file_path = (wpt_root / source_cleaned).resolve()
            try:
                file_path.relative_to(wpt_root.resolve())
            except ValueError:
                self.send_error(404)
                return
            if not file_path.exists():
                self.send_error(404)
                return
            if file_path.is_dir():
                index = file_path / "index.html"
                if not index.exists():
                    self.send_error(404)
                    return
                file_path = index
            try:
                body = file_path.read_bytes()
            except OSError:
                self.send_error(500)
                return
            if is_any_js_window_wrapper:
                self._send_bytes(
                    "text/html; charset=utf-8",
                    _any_js_window_wrapper(path, body),
                    emit_body=emit_body,
                    status_code=pipe_status_code or 200,
                )
                return
            if is_window_js_window_wrapper:
                self._send_bytes(
                    "text/html; charset=utf-8",
                    _window_js_window_wrapper(path, body),
                    emit_body=emit_body,
                    status_code=pipe_status_code or 200,
                )
                return
            if _needs_wpt_template_substitution(file_path.name, body, parsed.query):
                port = int(
                    getattr(
                        self.server,
                        "wpt_primary_port",
                        self.server.server_address[1],
                    )
                )
                alternate_port = int(getattr(self.server, "wpt_alternate_port", port))
                remote_port = int(getattr(self.server, "wpt_remote_port", alternate_port))
                primary_hostname = str(
                    getattr(
                        self.server,
                        "wpt_primary_hostname",
                        _host_header_hostname(self.headers.get("Host")),
                    )
                )
                body = _substitute_wpt_template_variables(
                    body,
                    port=port,
                    alternate_port=alternate_port,
                    remote_port=remote_port,
                    query=parsed.query,
                    request_path=path,
                    request_hostname=_host_header_hostname(self.headers.get("Host")),
                    primary_hostname=primary_hostname,
                )
            static_header_context = {
                "port": int(
                    getattr(
                        self.server,
                        "wpt_primary_port",
                        self.server.server_address[1],
                    )
                ),
                "alternate_port": int(
                    getattr(
                        self.server,
                        "wpt_alternate_port",
                        getattr(
                            self.server,
                            "wpt_primary_port",
                            self.server.server_address[1],
                        ),
                    )
                ),
                "remote_port": int(
                    getattr(
                        self.server,
                        "wpt_remote_port",
                        getattr(
                            self.server,
                            "wpt_alternate_port",
                            getattr(
                                self.server,
                                "wpt_primary_port",
                                self.server.server_address[1],
                            ),
                        ),
                    )
                ),
                "request_path": path,
                "request_hostname": _host_header_hostname(self.headers.get("Host")),
                "primary_hostname": str(
                    getattr(
                        self.server,
                        "wpt_primary_hostname",
                        _host_header_hostname(self.headers.get("Host")),
                    )
                ),
            }

            def static_headers() -> list[tuple[str, str]]:
                return _static_response_headers(
                    file_path,
                    parsed.query,
                    **static_header_context,
                )

            script_wrapper_global = script_js_wrapper_global(parsed.query)
            if script_wrapper_global is not None:
                body_text = body.decode("utf-8", errors="replace")
                if script_wrapper_global == SCRIPT_JS_WINDOW_GLOBAL and cleaned.endswith(
                    ".window.js"
                ):
                    wrapper = _wpt_window_js_wrapper_html(
                        cleaned,
                        body_text,
                        query=parsed.query,
                    ).encode("utf-8")
                elif (
                    script_wrapper_global == SCRIPT_JS_DEDICATED_WORKER_GLOBAL
                    and cleaned.endswith(".worker.js")
                ):
                    wrapper = _wpt_dedicated_worker_js_wrapper_html(
                        cleaned,
                        query=parsed.query,
                    ).encode("utf-8")
                else:
                    self.send_error(404)
                    return
                wrapper = _inject_bench_report_bridge_config(
                    wrapper,
                    self._harness_timeout_multiplier(path, parsed.query),
                )
                self._send_bytes(
                    "text/html; charset=utf-8",
                    wrapper,
                    emit_body=emit_body,
                    extra_headers=static_headers(),
                    status_code=pipe_status_code or 200,
                )
                return
            wrapper_global = any_js_wrapper_global(parsed.query)
            if wrapper_global is not None:
                if not cleaned.endswith(".any.js"):
                    self.send_error(404)
                    return
                body_text = body.decode("utf-8", errors="replace")
                if wrapper_global == ANY_JS_WINDOW_GLOBAL:
                    wrapper = _wpt_any_window_wrapper_html(
                        cleaned,
                        body_text,
                        query=parsed.query,
                    ).encode("utf-8")
                elif wrapper_global == ANY_JS_DEDICATED_WORKER_GLOBAL:
                    wrapper = _wpt_any_dedicated_worker_wrapper_html(
                        cleaned,
                        query=parsed.query,
                    ).encode("utf-8")
                else:
                    self.send_error(404)
                    return
                wrapper = _inject_bench_report_bridge_config(
                    wrapper,
                    self._harness_timeout_multiplier(path, parsed.query),
                )
                self._send_bytes(
                    "text/html; charset=utf-8",
                    wrapper,
                    emit_body=emit_body,
                    extra_headers=static_headers(),
                    status_code=pipe_status_code or 200,
                )
                return
            if is_any_js_worker_script_path(cleaned):
                body_text = body.decode("utf-8", errors="replace")
                wrapper = _wpt_any_dedicated_worker_wrapper_js(
                    source_cleaned,
                    body_text,
                    query=parsed.query,
                ).encode("utf-8")
                self._send_bytes(
                    "application/javascript; charset=utf-8",
                    wrapper,
                    emit_body=emit_body,
                    extra_headers=static_headers(),
                    status_code=pipe_status_code or 200,
                )
                return
            if file_path.suffix == ".asis":
                parts = _asis_response_parts(body)
                if parts is not None:
                    status_code, body, headers = parts
                    self._send_bytes(
                        "application/octet-stream",
                        body,
                        emit_body=emit_body,
                        extra_headers=[
                            *headers,
                            *static_headers(),
                        ],
                        status_code=pipe_status_code or status_code,
                    )
                    return
            mime, _ = mimetypes.guess_type(str(file_path))
            if mime is None:
                mime = "application/octet-stream"
            response_headers = static_headers()
            if mime == "text/html" and not _headers_include(
                response_headers, "Content-Length"
            ):
                body = _inject_bench_report_bridge_config(
                    body,
                    self._harness_timeout_multiplier(path, parsed.query),
                )
            delay_seconds = _pipe_trickle_delay_seconds(parsed.query)
            if delay_seconds > 0:
                time.sleep(delay_seconds)
            self._send_bytes(
                mime,
                body,
                emit_body=emit_body,
                extra_headers=response_headers,
                status_code=pipe_status_code or 200,
            )

        def _consume_request_body(self) -> bool:
            transfer_encoding = self.headers.get("Transfer-Encoding")
            length_str = self.headers.get("Content-Length")
            if transfer_encoding is not None:
                if length_str is not None:
                    return self._reject_request_body(400)
                codings = [
                    coding.strip().lower()
                    for coding in transfer_encoding.split(",")
                ]
                if (
                    not codings
                    or any(not coding for coding in codings)
                    or codings[-1] != "chunked"
                    or codings.count("chunked") != 1
                ):
                    return self._reject_request_body(400)
                return self._consume_chunked_request_body()
            if length_str is None:
                return True
            try:
                length = int(length_str)
            except ValueError:
                return self._reject_request_body(400)
            if length < 0:
                return self._reject_request_body(400)
            if length > MAX_REQUEST_BODY_BYTES:
                return self._reject_request_body(413)
            return self._discard_request_body_bytes(length)

        def _read_content_length_request_body(self) -> bytes | None:
            if self.headers.get("Transfer-Encoding") is not None:
                self._reject_request_body(400)
                return None
            length_str = self.headers.get("Content-Length")
            if length_str is None:
                return b""
            try:
                length = int(length_str)
            except ValueError:
                self._reject_request_body(400)
                return None
            if length < 0:
                self._reject_request_body(400)
                return None
            if length > MAX_REQUEST_BODY_BYTES:
                self._reject_request_body(413)
                return None
            try:
                raw = self.rfile.read(length)
            except (BrokenPipeError, ConnectionResetError, OSError):
                self.close_connection = True
                return None
            if len(raw) != length:
                self.close_connection = True
                return None
            return raw

        def _consume_chunked_request_body(self) -> bool:
            total = 0
            try:
                while True:
                    size_line = self.rfile.readline(MAX_REQUEST_BODY_LINE_BYTES + 1)
                    if (
                        not size_line
                        or len(size_line) > MAX_REQUEST_BODY_LINE_BYTES
                        or not size_line.endswith(b"\r\n")
                    ):
                        return self._reject_request_body(400)
                    size_token = size_line[:-2].split(b";", 1)[0].strip()
                    if re.fullmatch(rb"[0-9A-Fa-f]+", size_token) is None:
                        return self._reject_request_body(400)
                    chunk_size = int(size_token, 16)
                    if chunk_size == 0:
                        return self._consume_chunked_request_trailers()
                    if chunk_size > MAX_REQUEST_BODY_BYTES - total:
                        return self._reject_request_body(413)
                    if not self._discard_request_body_bytes(chunk_size):
                        return False
                    if self.rfile.read(2) != b"\r\n":
                        return self._reject_request_body(400)
                    total += chunk_size
            except (BrokenPipeError, ConnectionResetError, OSError):
                self.close_connection = True
                return False

        def _consume_chunked_request_trailers(self) -> bool:
            total = 0
            try:
                while True:
                    line = self.rfile.readline(MAX_REQUEST_BODY_LINE_BYTES + 1)
                    if (
                        not line
                        or len(line) > MAX_REQUEST_BODY_LINE_BYTES
                        or not line.endswith(b"\r\n")
                    ):
                        return self._reject_request_body(400)
                    total += len(line)
                    if total > MAX_REQUEST_BODY_LINE_BYTES:
                        return self._reject_request_body(413)
                    if line == b"\r\n":
                        return True
            except (BrokenPipeError, ConnectionResetError, OSError):
                self.close_connection = True
                return False

        def _discard_request_body_bytes(self, length: int) -> bool:
            try:
                remaining = length
                while remaining > 0:
                    chunk = self.rfile.read(min(remaining, 64 * 1024))
                    if not chunk:
                        self.close_connection = True
                        return False
                    remaining -= len(chunk)
                return True
            except (BrokenPipeError, ConnectionResetError, OSError):
                self.close_connection = True
                return False

        def _reject_request_body(self, status_code: int) -> bool:
            self.close_connection = True
            self.send_error(status_code)
            return False

        def _serve_xhr_delay(self, query: str, *, emit_body: bool) -> None:
            delay_seconds = _wpt_delay_seconds(query)
            if delay_seconds is None:
                self.send_error(400)
                return
            time.sleep(delay_seconds)
            self._send_bytes(
                "text/plain",
                b"TEST_DELAY",
                emit_body=emit_body,
                extra_headers=[
                    ("Access-Control-Allow-Origin", "*"),
                    ("Access-Control-Allow-Methods", "YO"),
                ],
            )

        def _serve_delayed_module_script(self, query: str, *, emit_body: bool) -> None:
            delay_seconds = _wpt_delay_seconds(query)
            if delay_seconds is None:
                self.send_error(400)
                return
            time.sleep(delay_seconds)
            self._send_bytes(
                "text/javascript",
                b"export let delayedLoaded = true;",
                emit_body=emit_body,
            )

        def _serve_nosniff_javascript(self, query: str, *, emit_body: bool) -> None:
            content_type, body = _nosniff_javascript_response(query)
            self.send_response(200)
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Length", str(len(body)))
            if content_type is not None:
                self.send_header("Content-Type", content_type)
            self.end_headers()
            if emit_body:
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    return

        def _send_bytes(
            self,
            content_type: str,
            body: bytes,
            *,
            emit_body: bool,
            extra_headers: list[tuple[str, str]] | None = None,
            status_code: int = 200,
        ) -> None:
            content_type, extra_headers = _response_content_type_and_extra_headers(
                content_type,
                extra_headers,
            )
            self.send_response(status_code)
            header_block = _static_response_header_block(content_type, extra_headers)
            for name, value in header_block:
                self.send_header(name, value)
            if not _headers_include(header_block, "Content-Length"):
                self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            if emit_body:
                declared_length = next(
                    (
                        value
                        for name, value in header_block
                        if name.lower() == "content-length"
                    ),
                    None,
                )
                if declared_length is not None:
                    try:
                        length = int(declared_length)
                    except ValueError:
                        length = len(body)
                    if 0 <= length < len(body):
                        body = body[:length]
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    return

        def _serve_csp_report(self, query: str, *, emit_body: bool = True) -> None:
            params = dict(parse_qsl(query, keep_blank_values=True))
            report_id = params.get("reportID")
            if not report_id:
                self.send_error(400)
                return
            if self.command == "GET":
                op = params.get("op", "")
                if op in ("", "retrieve_report"):
                    timeout = _float_query_param(params, "timeout", 0.5)
                    min_count = _int_query_param(params, "min_count", 1)
                    retain = "retain" in params
                    reports = report_store.retrieve_reports(
                        report_id,
                        timeout=timeout,
                        min_count=min_count,
                        retain=retain,
                    )
                    self._send_bytes(
                        "application/json; charset=utf-8",
                        json.dumps(reports).encode("utf-8"),
                        emit_body=emit_body,
                    )
                    return
                if op == "retrieve_cookies":
                    self._send_bytes(
                        "application/json; charset=utf-8",
                        b'{"reportCookies":"None"}',
                        emit_body=emit_body,
                    )
                    return
                if op == "retrieve_count":
                    body = json.dumps(
                        {"report_count": report_store.retrieve_count(report_id)}
                    ).encode("utf-8")
                    self._send_bytes(
                        "application/json; charset=utf-8",
                        body,
                        emit_body=emit_body,
                    )
                    return
                self.send_error(400)
                return

            if self.command != "POST":
                self.send_error(405)
                return
            length_str = self.headers.get("Content-Length") or "0"
            try:
                length = int(length_str)
            except ValueError:
                self.send_error(400)
                return
            if length <= 0 or length > 16 * 1024 * 1024:
                self.send_error(413)
                return
            try:
                raw = self.rfile.read(length)
                payload = json.loads(raw.decode("utf-8", errors="replace"))
            except (OSError, ValueError, UnicodeDecodeError):
                self.send_error(400)
                return
            if isinstance(payload, dict) and payload.get("op") == "DELETE":
                report_ids = payload.get("reportIDs")
                if not isinstance(report_ids, list):
                    self.send_error(400)
                    return
                report_store.clear([str(report_id) for report_id in report_ids])
                self._send_bytes(
                    "text/plain; charset=utf-8",
                    b"reports cleared",
                    emit_body=emit_body,
                )
                return
            reports = payload if isinstance(payload, list) else [payload]
            normalized_reports = [
                report for report in reports if isinstance(report, dict)
            ]
            content_type = self.headers.get("Content-Type", "")
            for report in normalized_reports:
                report.setdefault("metadata", {})["content_type"] = content_type
            report_store.append_reports(report_id, normalized_reports)
            self._send_bytes(
                "text/plain; charset=utf-8",
                b"Recorded report",
                emit_body=emit_body,
                extra_headers=[("Access-Control-Allow-Origin", "*")],
            )

        def _harness_timeout_multiplier(self, path: str, query: str) -> float:
            case_key = _case_key_from_request(path, query)
            multipliers = getattr(
                self.server,
                "wpt_harness_timeout_multipliers",
                {},
            )
            if isinstance(multipliers, dict):
                value = multipliers.get(case_key)
                if value is not None:
                    return _valid_timeout_multiplier(value)
            return _valid_timeout_multiplier(
                getattr(self.server, "wpt_harness_timeout_multiplier", 1.0)
            )

    return WptHandler


def _wpt_window_js_wrapper_html(
    script_path: str,
    script_source: str,
    *,
    query: str = "",
) -> str:
    script_src = html_escape(_wpt_script_js_source_url(script_path, query), quote=True)
    meta_scripts = _wpt_any_meta_script_tags(script_path, script_source)
    return (
        '<!doctype html><meta charset="utf-8">'
        '<script src="/resources/testharness.js"></script>'
        '<script src="/resources/testharnessreport.js"></script>'
        '<body><div id="log"></div>'
        f'{meta_scripts}<script src="{script_src}"></script>'
    )


def _wpt_dedicated_worker_js_wrapper_html(
    script_path: str,
    *,
    query: str = "",
) -> str:
    worker_src = html_escape(_wpt_script_js_source_url(script_path, query), quote=True)
    return (
        '<!doctype html><meta charset="utf-8">'
        '<script src="/resources/testharness.js"></script>'
        '<script src="/resources/testharnessreport.js"></script>'
        '<body><div id="log"></div>'
        "<script>"
        f'fetch_tests_from_worker(new Worker("{worker_src}"));'
        "</script>"
    )


def _wpt_script_js_source_url(script_path: str, query: str) -> str:
    script_url = "/" + script_path.lstrip("/")
    script_query = query_without_script_js_wrapper(query)
    if script_query:
        script_url = f"{script_url}?{script_query}"
    return script_url


def _wpt_any_window_wrapper_html(
    script_path: str,
    script_source: str,
    *,
    query: str = "",
) -> str:
    script_src = html_escape(_wpt_any_script_url(script_path, query), quote=True)
    meta_scripts = _wpt_any_meta_script_tags(script_path, script_source)
    return (
        '<!doctype html><meta charset="utf-8">'
        "<script>"
        "self.GLOBAL={"
        "isWindow:function(){return true;},"
        "isWorker:function(){return false;},"
        "isShadowRealm:function(){return false;}"
        "};"
        "</script>"
        '<script src="/resources/testharness.js"></script>'
        '<script src="/resources/testharnessreport.js"></script>'
        "<body><div id=\"log\"></div>"
        f"{meta_scripts}<script src=\"{script_src}\"></script>"
    )


def _wpt_any_dedicated_worker_wrapper_html(
    script_path: str,
    *,
    query: str = "",
) -> str:
    worker_src = "/" + any_js_worker_script_path(script_path).lstrip("/")
    worker_query = query_without_any_js_wrapper(query)
    if worker_query:
        worker_src = f"{worker_src}?{worker_query}"
    worker_src = html_escape(worker_src, quote=True)
    return (
        '<!doctype html><meta charset="utf-8">'
        '<script src="/resources/testharness.js"></script>'
        '<script src="/resources/testharnessreport.js"></script>'
        "<body><div id=\"log\"></div>"
        "<script>"
        f'fetch_tests_from_worker(new Worker("{worker_src}"));'
        "</script>"
    )


def _wpt_any_dedicated_worker_wrapper_js(
    script_path: str,
    script_source: str,
    *,
    query: str = "",
) -> str:
    script_src = _wpt_any_script_url(script_path, query)
    meta_imports = _wpt_any_meta_import_scripts(script_path, script_source)
    return (
        "self.GLOBAL={\n"
        "  isWindow:function(){return false;},\n"
        "  isWorker:function(){return true;},\n"
        "  isShadowRealm:function(){return false;},\n"
        "};\n"
        'importScripts("/resources/testharness.js");\n'
        f"{meta_imports}"
        f'importScripts("{_js_string_escape(script_src)}");\n'
        "done();\n"
    )


def _wpt_any_script_url(script_path: str, query: str) -> str:
    script_url = "/" + script_path.lstrip("/")
    script_query = query_without_any_js_wrapper(query)
    if script_query:
        script_url = f"{script_url}?{script_query}"
    return script_url


def _wpt_any_meta_script_tags(script_path: str, script_source: str) -> str:
    tags = []
    for reference in _extract_wpt_meta_script_references(script_source):
        script_url = _resolve_wpt_static_script_url(script_path, reference)
        if script_url is None:
            continue
        tags.append(f'<script src="{html_escape(script_url, quote=True)}"></script>')
    return "".join(tags)


def _wpt_any_meta_import_scripts(script_path: str, script_source: str) -> str:
    imports = []
    for reference in _extract_wpt_meta_script_references(script_source):
        script_url = _resolve_wpt_static_script_url(script_path, reference)
        if script_url is None:
            continue
        imports.append(f'importScripts("{_js_string_escape(script_url)}");\n')
    return "".join(imports)


def _js_string_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def _extract_wpt_meta_script_references(source: str) -> list[str]:
    references = []
    for line in source.splitlines():
        line = line.lstrip()
        if not line.startswith("//"):
            break
        meta = line.removeprefix("//").lstrip()
        if not meta.startswith("META:"):
            break
        value = meta.removeprefix("META:").strip()
        if value.startswith("script="):
            reference = value.removeprefix("script=").strip()
            if reference:
                references.append(reference)
    return references


def _resolve_wpt_static_script_url(script_path: str, reference: str) -> str | None:
    parts = urlsplit(reference)
    if parts.scheme or parts.netloc:
        return None
    if parts.path.startswith("/"):
        joined_path = parts.path
    else:
        base_parts = script_path.lstrip("/").split("/")[:-1]
        joined_path = "/" + "/".join([*base_parts, parts.path])
    resolved_path = _normalize_root_relative_path(joined_path)
    if resolved_path is None:
        return None
    return urlunsplit(("", "", resolved_path, parts.query, parts.fragment))


def _normalize_root_relative_path(path: str) -> str | None:
    resolved_parts = []
    for part in path.split("/"):
        if not part or part == ".":
            continue
        if part == "..":
            if not resolved_parts:
                return None
            resolved_parts.pop()
            continue
        resolved_parts.append(part)
    return "/" + "/".join(resolved_parts)


class ResultsStore:
    """Thread-safe per-case-path latest-payload store with wait-for-result."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._cv = threading.Condition(self._lock)
        self._payloads: dict[str, dict] = {}

    def clear(self, case_path: str) -> None:
        with self._cv:
            self._payloads.pop(case_path, None)

    def put(self, case_path: str, payload: dict) -> None:
        with self._cv:
            self._payloads[case_path] = payload
            self._cv.notify_all()

    def get(self, case_path: str) -> dict | None:
        with self._cv:
            return self._payloads.get(case_path)

    def wait_for_final(self, case_path: str, timeout: float) -> dict | None:
        """Block up to ``timeout`` seconds for a payload whose ``source`` is final
        (completion-callback / done-hook / done-hook-late). Returns ``None`` if
        timeout elapses without a final payload.
        """

        deadline = time.monotonic() + timeout
        with self._cv:
            while True:
                payload = self._payloads.get(case_path)
                if payload is not None:
                    src = payload.get("source")
                    if src in ("completion-callback", "done-hook", "done-hook-late"):
                        return payload
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                self._cv.wait(timeout=remaining)


class WptFixtureServer:
    """Serve ``wpt_root`` over loopback + optional global IPv6.

    Use as a context manager. ``base_url`` is the loopback URL all engines
    can use; ``external_base_url`` is the global IPv6 URL for engines that
    refuse loopback fixtures (Obscura).
    """

    def __init__(self, wpt_root: Path) -> None:
        self.wpt_root = wpt_root.resolve()
        if not self.wpt_root.exists():
            raise RuntimeError(f"WPT root does not exist: {self.wpt_root}")
        if not (self.wpt_root / "resources" / "testharness.js").exists():
            raise RuntimeError(
                f"WPT root missing resources/testharness.js: {self.wpt_root}"
            )
        self.results = ResultsStore()
        self.csp_reports = CspReportStore()
        handler_cls = _make_handler(self.wpt_root, self.results, self.csp_reports)
        self.httpd = _FixtureThreadingHTTPServer(("127.0.0.1", 0), handler_cls)
        self.port = int(self.httpd.server_address[1])
        self.alternate_httpd = _FixtureThreadingHTTPServer(
            ("127.0.0.1", 0), handler_cls
        )
        self.alternate_port = int(self.alternate_httpd.server_address[1])
        for httpd in (self.httpd, self.alternate_httpd):
            self._configure_httpd_defaults(
                httpd,
                primary_port=self.port,
                alternate_port=self.alternate_port,
                primary_hostname="localhost",
            )
        self.thread = threading.Thread(
            target=self.httpd.serve_forever,
            name="moli-benchmark-wpt-fixture",
            daemon=True,
        )
        self.alternate_thread = threading.Thread(
            target=self.alternate_httpd.serve_forever,
            name="moli-benchmark-wpt-fixture-alt-port",
            daemon=True,
        )
        self.external_host = _global_ipv6_address()
        self.external_httpd: ThreadingHTTPServer | None = None
        self.external_port: int | None = None
        self.external_thread: threading.Thread | None = None
        self.external_alternate_httpd: ThreadingHTTPServer | None = None
        self.external_alternate_port: int | None = None
        self.external_alternate_thread: threading.Thread | None = None
        self.external_remote_httpd: ThreadingHTTPServer | None = None
        self.external_remote_port: int | None = None
        self.external_remote_thread: threading.Thread | None = None
        if self.external_host is not None:
            try:
                self.external_httpd = _Ipv6ThreadingHTTPServer(
                    (self.external_host, 0), handler_cls
                )
                self.external_port = int(self.external_httpd.server_address[1])
                self.external_alternate_httpd = _Ipv6ThreadingHTTPServer(
                    (self.external_host, 0), handler_cls
                )
                self.external_alternate_port = int(
                    self.external_alternate_httpd.server_address[1]
                )
                self.external_remote_httpd = _Ipv6ThreadingHTTPServer(
                    (self.external_host, 0), handler_cls
                )
                self.external_remote_port = int(
                    self.external_remote_httpd.server_address[1]
                )
                self.external_thread = threading.Thread(
                    target=self.external_httpd.serve_forever,
                    name="moli-benchmark-wpt-fixture-ipv6",
                    daemon=True,
                )
                self.external_alternate_thread = threading.Thread(
                    target=self.external_alternate_httpd.serve_forever,
                    name="moli-benchmark-wpt-fixture-ipv6-alt",
                    daemon=True,
                )
                self.external_remote_thread = threading.Thread(
                    target=self.external_remote_httpd.serve_forever,
                    name="moli-benchmark-wpt-fixture-ipv6-remote",
                    daemon=True,
                )
                for httpd in (
                    self.external_httpd,
                    self.external_alternate_httpd,
                    self.external_remote_httpd,
                ):
                    self._configure_httpd_defaults(
                        httpd,
                        primary_port=self.external_port,
                        alternate_port=self.external_alternate_port,
                        remote_port=self.external_remote_port,
                        primary_hostname=_url_host_literal(str(self.external_host)),
                    )
            except OSError:
                for httpd in (
                    self.external_httpd,
                    self.external_alternate_httpd,
                    self.external_remote_httpd,
                ):
                    if httpd is not None:
                        httpd.server_close()
                self.external_host = None
                self.external_httpd = None
                self.external_port = None
                self.external_thread = None
                self.external_alternate_httpd = None
                self.external_alternate_port = None
                self.external_alternate_thread = None
                self.external_remote_httpd = None
                self.external_remote_port = None
                self.external_remote_thread = None

    @property
    def base_url(self) -> str:
        return f"http://localhost:{self.port}"

    @property
    def external_base_url(self) -> str | None:
        if self.external_host is None or self.external_port is None:
            return None
        return f"http://[{self.external_host}]:{self.external_port}"

    @property
    def external_alternate_base_url(self) -> str | None:
        if self.external_host is None or self.external_alternate_port is None:
            return None
        return f"http://[{self.external_host}]:{self.external_alternate_port}"

    @property
    def external_remote_base_url(self) -> str | None:
        if self.external_host is None or self.external_remote_port is None:
            return None
        return f"http://[{self.external_host}]:{self.external_remote_port}"

    @property
    def alternate_base_url(self) -> str:
        return f"http://localhost:{self.alternate_port}"

    def _configure_httpd_defaults(
        self,
        httpd: ThreadingHTTPServer,
        *,
        primary_port: int,
        alternate_port: int,
        remote_port: int | None = None,
        primary_hostname: str | None = None,
    ) -> None:
        if remote_port is None:
            remote_port = alternate_port
        if primary_hostname is None:
            primary_hostname = _url_host_literal(str(httpd.server_address[0]))
        setattr(httpd, "wpt_primary_port", primary_port)
        setattr(httpd, "wpt_alternate_port", alternate_port)
        setattr(httpd, "wpt_remote_port", remote_port)
        setattr(httpd, "wpt_primary_hostname", primary_hostname)
        setattr(httpd, "wpt_harness_timeout_multiplier", 1.0)
        setattr(httpd, "wpt_harness_timeout_multipliers", {})

    def url_for_case(self, case_path: str, *, external: bool = False) -> str:
        base = self.external_base_url if external and self.external_base_url else self.base_url
        return f"{base}/{case_path.lstrip('/')}"

    def set_harness_timeout_multipliers(
        self,
        case_multipliers: dict[str, float],
        *,
        default_multiplier: float = 1.0,
    ) -> None:
        normalized = {
            _normalize_harness_case_key(case_path): _valid_timeout_multiplier(multiplier)
            for case_path, multiplier in case_multipliers.items()
        }
        for httpd in (
            self.httpd,
            self.alternate_httpd,
            self.external_httpd,
            self.external_alternate_httpd,
            self.external_remote_httpd,
        ):
            if httpd is None:
                continue
            setattr(
                httpd,
                "wpt_harness_timeout_multiplier",
                _valid_timeout_multiplier(default_multiplier),
            )
            setattr(httpd, "wpt_harness_timeout_multipliers", normalized)

    def __enter__(self) -> "WptFixtureServer":
        self.thread.start()
        self.alternate_thread.start()
        if self.external_thread is not None:
            self.external_thread.start()
        if self.external_alternate_thread is not None:
            self.external_alternate_thread.start()
        if self.external_remote_thread is not None:
            self.external_remote_thread.start()
        time.sleep(0.025)
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=2)
        self.alternate_httpd.shutdown()
        self.alternate_httpd.server_close()
        self.alternate_thread.join(timeout=2)
        if self.external_httpd is not None:
            self.external_httpd.shutdown()
            self.external_httpd.server_close()
        if self.external_thread is not None:
            self.external_thread.join(timeout=2)
        if self.external_alternate_httpd is not None:
            self.external_alternate_httpd.shutdown()
            self.external_alternate_httpd.server_close()
        if self.external_alternate_thread is not None:
            self.external_alternate_thread.join(timeout=2)
        if self.external_remote_httpd is not None:
            self.external_remote_httpd.shutdown()
            self.external_remote_httpd.server_close()
        if self.external_remote_thread is not None:
            self.external_remote_thread.join(timeout=2)
