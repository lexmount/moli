//! Highlight values and their V8-traced live registry; style projection remains with the embedder.
use crate::interfaces as web_api_interfaces;
use crate::webidl_iterator::webidl_collection_reflect_apply_intrinsic;
use anyhow::{Result, anyhow};
use moli_v8_util::{throw_type_error, v8_string, v8str};
mod iterators;
use iterators::highlight_iterator_factory_callback;
pub struct HighlightRuntimeState<'s> {
    pub registry: v8::Local<'s, v8::Object>,
    pub highlight_constructor: v8::Local<'s, v8::Value>,
    pub registry_constructor: v8::Local<'s, v8::Value>,
}

pub fn build_highlight_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<HighlightRuntimeState<'s>> {
    let Some(source) = v8_string(scope, HIGHLIGHT_RUNTIME_SOURCE) else {
        return Err(anyhow!("failed to allocate Highlight runtime source"));
    };
    let Some(script) = v8::Script::compile(scope, source, None) else {
        return Err(anyhow!("failed to compile Highlight runtime source"));
    };
    let Some(initializer) = moli_v8_util::with_microtasks_scope(scope, |scope| script.run(scope))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(anyhow!("failed to run Highlight runtime source"));
    };
    let Some(iterator_factory) = v8::Function::builder(highlight_iterator_factory_callback)
        .length(3)
        .build(scope)
    else {
        return Err(anyhow!("failed to create Highlight iterator factory"));
    };
    let Some(call_callback) = webidl_collection_reflect_apply_intrinsic(scope) else {
        return Err(anyhow!(
            "failed to read the bootstrap-captured Reflect.apply intrinsic"
        ));
    };
    let Some(callback_is_callable) = v8::Function::builder(highlight_callback_is_callable_callback)
        .length(1)
        .build(scope)
    else {
        return Err(anyhow!(
            "failed to create the Highlight callback callability predicate"
        ));
    };
    let initialize_identity = v8::Function::new(scope, initialize_highlight_identity_callback)
        .ok_or_else(|| anyhow!("failed to create Highlight identity initializer"))?;
    let Some(value) = initializer.call(
        scope,
        v8::undefined(scope).into(),
        &[
            iterator_factory.into(),
            call_callback.into(),
            callback_is_callable.into(),
            initialize_identity.into(),
        ],
    ) else {
        return Err(anyhow!("failed to initialize Highlight runtime"));
    };
    let registry = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| anyhow!("Highlight runtime did not return a registry object"))?;
    web_api_interfaces::HighlightRegistry::DESCRIPTOR.initialize(scope, registry)?;
    let highlight_constructor = highlight_constructor(scope, registry)?;
    let registry_constructor = highlight_registry_constructor(scope, registry)?;
    Ok(HighlightRuntimeState {
        registry,
        highlight_constructor,
        registry_constructor,
    })
}

fn initialize_highlight_identity_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if let Err(error) = web_api_interfaces::Highlight::DESCRIPTOR.initialize(scope, object) {
        throw_type_error(scope, &error.to_string());
    }
}

fn highlight_callback_is_callable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let callable =
        v8::Local::<v8::Object>::try_from(args.get(0)).is_ok_and(|callback| callback.is_callable());
    rv.set(v8::Boolean::new(scope, callable).into());
}

fn highlight_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Value>> {
    registry
        .get(scope, v8str(scope, "__MoliHighlight").into())
        .ok_or_else(|| anyhow!("failed to read Highlight constructor"))
}

fn highlight_registry_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Value>> {
    registry
        .get(scope, v8str(scope, "__MoliHighlightRegistry").into())
        .ok_or_else(|| anyhow!("failed to read HighlightRegistry constructor"))
}

