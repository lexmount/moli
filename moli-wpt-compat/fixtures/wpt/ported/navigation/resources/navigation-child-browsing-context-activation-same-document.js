(function () {
  const config = globalThis.__lmChildBrowsingContextActivationSameDocumentConfig;
  if (!config) {
    return;
  }

  function wait(ms) {
    return new Promise(function (resolve) {
      setTimeout(resolve, ms);
    });
  }

  function wait_for_value(read, description) {
    return Promise.race([
      new Promise(function (resolve, reject) {
        const deadline = Date.now() + 1000;
        function poll() {
          const value = read();
          if (value) {
            resolve(value);
            return;
          }
          if (Date.now() >= deadline) {
            reject(new Error("timed out waiting for " + description));
            return;
          }
          setTimeout(poll, 10);
        }
        poll();
      }),
      wait(1100).then(function () {
        throw new Error("timed out waiting for " + description);
      }),
    ]);
  }

  promise_test(async function () {
    const iframe = document.getElementById("child");
    const childUrl = new URL(iframe.getAttribute("src"), location.href).href;
    const childWindow = iframe.contentWindow;

    await wait_for_value(function () {
      return (
        childWindow.location.href === childUrl &&
        childWindow.navigation.currentEntry?.url === childUrl &&
        childWindow.document.readyState === "complete"
      );
    }, "child initial navigation commit");

    const initial = snapshotChildActivation(iframe);

    let pushResult;
    const pushHashchange = wait_for_hashchange(childWindow, "#one", function () {
      pushResult = childWindow.navigation.navigate("#one", {
        history: "push",
        state: { step: 1 },
      });
    });
    await Promise.all([pushResult.finished, pushHashchange]);
    const afterPush = snapshotChildActivation(iframe);

    await wait_for_hashchange(childWindow, "", function () {
      childWindow.history.back();
    });
    const afterBack = snapshotChildActivation(iframe);

    await wait_for_hashchange(childWindow, "#one", function () {
      childWindow.history.forward();
    });
    const afterForward = snapshotChildActivation(iframe);

    assert_equals(
      initial,
      childUrl + "||replace|" + childUrl + "|true",
      "initial child activation should point at the initial child document",
    );
    assert_equals(
      afterPush,
      childUrl + "||replace|" + childUrl + "#one|true",
      "same-document child push should keep activation pinned to the initial child document",
    );
    assert_equals(
      afterBack,
      childUrl + "||replace|" + childUrl + "|true",
      "child history.back() should keep activation pinned to the initial child document",
    );
    assert_equals(
      afterForward,
      childUrl + "||replace|" + childUrl + "#one|true",
      "child history.forward() should keep activation pinned to the initial child document",
    );
  }, config.testName);

  function snapshotChildActivation(iframe) {
    const childWindow = iframe.contentWindow;
    const activation = childWindow.navigation.activation;
    return [
      String(activation?.entry?.url ?? ""),
      String(activation?.from?.url ?? ""),
      String(activation?.navigationType ?? ""),
      String(childWindow.navigation.currentEntry?.url ?? ""),
      String(childWindow.navigation.transition === null),
    ].join("|");
  }

  function wait_for_hashchange(childWindow, expectedHash, action) {
    return new Promise(function (resolve) {
      function onHashchange() {
        if (childWindow.location.hash !== expectedHash) {
          return;
        }
        childWindow.removeEventListener("hashchange", onHashchange);
        resolve();
      }
      childWindow.addEventListener("hashchange", onHashchange);
      action();
    });
  }
})();
