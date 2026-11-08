/* Native automation for the CDP WPT runner. Engine DOM and event APIs are left intact. */
(() => {
  const pending = new Map();
  const token = String(Math.random()) + ':' + Date.now();
  let serial = 0;

  function request(data, element = null, context = window, elements = []) {
    if (typeof window.__bench_wpt_native_input__ !== 'function') {
      return Promise.reject(new Error('Native WPT input requires the CDP runner'));
    }
    const id = ++serial;
    return new Promise((resolve, reject) => {
      pending.set(id, {resolve, reject, element, context, elements});
      try {
        window.__bench_wpt_native_input__(JSON.stringify({...data, id, token}));
      } catch (error) {
        pending.delete(id);
        reject(error);
      }
    });
  }

  function entry(id, expectedToken) {
    if (token !== expectedToken || !pending.has(id)) {
      throw new Error('Native input request belongs to an inactive document');
    }
    return pending.get(id);
  }

  function topPoint(context, x, y) {
    if (!Number.isFinite(x) || !Number.isFinite(y) ||
        x < 0 || y < 0 || x >= context.innerWidth || y >= context.innerHeight) {
      throw new Error('move target out of bounds');
    }
    while (context !== context.top) {
      const frame = context.frameElement;
      if (!frame) throw new Error('Native input cannot resolve cross-origin frame coordinates');
      if (context.parent.getComputedStyle(frame).transform !== 'none') {
        throw new Error('Native input cannot resolve transformed frame coordinates');
      }
      const rect = frame.getBoundingClientRect();
      const scaleX = frame.offsetWidth ? rect.width / frame.offsetWidth : 1;
      const scaleY = frame.offsetHeight ? rect.height / frame.offsetHeight : 1;
      x = rect.left + (frame.clientLeft + x) * scaleX;
      y = rect.top + (frame.clientTop + y) * scaleY;
      context = context.parent;
    }
    if (x < 0 || y < 0 || x >= context.innerWidth || y >= context.innerHeight) {
      throw new Error('move target out of bounds');
    }
    return [x, y];
  }

  window.__bench_wpt_native_input_state__ = {
    check(id, expectedToken) {
      entry(id, expectedToken);
      return true;
    },
    finish(id, expectedToken, error) {
      const item = entry(id, expectedToken);
      pending.delete(id);
      if (error !== null) item.reject(new Error(error));
      else item.resolve();
    },
    permissionFramePath(id, expectedToken) {
      let context = entry(id, expectedToken).context;
      if (!context || context.closed || context.top !== window.top) {
        throw new Error('Permission context is not in the current test target');
      }
      const path = [];
      while (context !== window.top) {
        const parent = context.parent;
        let index = 0;
        while (index < parent.length && parent[index] !== context) index++;
        if (index === parent.length) throw new Error('Permission frame is no longer attached');
        path.unshift(index);
        context = parent;
      }
      return path;
    },
    permissionFrameMatches(id, expectedToken, owner) {
      return owner.isConnected && owner.contentWindow === entry(id, expectedToken).context;
    },
    focus(id, expectedToken) {
      const element = entry(id, expectedToken).element;
      if (!element || !element.isConnected) throw new Error('stale element reference');
      const root = element.getRootNode();
      const wasFocused = root.activeElement === element;
      element.focus();
      if (element === element.ownerDocument.body && root.activeElement !== element) {
        const active = root.activeElement;
        if (active && typeof active.blur === 'function') active.blur();
      }
      if (root.activeElement !== element && element !== element.ownerDocument.body) {
        throw new Error('element not interactable');
      }
      if (!wasFocused && typeof element.setSelectionRange === 'function') {
        try {
          const end = String(element.value || '').length;
          element.setSelectionRange(end, end);
        } catch (error) { /* Non-text controls do not expose a text selection. */ }
      }
    },
    point(id, expectedToken, origin, x, y) {
      const item = entry(id, expectedToken);
      let context = item.element ? item.element.ownerDocument.defaultView : item.context;
      if (origin === 'pointer') {
        return topPoint(context.top, x, y);
      } else if (origin && typeof origin === 'object') {
        const element = item.elements[origin.element];
        if (!element || !element.isConnected) throw new Error('stale element reference');
        context = element.ownerDocument.defaultView;
        const rect = element.getClientRects()[0];
        if (!rect) throw new Error('element not interactable');
        const left = Math.max(0, Math.min(rect.left, rect.right));
        const right = Math.min(context.innerWidth, Math.max(rect.left, rect.right));
        const top = Math.max(0, Math.min(rect.top, rect.bottom));
        const bottom = Math.min(context.innerHeight, Math.max(rect.top, rect.bottom));
        x += Math.floor((left + right) / 2);
        y += Math.floor((top + bottom) / 2);
      } else if (origin !== 'viewport') {
        throw new Error('unsupported input origin');
      }
      return topPoint(context, x, y);
    },
  };

  const driver = window.test_driver_internal || (window.test_driver_internal = {});
  driver.in_automation = true;
  driver.click = (element, coords) => request({kind: 'click', x: coords.x, y: coords.y}, element);
  driver.send_keys = (element, keys) => request({kind: 'send_keys', keys}, element);
  driver.set_permission = async (params, context = null) => request({
    kind: 'set_permission', descriptor: params.descriptor, state: params.state,
  }, null, context || window);
  driver.action_sequence = (actions, context = null) => {
    const elements = [];
    const serialized = actions.map(source => ({...source, actions: source.actions.map(action => {
      if (action.origin && typeof action.origin === 'object') {
        const origin = {element: elements.push(action.origin) - 1};
        return {...action, origin};
      }
      return action;
    })}));
    return request({kind: 'actions', actions: serialized}, null, context || window, elements);
  };
})();