const HIGHLIGHT_RUNTIME_SOURCE: &str = r#"
((createIterator, callCallback, callbackIsCallable, initializeIdentity) => {
  // Generated maplike/setlike forEach performs an ordinary Call. The caller
  // supplies the Reflect.apply intrinsic captured during Window bootstrap;
  // reading callback.call would instead invoke a page-observable property.
  const rangeSlot = Symbol('MoliHighlightRanges');
  const typeSlot = Symbol('MoliHighlightType');
  const registryEntries = [];

  function activeCount(records) {
    let count = 0;
    for (const record of records) {
      if (!record.deleted) count++;
    }
    return count;
  }

  function highlightRecord(highlight, range) {
    const records = highlight && highlight[rangeSlot];
    if (!records) return null;
    return records.find(record => !record.deleted && record.range === range) || null;
  }

  function highlightRanges(highlight) {
    const records = highlight && highlight[rangeSlot];
    if (!records) return [];
    return records.filter(record => !record.deleted).map(record => record.range);
  }

  function registryRecord(name) {
    const key = String(name);
    return registryEntries.find(record => !record.deleted && record.name === key) || null;
  }

  function liveIterator(kind, records, valueForRecord) {
    return createIterator(kind, records, valueForRecord);
  }

  class Highlight {
    constructor(...ranges) {
      initializeIdentity(this);
      this[rangeSlot] = [];
      this[typeSlot] = 'highlight';
      this.priority = 0;
      this.add(...ranges);
    }
    get size() {
      return activeCount(this[rangeSlot]);
    }
    get type() {
      return this[typeSlot];
    }
    set type(value) {
      const next = String(value);
      if (next === 'highlight' || next === 'spelling-error' || next === 'grammar-error') {
        this[typeSlot] = next;
      }
    }
    add(...ranges) {
      for (const range of ranges) {
        if (!highlightRecord(this, range)) {
          this[rangeSlot].push({ range, deleted: false });
        }
      }
      return this;
    }
    clear() {
      for (const record of this[rangeSlot]) {
        record.deleted = true;
      }
    }
    delete(range) {
      const record = highlightRecord(this, range);
      if (!record) return false;
      record.deleted = true;
      return true;
    }
    has(range) {
      return !!highlightRecord(this, range);
    }
    entries() {
      return liveIterator(0, this[rangeSlot], record => [record.range, record.range]);
    }
    keys() {
      return this.values();
    }
    values() {
      return liveIterator(0, this[rangeSlot], record => record.range);
    }
    forEach(callback, thisArg = undefined) {
      if (!callbackIsCallable(callback)) {
        throw new TypeError('Highlight.forEach callback must be callable');
      }
      for (const range of this.values()) {
        callCallback(callback, thisArg, [range, range, this]);
      }
    }
    [Symbol.iterator]() {
      return this.values();
    }
  }

  function HighlightRegistry() {
    throw new TypeError('Illegal constructor');
  }

  Object.defineProperties(HighlightRegistry.prototype, {
    size: {
      configurable: true,
      enumerable: true,
      get() {
        return activeCount(registryEntries);
      }
    },
    set: {
      configurable: true,
      writable: true,
      value(name, highlight) {
        const key = String(name);
        const existing = registryRecord(key);
        if (existing) {
          existing.highlight = highlight;
          return this;
        }
        registryEntries.push({
          name: key,
          highlight,
          order: registryEntries.length,
          deleted: false
        });
        return this;
      }
    },
    get: {
      configurable: true,
      writable: true,
      value(name) {
        const record = registryRecord(name);
        return record ? record.highlight : undefined;
      }
    },
    has: {
      configurable: true,
      writable: true,
      value(name) {
        return !!registryRecord(name);
      }
    },
    delete: {
      configurable: true,
      writable: true,
      value(name) {
        const record = registryRecord(name);
        if (!record) return false;
        record.deleted = true;
        return true;
      }
    },
    clear: {
      configurable: true,
      writable: true,
      value() {
        for (const record of registryEntries) {
          record.deleted = true;
        }
      }
    },
    entries: {
      configurable: true,
      writable: true,
      value() {
        return liveIterator(1, registryEntries, record => [record.name, record.highlight]);
      }
    },
    keys: {
      configurable: true,
      writable: true,
      value() {
        return liveIterator(1, registryEntries, record => record.name);
      }
    },
    values: {
      configurable: true,
      writable: true,
      value() {
        return liveIterator(1, registryEntries, record => record.highlight);
      }
    },
    forEach: {
      configurable: true,
      writable: true,
      value(callback, thisArg = undefined) {
        if (!callbackIsCallable(callback)) {
          throw new TypeError('HighlightRegistry.forEach callback must be callable');
        }
        for (const [name, highlight] of this.entries()) {
          callCallback(callback, thisArg, [highlight, name, this]);
        }
      }
    }
  });
  HighlightRegistry.prototype[Symbol.iterator] = HighlightRegistry.prototype.entries;

  function isShadowRoot(value) {
    return !!value && value.nodeType === 11 && typeof value.host === 'object';
  }

  function rootOf(node) {
    return node && typeof node.getRootNode === 'function' ? node.getRootNode() : null;
  }

  function shadowRootAllowed(root, allowedShadowRoots) {
    return !isShadowRoot(root) || allowedShadowRoots.includes(root);
  }

  function pointInRect(rect, x, y) {
    return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  }

  function materializedRangeForBoundaryChecks(range) {
    if (range && typeof range.isPointInRange === 'function') {
      return range;
    }
    const start = range && range.startContainer;
    const end = range && range.endContainer;
    if (!start || !end) return null;
    const doc = start.ownerDocument || document;
    const materialized = doc.createRange();
    materialized.setStart(start, range.startOffset);
    materialized.setEnd(end, range.endOffset);
    return materialized;
  }

  function commonAncestorNode(a, b) {
    const ancestors = [];
    for (let current = a; current; current = current.parentNode) {
      ancestors.push(current);
    }
    for (let current = b; current; current = current.parentNode) {
      if (ancestors.includes(current)) return current;
    }
    return null;
  }

  function forEachTextDescendant(root, callback) {
    if (!root) return;
    if (root.nodeType === 3) {
      callback(root);
      return;
    }
    const doc = root.ownerDocument || document;
    const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let current = walker.nextNode(); current; current = walker.nextNode()) {
      callback(current);
    }
  }

  function rangeContainsTextOffset(range, node, offset) {
    try {
      if (!range.isPointInRange(node, offset)) return false;
      return range.endContainer !== node || range.endOffset !== offset;
    } catch {
      return false;
    }
  }

  function rangeContainsPoint(range, x, y, allowedShadowRoots) {
    const start = range && range.startContainer;
    const end = range && range.endContainer;
    if (!start || !end) return false;
    if (start === end && range.startOffset === range.endOffset) return false;
    const startRoot = rootOf(start);
    const endRoot = rootOf(end);
    if (startRoot !== endRoot || !shadowRootAllowed(startRoot, allowedShadowRoots)) {
      return false;
    }
    const materialized = materializedRangeForBoundaryChecks(range);
    if (!materialized || typeof materialized.getClientRects !== 'function') return false;
    const common = commonAncestorNode(start, end);
    let hit = false;
    forEachTextDescendant(common, textNode => {
      if (hit) return;
      const length = (textNode.data || '').length;
      const first = textNode === materialized.startContainer
        ? Math.min(materialized.startOffset, length)
        : 0;
      const last = textNode === materialized.endContainer
        ? Math.min(materialized.endOffset, length)
        : length;
      if (first >= last || !rangeContainsTextOffset(materialized, textNode, first)) return;
      try {
        const textRange = (textNode.ownerDocument || document).createRange();
        textRange.setStart(textNode, first);
        textRange.setEnd(textNode, last);
        const rects = textRange.getClientRects();
        for (let index = 0; index < rects.length; index++) {
          if (pointInRect(rects[index], x, y)) {
            hit = true;
            return;
          }
        }
      } catch {
        return;
      }
    });
    return hit;
  }

  Object.defineProperty(HighlightRegistry.prototype, 'highlightsFromPoint', {
    configurable: true,
    writable: true,
    value(x, y, options = {}) {
      if (arguments.length < 2) {
        throw new TypeError('highlightsFromPoint requires x and y coordinates');
      }
      x = Number(x);
      y = Number(y);
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        throw new TypeError('highlightsFromPoint coordinates must be finite numbers');
      }
      if (options === null || (typeof options !== 'undefined' && typeof options !== 'object')) {
        throw new TypeError('highlightsFromPoint options must be an object');
      }
      const allowedShadowRoots = [];
      if (options && options.shadowRoots !== undefined) {
        for (const root of options.shadowRoots) {
          if (!isShadowRoot(root)) {
            throw new TypeError('shadowRoots must contain ShadowRoot objects');
          }
          allowedShadowRoots.push(root);
        }
      }
      const matches = [];
      for (const entry of registryEntries) {
        if (entry.deleted) continue;
        const ranges = highlightRanges(entry.highlight);
        if (!ranges) continue;
        const hitRanges = ranges.filter(range => rangeContainsPoint(range, x, y, allowedShadowRoots));
        if (hitRanges.length > 0) {
          matches.push({
            highlight: entry.highlight,
            ranges: hitRanges,
            priority: Number(entry.highlight.priority) || 0,
            order: entry.order
          });
        }
      }
      matches.sort((a, b) => (b.priority - a.priority) || (b.order - a.order));
      return matches.map(match => ({ highlight: match.highlight, ranges: match.ranges }));
    }
  });

  const registry = Object.create(HighlightRegistry.prototype);
  Object.defineProperty(registry, '__MoliHighlight', { value: Highlight });
  Object.defineProperty(registry, '__MoliHighlightRegistry', { value: HighlightRegistry });
  return registry;
})
"#;
