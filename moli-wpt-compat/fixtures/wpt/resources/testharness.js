(function (global) {
  "use strict";

  const PASS = "PASS";
  const FAIL = "FAIL";
  const callbacks = {
    result: [],
    completion: [],
  };
  const tests = [];
  let pending = 0;
  let completionScheduled = false;
  let complete = false;
  let testsStarted = false;
  let singleTest = null;
  let explicitDonePending = false;
  let setupChain = Promise.resolve();
  let promiseTestChain = Promise.resolve();
  let allowUncaughtException = false;
  let allowUncaughtExceptionHandlerInstalled = false;

  class AssertionError extends Error {
    constructor(message) {
      super(message);
      this.name = "AssertionError";
    }
  }

  global.AssertionError = global.AssertionError || AssertionError;

  function assertionError(message) {
    return new global.AssertionError(message);
  }

  function messageFromError(error) {
    if (!error) {
      return "";
    }
    return String(error && error.message ? error.message : error);
  }

  function notifyResult(record) {
    for (const callback of callbacks.result) {
      try {
        callback(record);
      } catch (_) {}
    }
  }

  function maybeComplete() {
    if (!testsStarted || complete || pending > 0 || completionScheduled) {
      return;
    }
    completionScheduled = true;
    global.queueMicrotask(function () {
      completionScheduled = false;
      if (complete || pending > 0) {
        return;
      }
      complete = true;
      const status = {
        status: tests.every((test) => test.status === PASS) ? "OK" : "ERROR",
        message: "",
      };
      for (const callback of callbacks.completion) {
        try {
          callback(tests.slice(), status);
        } catch (_) {}
      }
    });
  }

  function record(name, status, message) {
    testsStarted = true;
    const entry = { name, status, message: message || "" };
    tests.push(entry);
    notifyResult(entry);
    maybeComplete();
  }

  function createTestContext() {
    const cleanups = [];
    const context = {
      add_cleanup(callback) {
        cleanups.push(callback);
      },
    };
    context.step = function (callback) {
      return callback.call(context);
    };
    context.step_func = function (callback, thisObj) {
      return function (...args) {
        return context.step(function () {
          return callback.apply(thisObj === undefined ? context : thisObj, args);
        });
      };
    };
    context.step_timeout = function (callback, timeout = 0) {
      return global.step_timeout(context.step_func(callback), timeout);
    };
    installWaitHelpers(context);
    context.unreached_func = function (message) {
      return context.step_func(function () {
        global.assert_unreached(message);
      });
    };
    return {
      context,
      runCleanups() {
        while (cleanups.length > 0) {
          cleanups.pop()();
        }
      },
    };
  }

  function installWaitHelpers(context) {
    context.step_wait_func = function (
      condition,
      callback,
      description,
      timeout = 5000,
      interval = 100,
    ) {
      const start = Date.now();
      const poll = function () {
        if (condition.call(context)) {
          if (callback) {
            callback.call(context);
          }
          return;
        }
        if (Date.now() - start >= timeout) {
          global.assert_unreached(description || "step_wait_func timed out");
        }
        context.step_timeout(poll, interval);
      };
      context.step_timeout(poll, 0);
    };
    context.step_wait_func_done = function (
      condition,
      callback,
      description,
      timeout,
      interval,
    ) {
      context.step_wait_func(
        condition,
        function () {
          if (callback) {
            callback.call(context);
          }
          context.done();
        },
        description,
        timeout,
        interval,
      );
    };
  }

  global.setup = function (options) {
    if (typeof options === "function") {
      try {
        options();
      } catch (error) {
        record("setup", FAIL, messageFromError(error));
      }
      return;
    }
    if (options && options.allow_uncaught_exception === true) {
      allowUncaughtException = true;
      if (
        !allowUncaughtExceptionHandlerInstalled &&
        typeof global.addEventListener === "function"
      ) {
        allowUncaughtExceptionHandlerInstalled = true;
        global.addEventListener("error", function (event) {
          if (
            allowUncaughtException &&
            event &&
            typeof event.preventDefault === "function"
          ) {
            event.preventDefault();
          }
        });
      }
    }
    if (options && options.explicit_done === true && !explicitDonePending) {
      testsStarted = true;
      pending += 1;
      explicitDonePending = true;
    }
    if (!options || options.single_test !== true || singleTest) {
      return;
    }
    testsStarted = true;
    pending += 1;
    singleTest = createTestContext();
  };

  global.done = function () {
    if (!singleTest) {
      if (explicitDonePending) {
        explicitDonePending = false;
        pending -= 1;
      }
      maybeComplete();
      return;
    }
    try {
      singleTest.runCleanups();
      record("single test", PASS, "");
    } catch (error) {
      record("single test", FAIL, messageFromError(error));
    }
    pending -= 1;
    singleTest = null;
    maybeComplete();
  };

  global.add_result_callback = function (callback) {
    callbacks.result.push(callback);
  };

  global.add_completion_callback = function (callback) {
    callbacks.completion.push(callback);
  };

  global.test = function (callback, name) {
    const test = createTestContext();
    try {
      callback.call(test.context, test.context);
      test.runCleanups();
      record(name || "unnamed test", PASS, "");
    } catch (error) {
      try {
        test.runCleanups();
      } catch (cleanupError) {
        error = cleanupError;
      }
      record(name || "unnamed test", FAIL, messageFromError(error));
    }
  };

  global.promise_setup = function (callback) {
    testsStarted = true;
    pending += 1;
    setupChain = setupChain
      .then(function () {
        return callback();
      })
      .catch(function (error) {
        record("promise_setup", FAIL, messageFromError(error));
      })
      .then(function () {
        pending -= 1;
        maybeComplete();
      });
    return setupChain;
  };

  global.promise_test = function (callback, name) {
    testsStarted = true;
    pending += 1;
    const test = createTestContext();
    promiseTestChain = promiseTestChain
      .then(function () {
        return setupChain;
      })
      .then(function () {
        return callback.call(test.context, test.context);
      })
      .then(
        function () {
          test.runCleanups();
          record(name || "unnamed promise_test", PASS, "");
        },
        function (error) {
          try {
            test.runCleanups();
          } catch (cleanupError) {
            error = cleanupError;
          }
          record(name || "unnamed promise_test", FAIL, messageFromError(error));
        },
      )
      .then(function () {
        pending -= 1;
        maybeComplete();
      });
  };

  global.async_test = function (callbackOrName, maybeName) {
    testsStarted = true;
    pending += 1;
    const callback =
      typeof callbackOrName === "function" ? callbackOrName : undefined;
    const name = callback ? maybeName : callbackOrName;
    const test = createTestContext();
    let finished = false;

    function finish(status, message) {
      if (finished) {
        return;
      }
      finished = true;
      try {
        test.runCleanups();
      } catch (error) {
        status = FAIL;
        message = messageFromError(error);
      }
      record(name || "unnamed async_test", status, message);
      pending -= 1;
      maybeComplete();
    }

    test.context.step = function (callback) {
      if (finished) {
        return;
      }
      try {
        callback.call(test.context);
      } catch (error) {
        finish(FAIL, messageFromError(error));
      }
    };
    test.context.step_func = function (callback, thisObj) {
      return function (...args) {
        test.context.step(function () {
          callback.apply(thisObj === undefined ? test.context : thisObj, args);
        });
      };
    };
    test.context.step_func_done = function (callback, thisObj) {
      return test.context.step_func(function (...args) {
        if (callback) {
          callback.apply(this, args);
        }
        test.context.done();
      }, thisObj);
    };
    test.context.step_timeout = function (callback, timeout = 0) {
      return global.step_timeout(test.context.step_func(callback), timeout);
    };
    installWaitHelpers(test.context);
    test.context.done = function () {
      finish(PASS, "");
    };
    if (callback) {
      test.context.step(function () {
        callback.call(test.context, test.context);
      });
    }
    return test.context;
  };

  global.format_value = function (value) {
    if (typeof value === "string") {
      return JSON.stringify(value);
    }
    if (typeof value === "symbol") {
      return String(value);
    }
    try {
      return JSON.stringify(value);
    } catch (_) {
      return String(value);
    }
  };

  global.step_timeout = function (callback, timeout = 0) {
    return global.setTimeout(callback, timeout);
  };

  global.on_event = function (target, type, handler) {
    target.addEventListener(type, handler);
  };

  global.assert_unreached = function (description) {
    throw assertionError(description || "assert_unreached");
  };

  global.assert_true = function (actual, description) {
    if (actual !== true) {
      throw assertionError(description || "expected true");
    }
  };

  global.assert_implements = function (actual, description) {
    if (actual !== true) {
      throw assertionError(description || "assert_implements");
    }
  };

  global.assert_false = function (actual, description) {
    if (actual !== false) {
      throw assertionError(description || "expected false");
    }
  };

  const same_value = function (actual, expected) {
    return actual === expected || Object.is(actual, expected);
  };

  const assert_object_property_equals = function (actual, expected, description) {
    if (Array.isArray(expected)) {
      global.assert_array_equals(actual, expected, description);
      return;
    }

    if (expected && typeof expected === "object") {
      global.assert_object_equals(actual, expected, description);
      return;
    }

    if (!same_value(actual, expected)) {
      throw assertionError(
        (description || "assert_object_equals") +
          ": expected " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_equals = function (actual, expected, description) {
    if (!same_value(actual, expected)) {
      throw assertionError(
        (description || "assert_equals") +
          ": expected " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_not_equals = function (actual, unexpected, description) {
    if (actual === unexpected) {
      throw assertionError(
        (description || "assert_not_equals") +
          ": got disallowed value " +
          String(actual),
      );
    }
  };

  global.assert_own_property = function (object, property, description) {
    if (!Object.prototype.hasOwnProperty.call(object, property)) {
      throw assertionError(
        (description || "assert_own_property") +
          ": expected own property " +
          String(property),
      );
    }
  };

  global.assert_approx_equals = function (actual, expected, epsilon, description) {
    if (Math.abs(actual - expected) > epsilon) {
      throw assertionError(
        (description || "assert_approx_equals") +
          ": expected " +
          String(actual) +
          " to be within " +
          String(epsilon) +
          " of " +
          String(expected),
      );
    }
  };

  global.assert_less_than = function (actual, expected, description) {
    if (!(actual < expected)) {
      throw assertionError(
        (description || "assert_less_than") +
          ": expected less than " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_greater_than = function (actual, expected, description) {
    if (!(actual > expected)) {
      throw assertionError(
        (description || "assert_greater_than") +
          ": expected greater than " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_less_than_equal = function (actual, expected, description) {
    if (!(actual <= expected)) {
      throw assertionError(
        (description || "assert_less_than_equal") +
          ": expected less than or equal to " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_greater_than_equal = function (actual, expected, description) {
    if (!(actual >= expected)) {
      throw assertionError(
        (description || "assert_greater_than_equal") +
          ": expected greater than or equal to " +
          String(expected) +
          ", got " +
          String(actual),
      );
    }
  };

  global.assert_class_string = function (object, classString, description) {
    const actual = Object.prototype.toString.call(object);
    const expected = "[object " + classString + "]";
    if (actual !== expected) {
      throw assertionError(
        (description || "assert_class_string") +
          ": expected " +
          expected +
          ", got " +
          actual,
      );
    }
  };

  global.assert_array_equals = function (actual, expected, description) {
    actual = Array.from(actual);
    expected = Array.from(expected);
    const same =
      Array.isArray(actual) &&
      Array.isArray(expected) &&
      actual.length === expected.length &&
      actual.every((value, index) => same_value(value, expected[index]));
    if (!same) {
      throw assertionError(
        (description || "assert_array_equals") +
          ": expected [" +
          expected.join(",") +
          "], got [" +
          Array.prototype.join.call(actual, ",") +
          "]",
      );
    }
  };

  global.assert_object_equals = function (actual, expected, description) {
    if (Array.isArray(actual) || Array.isArray(expected)) {
      global.assert_array_equals(actual, expected, description);
      return;
    }

    if (
      actual === null ||
      expected === null ||
      typeof actual !== "object" ||
      typeof expected !== "object"
    ) {
      throw assertionError(description || "assert_object_equals");
    }

    const actualKeys = Object.keys(actual).sort();
    const expectedKeys = Object.keys(expected).sort();
    global.assert_array_equals(
      actualKeys,
      expectedKeys,
      (description || "assert_object_equals") + ": keys",
    );

    expectedKeys.forEach((key) => {
      assert_object_property_equals(
        actual[key],
        expected[key],
        (description || "assert_object_equals") + "." + key,
      );
    });
  };

  global.assert_readonly = function (object, propertyName, description) {
    if (!(propertyName in object)) {
      throw assertionError(
        (description || "assert_readonly") +
          ": property " +
          String(propertyName) +
          " not found",
      );
    }

    let current = object;
    let descriptor;
    while (current && descriptor === undefined) {
      descriptor = Object.getOwnPropertyDescriptor(current, propertyName);
      current = Object.getPrototypeOf(current);
    }
    if (descriptor === undefined) {
      throw assertionError(
        (description || "assert_readonly") +
          ": could not find descriptor for " +
          String(propertyName),
      );
    }
    if (Object.prototype.hasOwnProperty.call(descriptor, "value")) {
      if (descriptor.writable !== false) {
        throw assertionError(
          (description || "assert_readonly") +
            ": expected non-writable data property",
        );
      }
      return;
    }
    if (descriptor.set !== undefined) {
      throw assertionError(
        (description || "assert_readonly") +
          ": expected accessor property without setter",
      );
    }
  };

  global.assert_in_array = function (actual, expected, description) {
    if (!Array.from(expected).some((value) => value === actual)) {
      throw assertionError(
        (description || "assert_in_array") +
          ": expected " +
          String(actual) +
          " in [" +
          Array.prototype.join.call(expected, ",") +
          "]",
      );
    }
  };

  global.assert_regexp_match = function (actual, expected, description) {
    if (!(expected instanceof RegExp) || !expected.test(String(actual))) {
      throw assertionError(
        (description || "assert_regexp_match") +
          ": expected " +
          String(actual) +
          " to match " +
          String(expected),
      );
    }
  };

  global.assert_equals_typed_array = function (actual, expected, description) {
    if (
      !ArrayBuffer.isView(actual) ||
      !ArrayBuffer.isView(expected) ||
      actual.length !== expected.length
    ) {
      throw assertionError(
        (description || "assert_equals_typed_array") +
          ": expected matching typed array views",
      );
    }
    for (let index = 0; index < actual.length; index += 1) {
      if (actual[index] !== expected[index]) {
        throw assertionError(
          (description || "assert_equals_typed_array") +
            ": mismatch at " +
            index +
            ", expected " +
            expected[index] +
            ", got " +
            actual[index],
        );
      }
    }
  };

  global.assert_throws_js = function (constructor, callback, description) {
    try {
      callback();
    } catch (error) {
      if (error instanceof constructor) {
        return;
      }
      throw assertionError(
        (description || "assert_throws_js") +
          ": expected " +
          (constructor && constructor.name ? constructor.name : String(constructor)) +
          ", got " +
          (error && error.constructor && error.constructor.name
            ? error.constructor.name
            : String(error)),
      );
    }
    throw assertionError((description || "assert_throws_js") + ": did not throw");
  };

  global.promise_rejects_js = function (test, constructor, promise, description) {
    return Promise.resolve(promise).then(
      function () {
        throw assertionError((description || "promise_rejects_js") + ": promise resolved");
      },
      function (error) {
        if (error instanceof constructor) {
          return;
        }
        throw assertionError(
          (description || "promise_rejects_js") +
            ": expected " +
            (constructor && constructor.name ? constructor.name : String(constructor)) +
            ", got " +
            (error && error.constructor && error.constructor.name
              ? error.constructor.name
              : String(error)),
        );
      },
    );
  };

  const domExceptionLegacyNames = {
    INDEX_SIZE_ERR: "IndexSizeError",
  };

  global.promise_rejects_dom = function (test, name, promise, description) {
    const expectedName = domExceptionLegacyNames[name] || name;
    return Promise.resolve(promise).then(
      function () {
        throw assertionError((description || "promise_rejects_dom") + ": promise resolved");
      },
      function (error) {
        if (error && error.name === expectedName) {
          return;
        }
        throw assertionError(
          (description || "promise_rejects_dom") +
            ": expected " +
            expectedName +
            ", got " +
            (error && error.name ? error.name : String(error)),
        );
      },
    );
  };

  global.promise_rejects_exactly = function (test, expected, promise, description) {
    return Promise.resolve(promise).then(
      function () {
        throw assertionError((description || "promise_rejects_exactly") + ": promise resolved");
      },
      function (error) {
        if (error === expected) {
          return;
        }
        throw assertionError(
          (description || "promise_rejects_exactly") +
            ": expected exact rejection value " +
            String(expected) +
            ", got " +
            String(error),
        );
      },
    );
  };

  global.EventWatcher = function (test, watchedNode, eventTypes, timeoutPromise) {
    const types = Array.isArray(eventTypes) ? eventTypes : [eventTypes];
    const active = [];

    function removeActive() {
      while (active.length > 0) {
        const entry = active.pop();
        entry.target.removeEventListener(entry.type, entry.listener);
      }
    }

    if (test && typeof test.add_cleanup === "function") {
      test.add_cleanup(removeActive);
    }

    this.wait_for = function (expectedTypes) {
      const waitTypes = Array.isArray(expectedTypes) ? expectedTypes : [expectedTypes];
      for (const type of waitTypes) {
        if (!types.includes(type)) {
          throw assertionError("EventWatcher cannot wait for unwatched event type " + type);
        }
      }

      removeActive();
      return new Promise(function (resolve, reject) {
        let settled = false;
        function settle(callback, value) {
          if (settled) {
            return;
          }
          settled = true;
          removeActive();
          callback(value);
        }

        for (const type of waitTypes) {
          const listener = function (event) {
            settle(resolve, event);
          };
          watchedNode.addEventListener(type, listener);
          active.push({ target: watchedNode, type, listener });
        }

        if (timeoutPromise) {
          Promise.resolve()
            .then(timeoutPromise)
            .then(function () {
              settle(reject, new Error("Timed out waiting for " + waitTypes.join(",")));
            });
        }
      });
    };
  };

  global.assert_throws_exactly = function (expected, callback, description) {
    try {
      callback();
    } catch (error) {
      if (error === expected) {
        return;
      }
      throw assertionError(
        (description || "assert_throws_exactly") +
          ": expected exact thrown value " +
          String(expected) +
          ", got " +
          String(error),
      );
    }
    throw assertionError((description || "assert_throws_exactly") + ": did not throw");
  };

  global.assert_throws_dom = function (name, funcOrConstructor, descriptionOrFunc, maybeDescription) {
    let constructor = global.DOMException;
    let callback = funcOrConstructor;
    let description = descriptionOrFunc;
    if (typeof funcOrConstructor === "function" && funcOrConstructor.name === "DOMException") {
      constructor = funcOrConstructor;
      callback = descriptionOrFunc;
      description = maybeDescription;
    } else if (maybeDescription !== undefined) {
      throw assertionError("Too many args passed to no-constructor version of assert_throws_dom");
    }
    const expectedName = domExceptionLegacyNames[name] || name;
    try {
      callback();
    } catch (error) {
      if (error instanceof global.AssertionError) {
        throw error;
      }
      if (error && error.name === expectedName) {
        if (error.constructor !== constructor) {
          throw assertionError(
            (description || "assert_throws_dom") + ": exception came from the wrong global",
          );
        }
        return;
      }
      throw assertionError(
        (description || "assert_throws_dom") +
          ": expected " +
          expectedName +
          ", got " +
          (error && error.name ? error.name : String(error)),
      );
    }
    throw assertionError((description || "assert_throws_dom") + ": did not throw");
  };

  global.assert_throws_quotaexceedederror = function (
    funcOrConstructor,
    requestedOrFunc,
    quotaOrRequested,
    descriptionOrQuota,
    maybeDescription,
  ) {
    let constructor = global.QuotaExceededError;
    let callback = funcOrConstructor;
    let requested = requestedOrFunc;
    let quota = quotaOrRequested;
    let description = descriptionOrQuota;

    if (
      funcOrConstructor &&
      typeof funcOrConstructor === "function" &&
      funcOrConstructor.name === "QuotaExceededError"
    ) {
      constructor = funcOrConstructor;
      callback = requestedOrFunc;
      requested = quotaOrRequested;
      quota = descriptionOrQuota;
      description = maybeDescription;
    } else if (maybeDescription !== undefined) {
      throw assertionError(
        "Too many args passed to no-constructor version of assert_throws_quotaexceedederror",
      );
    }

    try {
      callback();
    } catch (error) {
      if (!error || error.name !== "QuotaExceededError" || error.code != 22) {
        throw assertionError(
          (description || "assert_throws_quotaexceedederror") +
            ": expected QuotaExceededError, got " +
            (error && error.name ? error.name : String(error)),
        );
      }
      if (constructor && error.constructor !== constructor) {
        throw assertionError(
          (description || "assert_throws_quotaexceedederror") +
            ": exception came from the wrong global",
        );
      }
      if (
        requested !== null &&
        (typeof requested === "function"
          ? !requested(error.requested)
          : error.requested != requested)
      ) {
        throw assertionError(
          (description || "assert_throws_quotaexceedederror") +
            ": unexpected requested " +
            String(error.requested),
        );
      }
      if (
        quota !== null &&
        (typeof quota === "function" ? !quota(error.quota) : error.quota != quota)
      ) {
        throw assertionError(
          (description || "assert_throws_quotaexceedederror") +
            ": unexpected quota " +
            String(error.quota),
        );
      }
      return;
    }
    throw assertionError(
      (description || "assert_throws_quotaexceedederror") + ": did not throw",
    );
  };

  global.fetch_tests_from_worker = function (worker) {
    testsStarted = true;
    pending += 1;
    return new Promise(function (resolve) {
      const messageTarget = worker && worker.port ? worker.port : worker;
      const errorTarget = worker && worker.port ? worker : worker;

      function finish() {
        messageTarget.removeEventListener("message", onMessage);
        if (errorTarget && typeof errorTarget.removeEventListener === "function") {
          errorTarget.removeEventListener("error", onError);
        }
        pending -= 1;
        maybeComplete();
        resolve();
      }

      function onMessage(event) {
        const message = event.data || {};
        if (message.type === "result" && message.test) {
          record(
            message.test.name || "unnamed worker test",
            message.test.status === PASS ? PASS : FAIL,
            message.test.message || "",
          );
          return;
        }
        if (message.type === "complete") {
          if (
            message.status &&
            message.status.status &&
            message.status.status !== "OK"
          ) {
            record("worker harness", FAIL, message.status.message || "");
          }
          finish();
        }
      }

      function onError(event) {
        if (event && typeof event.preventDefault === "function") {
          event.preventDefault();
        }
        record(
          "worker harness",
          FAIL,
          (event && event.message) || "worker error",
        );
        finish();
      }

      messageTarget.addEventListener("message", onMessage);
      if (typeof messageTarget.start === "function") {
        messageTarget.start();
      }
      if (errorTarget && typeof errorTarget.addEventListener === "function") {
        errorTarget.addEventListener("error", onError);
      }
    });
  };

  const isSharedWorkerGlobal =
    typeof global.document === "undefined" &&
    typeof global.SharedWorkerGlobalScope === "function" &&
    global instanceof global.SharedWorkerGlobalScope;

  if (isSharedWorkerGlobal) {
    const sharedWorkerPorts = [];
    const sharedWorkerMessages = [];

    function broadcastToSharedWorkerPorts(message) {
      sharedWorkerMessages.push(message);
      for (const port of sharedWorkerPorts) {
        try {
          port.postMessage(message);
        } catch (_) {}
      }
    }

    global.addEventListener("connect", function (event) {
      const port = event.ports && event.ports[0] ? event.ports[0] : event.source;
      if (!port || typeof port.postMessage !== "function") {
        return;
      }
      sharedWorkerPorts.push(port);
      if (typeof port.start === "function") {
        port.start();
      }
      for (const message of sharedWorkerMessages) {
        try {
          port.postMessage(message);
        } catch (_) {}
      }
    });

    global.add_result_callback(function (test) {
      broadcastToSharedWorkerPorts({
        type: "result",
        test: {
          name: test.name,
          status: test.status,
          message: test.message || "",
        },
      });
    });
    global.add_completion_callback(function (_tests, status) {
      broadcastToSharedWorkerPorts({
        type: "complete",
        status: {
          status: status.status,
          message: status.message || "",
        },
      });
    });
  } else if (
    typeof global.document === "undefined" &&
    typeof global.postMessage === "function"
  ) {
    global.add_result_callback(function (test) {
      global.postMessage({
        type: "result",
        test: {
          name: test.name,
          status: test.status,
          message: test.message || "",
        },
      });
    });
    global.add_completion_callback(function (_tests, status) {
      global.postMessage({
        type: "complete",
        status: {
          status: status.status,
          message: status.message || "",
        },
      });
    });
  }
})(globalThis);
