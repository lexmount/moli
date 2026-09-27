
(() => {
  if (window.__lmChatGPTLiveTraceInstalled) return;
  window.__lmChatGPTLiveTraceInstalled = true;
  const events = [];
  const domMutationSamples = [];
  const maxEvents = 500;
  const maxConversationIdentityRecords = 48;
  const responseInfos = new WeakMap();
  const streamInfos = new WeakMap();
  const sourceStats = new Map();
  const idMapTraceSamples = [];
  const idMapTraceStats = {
    set: 0,
    setThreadKey: 0,
    get: 0,
    getHit: 0,
    getMiss: 0,
    getPresent: 0,
    getUndefined: 0,
    lastSetAt: 0,
    lastGetAt: 0,
  };
  const conversationIdentitySamples = [];
  const conversationMaterializationSamples = [];
  const conversationObjectIds = new WeakMap();
  const reactionObjectIds = new WeakMap();
  const conversationIdentityStats = {
    commitSamples: 0,
    changedSamples: 0,
    snapshotSamples: 0,
    lastSampleAt: 0,
    lastChangedAt: 0,
  };
  const conversationMaterializationStats = {
    commitSamples: 0,
    changedSamples: 0,
    streamSamples: 0,
    snapshotSamples: 0,
    lastSampleAt: 0,
    lastChangedAt: 0,
  };
  const navigationTraceSamples = [];
  const navigationTraceStats = {
    present: false,
    navigateCalls: 0,
    navigateErrors: 0,
    navigateCommitted: 0,
    navigateCommitRejected: 0,
    navigateFinished: 0,
    navigateFinishRejected: 0,
    navigateEvents: 0,
    currentEntryChanges: 0,
    navigateSuccess: 0,
    navigateError: 0,
    interceptCalls: 0,
    interceptHandlerStarted: 0,
    interceptHandlerSettled: 0,
    interceptHandlerRejected: 0,
    lastNavigateAt: 0,
    lastEventAt: 0,
    lastInterceptAt: 0,
    lastInterceptHandlerAt: 0,
  };
  const deepProbeCounts = new WeakMap();
  let seq = 0;
  let deepProbeCount = 0;
  let nextConversationObjectId = 0;
  let nextReactionObjectId = 0;
  let lastReactRoot = null;
  let lastDomSignature = '';
  let lastConversationIdentitySignature = '';
  let lastConversationMaterializationSignature = '';
  const eventLoopStats = {
    timeoutScheduled: 0,
    timeoutFired: 0,
    intervalScheduled: 0,
    intervalFired: 0,
    rafScheduled: 0,
    rafFired: 0,
    idleScheduled: 0,
    idleFired: 0,
    schedulerPostTaskScheduled: 0,
    schedulerPostTaskSettled: 0,
    microtaskScheduled: 0,
    microtaskFired: 0,
    heartbeat: 0,
    lastTimeoutFiredAt: 0,
    lastIntervalFiredAt: 0,
    lastRafFiredAt: 0,
    lastIdleFiredAt: 0,
    lastSchedulerPostTaskSettledAt: 0,
    lastMicrotaskFiredAt: 0,
    lastHeartbeatAt: 0,
  };
  const messageTaskStats = {
    messageChannelConstructed: 0,
    messagePortOnmessageSet: 0,
    messagePortOnmessageFired: 0,
    messagePortListenerAdded: 0,
    messagePortListenerFired: 0,
    messagePortPostMessage: 0,
    messagePortMessage: 0,
    messagePortStart: 0,
    windowPostMessage: 0,
    windowMessage: 0,
    lastMessageChannelConstructedAt: 0,
    lastMessagePortOnmessageSetAt: 0,
    lastMessagePortOnmessageFiredAt: 0,
    lastMessagePortListenerAddedAt: 0,
    lastMessagePortListenerFiredAt: 0,
    lastMessagePortPostMessageAt: 0,
    lastMessagePortMessageAt: 0,
    lastWindowPostMessageAt: 0,
    lastWindowMessageAt: 0,
  };
  const observerApiStats = {
    resizeConstructed: 0,
    resizeObserve: 0,
    resizeUnobserve: 0,
    resizeDisconnect: 0,
    resizeCallback: 0,
    resizeEntryCount: 0,
    intersectionConstructed: 0,
    intersectionObserve: 0,
    intersectionUnobserve: 0,
    intersectionDisconnect: 0,
    intersectionCallback: 0,
    intersectionEntryCount: 0,
    lastResizeCallbackAt: 0,
    lastIntersectionCallbackAt: 0,
  };
  const domMutationStats = {
    mutationObserverRecords: 0,
    mutationObserverInteresting: 0,
    appendChildCalls: 0,
    insertBeforeCalls: 0,
    replaceChildrenCalls: 0,
    interestingInserts: 0,
    interestingRemovals: 0,
    lastInterestingMutationAt: 0,
  };
  const reactFiberSamples = [];
  const reactCommitStats = {
    hookInstalled: false,
    hookPreexisting: false,
    rendererCount: 0,
    commitCount: 0,
    commitErrors: 0,
    lastCommitAt: 0,
    lastRendererId: 0,
    lastCommit: null,
  };

  function compactUrl(value) {
    try {
      const url = new URL(String(value), location.href);
      return `${url.origin}${url.pathname}`;
    } catch {
      return String(value || '').slice(0, 240);
    }
  }

  function isInterestingUrl(value) {
    const url = String(value || '');
    return url.includes('chatgpt.com/backend-api/f/conversation') ||
      url.includes('chatgpt.com/backend-api/conversation/') ||
      url.includes('chatgpt.com/backend-api/celsius/ws/user') ||
      url.includes('chatgpt.com/backend-api/sentinel/') ||
      url.includes('ws.chatgpt.com/');
  }

  function isTelemetryUrl(value) {
    const url = String(value || '');
    return url.includes('chatgpt.com/ces/') ||
      url.includes('chatgpt.com/backend-api/lat/');
  }

  function isConversationPostUrl(value) {
    try {
      const url = new URL(String(value || ''), window.location.href);
      return url.hostname === 'chatgpt.com' &&
        url.pathname === '/backend-api/f/conversation';
    } catch {
      return false;
    }
  }

  function nowMs() {
    try {
      return Math.round(performance.now());
    } catch {
      return Date.now();
    }
  }

  function record(type, data = {}) {
    events.push({seq: ++seq, t: nowMs(), type, ...data});
    if (events.length > maxEvents) events.splice(0, events.length - maxEvents);
  }

  function pushDomMutationSample(sample) {
    domMutationSamples.push(sample);
    if (domMutationSamples.length > 80) {
      domMutationSamples.splice(0, domMutationSamples.length - 80);
    }
  }

  function pushReactFiberSample(sample) {
    reactFiberSamples.push(sample);
    if (reactFiberSamples.length > 80) {
      reactFiberSamples.splice(0, reactFiberSamples.length - 80);
    }
  }

  function pushIdMapTraceSample(sample) {
    idMapTraceSamples.push({t: nowMs(), ...sample});
    if (idMapTraceSamples.length > 80) {
      idMapTraceSamples.splice(0, idMapTraceSamples.length - 80);
    }
  }

  function pushNavigationTraceSample(sample) {
    navigationTraceSamples.push({t: nowMs(), ...sample});
    if (navigationTraceSamples.length > 80) {
      navigationTraceSamples.splice(0, navigationTraceSamples.length - 80);
    }
  }

  function pushConversationIdentitySample(sample, options = {}) {
    const changed = !!options.changed;
    conversationIdentitySamples.push({t: nowMs(), changed, ...sample});
    if (conversationIdentitySamples.length > 80) {
      conversationIdentitySamples.splice(0, conversationIdentitySamples.length - 80);
    }
    conversationIdentityStats.lastSampleAt = nowMs();
    if (sample.reason === 'commit') conversationIdentityStats.commitSamples += 1;
    if (sample.reason === 'snapshot') conversationIdentityStats.snapshotSamples += 1;
    if (changed) {
      conversationIdentityStats.changedSamples += 1;
      conversationIdentityStats.lastChangedAt = nowMs();
    }
  }

  function pushConversationMaterializationSample(sample, options = {}) {
    const changed = !!options.changed;
    conversationMaterializationSamples.push({t: nowMs(), changed, ...sample});
    if (conversationMaterializationSamples.length > 80) {
      conversationMaterializationSamples.splice(0, conversationMaterializationSamples.length - 80);
    }
    conversationMaterializationStats.lastSampleAt = nowMs();
    if (sample.reason === 'commit') conversationMaterializationStats.commitSamples += 1;
    if (sample.reason === 'snapshot') conversationMaterializationStats.snapshotSamples += 1;
    if (String(sample.reason || '').startsWith('stream-read')) {
      conversationMaterializationStats.streamSamples += 1;
    }
    if (changed) {
      conversationMaterializationStats.changedSamples += 1;
      conversationMaterializationStats.lastChangedAt = nowMs();
    }
  }

  function textOf(el) {
    if (!el) return '';
    return String(el.value || el.textContent || el.getAttribute?.('aria-label') || '').trim();
  }

  function selectorSummary(selector) {
    try {
      const elements = [...document.querySelectorAll(selector)];
      const textLengths = elements.map((el) => textOf(el).length).filter((length) => length > 0);
      return {
        count: elements.length,
        textCount: textLengths.length,
        latestTextLen: textLengths.length ? textLengths[textLengths.length - 1] : 0,
        maxTextLen: textLengths.length ? Math.max(...textLengths) : 0,
      };
    } catch {
      return {count: -1, textCount: 0, latestTextLen: 0, maxTextLen: 0};
    }
  }

  function selectorCensus() {
    return {
      assistantRole: selectorSummary('[data-message-author-role="assistant"]'),
      userRole: selectorSummary('[data-message-author-role="user"]'),
      messageRole: selectorSummary('[data-message-author-role]'),
      messageId: selectorSummary('[data-message-id]'),
      conversationTurn: selectorSummary('[data-testid*="conversation-turn" i]'),
      article: selectorSummary('article, [role="article"]'),
      markdown: selectorSummary('.markdown, [class*="markdown" i]'),
      main: selectorSummary('main'),
      composer: selectorSummary('#prompt-textarea, textarea, [contenteditable="true"]'),
      stopButton: selectorSummary('button[data-testid*="stop" i], button[aria-label*="Stop" i], button[aria-label*="Cancel" i]'),
      appRoot: selectorSummary('#__next, #root, [data-reactroot]'),
    };
  }

  function elementBrief(el) {
    if (!el) return null;
    const className = typeof el.className === 'string' ? el.className : '';
    return {
      tag: el.tagName || '',
      id: el.id || '',
      role: el.getAttribute?.('role') || '',
      testid: el.getAttribute?.('data-testid') || '',
      dataTurn: el.getAttribute?.('data-turn') || '',
      ariaHidden: el.getAttribute?.('aria-hidden') || '',
      inert: el.hasAttribute?.('inert') || false,
      classHints: className
        .split(/\s+/)
        .filter((name) => /thread|conversation|message|turn|composer|markdown|viewport|virtual|list/i.test(name))
        .slice(0, 8),
      textLen: textOf(el).length,
      childCount: el.children?.length || 0,
    };
  }

  function nodeBrief(node) {
    if (!node) return null;
    if (node.nodeType === Node.ELEMENT_NODE) return elementBrief(node);
    return {
      nodeType: node.nodeType,
      nodeName: node.nodeName || '',
      textLen: textOf(node).length,
    };
  }

  function isInterestingDomNode(node) {
    if (!node) return false;
    if (node.nodeType === Node.TEXT_NODE) {
      return /OK|assistant|message|conversation|thread/i.test(String(node.nodeValue || ''));
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return false;
    const el = node;
    const className = typeof el.className === 'string' ? el.className : '';
    if (
      el.id === 'thread' ||
      el.hasAttribute?.('data-turn') ||
      el.hasAttribute?.('data-message-id') ||
      el.hasAttribute?.('data-message-author-role') ||
      el.getAttribute?.('role') === 'article' ||
      el.getAttribute?.('data-testid')?.includes('conversation') ||
      /thread|conversation|message|turn|markdown|composer/i.test(className)
    ) {
      return true;
    }
    try {
      return !!el.querySelector?.(
        '#thread, [data-turn], [data-message-id], [data-message-author-role], [role="article"], .markdown'
      );
    } catch {
      return false;
    }
  }

  function mutationRecordBrief(record) {
    const added = [...record.addedNodes || []].filter(isInterestingDomNode).slice(0, 8);
    const removed = [...record.removedNodes || []].filter(isInterestingDomNode).slice(0, 8);
    return {
      type: record.type,
      target: nodeBrief(record.target),
      attributeName: record.attributeName || '',
      added: added.map(nodeBrief),
      removed: removed.map(nodeBrief),
    };
  }

  function domTreeProbe() {
    const thread = document.getElementById('thread');
    const main = document.querySelector('main');
    return {
      thread: elementBrief(thread),
      threadDescendantCount: thread ? thread.querySelectorAll('*').length : 0,
      dataTurnCount: document.querySelectorAll('[data-turn]').length,
      articleCount: document.querySelectorAll('article, [role="article"]').length,
      mainChildren: main ? [...main.children].slice(0, 12).map(elementBrief) : [],
      bodyChildren: [...document.body?.children || []].slice(0, 12).map(elementBrief),
    };
  }

  function objectKeys(value, limit = 12) {
    try {
      return value && typeof value === 'object' ? Object.keys(value).slice(0, limit) : [];
    } catch {
      return [];
    }
  }

  function ownKeySummary(value, limit = 12) {
    try {
      if (!value || typeof value !== 'object') return [];
      return Reflect.ownKeys(value)
        .slice(0, limit)
        .map((key) => typeof key === 'symbol' ? `symbol:${String(key.description || '')}` : String(key));
    } catch {
      return [];
    }
  }

  function mapLikeSize(value) {
    if (!value) return 0;
    if (typeof value.size === 'number') return value.size;
    if (Array.isArray(value)) return value.length;
    if (typeof value === 'object') return objectKeys(value, 500).length;
    return 0;
  }

  function countBy(values) {
    const counts = {};
    for (const value of values) {
      const key = String(value || '');
      counts[key] = (counts[key] || 0) + 1;
    }
    return counts;
  }

  function valueShape(value, depth = 0) {
    if (value === null) return {kind: 'null'};
    if (value === undefined) return {kind: 'undefined'};
    if (depth > 2) return {kind: 'nested'};
    if (typeof value === 'function') {
      return {
        kind: 'function',
        name: String(value.name || '').slice(0, 80),
        length: Number(value.length) || 0,
      };
    }
    if (Array.isArray(value)) {
      return {
        kind: 'array',
        length: value.length,
        items: value.slice(0, 4).map((item) => valueShape(item, depth + 1)),
      };
    }
    if (value && typeof value === 'object') {
      const keys = objectKeys(value, 16);
      const ownKeys = ownKeySummary(value, 16);
      const fields = {};
      for (const key of [
        'type',
        'kind',
        'status',
        'state',
        'role',
        'author',
        'message_type',
        'fetchStatus',
        'isPending',
        'isSuccess',
        'isError',
        'isLoading',
        'version',
        '_treeVersion',
      ]) {
        const field = value[key];
        if (typeof field === 'string' || typeof field === 'number' || typeof field === 'boolean') {
          fields[key] = field;
        }
      }
      return {
        kind: 'object',
        tag: Object.prototype.toString.call(value),
        constructorName: value.constructor?.name || '',
        keys,
        ownKeys,
        fields,
      };
    }
    if (typeof value === 'string') {
      const shape = {kind: 'string', length: value.length};
      const id = idShape(value);
      if (id !== `string:${value.length}`) shape.id = id;
      return shape;
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
      return {kind: typeof value, scalar: value};
    }
    return {kind: typeof value};
  }

  function propKeyLabel(key) {
    return typeof key === 'symbol' ? `symbol:${String(key.description || '')}` : String(key);
  }

  function ownValueShapes(value, limit = 16) {
    const shapes = [];
    if (!value || typeof value !== 'object') return shapes;
    try {
      const keys = Reflect.ownKeys(value);
      for (let index = 0; index < keys.length && shapes.length < limit; index += 1) {
        const key = keys[index];
        const label = propKeyLabel(key);
        const entry = {key: label, index};
        try {
          const descriptor = Object.getOwnPropertyDescriptor(value, key);
          if (!descriptor) {
            entry.missingDescriptor = true;
          } else if ('value' in descriptor) {
            entry.shape = valueShape(descriptor.value, 1);
          } else {
            entry.accessor = {
              get: typeof descriptor.get,
              set: typeof descriptor.set,
            };
          }
        } catch (error) {
          entry.error = String(error?.name || error);
        }
        shapes.push(entry);
      }
    } catch (error) {
      shapes.push({error: String(error?.name || error)});
    }
    return shapes;
  }

  function collectionSummary(value, limit = 6) {
    const summary = valueShape(value, 1);
    try {
      if (value instanceof Map) {
        summary.collectionKind = 'Map';
        summary.size = value.size;
        summary.entries = [];
        let index = 0;
        for (const [key, item] of value.entries()) {
          if (index >= limit) break;
          summary.entries.push({
            key: valueShape(key, 1),
            value: valueShape(item, 1),
            valueOwnValueShapes: ownValueShapes(item, 8),
          });
          index += 1;
        }
      } else if (value instanceof Set) {
        summary.collectionKind = 'Set';
        summary.size = value.size;
        summary.entries = [];
        let index = 0;
        for (const item of value.values()) {
          if (index >= limit) break;
          summary.entries.push({
            value: valueShape(item, 1),
            valueOwnValueShapes: ownValueShapes(item, 8),
          });
          index += 1;
        }
      } else if (Array.isArray(value)) {
        summary.collectionKind = 'Array';
        summary.entries = value.slice(0, limit).map((item, index) => ({
          index,
          value: valueShape(item, 1),
          valueOwnValueShapes: ownValueShapes(item, 8),
        }));
      } else if (value && typeof value === 'object') {
        summary.collectionKind = 'Object';
        summary.entries = objectKeys(value, limit).map((key) => ({
          key,
          value: valueShape(value[key], 1),
          valueOwnValueShapes: ownValueShapes(value[key], 8),
        }));
      }
    } catch (error) {
      summary.collectionError = String(error?.name || error);
    }
    return summary;
  }

  function semanticScalarFields(value) {
    const fields = {};
    if (!value || typeof value !== 'object') return fields;
    for (const key of [
      'type',
      'kind',
      'status',
      'state',
      'role',
      'message_type',
      'fetchStatus',
      'isPending',
      'isSuccess',
      'isError',
      'isLoading',
      'isCompletionInProgress',
      'isNewThread',
      'hasUserMessage',
      'hasAssistantMessage',
      'conversation_id',
      'conversationId',
      'thread_id',
      'threadId',
      'clientThreadId',
      'serverThreadId',
      'message_id',
      'messageId',
      'parent_id',
      'parentMessageId',
      'id',
    ]) {
      try {
        const item = value[key];
        if (typeof item === 'string') {
          fields[key] = /(^id$|id$|Id$|_id$)/.test(key) ? idShape(item) : `string:${item.length}`;
        } else if (typeof item === 'number' || typeof item === 'boolean') {
          fields[key] = item;
        }
      } catch {}
    }
    try {
      const author = value.author;
      if (author && typeof author === 'object') {
        const role = author.role || author.type;
        if (typeof role === 'string') fields.authorRole = role;
      }
    } catch {}
    try {
      const content = value.content;
      if (content && typeof content === 'object') {
        fields.contentKeys = objectKeys(content, 8);
        if (typeof content.content_type === 'string') fields.contentType = content.content_type;
        if (Array.isArray(content.parts)) {
          fields.contentPartCount = content.parts.length;
          fields.contentPartLengths = content.parts.slice(0, 4).map((part) => {
            if (typeof part === 'string') return part.length;
            if (part && typeof part === 'object') return objectKeys(part, 6).join(',');
            return typeof part;
          });
        }
      }
    } catch {}
    try {
      if (Object.prototype.hasOwnProperty.call(value, 'hasValue')) {
        fields.hasValue = !!value.hasValue;
      }
      if (Object.prototype.hasOwnProperty.call(value, 'value')) {
        const boxed = value.value;
        fields.valueShape = valueShape(boxed, 1);
        if (typeof boxed === 'number' || typeof boxed === 'boolean') {
          fields.valueScalar = boxed;
        } else if (typeof boxed === 'string') {
          fields.valueId = idShape(boxed);
        }
      }
    } catch {}
    return fields;
  }

  function isReactionStoreLike(value) {
    if (!value || typeof value !== 'object') return false;
    let matchingKeys = 0;
    for (const key of [
      'reaction',
      'onStoreChange',
      'stateVersion',
      'name',
      'lastValue',
      'evaluate',
      'subscribe',
      'getSnapshot',
    ]) {
      try {
        if (Object.prototype.hasOwnProperty.call(value, key)) matchingKeys += 1;
      } catch {}
    }
    try {
      return matchingKeys >= 4 &&
        (typeof value.subscribe === 'function' || typeof value.getSnapshot === 'function');
    } catch {
      return matchingKeys >= 5;
    }
  }

  function reactionObjectLabel(value) {
    if (!value || typeof value !== 'object') return '';
    try {
      let label = reactionObjectIds.get(value);
      if (!label) {
        label = `reaction:${++nextReactionObjectId}`;
        reactionObjectIds.set(value, label);
      }
      return label;
    } catch {
      return '';
    }
  }

  function safeScalarShape(value, key = '') {
    if (value === null) return {kind: 'null'};
    if (value === undefined) return {kind: 'undefined'};
    if (typeof value === 'string') {
      const shape = {kind: 'string', length: value.length};
      if (/id|thread|conversation|message/i.test(key)) shape.id = idShape(value);
      if (
        /status|state|role|kind|type/i.test(key) &&
        /^[A-Za-z_-]{1,32}$/.test(value)
      ) {
        shape.scalar = value;
      }
      return shape;
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
      return {kind: typeof value, scalar: value};
    }
    return valueShape(value, 1);
  }

  function queryLikeValueSummary(value) {
    const summary = valueShape(value, 1);
    if (!value || typeof value !== 'object') return summary;
    const fields = {};
    for (const key of [
      'status',
      'fetchStatus',
      'isPending',
      'isSuccess',
      'isError',
      'isInitialLoading',
      'isLoading',
      'isFetched',
      'isFetching',
      'isStale',
      'isPlaceholderData',
      'dataUpdatedAt',
      'errorUpdatedAt',
      'failureCount',
      'failureReason',
      'conversationId',
      'threadId',
      'clientThreadId',
      'serverThreadId',
      'messageId',
      'id',
    ]) {
      try {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
          fields[key] = safeScalarShape(value[key], key);
        }
      } catch (error) {
        fields[key] = {error: String(error?.name || error)};
      }
    }
    if (Object.keys(fields).length) summary.queryFields = fields;
    for (const key of ['data', 'current', 'value', 'error', 'promise']) {
      try {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
          summary[`${key}Shape`] = valueShape(value[key], 1);
          const nestedFields = semanticScalarFields(value[key]);
          if (Object.keys(nestedFields).length) {
            summary[`${key}Fields`] = nestedFields;
          }
        }
      } catch (error) {
        summary[`${key}Error`] = String(error?.name || error);
      }
    }
    return summary;
  }

  function reactionStoreDetail(value) {
    if (!isReactionStoreLike(value)) return null;
    const detail = {
      kind: 'reaction-store',
      object: reactionObjectLabel(value),
      ownKeys: ownKeySummary(value, 16),
      hasSubscribe: false,
      hasGetSnapshot: false,
      hasEvaluate: false,
      hasOnStoreChange: false,
    };
    try {
      detail.hasSubscribe = typeof value.subscribe === 'function';
      detail.hasGetSnapshot = typeof value.getSnapshot === 'function';
      detail.hasEvaluate = typeof value.evaluate === 'function';
      detail.hasOnStoreChange = typeof value.onStoreChange === 'function';
    } catch {}
    for (const key of ['stateVersion', 'name']) {
      try {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
          detail[key] = safeScalarShape(value[key], key);
        }
      } catch (error) {
        detail[`${key}Error`] = String(error?.name || error);
      }
    }
    try {
      if (Object.prototype.hasOwnProperty.call(value, 'lastValue')) {
        detail.lastValue = queryLikeValueSummary(value.lastValue);
      }
    } catch (error) {
      detail.lastValueError = String(error?.name || error);
    }
    try {
      if (typeof value.getSnapshot === 'function') {
        detail.snapshot = queryLikeValueSummary(value.getSnapshot());
      }
    } catch (error) {
      detail.snapshotError = String(error?.name || error);
    }
    try {
      if (Object.prototype.hasOwnProperty.call(value, 'reaction')) {
        detail.reactionShape = valueShape(value.reaction, 1);
      }
    } catch (error) {
      detail.reactionError = String(error?.name || error);
    }
    return detail;
  }

  function semanticValueDetail(value, depth = 0) {
    const detail = valueShape(value, 1);
    if (depth >= 2 || !value || (typeof value !== 'object' && typeof value !== 'function')) {
      return detail;
    }
    if (Array.isArray(value)) {
      detail.collectionKind = 'Array';
      detail.entries = value.slice(0, 6).map((item, index) => ({
        index,
        value: semanticValueDetail(item, depth + 1),
      }));
      return detail;
    }
    if (value instanceof Map) {
      detail.collectionKind = 'Map';
      detail.size = value.size;
      detail.entries = [];
      let index = 0;
      for (const [key, item] of value.entries()) {
        if (index >= 6) break;
        detail.entries.push({
          key: semanticValueDetail(key, depth + 1),
          value: semanticValueDetail(item, depth + 1),
        });
        index += 1;
      }
      return detail;
    }
    if (value instanceof Set) {
      detail.collectionKind = 'Set';
      detail.size = value.size;
      detail.entries = [];
      let index = 0;
      for (const item of value.values()) {
        if (index >= 6) break;
        detail.entries.push({value: semanticValueDetail(item, depth + 1)});
        index += 1;
      }
      return detail;
    }
    if (typeof value === 'object') {
      detail.fields = {...(detail.fields || {}), ...semanticScalarFields(value)};
      const reactionDetail = reactionStoreDetail(value);
      if (reactionDetail) detail.reactionStore = reactionDetail;
      const conversationDetail = conversationResultDetail(value, depth + 1);
      if (conversationDetail) detail.conversationDetail = conversationDetail;
      const selectedValues = {};
      const startedAt = nowMs();
      try {
        for (const key of Reflect.ownKeys(value)) {
          if (Object.keys(selectedValues).length >= 12 || nowMs() - startedAt > 80) break;
          const label = propKeyLabel(key);
          if (
            !/conversation|thread|tree|turn|message|display|item|current|value|status|loading|content|author|role|server|client/i.test(label) ||
            label === 'children'
          ) {
            continue;
          }
          try {
            selectedValues[label] = semanticValueDetail(value[key], depth + 1);
          } catch (error) {
            selectedValues[label] = {error: String(error?.name || error)};
          }
        }
      } catch (error) {
        selectedValues.error = String(error?.name || error);
      }
      if (Object.keys(selectedValues).length) detail.selectedValues = selectedValues;
    }
    return detail;
  }

  function selectedNamedValueShapes(value, patterns, limit = 12) {
    const selected = {};
    if (!value || typeof value !== 'object') return selected;
    const startedAt = nowMs();
    try {
      const keys = Reflect.ownKeys(value);
      for (const key of keys) {
        if (Object.keys(selected).length >= limit || nowMs() - startedAt > 100) break;
        const label = propKeyLabel(key);
        if (!patterns.some((pattern) => pattern.test(label))) continue;
        let propValue;
        try {
          propValue = value[key];
        } catch (error) {
          selected[label] = {error: String(error?.name || error)};
          continue;
        }
        const shape = valueShape(propValue, 1);
        const detail = conversationResultDetail(propValue, 1);
        selected[label] = detail ? {...shape, detail} : shape;
      }
    } catch (error) {
      selected.error = String(error?.name || error);
    }
    return selected;
  }

  function storeLikeSummaries(value, limit = 8) {
    const stores = [];
    if (!value || typeof value !== 'object') return stores;
    const startedAt = nowMs();
    try {
      const keys = Reflect.ownKeys(value);
      for (const key of keys) {
        if (stores.length >= limit || nowMs() - startedAt > 150) break;
        let candidate;
        try {
          candidate = value[key];
        } catch (error) {
          stores.push({key: propKeyLabel(key), error: String(error?.name || error)});
          continue;
        }
        if (
          !candidate ||
          typeof candidate !== 'object' ||
          typeof candidate.getState !== 'function' ||
          typeof candidate.subscribe !== 'function'
        ) {
          continue;
        }
        const summary = {
          key: propKeyLabel(key),
          shape: valueShape(candidate, 1),
        };
        try {
          if (candidate._listeners instanceof Set || candidate._listeners instanceof Map) {
            summary.listenerCount = candidate._listeners.size;
          }
        } catch {}
        try {
          const state = candidate.getState();
          summary.stateShape = valueShape(state, 1);
          summary.stateOwnValueShapes = ownValueShapes(state, 16);
          summary.selectedStateValues = selectedNamedValueShapes(
            state,
            [/conversation/i, /thread/i, /tree/i, /turn/i, /display/i, /message/i],
            12,
          );
        } catch (error) {
          summary.stateError = String(error?.name || error);
        }
        stores.push(summary);
      }
    } catch (error) {
      stores.push({error: String(error?.name || error)});
    }
    return stores;
  }

  function idMapObjectSummary(value, limit = 12) {
    if (!value || typeof value !== 'object') return null;
    const keys = objectKeys(value, limit);
    return {
      count: objectKeys(value, 500).length,
      entries: keys.map((key) => {
        let mapped = '';
        try {
          mapped = typeof value[key] === 'string' ? value[key] : '';
        } catch {}
        return {key: idShape(key), value: mapped ? idShape(mapped) : ''};
      }),
    };
  }

  function threadStoreStateDetail(value) {
    if (!value || typeof value !== 'object') return null;
    if (
      !Object.prototype.hasOwnProperty.call(value, 'clientNewThreadIdToServerIdMapping') &&
      !Object.prototype.hasOwnProperty.call(value, 'threads')
    ) {
      return null;
    }
    const detail = {};
    try {
      detail.mapping = idMapObjectSummary(value.clientNewThreadIdToServerIdMapping, 16);
    } catch (error) {
      detail.mappingError = String(error?.name || error);
    }
    try {
      const threads = value.threads;
      detail.threadCount = threads && typeof threads === 'object' ? objectKeys(threads, 1000).length : 0;
      detail.threadKeys = threads && typeof threads === 'object'
        ? objectKeys(threads, 16).map((key) => idShape(key))
        : [];
    } catch (error) {
      detail.threadsError = String(error?.name || error);
    }
    return detail;
  }

  function hookValueDetail(value) {
    const detail = valueShape(value, 1);
    if (!value || typeof value !== 'object') return detail;
    try {
      const reactionDetail = reactionStoreDetail(value);
      if (reactionDetail) detail.reactionStore = reactionDetail;
    } catch {}
    try {
      const threadStore = threadStoreStateDetail(value);
      if (threadStore) detail.threadStore = threadStore;
    } catch {}
    try {
      const conversationDetail = conversationResultDetail(value, 1);
      if (conversationDetail) detail.conversationDetail = conversationDetail;
    } catch {}
    try {
      detail.selectedValues = selectedNamedValueShapes(
        value,
        [/conversation/i, /thread/i, /tree/i, /turn/i, /display/i, /message/i],
        10,
      );
    } catch {}
    return detail;
  }

  function hookStateSummary(fiber, limit = 8) {
    const hooks = [];
    const seen = new Set();
    let current = null;
    try {
      current = fiber?.memoizedState || null;
    } catch {
      return hooks;
    }
    while (current && hooks.length < limit && !seen.has(current)) {
      seen.add(current);
      const entry = {
        index: hooks.length,
        hookKeys: ownKeySummary(current, 12),
      };
      try {
        entry.memoizedState = hookValueDetail(current.memoizedState);
      } catch (error) {
        entry.memoizedStateError = String(error?.name || error);
      }
      try {
        if (current.baseState !== undefined) {
          entry.baseState = hookValueDetail(current.baseState);
        }
      } catch (error) {
        entry.baseStateError = String(error?.name || error);
      }
      try {
        if (current.queue) {
          entry.queueShape = valueShape(current.queue, 1);
          if (current.queue.value !== undefined) {
            entry.queueValue = hookValueDetail(current.queue.value);
          }
          if (current.queue.lastRenderedState !== undefined) {
            entry.lastRenderedState = hookValueDetail(current.queue.lastRenderedState);
          }
          if (typeof current.queue.getSnapshot === 'function') {
            try {
              const snapshot = current.queue.getSnapshot();
              if (snapshot !== undefined) entry.queueSnapshot = hookValueDetail(snapshot);
            } catch (error) {
              entry.queueSnapshotError = String(error?.name || error);
            }
          }
        }
      } catch (error) {
        entry.queueError = String(error?.name || error);
      }
      hooks.push(entry);
      try {
        current = current.next || null;
      } catch {
        break;
      }
    }
    return hooks;
  }

  function hookSemanticSummary(fiber, limit = 24) {
    const hooks = [];
    const seen = new Set();
    let current = null;
    try {
      current = fiber?.memoizedState || null;
    } catch {
      return hooks;
    }
    while (current && hooks.length < limit && !seen.has(current)) {
      seen.add(current);
      const entry = {
        index: hooks.length,
        hookKeys: ownKeySummary(current, 12),
      };
      try {
        entry.memoizedState = semanticValueDetail(current.memoizedState);
      } catch (error) {
        entry.memoizedStateError = String(error?.name || error);
      }
      try {
        if (current.baseState !== undefined) {
          entry.baseState = semanticValueDetail(current.baseState);
        }
      } catch (error) {
        entry.baseStateError = String(error?.name || error);
      }
      try {
        if (current.queue) {
          entry.queueShape = valueShape(current.queue, 1);
          if (current.queue.value !== undefined) {
            entry.queueValue = semanticValueDetail(current.queue.value);
          }
          if (current.queue.lastRenderedState !== undefined) {
            entry.lastRenderedState = semanticValueDetail(current.queue.lastRenderedState);
          }
          if (typeof current.queue.getSnapshot === 'function') {
            try {
              const snapshot = current.queue.getSnapshot();
              if (snapshot !== undefined) entry.queueSnapshot = semanticValueDetail(snapshot);
            } catch (error) {
              entry.queueSnapshotError = String(error?.name || error);
            }
          }
        }
      } catch (error) {
        entry.queueError = String(error?.name || error);
      }
      hooks.push(entry);
      try {
        current = current.next || null;
      } catch {
        break;
      }
    }
    return hooks;
  }

  function threadRendererProbes(limit = 16) {
    const probes = [];
    const stack = [];
    const seen = new Set();
    try {
      if (lastReactRoot?.current) stack.push({fiber: lastReactRoot.current, depth: 0});
      else if (lastReactRoot) stack.push({fiber: lastReactRoot, depth: 0});
    } catch {}
    while (stack.length && probes.length < limit && seen.size < 6000) {
      const {fiber, depth} = stack.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber);
      let name = '';
      let props = null;
      let hints = [];
      try {
        name = reactFiberName(fiber);
        props = fiber.memoizedProps;
        hints = reactFiberHints(fiber, name, props);
      } catch {}
      if (/^(d8|T2|Zqn|qJn|pY|iyr|u4n|qDr|\$Ar|NFe|BFe|e8)$/.test(name) ||
          isConversationThreadListFiber(name, props)) {
        probes.push({
          depth,
          name,
          tag: Number(fiber.tag),
          hints,
          props: compactFiberPropsForTrace(props),
          sourceHint: shouldRecordFiberSourceHint(name, props) ? reactFiberSourceHint(fiber) : undefined,
          hooks: hookSemanticSummary(fiber, 24),
        });
      }
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1});
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      visited: seen.size,
      count: probes.length,
      probes,
    };
  }

  function threadStoreHookSnapshots(limit = 12) {
    const snapshots = [];
    const stack = [];
    const seenFibers = new Set();
    try {
      if (lastReactRoot?.current) stack.push({fiber: lastReactRoot.current, depth: 0});
      else if (lastReactRoot) stack.push({fiber: lastReactRoot, depth: 0});
    } catch {}

    function pushIfThreadStore(fiber, depth, hookIndex, source, candidate) {
      if (snapshots.length >= limit || !candidate || typeof candidate !== 'object') return;
      const detail = threadStoreStateDetail(candidate);
      if (!detail) return;
      snapshots.push({
        depth,
        name: reactFiberName(fiber),
        tag: Number(fiber.tag),
        hookIndex,
        source,
        detail: hookValueDetail(candidate),
      });
    }

    while (stack.length && snapshots.length < limit && seenFibers.size < 5000) {
      const {fiber, depth} = stack.pop();
      if (!fiber || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber);
      try {
        let hook = fiber.memoizedState || null;
        const seenHooks = new Set();
        for (let hookIndex = 0; hook && hookIndex < 16 && !seenHooks.has(hook); hookIndex += 1) {
          seenHooks.add(hook);
          try {
            pushIfThreadStore(fiber, depth, hookIndex, 'memoizedState', hook.memoizedState);
            pushIfThreadStore(fiber, depth, hookIndex, 'baseState', hook.baseState);
            if (hook.queue) {
              pushIfThreadStore(fiber, depth, hookIndex, 'queue.value', hook.queue.value);
              pushIfThreadStore(
                fiber,
                depth,
                hookIndex,
                'queue.lastRenderedState',
                hook.queue.lastRenderedState,
              );
              if (typeof hook.queue.getSnapshot === 'function') {
                try {
                  pushIfThreadStore(
                    fiber,
                    depth,
                    hookIndex,
                    'queue.getSnapshot',
                    hook.queue.getSnapshot(),
                  );
                } catch {}
              }
            }
          } catch {}
          hook = hook.next || null;
        }
      } catch {}
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1});
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      visited: seenFibers.size,
      count: snapshots.length,
      snapshots,
    };
  }

  function reactionStoreProbes(limit = 96) {
    const probes = [];
    const stack = [];
    const seenFibers = new Set();
    const seenCandidates = new WeakSet();
    try {
      const rootFiber = lastReactRoot?.current || lastReactRoot || null;
      if (rootFiber) stack.push({fiber: rootFiber, depth: 0, source: 'root'});
    } catch {}
    try {
      const threadFiber = fiberForDomNode(document.getElementById('thread'));
      if (threadFiber) stack.push({fiber: threadFiber, depth: 0, source: 'thread'});
    } catch {}

    function pushCandidate(fiber, depth, hookIndex, source, candidate) {
      if (probes.length >= limit || !isReactionStoreLike(candidate)) return;
      try {
        if (seenCandidates.has(candidate)) return;
        seenCandidates.add(candidate);
      } catch {}
      let name = '';
      let props = null;
      let hints = [];
      try {
        name = reactFiberName(fiber);
        props = fiber.memoizedProps;
        hints = reactFiberHints(fiber, name, props);
      } catch {}
      probes.push({
        source,
        depth,
        name,
        tag: Number(fiber.tag),
        tagLabel: reactFiberTagLabel(Number(fiber.tag)),
        hints,
        path: fiberPathSummary(fiber, 10),
        hookIndex,
        hookSource: source,
        selectedThreadProps: identityThreadPropShapes(props),
        detail: reactionStoreDetail(candidate),
      });
    }

    while (stack.length && probes.length < limit && seenFibers.size < 9000) {
      const {fiber, depth, source} = stack.pop();
      if (!fiber || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber);
      try {
        let hook = fiber.memoizedState || null;
        const seenHooks = new Set();
        for (let hookIndex = 0; hook && hookIndex < 32 && !seenHooks.has(hook); hookIndex += 1) {
          seenHooks.add(hook);
          try {
            pushCandidate(fiber, depth, hookIndex, `${source}:memoizedState`, hook.memoizedState);
            if (hook.memoizedState && typeof hook.memoizedState === 'object') {
              pushCandidate(
                fiber,
                depth,
                hookIndex,
                `${source}:memoizedState.current`,
                hook.memoizedState.current,
              );
            }
            pushCandidate(fiber, depth, hookIndex, `${source}:baseState`, hook.baseState);
            if (hook.queue) {
              pushCandidate(fiber, depth, hookIndex, `${source}:queue.value`, hook.queue.value);
              pushCandidate(
                fiber,
                depth,
                hookIndex,
                `${source}:queue.lastRenderedState`,
                hook.queue.lastRenderedState,
              );
              if (typeof hook.queue.getSnapshot === 'function') {
                try {
                  pushCandidate(
                    fiber,
                    depth,
                    hookIndex,
                    `${source}:queue.getSnapshot`,
                    hook.queue.getSnapshot(),
                  );
                } catch {}
              }
            }
          } catch {}
          hook = hook.next || null;
        }
      } catch {}
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth, source});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1, source});
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      visited: seenFibers.size,
      count: probes.length,
      probes,
    };
  }

  function conversationResultDetail(value, depth = 0) {
    if (!value || typeof value !== 'object') return null;
    const detail = {keys: objectKeys(value, 20), ownKeys: ownKeySummary(value, 20)};
    try {
      if (value.tree && typeof value.tree === 'object') {
        detail.treeShape = valueShape(value.tree, 1);
        detail.treeOwnValueShapes = ownValueShapes(value.tree, 16);
        detail.treePrototypeMethods = prototypeMethodSummary(value.tree, 24);
        try {
          detail.treeCurrentLeafId = idShape(String(value.tree.currentLeafId || ''));
        } catch (error) {
          detail.treeCurrentLeafIdError = String(error?.name || error);
        }
        try {
          detail.treeNodes = collectionSummary(value.tree.nodes, 6);
        } catch (error) {
          detail.treeNodesError = String(error?.name || error);
        }
        try {
          if (typeof value.tree.getDisplayItems === 'function') {
            detail.treeDisplayItems = collectionSummary(
              value.tree.getDisplayItems(value.tree.currentLeafId),
              6,
            );
          }
        } catch (error) {
          detail.treeDisplayItemsError = String(error?.name || error);
        }
        try {
          if (typeof value.tree.getDisplayTurns === 'function') {
            detail.treeDisplayTurns = collectionSummary(
              value.tree.getDisplayTurns(value.tree.currentLeafId),
              6,
            );
          }
        } catch (error) {
          detail.treeDisplayTurnsError = String(error?.name || error);
        }
      }
    } catch (error) {
      detail.treeError = String(error?.name || error);
    }
    try {
      if (value.data && typeof value.data === 'object') {
        detail.dataShape = valueShape(value.data, 1);
        detail.dataOwnValueShapes = ownValueShapes(value.data, 16);
      }
    } catch (error) {
      detail.dataError = String(error?.name || error);
    }
    return detail;
  }

  function isConversationWrapper(value) {
    return !!value &&
      typeof value === 'object' &&
      typeof value.serverId$ === 'function' &&
      typeof value.id === 'string' &&
      value.ctx &&
      typeof value.ctx === 'object' &&
      value.config &&
      typeof value.config === 'object';
  }

  function conversationObjectLabel(value) {
    if (!value || typeof value !== 'object') return '';
    try {
      let label = conversationObjectIds.get(value);
      if (!label) {
        label = `conversation:${++nextConversationObjectId}`;
        conversationObjectIds.set(value, label);
      }
      return label;
    } catch {
      return '';
    }
  }

  function conversationIdentityShape(value) {
    if (!isConversationWrapper(value)) return valueShape(value, 1);
    const shape = {
      kind: 'conversation-wrapper',
      object: conversationObjectLabel(value),
      ownKeys: ownKeySummary(value, 12),
      ctxKeys: objectKeys(value.ctx, 12),
      configKeys: objectKeys(value.config, 12),
    };
    let serverId = '';
    try {
      serverId = String(value.serverId$() || '');
    } catch (error) {
      shape.serverIdError = String(error?.name || error);
    }
    shape.id = idShape(String(value.id || ''));
    shape.serverId = idShape(serverId);
    shape.idMatchesServerId = !!serverId && String(value.id || '') === serverId;
    return shape;
  }

  function reactElementTypeName(type) {
    try {
      if (typeof type === 'string') return type;
      if (typeof type === 'function') return type.displayName || type.name || '';
      if (type && typeof type === 'object') {
        return type.displayName ||
          type.name ||
          type.render?.displayName ||
          type.render?.name ||
          type.type?.displayName ||
          type.type?.name ||
          '';
      }
    } catch {}
    return '';
  }

  function identityThreadPropShapes(props) {
    const selected = {};
    if (!props || typeof props !== 'object') return selected;
    for (const key of [
      'conversationId',
      'urlThreadId',
      'clientThreadId',
      'serverThreadId',
      'threadId',
      'turnId',
      'forceRenderedTurnId',
    ]) {
      try {
        if (Object.prototype.hasOwnProperty.call(props, key)) {
          selected[key] = scalarValueShape(props[key], key);
        }
      } catch {}
    }
    return selected;
  }

  function pushConversationIdentityRecord(records, record) {
    if (!record || records.length >= maxConversationIdentityRecords) return;
    records.push(record);
  }

  function recordConversationIdentityFromProps(records, source, depth, name, tag, props, path) {
    if (!props || typeof props !== 'object' || records.length >= maxConversationIdentityRecords) return;
    let conversation = null;
    try {
      conversation = props.conversation;
    } catch {}
    const hasConversation = isConversationWrapper(conversation);
    const selectedThreadProps = identityThreadPropShapes(props);
    const hasSelectedThreadProps = Object.keys(selectedThreadProps).length > 0;
    if (!hasConversation && !hasSelectedThreadProps && !/^(KFe|JFe|XFe|Sbe|Zqn|qJn)$/.test(name)) {
      return;
    }
    const record = {
      source,
      depth,
      name,
      tag,
      path,
      selectedThreadProps,
    };
    if (hasConversation) {
      record.conversation = conversationIdentityShape(conversation);
    }
    pushConversationIdentityRecord(records, record);
  }

  function collectReactElementConversationIdentities(value, source, path, depth, records, seen) {
    if (depth > 5 || records.length >= maxConversationIdentityRecords) return;
    if (Array.isArray(value)) {
      for (let index = 0; index < value.length && records.length < maxConversationIdentityRecords; index += 1) {
        collectReactElementConversationIdentities(
          value[index],
          source,
          `${path}[${index}]`,
          depth + 1,
          records,
          seen,
        );
      }
      return;
    }
    if (!value || typeof value !== 'object') return;
    if (seen.has(value)) return;
    seen.add(value);
    const maybeElement = Object.prototype.hasOwnProperty.call(value, '$$typeof') &&
      Object.prototype.hasOwnProperty.call(value, 'type') &&
      Object.prototype.hasOwnProperty.call(value, 'props');
    if (!maybeElement) return;
    const name = reactElementTypeName(value.type);
    const props = value.props;
    recordConversationIdentityFromProps(records, `${source}:element`, depth, name, '', props, path);
    try {
      if (props && typeof props === 'object') {
        if (Object.prototype.hasOwnProperty.call(props, 'children')) {
          collectReactElementConversationIdentities(
            props.children,
            source,
            `${path}>children`,
            depth + 1,
            records,
            seen,
          );
        }
        if (Object.prototype.hasOwnProperty.call(props, 'fallback')) {
          collectReactElementConversationIdentities(
            props.fallback,
            source,
            `${path}>fallback`,
            depth + 1,
            records,
            seen,
          );
        }
      }
    } catch {}
  }

  function conversationIdentitySnapshot(root, reason = 'snapshot') {
    const records = [];
    const seenFibers = new Set();
    const seenElements = new WeakSet();
    const stack = [];
    let threadFiber = null;
    try {
      const rootFiber = root?.current || root || lastReactRoot?.current || lastReactRoot || null;
      if (rootFiber) stack.push({fiber: rootFiber, depth: 0, source: 'root'});
    } catch {}
    try {
      threadFiber = fiberForDomNode(document.getElementById('thread'));
      if (threadFiber) stack.push({fiber: threadFiber, depth: 0, source: 'thread'});
    } catch {}
    while (stack.length && records.length < maxConversationIdentityRecords && seenFibers.size < 8000) {
      const {fiber, depth, source} = stack.pop();
      if (!fiber || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber);
      let name = '';
      let props = null;
      let tag = 0;
      try {
        name = reactFiberName(fiber);
        props = fiber.memoizedProps;
        tag = Number(fiber.tag);
      } catch {}
      const path = fiberPathSummary(fiber, 8).map((item) => item.name || item.tagLabel).join('>');
      recordConversationIdentityFromProps(records, source, depth, name, tag, props, path);
      try {
        if (tag === 13 && props && typeof props === 'object') {
          collectReactElementConversationIdentities(
            props.children,
            `${source}:suspense`,
            `${path}>children`,
            0,
            records,
            seenElements,
          );
        }
      } catch {}
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth, source});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1, source});
      } catch {}
    }
    const identities = records
      .map((record) => {
        const conversation = record.conversation || {};
        return [
          record.source,
          record.name,
          conversation.object || '',
          conversation.id || '',
          conversation.serverId || '',
          String(!!conversation.idMatchesServerId),
        ].join('/');
      })
      .join('|');
    return {
      reason,
      url: compactUrl(location.href),
      threadFiberPresent: !!threadFiber,
      visited: seenFibers.size,
      recordCount: records.length,
      signature: identities.slice(0, 2000),
      records,
    };
  }

  function recordConversationIdentitySnapshot(root, reason) {
    let sample = null;
    try {
      sample = conversationIdentitySnapshot(root, reason);
      const changed = sample.signature !== lastConversationIdentitySignature;
      if (changed) {
        lastConversationIdentitySignature = sample.signature;
      }
      pushConversationIdentitySample(sample, {changed});
      if (changed || reason !== 'commit') {
        record('conversation-identity', {
          reason,
          changed,
          recordCount: sample.recordCount,
          records: sample.records.slice(0, 8),
        });
      }
    } catch (error) {
      record('conversation-identity-error', {
        reason,
        name: String(error?.name || ''),
        message: String(error?.message || error).slice(0, 240),
      });
    }
    return sample;
  }

  function conversationIdentityTraceState() {
    const current = recordConversationIdentitySnapshot(null, 'snapshot');
    return {
      ...conversationIdentityStats,
      now: nowMs(),
      current,
      samples: conversationIdentitySamples.slice(-24),
    };
  }

  function shouldDeepProbeConversationWrapper(value) {
    if (!isConversationWrapper(value)) return false;
    if (deepProbeCount >= 24) return false;
    try {
      const count = deepProbeCounts.get(value) || 0;
      if (count >= 3) return false;
      deepProbeCounts.set(value, count + 1);
      deepProbeCount += 1;
      return true;
    } catch {
      return false;
    }
  }

  function zeroArgFunctionResultShapes(value, limit = 16, depth = 0) {
    const shapes = [];
    if (!value || typeof value !== 'object') return shapes;
    const startedAt = nowMs();
    try {
      const keys = Reflect.ownKeys(value);
      for (let index = 0; index < keys.length && shapes.length < limit; index += 1) {
        if (nowMs() - startedAt > 150) {
          shapes.push({truncated: true, reason: 'time-budget'});
          break;
        }
        const key = keys[index];
        const descriptor = Object.getOwnPropertyDescriptor(value, key);
        if (!descriptor || !('value' in descriptor)) continue;
        const fn = descriptor.value;
        if (typeof fn !== 'function' || fn.length !== 0) continue;
        const entry = {key: propKeyLabel(key), index, name: String(fn.name || '').slice(0, 80)};
        try {
          const result = fn.call(value);
          entry.resultShape = valueShape(result, 1);
          const detail = depth < 2 ? conversationResultDetail(result, depth + 1) : null;
          if (detail) entry.resultDetail = detail;
        } catch (error) {
          entry.error = String(error?.name || error);
        }
        shapes.push(entry);
      }
    } catch (error) {
      shapes.push({error: String(error?.name || error)});
    }
    return shapes;
  }

  function prototypeMethodSummary(value, limit = 24) {
    const methods = [];
    if (!value || typeof value !== 'object') return methods;
    const seen = new Set();
    try {
      let proto = Object.getPrototypeOf(value);
      let depth = 0;
      while (proto && proto !== Object.prototype && depth < 4 && methods.length < limit) {
        for (const key of Reflect.ownKeys(proto)) {
          if (methods.length >= limit) break;
          const label = propKeyLabel(key);
          if (label === 'constructor' || seen.has(label)) continue;
          seen.add(label);
          try {
            const descriptor = Object.getOwnPropertyDescriptor(proto, key);
            if (!descriptor) continue;
            const valueKind = 'value' in descriptor ? typeof descriptor.value : '';
            const accessorKind = 'get' in descriptor && descriptor.get ? 'getter' : '';
            if (valueKind === 'function' || accessorKind) {
              methods.push({
                key: label,
                kind: valueKind === 'function' ? 'method' : accessorKind,
                name: valueKind === 'function' ? String(descriptor.value.name || '').slice(0, 80) : '',
                length: valueKind === 'function' ? Number(descriptor.value.length) || 0 : 0,
              });
            }
          } catch (error) {
            methods.push({key: label, error: String(error?.name || error)});
          }
        }
        proto = Object.getPrototypeOf(proto);
        depth += 1;
      }
    } catch (error) {
      methods.push({error: String(error?.name || error)});
    }
    return methods;
  }

  function valueProbe(value) {
    const result = {kind: shapeKind(value)};
    try { result.tag = Object.prototype.toString.call(value); } catch (error) { result.tagError = String(error?.name || error); }
    try { result.constructorName = value?.constructor?.name || ''; } catch (error) { result.constructorError = String(error?.name || error); }
    try { result.keys = value && typeof value === 'object' ? Object.keys(value).slice(0, 20) : []; } catch (error) { result.keysError = String(error?.name || error); }
    try {
      result.ownKeys = value && typeof value === 'object'
        ? Reflect.ownKeys(value).slice(0, 20).map((key) => typeof key === 'symbol' ? `symbol:${String(key.description || '')}` : String(key))
        : [];
    } catch (error) {
      result.ownKeysError = String(error?.name || error);
    }
    try { result.thenType = typeof value?.then; } catch {}
    try { result.mapLikeSize = mapLikeSize(value); } catch {}
    const childShapes = {};
    for (const key of result.keys || []) {
      try {
        childShapes[key] = shapeKind(value[key]);
      } catch (error) {
        childShapes[key] = `error:${String(error?.name || error)}`;
      }
    }
    result.childShapes = childShapes;
    return result;
  }

  function reactRouterState() {
    const router = window.__reactRouterDataRouter;
    const state = router?.state;
    if (!state || typeof state !== 'object') return {present: !!router};
    const loaderDataKeys = objectKeys(state.loaderData, 40);
    const actionDataKeys = objectKeys(state.actionData, 40);
    const errorKeys = objectKeys(state.errors, 40);
    const matches = Array.isArray(state.matches)
      ? state.matches.slice(-12).map((match) => ({
          id: String(match?.id || '').slice(0, 160),
          pathname: String(match?.pathname || match?.pathnameBase || '').slice(0, 160),
          hasLoaderData: loaderDataKeys.includes(String(match?.id || '')),
        }))
      : [];
    const loaderDataShapes = {};
    const loaderDataProbes = {};
    try {
      for (const key of loaderDataKeys.slice(0, 10)) {
        loaderDataShapes[key] = valueShape(state.loaderData?.[key]);
        loaderDataProbes[key] = valueProbe(state.loaderData?.[key]);
      }
    } catch {}
    return {
      present: true,
      locationPathname: String(state.location?.pathname || ''),
      navigationState: String(state.navigation?.state || ''),
      revalidation: String(state.revalidation || ''),
      loaderDataKeys,
      loaderDataShapes,
      loaderDataProbes,
      actionDataKeys,
      errorKeys,
      fetcherCount: mapLikeSize(state.fetchers),
      blockerCount: mapLikeSize(state.blockers),
      matches,
    };
  }

  function reactQueryState() {
    const cache = window.__REACT_QUERY_CACHE__;
    if (!cache) return {present: false};
    let queries = [];
    try {
      if (typeof cache.getAll === 'function') {
        queries = cache.getAll();
      } else if (Array.isArray(cache.queries)) {
        queries = cache.queries;
      } else if (Array.isArray(cache)) {
        queries = cache;
      }
    } catch {}
    const queryStates = queries
      .slice(0, 80)
      .map((query) => query?.state || query)
      .filter((state) => state && typeof state === 'object');
    return {
      present: true,
      kind: Object.prototype.toString.call(cache),
      keys: objectKeys(cache, 16),
      queryCount: queries.length,
      statusCounts: countBy(queryStates.map((state) => state.status)),
      fetchStatusCounts: countBy(queryStates.map((state) => state.fetchStatus)),
    };
  }

  function routeModulesState() {
    const modules = window.__reactRouterRouteModules;
    if (!modules || typeof modules !== 'object') return {present: !!modules};
    const keys = objectKeys(modules, 80);
    const selected = {};
    for (const key of keys.filter((key) => /conversation|root/i.test(key)).slice(0, 12)) {
      const module = modules[key];
      const exportShapes = {};
      for (const exportName of objectKeys(module, 60)) {
        const value = module?.[exportName];
        if (
          exportName === 'default' ||
          /loader|action|component|ErrorBoundary|HydrateFallback|shouldRevalidate/i.test(exportName)
        ) {
          exportShapes[exportName] = {
            type: typeof value,
            name: typeof value === 'function' ? String(value.name || '') : '',
            keys: value && typeof value === 'object' ? objectKeys(value, 20) : [],
          };
        }
      }
      selected[key] = {
        keys: objectKeys(module, 60),
        ownKeys: ownKeySummary(module, 60),
        exportShapes,
      };
    }
    return {
      present: true,
      kind: Object.prototype.toString.call(modules),
      keys,
      selected,
    };
  }

  function appRuntimeState() {
    const windowKeys = [];
    try {
      for (const key of Object.keys(window)) {
        if (/react|router|remix|next/i.test(key)) windowKeys.push(key);
        if (windowKeys.length >= 20) break;
      }
    } catch {}
    return {
      windowKeys,
      router: reactRouterState(),
      routeModules: routeModulesState(),
      queryCache: reactQueryState(),
      nextData: !!document.querySelector('script#__NEXT_DATA__'),
      serializedAppScripts: document.querySelectorAll('script[type="application/json"], script[data-flight], script[data-rsc]').length,
      platformTaskApis: {
        requestIdleCallback: typeof window.requestIdleCallback,
        cancelIdleCallback: typeof window.cancelIdleCallback,
        scheduler: typeof window.scheduler,
        schedulerPostTask: typeof window.scheduler?.postTask,
        MessageChannel: typeof window.MessageChannel,
        ResizeObserver: typeof window.ResizeObserver,
        IntersectionObserver: typeof window.IntersectionObserver,
      },
      observerApis: observerApiState(),
      reactCommits: reactCommitState(),
      activeElement: document.activeElement
        ? {
            tag: document.activeElement.tagName,
            id: document.activeElement.id || '',
            role: document.activeElement.getAttribute?.('role') || '',
            testid: document.activeElement.getAttribute?.('data-testid') || '',
          }
        : null,
    };
  }

  function recordAppState(reason) {
    record('app-state', {reason, state: appRuntimeState()});
  }

  function eventLoopState() {
    return {...eventLoopStats, now: nowMs()};
  }

  function messageTaskState() {
    return {...messageTaskStats, now: nowMs()};
  }

  function observerApiState() {
    return {...observerApiStats, now: nowMs()};
  }

  function domMutationState() {
    return {...domMutationStats, now: nowMs(), samples: domMutationSamples.slice(-40)};
  }

  function reactCommitState() {
    return {
      ...reactCommitStats,
      now: nowMs(),
      samples: reactFiberSamples.slice(-40),
    };
  }

  function reactFiberName(fiber) {
    try {
      const type = fiber?.elementType || fiber?.type;
      if (typeof type === 'string') return type;
      if (typeof type === 'function') return type.displayName || type.name || '';
      if (type && typeof type === 'object') {
        return type.displayName ||
          type.name ||
          type.render?.displayName ||
          type.render?.name ||
          type.type?.displayName ||
          type.type?.name ||
          '';
      }
    } catch {}
    return '';
  }

  function reactFiberSourceHint(fiber) {
    try {
      const type = fiber?.type || fiber?.elementType;
      const fn = typeof type === 'function' ? type :
        typeof type?.render === 'function' ? type.render :
        typeof type?.type === 'function' ? type.type : null;
      if (!fn) return '';
      return Function.prototype.toString.call(fn).replace(/\s+/g, ' ').slice(0, 2200);
    } catch {}
    return '';
  }

  function isConversationThreadListFiber(name, props) {
    return name === 's' &&
      !!props &&
      typeof props === 'object' &&
      Object.prototype.hasOwnProperty.call(props, 'conversation') &&
      Object.prototype.hasOwnProperty.call(props, 'onRequestCompletion') &&
      Object.prototype.hasOwnProperty.call(props, 'scrollContainerRef') &&
      Object.prototype.hasOwnProperty.call(props, 'disableScrollToMessage');
  }

  function shouldRecordFiberSourceHint(name, props = null) {
    return /^(NFe|BFe|e8|T2|qDr|\$Ar|XFe|pY|iyr|Sbe|KFe|JFe|nFe|uje|nJn|eje|Zqn|qJn|d8|u4n|T9|gQe)$/.test(name) ||
      isConversationThreadListFiber(name, props);
  }

  function propHintString(value) {
    if (typeof value !== 'string') return '';
    return value.length > 160 ? `${value.slice(0, 160)}...` : value;
  }

  function scalarValueShape(value, key = '') {
    const detail = valueShape(value);
    if (typeof value === 'boolean' || typeof value === 'number') {
      detail.scalar = value;
    } else if (typeof value === 'string') {
      detail.scalar = safeScalarToken(value);
      if (/id|thread|conversation|turn|message/i.test(String(key || ''))) {
        detail.id = idShape(value);
      }
    }
    return detail;
  }

  function selectedPropValueDetail(value, options = {}) {
    const key = String(options.key || '');
    const detail = scalarValueShape(value, key);
    if (typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string') return detail;
    if (typeof value === 'function' && options.callZeroArg && value.length === 0) {
      try {
        detail.zeroArgResult = scalarValueShape(value(), key);
      } catch (error) {
        detail.zeroArgError = String(error?.name || error);
      }
      return detail;
    }
    if (!value || typeof value !== 'object') return detail;
    try {
      if (typeof value.id === 'string') detail.id = idShape(value.id);
    } catch {}
    try {
      if (typeof value.serverId$ === 'function') {
        detail.serverId = idShape(String(value.serverId$() || ''));
      }
    } catch (error) {
      detail.serverIdError = String(error?.name || error);
    }
    try {
      if (value.ctx && typeof value.ctx === 'object') {
        detail.ctxKeys = objectKeys(value.ctx, 12);
      }
    } catch {}
    try {
      if (value.config && typeof value.config === 'object') {
        detail.configKeys = objectKeys(value.config, 12);
      }
    } catch {}
    try {
      detail.ownValueShapes = ownValueShapes(value, 16);
    } catch {}
    try {
      detail.prototypeMethods = prototypeMethodSummary(value, 24);
    } catch {}
    if (
      (options.forceDeepConversationProbe && isConversationWrapper(value)) ||
      shouldDeepProbeConversationWrapper(value)
    ) {
      try {
        detail.zeroArgFunctionResults = zeroArgFunctionResultShapes(value, 16);
      } catch {}
      try {
        detail.ctxOwnValueShapes = ownValueShapes(value.ctx, 16);
      } catch {}
      try {
        detail.ctxStoreLike = storeLikeSummaries(value.ctx, 8);
      } catch {}
      try {
        detail.ctxZeroArgFunctionResults = zeroArgFunctionResultShapes(value.ctx, 16);
      } catch {}
    }
    return detail;
  }

  function reactPropsSummary(props) {
    if (!props || typeof props !== 'object') return {kind: typeof props};
    const keys = objectKeys(props, 30);
    const className = typeof props.className === 'string' ? props.className : '';
    const children = props.children;
    const selectedValueShapes = {};
    for (const key of [
      'conversation',
      'conversationId',
      'urlThreadId',
      'thread',
      'threadId',
      'serverThreadId',
      'clientThreadId',
      'turn',
      'turnId',
      'turnIndex',
      'message',
      'messages',
      'displayItems',
      'isThreadContentLoading',
      'isNewThread',
      'when$',
      'fallback',
      'renderEmptyState',
      'renderEmptyFooter',
      'hideComposer',
      'isComposerPinnedToBottomOnEmptyState',
      'isScrolledFromBottom$',
      'shouldUseUnifiedComposer',
      'isCompletionInProgress',
      'isGizmoThread',
      'isProjectThread',
      'layoutMode',
      'currentModelId',
      'pageLoadSearchQuery',
      'forceRenderedTurnId',
      'children',
    ]) {
      if (Object.prototype.hasOwnProperty.call(props, key)) {
        selectedValueShapes[key] = selectedPropValueDetail(props[key], {
          key,
          callZeroArg: key.endsWith('$') || key === 'when$',
        });
      }
    }
    return {
      kind: 'object',
      keys,
      id: propHintString(props.id),
      role: propHintString(props.role),
      testid: propHintString(props['data-testid']),
      ariaHidden: propHintString(props['aria-hidden']),
      inert: !!props.inert,
      dataTurn: propHintString(props['data-turn']),
      dataMessageId: props['data-message-id'] ? idShape(String(props['data-message-id'])) : undefined,
      dataMessageAuthorRole: propHintString(props['data-message-author-role']),
      classHints: className
        .split(/\s+/)
        .filter((name) => /thread|conversation|message|turn|composer|markdown|viewport|virtual|list/i.test(name))
        .slice(0, 8),
      childKind: Array.isArray(children) ? `array:${children.length}` : typeof children,
      textChildLen: typeof children === 'string' ? children.length : 0,
      hasDangerousHtml: !!props.dangerouslySetInnerHTML,
      selectedValueShapes,
    };
  }

  function reactFiberHints(fiber, name, props) {
    const hints = [];
    const haystack = [
      name,
      props?.id,
      props?.role,
      props?.className,
      props?.['data-testid'],
      props?.['data-turn'],
      props?.['data-message-id'],
      props?.['data-message-author-role'],
    ]
      .filter((value) => typeof value === 'string')
      .join(' ');
    if (/conversation/i.test(haystack)) hints.push('conversation');
    if (/thread/i.test(haystack)) hints.push('thread');
    if (/message/i.test(haystack)) hints.push('message');
    if (/turn/i.test(haystack)) hints.push('turn');
    if (/markdown/i.test(haystack)) hints.push('markdown');
    if (/composer|prompt/i.test(haystack)) hints.push('composer');
    if (props && typeof props === 'object') {
      if (Object.prototype.hasOwnProperty.call(props, 'conversation')) hints.push('prop:conversation');
      if (Object.prototype.hasOwnProperty.call(props, 'conversationId')) hints.push('prop:conversationId');
      if (Object.prototype.hasOwnProperty.call(props, 'threadId')) hints.push('prop:threadId');
      if (Object.prototype.hasOwnProperty.call(props, 'turn')) hints.push('prop:turn');
      if (Object.prototype.hasOwnProperty.call(props, 'message')) hints.push('prop:message');
      if (Object.prototype.hasOwnProperty.call(props, 'messages')) hints.push('prop:messages');
    }
    return hints;
  }

  function summarizeReactCommitRoot(root) {
    const summary = {
      visited: 0,
      hostComponents: 0,
      hostText: 0,
      conversationHints: 0,
      messageHints: 0,
      turnHints: 0,
      markdownHints: 0,
      composerHints: 0,
      dataTurnProps: 0,
      dataMessageProps: 0,
      dataRoleProps: 0,
      textFiberLenMax: 0,
      samples: [],
    };
    const stack = [];
    try {
      if (root?.current) stack.push(root.current);
      else if (root) stack.push(root);
    } catch {}
    const seen = new Set();
    while (stack.length && summary.visited < 5000) {
      const fiber = stack.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber);
      summary.visited += 1;
      const props = fiber.memoizedProps;
      const name = reactFiberName(fiber);
      const tag = Number(fiber.tag);
      if (tag === 5) summary.hostComponents += 1;
      if (tag === 6) {
        summary.hostText += 1;
        const textLen = String(fiber.memoizedProps || '').length;
        if (textLen > summary.textFiberLenMax) summary.textFiberLenMax = textLen;
      }
      if (props && typeof props === 'object') {
        if (props['data-turn'] != null) summary.dataTurnProps += 1;
        if (props['data-message-id'] != null) summary.dataMessageProps += 1;
        if (props['data-message-author-role'] != null) summary.dataRoleProps += 1;
      }
      const hints = reactFiberHints(fiber, name, props);
      if (hints.includes('conversation')) summary.conversationHints += 1;
      if (hints.includes('message')) summary.messageHints += 1;
      if (hints.includes('turn')) summary.turnHints += 1;
      if (hints.includes('markdown')) summary.markdownHints += 1;
      if (hints.includes('composer')) summary.composerHints += 1;
      if (hints.length && summary.samples.length < 16) {
        summary.samples.push({
          name,
          tag,
          hints,
          props: reactPropsSummary(props),
          stateNode: fiber.stateNode?.nodeType === Node.ELEMENT_NODE ? elementBrief(fiber.stateNode) : null,
        });
      }
      if (fiber.sibling) stack.push(fiber.sibling);
      if (fiber.child) stack.push(fiber.child);
    }
    return summary;
  }

  function currentConversationWrapperSnapshots(limit = 4) {
    const snapshots = [];
    const stack = [];
    const seenFibers = new Set();
    const seenConversations = new WeakSet();
    try {
      if (lastReactRoot?.current) stack.push(lastReactRoot.current);
      else if (lastReactRoot) stack.push(lastReactRoot);
    } catch {}
    while (stack.length && snapshots.length < limit && seenFibers.size < 5000) {
      const fiber = stack.pop();
      if (!fiber || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber);
      try {
        const props = fiber.memoizedProps;
        const conversation = props?.conversation;
        if (
          isConversationWrapper(conversation) &&
          !seenConversations.has(conversation)
        ) {
          seenConversations.add(conversation);
          snapshots.push({
            name: reactFiberName(fiber),
            tag: Number(fiber.tag),
            hints: reactFiberHints(fiber, reactFiberName(fiber), props),
            conversation: selectedPropValueDetail(conversation, {forceDeepConversationProbe: true}),
            subtree: conversationSubtreeSummary(fiber, 160),
          });
        }
      } catch (error) {
        snapshots.push({
          error: String(error?.name || error),
          message: String(error?.message || error).slice(0, 160),
        });
      }
      try {
        if (fiber.sibling) stack.push(fiber.sibling);
        if (fiber.child) stack.push(fiber.child);
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      visited: seenFibers.size,
      count: snapshots.length,
      snapshots,
    };
  }

  function collectionItems(value, limit = 8) {
    const items = [];
    if (!value) return items;
    try {
      if (Array.isArray(value)) {
        return value.slice(0, limit);
      }
      if (value instanceof Map) {
        for (const item of value.values()) {
          if (items.length >= limit) break;
          items.push(item);
        }
        return items;
      }
      if (value instanceof Set) {
        for (const item of value.values()) {
          if (items.length >= limit) break;
          items.push(item);
        }
        return items;
      }
      if (typeof value === 'object') {
        for (const key of Object.keys(value)) {
          if (items.length >= limit) break;
          items.push(value[key]);
        }
      }
    } catch {}
    return items;
  }

  function turnMaterializationSummary(turn) {
    const summary = valueShape(turn, 1);
    if (!turn || typeof turn !== 'object') return summary;
    try {
      if (typeof turn.id === 'string') summary.id = idShape(turn.id);
    } catch {}
    try {
      if (typeof turn.role === 'string') summary.role = turn.role;
    } catch {}
    try {
      const messages = Array.isArray(turn.messages) ? turn.messages : [];
      summary.messageCount = messages.length;
      summary.messageRoles = countBy(messages.slice(0, 12).map((message) => {
        try {
          return message?.author?.role || message?.author?.type || '';
        } catch {
          return '';
        }
      }));
      summary.messageStatuses = countBy(messages.slice(0, 12).map((message) => {
        try {
          return message?.status || '';
        } catch {
          return '';
        }
      }));
    } catch {}
    try {
      if (Array.isArray(turn.messageGroups)) {
        summary.messageGroupCount = turn.messageGroups.length;
        summary.messageGroupTypes = countBy(
          turn.messageGroups.slice(0, 12).map((group) => group?.type),
        );
      }
    } catch {}
    return summary;
  }

  function displayItemMaterializationSummary(item) {
    const summary = valueShape(item, 1);
    if (!item || typeof item !== 'object') return summary;
    try {
      if (typeof item.type === 'string') summary.type = item.type;
    } catch {}
    try {
      if (typeof item.id === 'string') summary.id = idShape(item.id);
    } catch {}
    try {
      if (item.turn) summary.turn = turnMaterializationSummary(item.turn);
    } catch {}
    return summary;
  }

  function conversationTreeMaterializationSummary(value) {
    if (!value || typeof value !== 'object' || !value.tree || typeof value.tree !== 'object') {
      return null;
    }
    const tree = value.tree;
    const summary = {
      resultShape: valueShape(value, 1),
      resultFields: semanticScalarFields(value),
      treeShape: valueShape(tree, 1),
    };
    try {
      if (typeof value.version === 'number') summary.version = value.version;
      if (typeof value._treeVersion === 'number') summary.treeVersion = value._treeVersion;
      if (typeof value.isLoading === 'boolean') summary.isLoading = value.isLoading;
    } catch {}
    let currentLeafId = '';
    try {
      currentLeafId = String(tree.currentLeafId || '');
      summary.currentLeafId = idShape(currentLeafId);
    } catch (error) {
      summary.currentLeafIdError = String(error?.name || error);
    }
    try {
      summary.nodeCount = mapLikeSize(tree.nodes);
    } catch (error) {
      summary.nodeCountError = String(error?.name || error);
    }
    try {
      if (typeof tree.getDisplayTurns === 'function') {
        const turns = tree.getDisplayTurns(currentLeafId);
        summary.displayTurnCount = mapLikeSize(turns);
        summary.displayTurnRoles = collectionItems(turns, 12).map((turn) => {
          try {
            return String(turn?.role || '');
          } catch {
            return '';
          }
        });
        summary.displayTurns = collectionItems(turns, 6).map(turnMaterializationSummary);
      }
    } catch (error) {
      summary.displayTurnsError = String(error?.name || error);
    }
    try {
      if (typeof tree.getDisplayItems === 'function') {
        const items = tree.getDisplayItems(currentLeafId);
        summary.displayItemCount = mapLikeSize(items);
        summary.displayItemTypes = collectionItems(items, 12).map((item) => {
          try {
            return String(item?.type || '');
          } catch {
            return '';
          }
        });
        summary.displayItems = collectionItems(items, 6).map(displayItemMaterializationSummary);
      }
    } catch (error) {
      summary.displayItemsError = String(error?.name || error);
    }
    return summary;
  }

  function conversationWrapperMaterializationProbe(limit = 4, options = {}) {
    const includeSubtree = options.includeSubtree !== false;
    const maxFibers = Number(options.maxFibers || 9000);
    const probes = [];
    const stack = [];
    const seenFibers = new Set();
    const seenConversations = new WeakSet();
    try {
      const rootFiber = lastReactRoot?.current || lastReactRoot || null;
      if (rootFiber) stack.push({fiber: rootFiber, depth: 0, source: 'root'});
    } catch {}
    try {
      const threadFiber = fiberForDomNode(document.getElementById('thread'));
      if (threadFiber) stack.push({fiber: threadFiber, depth: 0, source: 'thread'});
    } catch {}
    while (stack.length && probes.length < limit && seenFibers.size < maxFibers) {
      const {fiber, depth, source} = stack.pop();
      if (!fiber || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber);
      try {
        const props = fiber.memoizedProps;
        const conversation = props?.conversation;
        if (isConversationWrapper(conversation) && !seenConversations.has(conversation)) {
          seenConversations.add(conversation);
          const zeroArgResults = [];
          const keys = Reflect.ownKeys(conversation);
          for (let index = 0; index < keys.length && zeroArgResults.length < 16; index += 1) {
            const key = keys[index];
            let descriptor = null;
            try {
              descriptor = Object.getOwnPropertyDescriptor(conversation, key);
            } catch {}
            const fn = descriptor && 'value' in descriptor ? descriptor.value : null;
            if (typeof fn !== 'function' || fn.length !== 0) continue;
            const entry = {index, key: propKeyLabel(key), name: String(fn.name || '').slice(0, 80)};
            try {
              const result = fn.call(conversation);
              entry.resultShape = valueShape(result, 1);
              const materialized = conversationTreeMaterializationSummary(result);
              if (materialized) entry.materializedTree = materialized;
            } catch (error) {
              entry.error = String(error?.name || error);
            }
            zeroArgResults.push(entry);
          }
          probes.push({
            source,
            depth,
            name: reactFiberName(fiber),
            tag: Number(fiber.tag),
            tagLabel: reactFiberTagLabel(Number(fiber.tag)),
            hints: reactFiberHints(fiber, reactFiberName(fiber), props),
            path: fiberPathSummary(fiber, 10),
            conversation: conversationIdentityShape(conversation),
            zeroArgResults,
            subtree: includeSubtree ? conversationSubtreeSummary(fiber, 80) : undefined,
          });
        }
      } catch (error) {
        probes.push({
          source,
          depth,
          error: String(error?.name || error),
          message: String(error?.message || error).slice(0, 160),
        });
      }
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth, source});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1, source});
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      visited: seenFibers.size,
      count: probes.length,
      probes,
    };
  }

  function conversationMaterializationSignature(probe) {
    if (!probe || !Array.isArray(probe.probes)) return '';
    return probe.probes.map((item) => {
      const conversation = item.conversation || {};
      const trees = [];
      for (const result of item.zeroArgResults || []) {
        const tree = result.materializedTree;
        if (!tree) continue;
        trees.push([
          result.index,
          tree.version ?? '',
          tree.treeVersion ?? '',
          tree.isLoading ?? '',
          tree.displayTurnCount ?? '',
          (tree.displayTurnRoles || []).join(','),
          tree.displayItemCount ?? '',
        ].join(':'));
      }
      return [
        item.source,
        item.name,
        conversation.object || '',
        conversation.id || '',
        conversation.serverId || '',
        String(!!conversation.idMatchesServerId),
        trees.join(';'),
      ].join('/');
    }).join('|').slice(0, 2400);
  }

  function recordConversationMaterializationSnapshot(reason, options = {}) {
    let sample = null;
    try {
      sample = conversationWrapperMaterializationProbe(2, {
        includeSubtree: options.includeSubtree === true,
        maxFibers: options.maxFibers || 1200,
      });
      sample.reason = reason;
      sample.url = compactUrl(location.href);
      sample.signature = conversationMaterializationSignature(sample);
      const changed = sample.signature !== lastConversationMaterializationSignature;
      if (changed) {
        lastConversationMaterializationSignature = sample.signature;
      }
      pushConversationMaterializationSample(sample, {changed});
      if (changed || reason !== 'commit') {
        record('conversation-materialization', {
          reason,
          changed,
          count: sample.count,
          probes: sample.probes.slice(0, 2).map((probe) => ({
            source: probe.source,
            name: probe.name,
            conversation: probe.conversation,
            zeroArgResults: (probe.zeroArgResults || [])
              .filter((item) => item.materializedTree)
              .slice(0, 4)
              .map((item) => ({
                index: item.index,
                key: item.key,
                materializedTree: item.materializedTree,
              })),
          })),
        });
      }
    } catch (error) {
      record('conversation-materialization-error', {
        reason,
        name: String(error?.name || ''),
        message: String(error?.message || error).slice(0, 240),
      });
    }
    return sample;
  }

  function conversationMaterializationTraceState() {
    const current = recordConversationMaterializationSnapshot('snapshot', {
      includeSubtree: true,
      maxFibers: 9000,
    });
    return {
      ...conversationMaterializationStats,
      now: nowMs(),
      current,
      samples: conversationMaterializationSamples.slice(-24),
    };
  }

  function compactSelectedPropValue(key, value) {
    const detail = selectedPropValueDetail(value, {
      key,
      callZeroArg: String(key).endsWith('$') || key === 'when$',
    });
    try {
      if (typeof value === 'string' && /id|thread|conversation|turn|message/i.test(key)) {
        detail.id = idShape(value);
      }
    } catch {}
    return detail;
  }

  function compactFiberPropsForTrace(props) {
    if (!props || typeof props !== 'object') return {kind: typeof props};
    const selected = {};
    for (const key of [
      'conversation',
      'conversationId',
      'urlThreadId',
      'clientThreadId',
      'serverThreadId',
      'threadId',
      'turn',
      'turnId',
      'turnIndex',
      'turns',
      'message',
      'messages',
      'displayItems',
      'conversationTurns',
      'isThreadContentLoading',
      'isNewThread',
      'when$',
      'fallback',
      'renderEmptyState',
      'renderEmptyFooter',
      'hideComposer',
      'isComposerPinnedToBottomOnEmptyState',
      'isScrolledFromBottom$',
      'shouldUseUnifiedComposer',
      'isCompletionInProgress',
      'isGizmoThread',
      'isProjectThread',
      'layoutMode',
      'currentModelId',
      'pageLoadSearchQuery',
      'forceRenderedTurnId',
      'items',
      'children',
    ]) {
      if (Object.prototype.hasOwnProperty.call(props, key)) {
        selected[key] = compactSelectedPropValue(key, props[key]);
      }
    }
    return {
      keys: objectKeys(props, 24),
      id: propHintString(props.id),
      role: propHintString(props.role),
      testid: propHintString(props['data-testid']),
      ariaHidden: propHintString(props['aria-hidden']),
      inert: !!props.inert,
      dataTurn: propHintString(props['data-turn']),
      dataMessageId: props['data-message-id'] ? idShape(String(props['data-message-id'])) : undefined,
      dataMessageAuthorRole: propHintString(props['data-message-author-role']),
      selectedValueShapes: selected,
    };
  }

  function conversationSubtreeSummary(rootFiber, limit = 80) {
    const nodes = [];
    const stack = [];
    const seen = new Set();
    try {
      if (rootFiber?.child) stack.push({fiber: rootFiber.child, depth: 1});
    } catch {}
    while (stack.length && nodes.length < limit && seen.size < 2500) {
      const {fiber, depth} = stack.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber);
      const name = reactFiberName(fiber);
      let props = null;
      let hints = [];
      let stateNode = null;
      try {
        props = fiber.memoizedProps;
        hints = reactFiberHints(fiber, name, props);
        stateNode = fiber.stateNode?.nodeType === Node.ELEMENT_NODE
          ? elementBrief(fiber.stateNode)
          : null;
      } catch {}
      const stateNodeInteresting = !!stateNode && /thread|conversation|message|turn|composer|markdown|viewport|virtual|list/i.test([
        stateNode.id,
        stateNode.role,
        stateNode.testid,
        stateNode.dataTurn,
        ...(stateNode.classHints || []),
      ].join(' '));
      const propInteresting = hints.some((hint) => hint.startsWith('prop:'));
      if (
        propInteresting ||
        stateNodeInteresting ||
        /conversation|thread|turn|markdown|virtual|list|item/i.test(name)
      ) {
        const hookInteresting = propInteresting ||
          /^(NFe|BFe|e8|T2|qDr|\$Ar|XFe|pY|iyr)$/.test(name);
        nodes.push({
          depth,
          name,
          tag: Number(fiber.tag),
          hints,
          props: compactFiberPropsForTrace(props),
          sourceHint: shouldRecordFiberSourceHint(name, props) ? reactFiberSourceHint(fiber) : undefined,
          hooks: hookInteresting ? hookStateSummary(fiber, 8) : undefined,
          stateNode,
        });
      }
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1});
      } catch {}
    }
    return {
      visited: seen.size,
      count: nodes.length,
      nodes,
    };
  }

  function fiberForDomNode(node) {
    if (!node || typeof node !== 'object') return null;
    try {
      for (const key of Reflect.ownKeys(node)) {
        const label = String(key);
        if (label.startsWith('__reactFiber$') || label.startsWith('__reactInternalInstance$')) {
          return node[key] || null;
        }
      }
    } catch {}
    return null;
  }

  function reactFiberTagLabel(tag) {
    switch (Number(tag)) {
      case 0: return 'FunctionComponent';
      case 3: return 'HostRoot';
      case 5: return 'HostComponent';
      case 6: return 'HostText';
      case 7: return 'Fragment';
      case 10: return 'ContextProvider';
      case 11: return 'ForwardRef';
      case 13: return 'Suspense';
      case 14: return 'MemoComponent';
      case 15: return 'SimpleMemoComponent';
      case 22: return 'Offscreen';
      default: return `tag:${Number(tag)}`;
    }
  }

  function fiberNumericField(fiber, key) {
    try {
      const value = fiber?.[key];
      if (typeof value === 'number') return value;
      if (typeof value === 'bigint') return String(value);
    } catch {}
    return undefined;
  }

  function fiberInternalFieldSummary(fiber, key) {
    const summary = {};
    let value;
    try {
      value = fiber?.[key];
      summary.present = value !== null && value !== undefined;
      summary.shape = valueShape(value, 1);
    } catch (error) {
      summary.error = String(error?.name || error);
      return summary;
    }
    if (value && typeof value === 'object') {
      try {
        summary.ownValueShapes = ownValueShapes(value, 12);
      } catch (error) {
        summary.ownValueShapesError = String(error?.name || error);
      }
      try {
        summary.selectedValues = selectedNamedValueShapes(
          value,
          [/then/i, /wake/i, /lane/i, /cache/i, /retry/i, /transition/i, /pending/i, /base/i, /tree/i, /status/i, /value/i],
          12,
        );
      } catch (error) {
        summary.selectedValuesError = String(error?.name || error);
      }
    }
    return summary;
  }

  function suspenseChildChainSummary(startFiber, limit = 8) {
    const nodes = [];
    const seen = new Set();
    let current = startFiber || null;
    while (current && nodes.length < limit && !seen.has(current)) {
      seen.add(current);
      const name = reactFiberName(current);
      const tag = Number(current.tag);
      let props = null;
      let hints = [];
      let stateNode = null;
      try {
        props = current.memoizedProps;
        hints = reactFiberHints(current, name, props);
        stateNode = current.stateNode?.nodeType === Node.ELEMENT_NODE
          ? elementBrief(current.stateNode)
          : null;
      } catch {}
      nodes.push({
        index: nodes.length,
        name,
        tag,
        tagLabel: reactFiberTagLabel(tag),
        hints,
        lanes: fiberNumericField(current, 'lanes'),
        childLanes: fiberNumericField(current, 'childLanes'),
        flags: fiberNumericField(current, 'flags'),
        subtreeFlags: fiberNumericField(current, 'subtreeFlags'),
        memoizedState: tag === 13 || tag === 22
          ? fiberInternalFieldSummary(current, 'memoizedState')
          : undefined,
        props: compactFiberPropsForTrace(props),
        stateNode,
      });
      try {
        current = current.sibling || null;
      } catch {
        break;
      }
    }
    return nodes;
  }

  function fiberPathSummary(fiber, limit = 12) {
    const path = [];
    const seen = new Set();
    let current = fiber || null;
    while (current && path.length < limit && !seen.has(current)) {
      seen.add(current);
      const name = reactFiberName(current);
      const tag = Number(current.tag);
      path.push({
        name,
        tag,
        tagLabel: reactFiberTagLabel(tag),
      });
      try {
        current = current.return || null;
      } catch {
        break;
      }
    }
    return path.reverse();
  }

  function reactElementTypeSummary(type, props = null, depth = 0) {
    if (depth >= 3) return {kind: 'nested'};
    if (typeof type === 'string') return {kind: 'host', name: type};
    if (typeof type === 'function') {
      const name = String(type.displayName || type.name || '').slice(0, 80);
      return {
        kind: 'function',
        name,
        sourceHint: shouldRecordFiberSourceHint(name, props)
          ? Function.prototype.toString.call(type).replace(/\s+/g, ' ').slice(0, 2200)
          : undefined,
      };
    }
    if (!type || typeof type !== 'object') return valueShape(type, 1);
    const summary = {
      kind: 'object',
      tag: Object.prototype.toString.call(type),
      keys: objectKeys(type, 12),
      ownKeys: ownKeySummary(type, 12),
    };
    try {
      if (Object.prototype.hasOwnProperty.call(type, '_payload')) {
        const payload = type._payload;
        summary.lazyPayload = {
          shape: valueShape(payload, 1),
          ownValueShapes: ownValueShapes(payload, 8),
        };
        if (payload && typeof payload === 'object') {
          summary.lazyPayload.status = typeof payload._status === 'number'
            ? payload._status
            : undefined;
          summary.lazyPayload.result = valueShape(payload._result, 1);
          if (payload._result && typeof payload._result === 'object') {
            summary.lazyPayload.resultOwnValueShapes = ownValueShapes(payload._result, 8);
          }
        }
      }
      if (typeof type._init === 'function') {
        summary.hasLazyInit = true;
      }
      if (type.render) {
        summary.render = reactElementTypeSummary(type.render, props, depth + 1);
      }
      if (type.type) {
        summary.innerType = reactElementTypeSummary(type.type, props, depth + 1);
      }
    } catch (error) {
      summary.error = String(error?.name || error);
    }
    return summary;
  }

  function reactElementTreeSummary(value, depth = 0) {
    if (depth > 4) return {kind: 'nested'};
    if (Array.isArray(value)) {
      return {
        kind: 'array',
        length: value.length,
        items: value.slice(0, 6).map((item) => reactElementTreeSummary(item, depth + 1)),
      };
    }
    if (!value || typeof value !== 'object') return valueShape(value, 1);
    const maybeElement = Object.prototype.hasOwnProperty.call(value, '$$typeof') &&
      Object.prototype.hasOwnProperty.call(value, 'type') &&
      Object.prototype.hasOwnProperty.call(value, 'props');
    if (!maybeElement) return valueShape(value, 1);
    const props = value.props;
    const summary = {
      kind: 'react-element',
      key: value.key == null ? null : String(value.key).slice(0, 80),
      type: reactElementTypeSummary(value.type, props),
    };
    if (props && typeof props === 'object') {
      summary.propKeys = objectKeys(props, 16);
      const selected = {};
      for (const key of [
        'conversation',
        'isThreadContentLoading',
        'pageLoadSearchQuery',
        'conversationId',
        'clientThreadId',
        'serverThreadId',
        'threadId',
        'turnId',
      ]) {
        if (Object.prototype.hasOwnProperty.call(props, key)) {
          selected[key] = compactSelectedPropValue(key, props[key]);
        }
      }
      if (Object.keys(selected).length) summary.selectedProps = selected;
      if (Object.prototype.hasOwnProperty.call(props, 'children')) {
        summary.children = reactElementTreeSummary(props.children, depth + 1);
      }
      if (Object.prototype.hasOwnProperty.call(props, 'fallback')) {
        summary.fallback = reactElementTreeSummary(props.fallback, depth + 1);
      }
    }
    return summary;
  }

  function suspenseBoundaryInternalSummary(fiber) {
    let props = null;
    try {
      props = fiber?.memoizedProps || null;
    } catch {}
    return {
      mode: fiberNumericField(fiber, 'mode'),
      flags: fiberNumericField(fiber, 'flags'),
      subtreeFlags: fiberNumericField(fiber, 'subtreeFlags'),
      lanes: fiberNumericField(fiber, 'lanes'),
      childLanes: fiberNumericField(fiber, 'childLanes'),
      memoizedState: fiberInternalFieldSummary(fiber, 'memoizedState'),
      updateQueue: fiberInternalFieldSummary(fiber, 'updateQueue'),
      dependencies: fiberInternalFieldSummary(fiber, 'dependencies'),
      children: suspenseChildChainSummary(fiber?.child || null, 8),
      elementTree: reactElementTreeSummary(props?.children, 0),
      alternate: suspenseBoundaryAlternateSummary(fiber),
    };
  }

  function suspenseBoundaryAlternateSummary(fiber) {
    let alternate = null;
    try {
      alternate = fiber?.alternate || null;
    } catch {}
    if (!alternate) return {present: false};
    const tag = Number(alternate.tag);
    let props = null;
    try {
      props = alternate.memoizedProps || null;
    } catch {}
    return {
      present: true,
      name: reactFiberName(alternate),
      tag,
      tagLabel: reactFiberTagLabel(tag),
      mode: fiberNumericField(alternate, 'mode'),
      flags: fiberNumericField(alternate, 'flags'),
      subtreeFlags: fiberNumericField(alternate, 'subtreeFlags'),
      lanes: fiberNumericField(alternate, 'lanes'),
      childLanes: fiberNumericField(alternate, 'childLanes'),
      memoizedState: tag === 13 || tag === 22
        ? fiberInternalFieldSummary(alternate, 'memoizedState')
        : undefined,
      updateQueue: fiberInternalFieldSummary(alternate, 'updateQueue'),
      dependencies: fiberInternalFieldSummary(alternate, 'dependencies'),
      children: suspenseChildChainSummary(alternate.child || null, 8),
      elementTree: reactElementTreeSummary(props?.children, 0),
    };
  }

  function reactRootLaneSummary() {
    let rootFiber = null;
    let root = null;
    try {
      rootFiber = lastReactRoot?.current || lastReactRoot || null;
      root = rootFiber?.stateNode || null;
    } catch {}
    const summary = {
      rootPresent: !!rootFiber,
      stateNodePresent: !!root,
      rootFiberLanes: fiberNumericField(rootFiber, 'lanes'),
      rootFiberChildLanes: fiberNumericField(rootFiber, 'childLanes'),
    };
    if (!root || typeof root !== 'object') return summary;
    for (const key of [
      'pendingLanes',
      'suspendedLanes',
      'pingedLanes',
      'expiredLanes',
      'errorRecoveryDisabledLanes',
      'shellSuspendCounter',
      'entangledLanes',
      'finishedLanes',
    ]) {
      const value = fiberNumericField(root, key);
      if (value !== undefined) summary[key] = value;
    }
    try {
      if (root.entanglements) summary.entanglementsShape = valueShape(root.entanglements, 1);
      if (root.hiddenUpdates) summary.hiddenUpdatesShape = valueShape(root.hiddenUpdates, 1);
    } catch {}
    return summary;
  }

  function suspenseBoundaryProbes(limit = 24) {
    const boundaries = [];
    const stack = [];
    const seen = new Set();
    let threadFiber = null;
    try {
      const rootFiber = lastReactRoot?.current || lastReactRoot || null;
      if (rootFiber) stack.push({fiber: rootFiber, depth: 0, source: 'root'});
    } catch {}
    try {
      threadFiber = fiberForDomNode(document.getElementById('thread'));
      if (threadFiber) stack.push({fiber: threadFiber, depth: 0, source: 'thread'});
    } catch {}
    while (stack.length && boundaries.length < limit && seen.size < 8000) {
      const {fiber, depth, source} = stack.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber);
      const tag = Number(fiber.tag);
      if (tag === 13 || tag === 22) {
        const name = reactFiberName(fiber);
        let props = null;
        let hints = [];
        try {
          props = fiber.memoizedProps;
          hints = reactFiberHints(fiber, name, props);
        } catch {}
        boundaries.push({
          source,
          depth,
          name,
          tag,
          tagLabel: reactFiberTagLabel(tag),
          hints,
          path: fiberPathSummary(fiber, 12),
          props: compactFiberPropsForTrace(props),
          internal: suspenseBoundaryInternalSummary(fiber),
        });
      }
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth, source});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1, source});
      } catch {}
    }
    return {
      rootPresent: !!lastReactRoot,
      threadFiberPresent: !!threadFiber,
      rootLanes: reactRootLaneSummary(),
      visited: seen.size,
      count: boundaries.length,
      boundaries,
    };
  }

  function compactFiberForTrace(fiber, depth = 0) {
    if (!fiber) return null;
    let props = null;
    let stateNode = null;
    let name = '';
    let hints = [];
    try {
      name = reactFiberName(fiber);
      props = fiber.memoizedProps;
      hints = reactFiberHints(fiber, name, props);
      stateNode = fiber.stateNode?.nodeType === Node.ELEMENT_NODE
        ? elementBrief(fiber.stateNode)
        : null;
    } catch {}
    return {
      depth,
      name,
      tag: Number(fiber.tag),
      hints,
      props: compactFiberPropsForTrace(props),
      sourceHint: shouldRecordFiberSourceHint(name, props) ? reactFiberSourceHint(fiber) : undefined,
      hooks: (hints.some((hint) => hint.startsWith('prop:')) ||
        /^(NFe|BFe|e8|T2|qDr|\$Ar|XFe|pY|iyr)$/.test(name))
        ? hookStateSummary(fiber, 8)
        : undefined,
      stateNode,
    };
  }

  function fiberAncestorSummary(fiber, limit = 40) {
    const nodes = [];
    const seen = new Set();
    let current = fiber;
    while (current && nodes.length < limit && !seen.has(current)) {
      seen.add(current);
      nodes.push(compactFiberForTrace(current, nodes.length));
      try {
        current = current.return || null;
      } catch {
        break;
      }
    }
    return nodes;
  }

  function fiberDescendantSummary(rootFiber, limit = 120) {
    const nodes = [];
    const stack = [];
    const seen = new Set();
    try {
      if (rootFiber?.child) stack.push({fiber: rootFiber.child, depth: 1});
    } catch {}
    while (stack.length && nodes.length < limit && seen.size < 2500) {
      const {fiber, depth} = stack.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber);
      const compact = compactFiberForTrace(fiber, depth);
      const selected = compact?.props?.selectedValueShapes || {};
      const hasSelectedProps = Object.keys(selected).length > 0;
      const stateNode = compact?.stateNode;
      const stateNodeInteresting = !!stateNode && /thread|conversation|message|turn|composer|markdown|viewport|virtual|list/i.test([
        stateNode.id,
        stateNode.role,
        stateNode.testid,
        stateNode.dataTurn,
        ...(stateNode.classHints || []),
      ].join(' '));
      const name = compact?.name || '';
      if (
        hasSelectedProps ||
        stateNodeInteresting ||
        /conversation|thread|turn|markdown|virtual|list|item/i.test(name)
      ) {
        nodes.push(compact);
      }
      try {
        if (fiber.sibling) stack.push({fiber: fiber.sibling, depth});
        if (fiber.child) stack.push({fiber: fiber.child, depth: depth + 1});
      } catch {}
    }
    return {
      visited: seen.size,
      count: nodes.length,
      nodes,
    };
  }

  function reactThreadFiberSnapshot() {
    const thread = document.getElementById('thread');
    const fiber = fiberForDomNode(thread);
    return {
      thread: elementBrief(thread),
      fiberPresent: !!fiber,
      ancestors: fiberAncestorSummary(fiber, 36),
      subtree: fiberDescendantSummary(fiber, 160),
    };
  }

  function installReactCommitTrace() {
    const existing = window.__REACT_DEVTOOLS_GLOBAL_HOOK__;
    if (existing?.__lmChatGPTLiveTraceWrapped) {
      reactCommitStats.hookInstalled = true;
      return;
    }
    const renderers = existing?.renderers instanceof Map ? existing.renderers : new Map();
    let nextRendererId = renderers.size || 0;
    const nativeInject = typeof existing?.inject === 'function' ? existing.inject.bind(existing) : null;
    const nativeCommitRoot = typeof existing?.onCommitFiberRoot === 'function'
      ? existing.onCommitFiberRoot.bind(existing)
      : null;
    const nativeCommitUnmount = typeof existing?.onCommitFiberUnmount === 'function'
      ? existing.onCommitFiberUnmount.bind(existing)
      : null;
    const hook = {
      ...(existing || {}),
      supportsFiber: true,
      renderers,
      __lmChatGPTLiveTraceWrapped: true,
      inject(renderer) {
        let id;
        try {
          id = nativeInject ? nativeInject(renderer) : undefined;
        } catch (error) {
          record('react-devtools-inject-error', {name: error?.name || '', message: String(error?.message || error).slice(0, 200)});
        }
        if (typeof id !== 'number') id = ++nextRendererId;
        try { renderers.set(id, renderer); } catch {}
        reactCommitStats.rendererCount = renderers.size;
        reactCommitStats.lastRendererId = id;
        record('react-renderer-inject', {
          id,
          version: String(renderer?.version || '').slice(0, 80),
          packageName: String(renderer?.rendererPackageName || '').slice(0, 80),
        });
        return id;
      },
      onCommitFiberRoot(id, root, priorityLevel, didError) {
        reactCommitStats.commitCount += 1;
        reactCommitStats.lastCommitAt = nowMs();
        reactCommitStats.lastRendererId = Number(id) || reactCommitStats.lastRendererId;
        lastReactRoot = root || lastReactRoot;
        recordConversationIdentitySnapshot(root, 'commit');
        recordConversationMaterializationSnapshot('commit');
        try {
          const summary = summarizeReactCommitRoot(root);
          reactCommitStats.lastCommit = {
            id,
            didError: !!didError,
            priorityLevel: typeof priorityLevel === 'number' ? priorityLevel : undefined,
            ...summary,
          };
          if (
            summary.conversationHints ||
            summary.messageHints ||
            summary.turnHints ||
            summary.markdownHints ||
            summary.dataTurnProps ||
            summary.dataMessageProps ||
            summary.dataRoleProps
          ) {
            const sample = {time: nowMs(), id, ...summary};
            pushReactFiberSample(sample);
            record('react-commit', sample);
          }
        } catch (error) {
          reactCommitStats.commitErrors += 1;
          record('react-commit-trace-error', {
            name: error?.name || '',
            message: String(error?.message || error).slice(0, 240),
          });
        }
        if (nativeCommitRoot) {
          try {
            return nativeCommitRoot(id, root, priorityLevel, didError);
          } catch (error) {
            record('react-devtools-commit-error', {name: error?.name || '', message: String(error?.message || error).slice(0, 200)});
          }
        }
      },
      onCommitFiberUnmount(id, fiber) {
        if (nativeCommitUnmount) {
          try {
            return nativeCommitUnmount(id, fiber);
          } catch {}
        }
      },
      sub(event, fn) {
        try {
          return typeof existing?.sub === 'function' ? existing.sub(event, fn) : () => {};
        } catch {
          return () => {};
        }
      },
      on(event, fn) {
        try {
          return typeof existing?.on === 'function' ? existing.on(event, fn) : undefined;
        } catch {}
      },
      off(event, fn) {
        try {
          return typeof existing?.off === 'function' ? existing.off(event, fn) : undefined;
        } catch {}
      },
      emit(event, data) {
        try {
          return typeof existing?.emit === 'function' ? existing.emit(event, data) : undefined;
        } catch {}
      },
    };
    reactCommitStats.hookInstalled = true;
    reactCommitStats.hookPreexisting = !!existing;
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = hook;
  }

  function all(selectors) {
    for (const selector of selectors) {
      try {
        const found = [...document.querySelectorAll(selector)];
        if (found.length) return found;
      } catch {}
    }
    return [];
  }

  function conversationDomState() {
    const assistantTexts = all([
      '[data-message-author-role="assistant"]',
      '[data-testid*="assistant" i]',
      '[class*="assistant" i] .markdown',
      '.markdown',
    ]).map(textOf).filter(Boolean);
    const userTexts = all([
      '[data-message-author-role="user"]',
      '[data-testid*="user" i]',
    ]).map(textOf).filter(Boolean);
    const stopButtons = all([
      'button[data-testid*="stop" i]',
      'button[aria-label*="Stop" i]',
      'button[aria-label*="Cancel" i]',
    ]);
    return {
      url: compactUrl(location.href),
      readyState: document.readyState,
      assistantCount: assistantTexts.length,
      latestAssistantLen: assistantTexts.length ? assistantTexts[assistantTexts.length - 1].length : 0,
      userCount: userTexts.length,
      latestUserLen: userTexts.length ? userTexts[userTexts.length - 1].length : 0,
      stopButtonCount: stopButtons.length,
      bodyTextLen: textOf(document.body).length,
      selectorCensus: selectorCensus(),
      domTree: domTreeProbe(),
      appRuntime: appRuntimeState(),
      eventLoop: eventLoopState(),
      messageTasks: messageTaskState(),
      observerApis: observerApiState(),
      domMutations: domMutationState(),
      reactCommits: reactCommitState(),
    };
  }

  function recordDomState(reason, extra = {}) {
    const state = conversationDomState();
    const signature = JSON.stringify(state);
    if (signature === lastDomSignature && reason === 'mutation') return;
    lastDomSignature = signature;
    record('dom-state', {reason, ...extra, state});
  }

  function installMutationTrace() {
    const target = document.documentElement || document.body;
    if (!target || target.__lmChatGPTLiveTraceMutationObserver) return;
    const observer = new MutationObserver((mutations) => {
      let addedNodes = 0;
      let removedNodes = 0;
      let textMutations = 0;
      for (const mutation of mutations) {
        addedNodes += mutation.addedNodes ? mutation.addedNodes.length : 0;
        removedNodes += mutation.removedNodes ? mutation.removedNodes.length : 0;
        if (mutation.type === 'characterData') textMutations += 1;
      }
      recordDomState('mutation', {mutations: mutations.length, addedNodes, removedNodes, textMutations});
    });
    observer.observe(target, {subtree: true, childList: true, characterData: true});
    target.__lmChatGPTLiveTraceMutationObserver = observer;
    recordDomState('observer-start');
  }

  function installEventLoopTrace() {
    const nativeSetTimeout = window.setTimeout;
    const nativeSetInterval = window.setInterval;
    const nativeRequestAnimationFrame = window.requestAnimationFrame;
    const nativeRequestIdleCallback = window.requestIdleCallback;
    const nativeSchedulerPostTask = window.scheduler?.postTask;
    const nativeQueueMicrotask = window.queueMicrotask;
    if (typeof nativeSetTimeout === 'function') {
      window.setTimeout = function tracedSetTimeout(callback, delay, ...args) {
        eventLoopStats.timeoutScheduled += 1;
        if (typeof callback !== 'function') return nativeSetTimeout.call(this, callback, delay, ...args);
        return nativeSetTimeout.call(this, function tracedTimeoutCallback(...callbackArgs) {
          eventLoopStats.timeoutFired += 1;
          eventLoopStats.lastTimeoutFiredAt = nowMs();
          return callback.apply(this, callbackArgs);
        }, delay, ...args);
      };
    }
    if (typeof nativeSetInterval === 'function') {
      window.setInterval = function tracedSetInterval(callback, delay, ...args) {
        eventLoopStats.intervalScheduled += 1;
        if (typeof callback !== 'function') return nativeSetInterval.call(this, callback, delay, ...args);
        return nativeSetInterval.call(this, function tracedIntervalCallback(...callbackArgs) {
          eventLoopStats.intervalFired += 1;
          eventLoopStats.lastIntervalFiredAt = nowMs();
          return callback.apply(this, callbackArgs);
        }, delay, ...args);
      };
      nativeSetInterval.call(window, () => {
        eventLoopStats.heartbeat += 1;
        eventLoopStats.lastHeartbeatAt = nowMs();
      }, 1000);
    }
    if (typeof nativeRequestAnimationFrame === 'function') {
      window.requestAnimationFrame = function tracedRequestAnimationFrame(callback) {
        eventLoopStats.rafScheduled += 1;
        if (typeof callback !== 'function') return nativeRequestAnimationFrame.call(this, callback);
        return nativeRequestAnimationFrame.call(this, function tracedAnimationFrameCallback(timestamp) {
          eventLoopStats.rafFired += 1;
          eventLoopStats.lastRafFiredAt = nowMs();
          return callback.call(this, timestamp);
        });
      };
    }
    if (typeof nativeRequestIdleCallback === 'function') {
      window.requestIdleCallback = function tracedRequestIdleCallback(callback, options) {
        eventLoopStats.idleScheduled += 1;
        if (typeof callback !== 'function') return nativeRequestIdleCallback.call(this, callback, options);
        return nativeRequestIdleCallback.call(this, function tracedIdleCallback(deadline) {
          eventLoopStats.idleFired += 1;
          eventLoopStats.lastIdleFiredAt = nowMs();
          return callback.call(this, deadline);
        }, options);
      };
    }
    if (typeof nativeSchedulerPostTask === 'function') {
      window.scheduler.postTask = function tracedSchedulerPostTask(callback, options) {
        eventLoopStats.schedulerPostTaskScheduled += 1;
        const result = nativeSchedulerPostTask.call(this, callback, options);
        if (result?.then) {
          result.then(
            () => {
              eventLoopStats.schedulerPostTaskSettled += 1;
              eventLoopStats.lastSchedulerPostTaskSettledAt = nowMs();
            },
            () => {
              eventLoopStats.schedulerPostTaskSettled += 1;
              eventLoopStats.lastSchedulerPostTaskSettledAt = nowMs();
            },
          );
        }
        return result;
      };
    }
    if (typeof nativeQueueMicrotask === 'function') {
      window.queueMicrotask = function tracedQueueMicrotask(callback) {
        eventLoopStats.microtaskScheduled += 1;
        if (typeof callback !== 'function') return nativeQueueMicrotask.call(this, callback);
        return nativeQueueMicrotask.call(this, function tracedMicrotaskCallback() {
          eventLoopStats.microtaskFired += 1;
          eventLoopStats.lastMicrotaskFiredAt = nowMs();
          return callback.call(this);
        });
      };
    }
  }

  function installHistoryTrace() {
    for (const name of ['pushState', 'replaceState']) {
      const nativeMethod = history[name];
      if (typeof nativeMethod !== 'function') continue;
      history[name] = function tracedHistoryMethod(state, title, url) {
        const result = nativeMethod.apply(this, arguments);
        record('history', {method: name, url: compactUrl(url || location.href), stateKind: typeof state});
        recordDomState(`history-${name}`);
        return result;
      };
    }
    window.addEventListener('popstate', () => {
      record('history', {method: 'popstate', url: compactUrl(location.href)});
      recordDomState('history-popstate');
    });
    window.addEventListener('hashchange', () => {
      record('history', {method: 'hashchange', url: compactUrl(location.href)});
      recordDomState('history-hashchange');
    });
  }

  function navigationEntrySummary(entry) {
    if (!entry || typeof entry !== 'object') return null;
    const summary = {};
    try {
      summary.url = compactUrl(entry.url || '');
    } catch {}
    for (const key of ['key', 'id', 'index', 'sameDocument']) {
      try {
        const value = entry[key];
        if (typeof value === 'string') summary[key] = idShape(value);
        else if (typeof value === 'number' || typeof value === 'boolean') summary[key] = value;
      } catch {}
    }
    try {
      const state = typeof entry.getState === 'function' ? entry.getState() : entry.state;
      summary.stateShape = valueShape(state, 1);
    } catch (error) {
      summary.stateError = String(error?.name || error);
    }
    return summary;
  }

  function navigationTraceState() {
    const nav = window.navigation;
    const currentEntry = nav && typeof nav === 'object' ? nav.currentEntry : null;
    return {
      ...navigationTraceStats,
      present: !!nav,
      canGoBack: !!nav?.canGoBack,
      canGoForward: !!nav?.canGoForward,
      currentEntry: navigationEntrySummary(currentEntry),
      sampleCount: navigationTraceSamples.length,
      samples: navigationTraceSamples.slice(-40),
    };
  }

  function installNavigateEventInterceptTrace(event) {
    if (!event || event.__lmChatGPTLiveTraceInterceptWrapped) return;
    const nativeIntercept = event.intercept;
    if (typeof nativeIntercept !== 'function') return;
    try {
      event.intercept = function tracedNavigateEventIntercept(options = {}) {
        navigationTraceStats.interceptCalls += 1;
        navigationTraceStats.lastInterceptAt = nowMs();
        const sample = {
          op: 'intercept-call',
          navigationType: String(event.navigationType || ''),
          optionsKeys: options && typeof options === 'object' ? objectKeys(options, 12) : [],
          currentEntry: navigationEntrySummary(window.navigation?.currentEntry),
          destination: navigationEntrySummary(event.destination),
        };
        pushNavigationTraceSample(sample);
        record('navigation-api', sample);
        let nextOptions = options;
        try {
          if (options && typeof options === 'object' && typeof options.handler === 'function') {
            const nativeHandler = options.handler;
            nextOptions = {
              ...options,
              handler(...handlerArgs) {
                navigationTraceStats.interceptHandlerStarted += 1;
                navigationTraceStats.lastInterceptHandlerAt = nowMs();
                record('navigation-api', {
                  op: 'intercept-handler-start',
                  navigationType: String(event.navigationType || ''),
                });
                let result;
                try {
                  result = nativeHandler.apply(this, handlerArgs);
                } catch (error) {
                  navigationTraceStats.interceptHandlerRejected += 1;
                  record('navigation-api', {
                    op: 'intercept-handler-throw',
                    name: String(error?.name || ''),
                    message: String(error?.message || error).slice(0, 240),
                  });
                  throw error;
                }
                if (result?.then) {
                  result.then(
                    () => {
                      navigationTraceStats.interceptHandlerSettled += 1;
                      navigationTraceStats.lastInterceptHandlerAt = nowMs();
                      record('navigation-api', {op: 'intercept-handler-resolved'});
                      recordAppState('navigation-intercept-handler-resolved');
                    },
                    (error) => {
                      navigationTraceStats.interceptHandlerRejected += 1;
                      navigationTraceStats.lastInterceptHandlerAt = nowMs();
                      record('navigation-api', {
                        op: 'intercept-handler-rejected',
                        name: String(error?.name || ''),
                        message: String(error?.message || error).slice(0, 240),
                      });
                      recordAppState('navigation-intercept-handler-rejected');
                    },
                  );
                } else {
                  navigationTraceStats.interceptHandlerSettled += 1;
                  navigationTraceStats.lastInterceptHandlerAt = nowMs();
                  record('navigation-api', {op: 'intercept-handler-return'});
                  recordAppState('navigation-intercept-handler-return');
                }
                return result;
              },
            };
          }
        } catch {}
        return nativeIntercept.call(this, nextOptions);
      };
      Object.defineProperty(event, '__lmChatGPTLiveTraceInterceptWrapped', {
        value: true,
        configurable: true,
      });
    } catch (error) {
      pushNavigationTraceSample({
        op: 'intercept-wrap-error',
        name: String(error?.name || ''),
        message: String(error?.message || error).slice(0, 240),
      });
    }
  }

  function installNavigationApiTrace() {
    const nav = window.navigation;
    if (!nav || typeof nav !== 'object') return;
    navigationTraceStats.present = true;
    if (!nav.__lmChatGPTLiveTraceNavigationWrapped) {
      const nativeNavigate = nav.navigate;
      if (typeof nativeNavigate === 'function') {
        try {
          nav.navigate = function tracedNavigationNavigate(url, options) {
            navigationTraceStats.navigateCalls += 1;
            navigationTraceStats.lastNavigateAt = nowMs();
            const sample = {
              op: 'navigate-call',
              url: compactUrl(url || ''),
              optionsKeys: options && typeof options === 'object' ? objectKeys(options, 12) : [],
              currentEntry: navigationEntrySummary(nav.currentEntry),
            };
            pushNavigationTraceSample(sample);
            record('navigation-api', sample);
            let result;
            try {
              result = nativeNavigate.apply(this, arguments);
            } catch (error) {
              navigationTraceStats.navigateErrors += 1;
              const errorSample = {
                op: 'navigate-throw',
                name: String(error?.name || ''),
                message: String(error?.message || error).slice(0, 240),
              };
              pushNavigationTraceSample(errorSample);
              record('navigation-api', errorSample);
              throw error;
            }
            if (result?.committed?.then) {
              result.committed.then(
                (entry) => {
                  navigationTraceStats.navigateCommitted += 1;
                  const commitSample = {
                    op: 'navigate-committed',
                    entry: navigationEntrySummary(entry),
                    currentEntry: navigationEntrySummary(nav.currentEntry),
                  };
                  pushNavigationTraceSample(commitSample);
                  record('navigation-api', commitSample);
                  recordAppState('navigation-committed');
                },
                (error) => {
                  navigationTraceStats.navigateCommitRejected += 1;
                  const rejectSample = {
                    op: 'navigate-commit-rejected',
                    name: String(error?.name || ''),
                    message: String(error?.message || error).slice(0, 240),
                  };
                  pushNavigationTraceSample(rejectSample);
                  record('navigation-api', rejectSample);
                },
              );
            }
            if (result?.finished?.then) {
              result.finished.then(
                (entry) => {
                  navigationTraceStats.navigateFinished += 1;
                  const finishSample = {
                    op: 'navigate-finished',
                    entry: navigationEntrySummary(entry),
                    currentEntry: navigationEntrySummary(nav.currentEntry),
                  };
                  pushNavigationTraceSample(finishSample);
                  record('navigation-api', finishSample);
                  recordAppState('navigation-finished');
                },
                (error) => {
                  navigationTraceStats.navigateFinishRejected += 1;
                  const rejectSample = {
                    op: 'navigate-finish-rejected',
                    name: String(error?.name || ''),
                    message: String(error?.message || error).slice(0, 240),
                  };
                  pushNavigationTraceSample(rejectSample);
                  record('navigation-api', rejectSample);
                },
              );
            }
            return result;
          };
        } catch (error) {
          pushNavigationTraceSample({
            op: 'navigate-wrap-error',
            name: String(error?.name || ''),
            message: String(error?.message || error).slice(0, 240),
          });
        }
      }
      try {
        Object.defineProperty(nav, '__lmChatGPTLiveTraceNavigationWrapped', {
          value: true,
          configurable: true,
        });
      } catch {}
    }
    for (const type of ['navigate', 'currententrychange', 'navigatesuccess', 'navigateerror']) {
      try {
        nav.addEventListener(type, (event) => {
          navigationTraceStats.lastEventAt = nowMs();
          if (type === 'navigate') navigationTraceStats.navigateEvents += 1;
          if (type === 'currententrychange') navigationTraceStats.currentEntryChanges += 1;
          if (type === 'navigatesuccess') navigationTraceStats.navigateSuccess += 1;
          if (type === 'navigateerror') navigationTraceStats.navigateError += 1;
          if (type === 'navigate') installNavigateEventInterceptTrace(event);
          const sample = {
            op: type,
            navigationType: String(event.navigationType || ''),
            canIntercept: !!event.canIntercept,
            hashChange: !!event.hashChange,
            userInitiated: !!event.userInitiated,
            destination: navigationEntrySummary(event.destination),
            from: navigationEntrySummary(event.from),
            currentEntry: navigationEntrySummary(nav.currentEntry),
          };
          pushNavigationTraceSample(sample);
          record('navigation-api', sample);
          recordAppState(`navigation-${type}`);
        });
      } catch {}
    }
  }

  function looksLikeThreadId(value) {
    if (typeof value !== 'string') return false;
    if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)) {
      return true;
    }
    return /^[0-9a-f]{40}$/i.test(value);
  }

  function idMapTraceState() {
    return {
      ...idMapTraceStats,
      sampleCount: idMapTraceSamples.length,
      samples: idMapTraceSamples.slice(-40),
    };
  }

  function installIdMapTrace() {
    const proto = Map.prototype;
    if (proto.__lmChatGPTLiveTraceIdMapWrapped) return;
    const nativeSet = proto.set;
    const nativeGet = proto.get;
    if (typeof nativeSet === 'function') {
      proto.set = function tracedMapSet(key, value) {
        if (looksLikeThreadId(key)) {
          idMapTraceStats.set += 1;
          idMapTraceStats.setThreadKey += 1;
          idMapTraceStats.lastSetAt = nowMs();
          const sample = {
            op: 'set',
            key: idShape(key),
            value: looksLikeThreadId(value) ? idShape(value) : undefined,
            valueShape: valueShape(value, 1),
            valueFields: semanticScalarFields(value),
            keyLength: key.length,
            valueLength: typeof value === 'string' ? value.length : undefined,
            mapSizeBefore: typeof this?.size === 'number' ? this.size : undefined,
          };
          pushIdMapTraceSample(sample);
          record('id-map-set', sample);
        } else if (looksLikeThreadId(value)) {
          idMapTraceStats.set += 1;
          idMapTraceStats.lastSetAt = nowMs();
          const sample = {
            op: 'set-value-id',
            keyShape: valueShape(key, 1),
            value: idShape(value),
            valueLength: value.length,
            mapSizeBefore: typeof this?.size === 'number' ? this.size : undefined,
          };
          pushIdMapTraceSample(sample);
          record('id-map-set', sample);
        }
        return nativeSet.apply(this, arguments);
      };
    }
    if (typeof nativeGet === 'function') {
      proto.get = function tracedMapGet(key) {
        const result = nativeGet.apply(this, arguments);
        if (looksLikeThreadId(key)) {
          idMapTraceStats.get += 1;
          idMapTraceStats.lastGetAt = nowMs();
          const stringHit = looksLikeThreadId(result);
          const present = result !== undefined;
          if (stringHit) idMapTraceStats.getHit += 1;
          else idMapTraceStats.getMiss += 1;
          if (present) idMapTraceStats.getPresent += 1;
          else idMapTraceStats.getUndefined += 1;
          if (present || idMapTraceSamples.length < 20) {
            const sample = {
              op: 'get',
              key: idShape(key),
              result: stringHit ? idShape(result) : undefined,
              stringHit,
              present,
              resultShape: valueShape(result, 1),
              resultFields: semanticScalarFields(result),
              keyLength: key.length,
              resultLength: typeof result === 'string' ? result.length : undefined,
              mapSize: typeof this?.size === 'number' ? this.size : undefined,
            };
            pushIdMapTraceSample(sample);
            record('id-map-get', sample);
          }
        }
        return result;
      };
    }
    Object.defineProperty(proto, '__lmChatGPTLiveTraceIdMapWrapped', {
      value: true,
      configurable: true,
    });
  }

  function summarizeData(data) {
    function summarizeJsonValue(value, depth = 0) {
      if (depth > 2) return {kind: 'nested'};
      if (Array.isArray(value)) {
        return {
          kind: 'array',
          length: value.length,
          items: value.slice(0, 4).map((item) => summarizeJsonValue(item, depth + 1)),
        };
      }
      if (value && typeof value === 'object') {
        const keys = Object.keys(value).slice(0, 12);
        const safeFields = {};
        for (const key of ['type', 'event', 'kind', 'action', 'status', 'state', 'phase', 'message_type']) {
          const field = value[key];
          if (typeof field === 'string' || typeof field === 'number' || typeof field === 'boolean') {
            safeFields[key] = field;
          }
        }
        return {kind: 'object', keys, fields: safeFields};
      }
      if (typeof value === 'string') return {kind: 'string', length: value.length};
      return {kind: typeof value};
    }

    if (typeof data === 'string') {
      const summary = {kind: 'string', length: data.length};
      try {
        const parsed = JSON.parse(data);
        if (parsed && typeof parsed === 'object') {
          summary.json = summarizeJsonValue(parsed);
          summary.jsonKeys = Object.keys(parsed).slice(0, 8);
          if (typeof parsed.type === 'string') summary.jsonType = parsed.type;
          if (typeof parsed.event === 'string') summary.jsonEvent = parsed.event;
        }
      } catch {}
      return summary;
    }
    if (data instanceof ArrayBuffer) return {kind: 'arraybuffer', byteLength: data.byteLength};
    if (ArrayBuffer.isView(data)) return {kind: data.constructor?.name || 'typedarray', byteLength: data.byteLength};
    if (typeof Blob !== 'undefined' && data instanceof Blob) {
      return {kind: 'blob', size: data.size, type: data.type || ''};
    }
    return {kind: typeof data};
  }

  function installMessageTaskTrace() {
    const NativeMessageChannel = window.MessageChannel;
    if (typeof NativeMessageChannel === 'function') {
      function wrapPort(port, portName) {
        if (!port || port.__lmChatGPTLiveTraceWrapped) return port;
        try {
          Object.defineProperty(port, '__lmChatGPTLiveTraceWrapped', {value: true});
        } catch {}
        try {
          const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(port), 'onmessage');
          if (descriptor?.set && descriptor?.get) {
            Object.defineProperty(port, 'onmessage', {
              configurable: true,
              enumerable: descriptor.enumerable,
              get() {
                return descriptor.get.call(this);
              },
              set(callback) {
                messageTaskStats.messagePortOnmessageSet += 1;
                messageTaskStats.lastMessagePortOnmessageSetAt = nowMs();
                record('message-port-onmessage-set', {port: portName, callbackKind: typeof callback});
                if (typeof callback !== 'function') return descriptor.set.call(this, callback);
                const wrapped = function tracedPortOnmessage(event) {
                  messageTaskStats.messagePortOnmessageFired += 1;
                  messageTaskStats.lastMessagePortOnmessageFiredAt = nowMs();
                  record('message-port-onmessage-fired', {port: portName, data: summarizeData(event?.data)});
                  recordAppState('message-port-onmessage-fired');
                  return callback.apply(this, arguments);
                };
                return descriptor.set.call(this, wrapped);
              },
            });
          }
        } catch {}
        try {
          const nativeAddEventListener = port.addEventListener;
          const nativeRemoveEventListener = port.removeEventListener;
          const listenerWrappers = new WeakMap();
          if (typeof nativeAddEventListener === 'function') {
            port.addEventListener = function tracedPortAddEventListener(type, listener) {
              if (String(type) !== 'message' || typeof listener !== 'function') {
                return nativeAddEventListener.apply(this, arguments);
              }
              messageTaskStats.messagePortListenerAdded += 1;
              messageTaskStats.lastMessagePortListenerAddedAt = nowMs();
              record('message-port-listener-add', {port: portName});
              let wrapped = listenerWrappers.get(listener);
              if (!wrapped) {
                wrapped = function tracedMessagePortListener(event) {
                  messageTaskStats.messagePortListenerFired += 1;
                  messageTaskStats.lastMessagePortListenerFiredAt = nowMs();
                  record('message-port-listener-fired', {port: portName, data: summarizeData(event?.data)});
                  recordAppState('message-port-listener-fired');
                  return listener.apply(this, arguments);
                };
                listenerWrappers.set(listener, wrapped);
              }
              return nativeAddEventListener.call(this, type, wrapped, arguments[2]);
            };
          }
          if (typeof nativeRemoveEventListener === 'function') {
            port.removeEventListener = function tracedPortRemoveEventListener(type, listener) {
              const wrapped = typeof listener === 'function' ? listenerWrappers.get(listener) : undefined;
              return nativeRemoveEventListener.call(this, type, wrapped || listener, arguments[2]);
            };
          }
        } catch {}
        try {
          const nativePostMessage = port.postMessage;
          if (typeof nativePostMessage === 'function') {
            port.postMessage = function tracedPortPostMessage(message) {
              messageTaskStats.messagePortPostMessage += 1;
              messageTaskStats.lastMessagePortPostMessageAt = nowMs();
              record('message-port-post', {port: portName, data: summarizeData(message)});
              return nativePostMessage.apply(this, arguments);
            };
          }
        } catch {}
        try {
          const nativeStart = port.start;
          if (typeof nativeStart === 'function') {
            port.start = function tracedPortStart() {
              messageTaskStats.messagePortStart += 1;
              return nativeStart.apply(this, arguments);
            };
          }
        } catch {}
        try {
          port.addEventListener?.('message', (event) => {
            messageTaskStats.messagePortMessage += 1;
            messageTaskStats.lastMessagePortMessageAt = nowMs();
            record('message-port-message', {port: portName, data: summarizeData(event.data)});
            recordAppState('message-port-message');
          });
        } catch {}
        return port;
      }

      function TracedMessageChannel() {
        messageTaskStats.messageChannelConstructed += 1;
        messageTaskStats.lastMessageChannelConstructedAt = nowMs();
        record('message-channel-create');
        const channel = new NativeMessageChannel();
        wrapPort(channel.port1, 'port1');
        wrapPort(channel.port2, 'port2');
        return channel;
      }
      TracedMessageChannel.prototype = NativeMessageChannel.prototype;
      Object.setPrototypeOf(TracedMessageChannel, NativeMessageChannel);
      window.MessageChannel = TracedMessageChannel;
    }

    const nativeWindowPostMessage = window.postMessage;
    if (typeof nativeWindowPostMessage === 'function') {
      window.postMessage = function tracedWindowPostMessage(message, targetOrigin) {
        messageTaskStats.windowPostMessage += 1;
        messageTaskStats.lastWindowPostMessageAt = nowMs();
        record('window-post-message', {
          targetOrigin: String(targetOrigin || '').slice(0, 120),
          data: summarizeData(message),
        });
        return nativeWindowPostMessage.apply(this, arguments);
      };
      window.addEventListener('message', (event) => {
        messageTaskStats.windowMessage += 1;
        messageTaskStats.lastWindowMessageAt = nowMs();
        record('window-message', {
          origin: String(event.origin || '').slice(0, 120),
          data: summarizeData(event.data),
        });
        recordAppState('window-message');
      });
    }
  }

  function resizeEntryBrief(entry) {
    const rect = entry?.contentRect;
    return {
      target: elementBrief(entry?.target),
      contentRect: rect
        ? {
            width: Math.round(Number(rect.width) || 0),
            height: Math.round(Number(rect.height) || 0),
          }
        : null,
      borderBoxSize: Array.isArray(entry?.borderBoxSize)
        ? entry.borderBoxSize.slice(0, 2).map((box) => ({
            inlineSize: Math.round(Number(box?.inlineSize) || 0),
            blockSize: Math.round(Number(box?.blockSize) || 0),
          }))
        : undefined,
    };
  }

  function intersectionEntryBrief(entry) {
    return {
      target: elementBrief(entry?.target),
      isIntersecting: !!entry?.isIntersecting,
      intersectionRatio: Number(entry?.intersectionRatio || 0),
    };
  }

  function installObserverApiTrace() {
    const NativeResizeObserver = window.ResizeObserver;
    if (typeof NativeResizeObserver === 'function' && !NativeResizeObserver.__lmChatGPTLiveTraceWrapped) {
      function TracedResizeObserver(callback) {
        observerApiStats.resizeConstructed += 1;
        record('resize-observer-create');
        const wrapped = function tracedResizeObserverCallback(entries, observer) {
          observerApiStats.resizeCallback += 1;
          observerApiStats.resizeEntryCount += entries?.length || 0;
          observerApiStats.lastResizeCallbackAt = nowMs();
          record('resize-observer-callback', {
            count: entries?.length || 0,
            entries: [...entries || []].slice(0, 8).map(resizeEntryBrief),
          });
          recordAppState('resize-observer-callback');
          return callback.apply(this, arguments);
        };
        return new NativeResizeObserver(wrapped);
      }
      TracedResizeObserver.prototype = NativeResizeObserver.prototype;
      Object.setPrototypeOf(TracedResizeObserver, NativeResizeObserver);
      TracedResizeObserver.__lmChatGPTLiveTraceWrapped = true;
      window.ResizeObserver = TracedResizeObserver;

      const nativeObserve = NativeResizeObserver.prototype.observe;
      if (typeof nativeObserve === 'function' && !nativeObserve.__lmChatGPTLiveTraceWrapped) {
        NativeResizeObserver.prototype.observe = function tracedResizeObserverObserve(target, options) {
          observerApiStats.resizeObserve += 1;
          record('resize-observer-observe', {target: elementBrief(target), box: String(options?.box || '')});
          return nativeObserve.apply(this, arguments);
        };
        NativeResizeObserver.prototype.observe.__lmChatGPTLiveTraceWrapped = true;
      }
      const nativeUnobserve = NativeResizeObserver.prototype.unobserve;
      if (typeof nativeUnobserve === 'function' && !nativeUnobserve.__lmChatGPTLiveTraceWrapped) {
        NativeResizeObserver.prototype.unobserve = function tracedResizeObserverUnobserve(target) {
          observerApiStats.resizeUnobserve += 1;
          return nativeUnobserve.apply(this, arguments);
        };
        NativeResizeObserver.prototype.unobserve.__lmChatGPTLiveTraceWrapped = true;
      }
      const nativeDisconnect = NativeResizeObserver.prototype.disconnect;
      if (typeof nativeDisconnect === 'function' && !nativeDisconnect.__lmChatGPTLiveTraceWrapped) {
        NativeResizeObserver.prototype.disconnect = function tracedResizeObserverDisconnect() {
          observerApiStats.resizeDisconnect += 1;
          return nativeDisconnect.apply(this, arguments);
        };
        NativeResizeObserver.prototype.disconnect.__lmChatGPTLiveTraceWrapped = true;
      }
    }

    const NativeIntersectionObserver = window.IntersectionObserver;
    if (typeof NativeIntersectionObserver === 'function' && !NativeIntersectionObserver.__lmChatGPTLiveTraceWrapped) {
      function TracedIntersectionObserver(callback, options) {
        observerApiStats.intersectionConstructed += 1;
        record('intersection-observer-create', {
          root: elementBrief(options?.root),
          rootMargin: String(options?.rootMargin || ''),
        });
        const wrapped = function tracedIntersectionObserverCallback(entries, observer) {
          observerApiStats.intersectionCallback += 1;
          observerApiStats.intersectionEntryCount += entries?.length || 0;
          observerApiStats.lastIntersectionCallbackAt = nowMs();
          record('intersection-observer-callback', {
            count: entries?.length || 0,
            entries: [...entries || []].slice(0, 8).map(intersectionEntryBrief),
          });
          recordAppState('intersection-observer-callback');
          return callback.apply(this, arguments);
        };
        return new NativeIntersectionObserver(wrapped, options);
      }
      TracedIntersectionObserver.prototype = NativeIntersectionObserver.prototype;
      Object.setPrototypeOf(TracedIntersectionObserver, NativeIntersectionObserver);
      TracedIntersectionObserver.__lmChatGPTLiveTraceWrapped = true;
      window.IntersectionObserver = TracedIntersectionObserver;

      const nativeObserve = NativeIntersectionObserver.prototype.observe;
      if (typeof nativeObserve === 'function' && !nativeObserve.__lmChatGPTLiveTraceWrapped) {
        NativeIntersectionObserver.prototype.observe = function tracedIntersectionObserverObserve(target) {
          observerApiStats.intersectionObserve += 1;
          record('intersection-observer-observe', {target: elementBrief(target)});
          return nativeObserve.apply(this, arguments);
        };
        NativeIntersectionObserver.prototype.observe.__lmChatGPTLiveTraceWrapped = true;
      }
      const nativeUnobserve = NativeIntersectionObserver.prototype.unobserve;
      if (typeof nativeUnobserve === 'function' && !nativeUnobserve.__lmChatGPTLiveTraceWrapped) {
        NativeIntersectionObserver.prototype.unobserve = function tracedIntersectionObserverUnobserve(target) {
          observerApiStats.intersectionUnobserve += 1;
          return nativeUnobserve.apply(this, arguments);
        };
        NativeIntersectionObserver.prototype.unobserve.__lmChatGPTLiveTraceWrapped = true;
      }
      const nativeDisconnect = NativeIntersectionObserver.prototype.disconnect;
      if (typeof nativeDisconnect === 'function' && !nativeDisconnect.__lmChatGPTLiveTraceWrapped) {
        NativeIntersectionObserver.prototype.disconnect = function tracedIntersectionObserverDisconnect() {
          observerApiStats.intersectionDisconnect += 1;
          return nativeDisconnect.apply(this, arguments);
        };
        NativeIntersectionObserver.prototype.disconnect.__lmChatGPTLiveTraceWrapped = true;
      }
    }
  }

  function recordInterestingInsert(kind, parent, child) {
    if (!isInterestingDomNode(parent) && !isInterestingDomNode(child)) return;
    domMutationStats.interestingInserts += 1;
    domMutationStats.lastInterestingMutationAt = nowMs();
    const sample = {
      time: nowMs(),
      kind,
      parent: nodeBrief(parent),
      child: nodeBrief(child),
    };
    pushDomMutationSample(sample);
    record('dom-insert', sample);
  }

  function installDomMutationTrace() {
    const NativeMutationObserver = window.MutationObserver;
    if (typeof NativeMutationObserver === 'function' && !window.__lmChatGPTLiveTraceMutationObserver) {
      try {
        const observer = new NativeMutationObserver((records) => {
          domMutationStats.mutationObserverRecords += records.length;
          const interesting = records.filter((record) => {
            const childHit = [...record.addedNodes || [], ...record.removedNodes || []].some(isInterestingDomNode);
            return childHit || isInterestingDomNode(record.target);
          });
          if (!interesting.length) return;
          domMutationStats.mutationObserverInteresting += interesting.length;
          domMutationStats.lastInterestingMutationAt = nowMs();
          for (const record of interesting) {
            domMutationStats.interestingInserts += [...record.addedNodes || []].filter(isInterestingDomNode).length;
            domMutationStats.interestingRemovals += [...record.removedNodes || []].filter(isInterestingDomNode).length;
          }
          const sample = {
            time: nowMs(),
            kind: 'MutationObserver',
            count: records.length,
            interestingCount: interesting.length,
            records: interesting.slice(0, 10).map(mutationRecordBrief),
          };
          pushDomMutationSample(sample);
          record('dom-mutation', sample);
          recordDomState('dom-mutation');
        });
        observer.observe(document.documentElement || document, {
          childList: true,
          subtree: true,
          attributes: true,
          attributeFilter: [
            'data-turn',
            'data-message-id',
            'data-message-author-role',
            'data-testid',
            'role',
            'class',
            'id',
          ],
        });
        window.__lmChatGPTLiveTraceMutationObserver = observer;
      } catch {}
    }

    const nativeAppendChild = Node.prototype.appendChild;
    if (typeof nativeAppendChild === 'function' && !nativeAppendChild.__lmChatGPTLiveTraceWrapped) {
      Node.prototype.appendChild = function tracedAppendChild(child) {
        domMutationStats.appendChildCalls += 1;
        recordInterestingInsert('appendChild', this, child);
        return nativeAppendChild.apply(this, arguments);
      };
      Node.prototype.appendChild.__lmChatGPTLiveTraceWrapped = true;
    }

    const nativeInsertBefore = Node.prototype.insertBefore;
    if (typeof nativeInsertBefore === 'function' && !nativeInsertBefore.__lmChatGPTLiveTraceWrapped) {
      Node.prototype.insertBefore = function tracedInsertBefore(child) {
        domMutationStats.insertBeforeCalls += 1;
        recordInterestingInsert('insertBefore', this, child);
        return nativeInsertBefore.apply(this, arguments);
      };
      Node.prototype.insertBefore.__lmChatGPTLiveTraceWrapped = true;
    }

    const nativeReplaceChildren = Element.prototype.replaceChildren;
    if (typeof nativeReplaceChildren === 'function' && !nativeReplaceChildren.__lmChatGPTLiveTraceWrapped) {
      Element.prototype.replaceChildren = function tracedReplaceChildren(...children) {
        domMutationStats.replaceChildrenCalls += 1;
        for (const child of children) recordInterestingInsert('replaceChildren', this, child);
        return nativeReplaceChildren.apply(this, arguments);
      };
      Element.prototype.replaceChildren.__lmChatGPTLiveTraceWrapped = true;
    }
  }

  function sourceSummary(info) {
    if (!info) return null;
    return {
      url: info.url,
      method: info.method,
      status: info.status,
      ok: info.ok,
    };
  }

  function sourceKey(info) {
    if (!info) return '';
    return `${info.method || ''} ${info.url || ''}`;
  }

  function sourceStat(info) {
    const key = sourceKey(info);
    if (!key) return null;
    let stat = sourceStats.get(key);
    if (!stat) {
      stat = {
        source: sourceSummary(info),
        fetchResponses: 0,
        responseClones: 0,
        bodyReads: {},
        getReaders: 0,
        streamReads: 0,
        streamDoneReads: 0,
        totalBytes: 0,
        lastChunk: null,
        lastDataChunk: null,
        frameStats: {
          jsonValues: 0,
          dataLines: 0,
          doneLines: 0,
          typeCounts: {},
          patchCount: 0,
          patchPathCounts: {},
          patchOpCounts: {},
          patchFieldKindCounts: {},
          patchValueKeySetCounts: {},
          keySetCounts: {},
          roleCounts: {},
          stringPathCounts: {},
          errorEventCount: 0,
          errorCodeCounts: {},
          errorFieldShapes: {},
          errorMessageShapes: {},
          patchSamples: [],
        },
      };
      sourceStats.set(key, stat);
    }
    return stat;
  }

  function incrementSourceBodyRead(info, methodName) {
    const stat = sourceStat(info);
    if (!stat) return;
    stat.bodyReads[methodName] = (stat.bodyReads[methodName] || 0) + 1;
  }

  function recordSourceChunk(info, done, chunk) {
    const stat = sourceStat(info);
    if (!stat) return;
    stat.streamReads += 1;
    if (done) stat.streamDoneReads += 1;
    const byteLength = Number(chunk?.byteLength || chunk?.size || 0);
    if (Number.isFinite(byteLength)) stat.totalBytes += byteLength;
    stat.lastChunk = chunk;
    if (!done) stat.lastDataChunk = chunk;
    if (chunk?.textFrames?.stats) mergeFrameStats(stat.frameStats, chunk.textFrames.stats);
  }

  function sourceStatsSnapshot() {
    return [...sourceStats.values()]
      .filter((stat) => stat?.source?.url && isInterestingUrl(stat.source.url))
      .slice(-40);
  }

  function isConversationStreamSource(source) {
    const url = String(source?.url || '');
    return url.includes('chatgpt.com/backend-api/f/conversation');
  }

  function recordConversationStreamAppState(source, done, chunk) {
    if (!isConversationStreamSource(source)) return;
    const hasPatch = Number(chunk?.textFrames?.stats?.patchCount || 0) > 0;
    recordAppState(done ? 'conversation-stream-done' : 'conversation-stream-chunk');
    window.queueMicrotask?.(() => recordConversationMaterializationSnapshot(
      done ? 'stream-read-done-microtask' : 'stream-read-chunk-microtask',
    ));
    if (hasPatch) {
      window.queueMicrotask?.(() => recordAppState('conversation-stream-patch-microtask'));
      window.setTimeout?.(() => {
        recordAppState('conversation-stream-patch-timeout');
        recordConversationMaterializationSnapshot(
          done ? 'stream-read-done-timeout' : 'stream-read-patch-timeout',
        );
      }, 0);
    }
  }

  function incrementCount(target, key, amount = 1, maxKeys = 80) {
    if (!key) return;
    if (!Object.prototype.hasOwnProperty.call(target, key) && Object.keys(target).length >= maxKeys) {
      key = '<other>';
    }
    target[key] = (target[key] || 0) + amount;
  }

  function mergeCountMap(target, source) {
    if (!source || typeof source !== 'object') return;
    for (const [key, value] of Object.entries(source)) {
      incrementCount(target, key, Number(value) || 0);
    }
  }

  function mergeFrameStats(target, source) {
    if (!source || typeof source !== 'object') return;
    target.jsonValues += Number(source.jsonValues || 0);
    target.dataLines += Number(source.dataLines || 0);
    target.doneLines += Number(source.doneLines || 0);
    target.patchCount += Number(source.patchCount || 0);
    mergeCountMap(target.typeCounts, source.typeCounts);
    mergeCountMap(target.patchPathCounts, source.patchPathCounts);
    mergeCountMap(target.patchOpCounts, source.patchOpCounts);
    mergeCountMap(target.patchFieldKindCounts, source.patchFieldKindCounts);
    mergeCountMap(target.patchValueKeySetCounts, source.patchValueKeySetCounts);
    mergeCountMap(target.keySetCounts, source.keySetCounts);
    mergeCountMap(target.roleCounts, source.roleCounts);
    mergeCountMap(target.stringPathCounts, source.stringPathCounts);
    target.errorEventCount += Number(source.errorEventCount || 0);
    mergeCountMap(target.errorCodeCounts, source.errorCodeCounts);
    mergeCountMap(target.errorFieldShapes, source.errorFieldShapes);
    mergeCountMap(target.errorMessageShapes, source.errorMessageShapes);
    if (Array.isArray(source.patchSamples)) {
      for (const sample of source.patchSamples) {
        addPatchSample(target.patchSamples, sample);
      }
    }
  }

  function safePathSegment(segment) {
    if (typeof segment === 'number') return '#';
    if (typeof segment !== 'string') return typeof segment;
    if (segment === '#') return '#';
    if (/^\d+$/.test(segment)) return '#';
    if (/^[0-9a-f]{8,}$/i.test(segment) || /^[0-9a-f-]{16,}$/i.test(segment)) {
      return '<id>';
    }
    if (/^[A-Za-z_$][A-Za-z0-9_$-]{0,32}$/.test(segment)) return segment;
    return `<str:${segment.length}>`;
  }

  function patchPathStringShape(path) {
    const raw = String(path || '');
    if (!raw) return '<str:0>';
    const parts = raw.split(/[/.]+/).filter(Boolean);
    if (parts.length <= 1) return safePathSegment(raw);
    return parts.slice(0, 12).map(safePathSegment).join('.');
  }

  function patchPathShape(path) {
    if (typeof path === 'string') return patchPathStringShape(path);
    if (typeof path === 'number') return '#';
    if (!Array.isArray(path)) return '';
    return path.slice(0, 12).map(safePathSegment).join('.');
  }

  function safeScalarToken(value) {
    if (typeof value === 'string') {
      if (/^[A-Za-z0-9_$:.-]{1,32}$/.test(value)) return value;
      return `string:${value.length}`;
    }
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    if (value === null) return 'null';
    return Array.isArray(value) ? 'array' : typeof value;
  }

  function shapeKind(value) {
    if (Array.isArray(value)) return `array:${value.length}`;
    if (value === null) return 'null';
    if (ArrayBuffer.isView(value)) return value.constructor?.name || 'typedarray';
    if (value && typeof value === 'object') return 'object';
    if (typeof value === 'string') return `string:${value.length}`;
    return typeof value;
  }

  function sortedKeyShape(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return '';
    return Object.keys(value).sort().slice(0, 8).join(',');
  }

  function isInterestingPatchSample(sample) {
    const valueKind = sample?.fieldKinds?.v || '';
    return (
      valueKind.startsWith('string:') ||
      valueKind.startsWith('array:') ||
      sample?.op === 'append' ||
      sample?.op === 'replace'
    );
  }

  function addPatchSample(samples, sample) {
    if (!sample) return;
    sample.interesting = isInterestingPatchSample(sample);
    if (samples.length < 8) {
      samples.push(sample);
      return;
    }
    if (!sample.interesting) return;
    const replaceIndex = samples.findIndex((existing) => !existing?.interesting);
    if (replaceIndex >= 0) samples[replaceIndex] = sample;
  }

  function summarizeJsonFrame(value, stats, path = [], depth = 0) {
    stats.jsonValues += 1;
    if (depth > 6) return;
    if (Array.isArray(value)) {
      for (const item of value.slice(0, 80)) summarizeJsonFrame(item, stats, path.concat('#'), depth + 1);
      return;
    }
    if (!value || typeof value !== 'object') {
      if (typeof value === 'string') {
        incrementCount(stats.stringPathCounts, `${patchPathShape(path) || '<root>'}:len${value.length}`);
      }
      return;
    }
    const keys = Object.keys(value).sort();
    incrementCount(stats.keySetCounts, keys.slice(0, 8).join(','));
    if (typeof value.type === 'string') incrementCount(stats.typeCounts, value.type);
    if (typeof value.role === 'string') incrementCount(stats.roleCounts, safeScalarToken(value.role));
    if (value.author && typeof value.author.role === 'string') {
      incrementCount(stats.roleCounts, safeScalarToken(value.author.role));
    }
    if (
      Object.prototype.hasOwnProperty.call(value, 'error') ||
      Object.prototype.hasOwnProperty.call(value, 'error_code')
    ) {
      stats.errorEventCount += 1;
      if (typeof value.error_code === 'string') {
        incrementCount(stats.errorCodeCounts, safeScalarToken(value.error_code));
      }
      if (Object.prototype.hasOwnProperty.call(value, 'error')) {
        incrementCount(stats.errorFieldShapes, shapeKind(value.error));
      }
      if (Object.prototype.hasOwnProperty.call(value, 'message')) {
        incrementCount(stats.errorFieldShapes, `message:${shapeKind(value.message)}`);
      }
      if (typeof value.error === 'string') {
        incrementCount(stats.errorMessageShapes, `error:len${value.error.length}`);
      }
      if (typeof value.message === 'string') {
        incrementCount(stats.errorMessageShapes, `message:len${value.message.length}`);
      }
    }
    if (
      Object.prototype.hasOwnProperty.call(value, 'p') ||
      Object.prototype.hasOwnProperty.call(value, 'o') ||
      Object.prototype.hasOwnProperty.call(value, 'v') ||
      Object.prototype.hasOwnProperty.call(value, 'c')
    ) {
      stats.patchCount += 1;
      const patchPath = patchPathShape(value.p) || patchPathShape(path) || '<missing>';
      incrementCount(stats.patchPathCounts, patchPath);
      if (Object.prototype.hasOwnProperty.call(value, 'o')) {
        incrementCount(stats.patchOpCounts, safeScalarToken(value.o));
      }
      for (const field of ['p', 'o', 'v', 'c']) {
        if (Object.prototype.hasOwnProperty.call(value, field)) {
          incrementCount(stats.patchFieldKindCounts, `${field}:${shapeKind(value[field])}`);
        }
      }
      const valueKeyShape = sortedKeyShape(value.v);
      if (valueKeyShape) incrementCount(stats.patchValueKeySetCounts, valueKeyShape);
      const fieldKinds = {};
      for (const field of ['p', 'o', 'v', 'c']) {
        if (Object.prototype.hasOwnProperty.call(value, field)) {
          fieldKinds[field] = shapeKind(value[field]);
        }
      }
      addPatchSample(stats.patchSamples, {
        path: patchPath,
        keys: keys.slice(0, 8),
        op: Object.prototype.hasOwnProperty.call(value, 'o') ? safeScalarToken(value.o) : '',
        fieldKinds,
        valueShape: valueShape(value.v),
        containerShape: valueShape(value.c),
      });
    }
    for (const key of keys.slice(0, 40)) {
      summarizeJsonFrame(value[key], stats, path.concat(key), depth + 1);
    }
  }

  function summarizeTextFrames(text) {
    const allLines = String(text || '').split(/\r?\n/).filter(Boolean);
    const lines = allLines.slice(0, 30);
    const dataShapes = [];
    const stats = {
      jsonValues: 0,
      dataLines: 0,
      doneLines: 0,
      typeCounts: {},
      patchCount: 0,
      patchPathCounts: {},
      patchOpCounts: {},
      patchFieldKindCounts: {},
      patchValueKeySetCounts: {},
      keySetCounts: {},
      roleCounts: {},
      stringPathCounts: {},
      errorEventCount: 0,
      errorCodeCounts: {},
      errorFieldShapes: {},
      errorMessageShapes: {},
      patchSamples: [],
    };
    for (const line of allLines) {
      const trimmed = line.trim();
      const payload = trimmed.startsWith('data:') ? trimmed.slice(5).trim() : trimmed;
      if (!payload) continue;
      if (trimmed.startsWith('data:')) stats.dataLines += 1;
      if (payload === '[DONE]') {
        stats.doneLines += 1;
        continue;
      }
      if (!payload.startsWith('{') && !payload.startsWith('[')) continue;
      try {
        const parsed = JSON.parse(payload);
        summarizeJsonFrame(parsed, stats);
        if (dataShapes.length < 8 && lines.includes(line)) dataShapes.push(valueShape(parsed));
      } catch {}
    }
    return {
      textLength: String(text || '').length,
      lineCount: allLines.length,
      sampledLineCount: lines.length,
      dataLineCount: stats.dataLines,
      doneCount: stats.doneLines,
      stats,
      dataShapes,
    };
  }

  const REQUEST_BODY_MARKERS = [
    'local_conversation_event',
    'missing_existing_node',
    'missing_expected_parent',
    'skipped_missing_parent',
    'inserted_without_parent',
    'inserted',
    'existing_message_update_queued',
    'existing_message_updated',
    'conversation.input_missing_expected_parent',
    'chatgpt_web_conversation_tree_node_not_found',
    'getNodeByIdOrMessageId',
    'streamError',
    'messageReceived',
    'assistantMessageReceived',
    'completionFinished',
    'Attribution: Client Thread to Server Thread',
    'Create New Thread',
    'Init new thread',
    'conversation.server_overwrite_stale',
    'chatgpt_web_conversation_server_overwrite_stale',
  ];

  function markerCounts(text) {
    const counts = {};
    const source = String(text || '');
    for (const marker of REQUEST_BODY_MARKERS) {
      let count = 0;
      let index = source.indexOf(marker);
      while (index !== -1) {
        count += 1;
        index = source.indexOf(marker, index + marker.length);
      }
      if (count) counts[marker] = count;
    }
    return counts;
  }

  function hasMarkers(counts) {
    return !!counts && Object.keys(counts).length > 0;
  }

  function summarizeRequestBody(body) {
    if (body == null) return {kind: 'none'};
    if (typeof body === 'string') {
      const sample = body.slice(0, 120000);
      return {
        kind: 'string',
        length: body.length,
        markers: markerCounts(sample),
        chatRequest: summarizeChatRequestText(sample),
      };
    }
    if (body instanceof URLSearchParams) {
      const text = body.toString();
      return {
        kind: 'URLSearchParams',
        length: text.length,
        keys: [...body.keys()].slice(0, 40),
        markers: markerCounts(text.slice(0, 120000)),
        chatRequest: summarizeChatRequestText(text.slice(0, 120000)),
      };
    }
    if (body instanceof FormData) {
      const keys = [];
      try {
        for (const key of body.keys()) keys.push(String(key));
      } catch {}
      return {kind: 'FormData', keys: keys.slice(0, 40), markers: {}};
    }
    if (body instanceof Blob) {
      return {kind: 'Blob', size: body.size, type: body.type || '', markers: {}};
    }
    if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
      const byteLength = body.byteLength || 0;
      let markers = {};
      try {
        const view = body instanceof ArrayBuffer
          ? new Uint8Array(body)
          : new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
        const text = new TextDecoder('utf-8', {fatal: false}).decode(
          view.slice(0, Math.min(view.byteLength, 120000))
        );
        markers = markerCounts(text);
        return {
          kind: Object.prototype.toString.call(body).slice(8, -1),
          byteLength,
          markers,
          chatRequest: summarizeChatRequestText(text),
        };
      } catch {}
      return {kind: Object.prototype.toString.call(body).slice(8, -1), byteLength, markers};
    }
    return {kind: Object.prototype.toString.call(body), keys: objectKeys(body, 20), markers: {}};
  }

  function idShape(value) {
    if (typeof value !== 'string' || !value) return undefined;
    const match = value.match(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i);
    if (match) return match[0].slice(0, 8) + '...' + match[0].slice(-4);
    if (/^[A-Za-z0-9_-]{12,}$/.test(value)) return value.slice(0, 6) + '...' + value.slice(-4);
    return `string:${value.length}`;
  }

  function messageRequestShape(message) {
    if (!message || typeof message !== 'object') return {kind: typeof message};
    const parts = Array.isArray(message.content?.parts) ? message.content.parts : [];
    return {
      id: idShape(message.id),
      role: typeof message.author?.role === 'string' ? message.author.role : '',
      recipient: typeof message.recipient === 'string' ? message.recipient : '',
      parent: idShape(message.metadata?.parent_id),
      requestId: idShape(message.metadata?.request_id),
      contentType: typeof message.content?.content_type === 'string' ? message.content.content_type : '',
      partLengths: parts.slice(0, 6).map((part) => String(part || '').length),
    };
  }

  function summarizeChatRequestText(text) {
    if (!text || !String(text).trim().startsWith('{')) return undefined;
    try {
      const data = JSON.parse(text);
      if (!data || typeof data !== 'object') return undefined;
      return {
        keys: objectKeys(data, 40),
        action: typeof data.action === 'string' ? data.action : '',
        conversationId: idShape(data.conversation_id),
        threadId: idShape(data.thread_id ?? data.threadId),
        parentMessageId: idShape(data.parent_message_id ?? data.parentMessageId),
        forceParagen: data.force_paragen === true,
        messages: Array.isArray(data.messages)
          ? data.messages.slice(0, 8).map(messageRequestShape)
          : [],
      };
    } catch {
      return undefined;
    }
  }

  function installRequestConstructorTrace() {
    const NativeRequest = window.Request;
    if (typeof NativeRequest !== 'function' || NativeRequest.__lmChatGPTLiveTraceWrapped) return;
    function TracedRequest(input, init) {
      if (!new.target) {
        throw new TypeError("Failed to construct 'Request': Please use the 'new' operator.");
      }
      const request = new NativeRequest(input, init);
      try {
        const rawUrl = request.url || input?.url || input;
        const method = String(request.method || init?.method || input?.method || 'GET').toUpperCase();
        if (isConversationPostUrl(rawUrl)) {
          record('request-create', {
            url: compactUrl(rawUrl),
            method,
            inputKind: Object.prototype.toString.call(input),
            body: summarizeRequestBody(init?.body),
          });
        }
      } catch {}
      return request;
    }
    TracedRequest.prototype = NativeRequest.prototype;
    Object.setPrototypeOf(TracedRequest, NativeRequest);
    TracedRequest.__lmChatGPTLiveTraceWrapped = true;
    window.Request = TracedRequest;
  }

  function summarizeStreamChunk(value, source) {
    const base = summarizeData(value);
    if (!source || !isInterestingUrl(source.url)) return base;
    let text = '';
    try {
      if (typeof value === 'string') {
        text = value;
      } else if (value instanceof ArrayBuffer && typeof TextDecoder !== 'undefined') {
        text = new TextDecoder('utf-8', {fatal: false}).decode(new Uint8Array(value));
      } else if (ArrayBuffer.isView(value) && typeof TextDecoder !== 'undefined') {
        text = new TextDecoder('utf-8', {fatal: false}).decode(value);
      }
    } catch {}
    if (!text) return base;
    return {...base, textFrames: summarizeTextFrames(text)};
  }

  const nativeFetch = window.fetch;
  if (typeof nativeFetch === 'function') {
    window.fetch = function tracedFetch(input, init) {
      const rawUrl = input?.url || input;
      const url = compactUrl(rawUrl);
      const method = String(init?.method || input?.method || 'GET').toUpperCase();
      const interesting = isInterestingUrl(rawUrl);
      const conversationPost = isConversationPostUrl(rawUrl);
      if (isTelemetryUrl(rawUrl)) {
        const bodySummary = summarizeRequestBody(init?.body);
        if (hasMarkers(bodySummary.markers) || bodySummary.kind !== 'none') {
          record('fetch-request-body', {url, method, body: bodySummary});
        }
      } else if (conversationPost) {
        const bodySummary = summarizeRequestBody(init?.body);
        record('fetch-request-body', {url, method, body: bodySummary});
        if (bodySummary.kind === 'none' && input instanceof Request) {
          try {
            input.clone().text().then((text) => {
              record('fetch-request-body', {
                url,
                method,
                body: {
                  kind: 'RequestCloneText',
                  length: text.length,
                  markers: markerCounts(text.slice(0, 120000)),
                  chatRequest: summarizeChatRequestText(text.slice(0, 120000)),
                },
              });
            }).catch(() => {});
          } catch {}
        }
      }
      if (interesting) record('fetch-start', {url, method});
      return nativeFetch.apply(this, arguments).then((response) => {
        const responseUrl = compactUrl(response?.url || rawUrl);
        if (interesting || isInterestingUrl(response?.url)) {
          const info = {
            url: responseUrl,
            method,
            status: response.status,
            ok: !!response.ok,
          };
          responseInfos.set(response, info);
          if (response?.body) streamInfos.set(response.body, info);
          sourceStat(info).fetchResponses += 1;
          record('fetch-response', {
            url: responseUrl,
            method,
            status: response.status,
            ok: !!response.ok,
            redirected: !!response.redirected,
            hasBody: !!response.body,
          });
        }
        return response;
      }, (error) => {
        if (interesting) {
          record('fetch-error', {url, method, name: error?.name || '', message: String(error?.message || error).slice(0, 240)});
        }
        throw error;
      });
    };
  }
  if (window.Response?.prototype?.clone) {
    const nativeResponseClone = window.Response.prototype.clone;
    window.Response.prototype.clone = function tracedResponseClone(...args) {
      const cloned = nativeResponseClone.apply(this, args);
      const info = responseInfos.get(this);
      if (info) {
        responseInfos.set(cloned, info);
        if (cloned?.body) streamInfos.set(cloned.body, info);
        sourceStat(info).responseClones += 1;
        record('response-clone', {source: sourceSummary(info), hasBody: !!cloned?.body});
      }
      return cloned;
    };
  }
  for (const methodName of ['arrayBuffer', 'blob', 'formData', 'json', 'text']) {
    const nativeMethod = window.Response?.prototype?.[methodName];
    if (typeof nativeMethod !== 'function') continue;
    window.Response.prototype[methodName] = function tracedBodyMethod(...args) {
      const info = responseInfos.get(this);
      if (info && isInterestingUrl(info.url)) {
        incrementSourceBodyRead(info, methodName);
        record('response-body-read', {source: sourceSummary(info), method: methodName});
      }
      return nativeMethod.apply(this, args);
    };
  }

  const nativeWebSocket = window.WebSocket;
  if (typeof nativeWebSocket === 'function') {
    function TracedWebSocket(url, protocols) {
      const compact = compactUrl(url);
      const interesting = isInterestingUrl(url);
      if (interesting) record('ws-create', {url: compact});
      const ws = protocols === undefined ? new nativeWebSocket(url) : new nativeWebSocket(url, protocols);
      if (interesting) {
        ws.addEventListener('open', () => record('ws-open', {url: compact}));
        ws.addEventListener('message', (event) => {
          record('ws-message', {url: compact, data: summarizeData(event.data)});
          recordAppState('ws-message');
          window.queueMicrotask?.(() => recordAppState('ws-message-microtask'));
          window.setTimeout?.(() => recordAppState('ws-message-timeout'), 0);
        });
        ws.addEventListener('error', () => record('ws-error', {url: compact}));
        ws.addEventListener('close', (event) => record('ws-close', {url: compact, code: event.code, reasonLen: String(event.reason || '').length}));
      }
      return ws;
    }
    TracedWebSocket.prototype = nativeWebSocket.prototype;
    Object.setPrototypeOf(TracedWebSocket, nativeWebSocket);
    for (const name of ['CONNECTING', 'OPEN', 'CLOSING', 'CLOSED']) {
      try {
        Object.defineProperty(TracedWebSocket, name, {value: nativeWebSocket[name]});
      } catch {}
    }
    window.WebSocket = TracedWebSocket;
  }

  if (window.ReadableStream?.prototype?.getReader) {
    const nativeGetReader = window.ReadableStream.prototype.getReader;
    window.ReadableStream.prototype.getReader = function tracedGetReader(...args) {
      const source = streamInfos.get(this);
      if (source) sourceStat(source).getReaders += 1;
      record('stream-get-reader', {mode: args[0]?.mode || '', source: sourceSummary(source)});
      const reader = nativeGetReader.apply(this, args);
      if (reader?.read && !reader.__lmChatGPTLiveTraceReadWrapped) {
        const nativeRead = reader.read.bind(reader);
        reader.read = function tracedReaderRead(...readArgs) {
          return nativeRead(...readArgs).then((result) => {
            const chunk = summarizeStreamChunk(result.value, source);
            recordSourceChunk(source, !!result.done, chunk);
            record('stream-read', {
              done: !!result.done,
              source: sourceSummary(source),
              chunk,
            });
            recordConversationStreamAppState(source, !!result.done, chunk);
            return result;
          });
        };
        reader.__lmChatGPTLiveTraceReadWrapped = true;
      }
      return reader;
    };
  }
  if (window.TextDecoderStream) {
    const NativeTextDecoderStream = window.TextDecoderStream;
    window.TextDecoderStream = function tracedTextDecoderStream(...args) {
      record('text-decoder-stream', {encoding: args[0] || 'utf-8'});
      return new NativeTextDecoderStream(...args);
    };
    window.TextDecoderStream.prototype = NativeTextDecoderStream.prototype;
    Object.setPrototypeOf(window.TextDecoderStream, NativeTextDecoderStream);
  }

  installReactCommitTrace();
  installEventLoopTrace();
  installMessageTaskTrace();
  installObserverApiTrace();
  installDomMutationTrace();
  installRequestConstructorTrace();
  installHistoryTrace();
  installNavigationApiTrace();
  installIdMapTrace();

  if (document.documentElement) {
    installMutationTrace();
  } else {
    document.addEventListener('DOMContentLoaded', installMutationTrace, {once: true});
  }

  window.__lmChatGPTLiveTraceSnapshot = function liveTraceSnapshot() {
    const snapshot = {snapshotErrors: []};
    function field(name, producer) {
      try {
        snapshot[name] = producer();
      } catch (error) {
        snapshot.snapshotErrors.push({
          field: name,
          name: String(error?.name || ''),
          message: String(error?.message || error).slice(0, 240),
        });
      }
    }
    field('domStateRecord', () => {
      recordDomState('snapshot');
      return true;
    });
    field('url', () => compactUrl(location.href));
    field('state', () => conversationDomState());
    field('eventLoop', () => eventLoopState());
    field('messageTasks', () => messageTaskState());
    field('domMutations', () => domMutationState());
    field('reactCommits', () => reactCommitState());
    field('reactConversationWrappers', () => currentConversationWrapperSnapshots());
    field('conversationMaterialization', () => conversationMaterializationTraceState());
    field('conversationIdentityTrace', () => conversationIdentityTraceState());
    field('reactThreadFiber', () => reactThreadFiberSnapshot());
    field('threadRendererProbes', () => threadRendererProbes());
    field('suspenseBoundaryProbes', () => suspenseBoundaryProbes());
    field('reactionStoreProbes', () => reactionStoreProbes());
    field('threadStoreHooks', () => threadStoreHookSnapshots());
    field('navigationApi', () => navigationTraceState());
    field('idMapTrace', () => idMapTraceState());
    field('sourceStats', () => sourceStatsSnapshot());
    field('events', () => events.slice(-240));
    return snapshot;
  };
})();
