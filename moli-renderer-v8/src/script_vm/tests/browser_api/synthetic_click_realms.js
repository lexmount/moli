() => {
  const failures = [];
  const realms = [window, document.getElementById("child").contentWindow];
  const pointers = realms.map(w => w.PointerEvent);
  const mice = realms.map(w => w.MouseEvent);

  function click(button, methodRealm, view, eventRealm, label) {
    let hits = 0;
    button.addEventListener("click", function(event) {
      hits++;
      const expected = {
        view,
        target: button,
        pointerId: -1,
        pointerType: "",
        isPrimary: false,
        width: 1,
        height: 1,
        pressure: 0,
        detail: 0,
        button: 0,
        buttons: 0,
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
        isTrusted: false,
        bubbles: true,
        cancelable: true,
        composed: true,
      };
      for (const [key, value] of Object.entries(expected)) {
        if (event[key] !== value) failures.push(`${label}: ${key}`);
      }
      if (Object.getPrototypeOf(event) !== pointers[eventRealm].prototype)
        failures.push(`${label}: PointerEvent prototype`);
      if (!(event instanceof mice[eventRealm]))
        failures.push(`${label}: MouseEvent inheritance`);
      if (this !== button) failures.push(`${label}: listener receiver`);
      event.preventDefault();
    }, { once: true });
    realms[methodRealm].HTMLElement.prototype.click.call(button);
    if (hits !== 1) failures.push(`${label}: dispatch count ${hits}`);
  }

  for (let origin = 0; origin < realms.length; origin++) {
    for (const attached of [false, true]) {
      const button = realms[origin].document.createElement("button");
      if (attached) realms[origin].document.body.appendChild(button);
      for (let method = 0; method < realms.length; method++)
        click(button, method, realms[origin], origin, `origin ${origin}/${attached}/${method}`);

      const destination = 1 - origin;
      realms[destination].document.adoptNode(button);
      if (attached) realms[destination].document.body.appendChild(button);
      for (let method = 0; method < realms.length; method++)
        click(button, method, realms[destination], destination, `adopt ${origin}/${attached}/${method}`);

      // The Document's creation realm remains meaningful when it has no Window.
      for (let creator = 0; creator < realms.length; creator++) {
        const documents = [
          realms[creator].document.implementation.createHTMLDocument(""),
          new realms[creator].DOMParser().parseFromString("<!doctype html><body>", "text/html"),
        ];
        for (const [kind, doc] of documents.entries()) {
          doc.adoptNode(button);
          if (attached) doc.body.appendChild(button);
          for (let method = 0; method < realms.length; method++)
            click(button, method, null, creator, `windowless ${origin}/${attached}/${creator}/${kind}/${method}`);
        }
      }

      realms[origin].document.adoptNode(button);
      if (attached) realms[origin].document.body.appendChild(button);
      for (let method = 0; method < realms.length; method++)
        click(button, method, realms[origin], origin, `return ${origin}/${attached}/${method}`);
      button.remove();
    }
  }

  for (let realm = 0; realm < realms.length; realm++) {
    const w = realms[realm];
    const descriptor = Object.getOwnPropertyDescriptor(w, "PointerEvent");
    Object.defineProperty(w, "PointerEvent", {
      configurable: true,
      get() { throw new Error("author PointerEvent getter"); },
    });
    try {
      click(w.document.createElement("button"), 1 - realm, w, realm, `intrinsic ${realm}`);
    } finally {
      Object.defineProperty(w, "PointerEvent", descriptor);
    }
  }

  // Author-created events keep their own initializer and constructor realm.
  for (let creator = 0; creator < realms.length; creator++) {
    const button = realms[1 - creator].document.createElement("button");
    for (const [Constructor, pointerId] of [[mice[creator], undefined], [pointers[creator], 0]]) {
      let hits = 0;
      const event = new Constructor("click", { bubbles: true, cancelable: true });
      button.addEventListener("click", received => {
        hits++;
        if (received !== event || received.view !== null || received.pointerId !== pointerId ||
            Object.getPrototypeOf(received) !== Constructor.prototype || received.isTrusted)
          failures.push(`author event ${creator}/${Constructor.name}`);
        received.preventDefault();
      }, { once: true });
      button.dispatchEvent(event);
      if (hits !== 1) failures.push(`author event ${creator}/${Constructor.name}: dispatch count`);
    }
  }
  return JSON.stringify(failures);
}
