globalThis.__mutationRecordNodeListResult = "pending";
(async () => {
  const check = (value, message) => {
    if (!value) throw new Error(message);
  };
  const NodeListConstructor = NodeList;
  const item = NodeList.prototype.item;
  const length = Object.getOwnPropertyDescriptor(NodeList.prototype, "length").get;
  const frame = globalThis.__observerCallbackFrame || document.querySelector("iframe");
  const child = frame.contentWindow;
  const childItem = child.NodeList.prototype.item;
  let label = "";
  function checkList(list, nodes) {
    check(!Array.isArray(list), label + ": NodeList must not be an Array");
    check(Object.prototype.toString.call(list) === "[object NodeList]", label + ": tag");
    check(length.call(list) === nodes.length, label + ": native length");
    check(item.call(list, nodes.length) === null, label + ": out-of-bounds item");
    check(list[nodes.length] === undefined, label + ": out-of-bounds index");
    check(childItem.call(list, nodes.length) === null, label + ": cross-realm item");
    const values = [...list];
    const entries = [...list.entries()];
    check(values.length === nodes.length, label + ": iterator length");
    check([...list.keys()].every((key, i) => key === i), label + ": keys");
    check([...list.values()].every((node, i) => node === nodes[i]), label + ": values");
    const receiver = {};
    let visited = 0;
    list.forEach(function (node, index, collection) {
      check(this === receiver && collection === list, label + ": forEach arguments");
      check(node === nodes[index], label + ": forEach node");
      visited++;
    }, receiver);
    check(visited === nodes.length, label + ": forEach count");
    for (let i = 0; i < nodes.length; i++) {
      check(list[i] === nodes[i] && item.call(list, i) === nodes[i], label + ": identity");
      check(values[i] === nodes[i], label + ": iterator identity");
      check(entries[i][0] === i && entries[i][1] === nodes[i], label + ": entries");
    }
    Reflect.set(list, nodes.length, null);
    check(list[nodes.length] === undefined, label + ": unsupported index write");
    Reflect.defineProperty(list, nodes.length, {value: null});
    check(list[nodes.length] === undefined, label + ": unsupported index definition");
    if (nodes.length) {
      const descriptor = Object.getOwnPropertyDescriptor(list, "0");
      check(descriptor.value === nodes[0] && !descriptor.writable &&
            descriptor.enumerable && descriptor.configurable, label + ": indexed descriptor");
      Reflect.set(list, 0, null);
      check(list[0] === nodes[0], label + ": indexed write");
      check(Reflect.deleteProperty(list, 0) === false, label + ": indexed delete");
      Reflect.defineProperty(list, 0, {value: null});
      check(list[0] === nodes[0], label + ": mutations preserve snapshot");
    }
    list.extra = 42;
    check(list.extra === 42, label + ": expando");
    delete list.extra;
  }
  function checkRecord(record, added, removed) {
    check(record.addedNodes === record.addedNodes && record.removedNodes === record.removedNodes,
          label + ": stable list identity");
    checkList(record.addedNodes, added);
    checkList(record.removedNodes, removed);
  }
  const documents = [document, document.implementation.createHTMLDocument(""),
                     document.implementation.createDocument(null, "root"), child.document];
  for (const [index, owner] of documents.entries()) {
    label = "document " + index;
    const host = owner.createElement("div");
    const text = owner.createTextNode("before");
    host.appendChild(text);
    const observer = new MutationObserver(() => {});
    observer.observe(host, {attributes: true, characterData: true, childList: true, subtree: true});
    host.setAttribute("data-test", "value");
    text.data = "after";
    const scalarRecords = observer.takeRecords();
    check(Array.isArray(scalarRecords) && scalarRecords.length === 2, label + ": record sequence");
    check(scalarRecords[0].type === "attributes" && scalarRecords[1].type === "characterData",
          label + ": record types");
    scalarRecords.forEach(record => checkRecord(record, [], []));
    const node = owner.createElement("span");
    host.appendChild(node);
    const added = observer.takeRecords();
    check(added.length === 1 && added[0].type === "childList", label + ": insertion");
    checkRecord(added[0], [node], []);
    host.removeChild(node);
    const removed = observer.takeRecords();
    check(removed.length === 1, label + ": removal record");
    checkRecord(removed[0], [], [node]);
    host.appendChild(owner.createElement("other"));
    observer.disconnect();
    checkRecord(added[0], [node], []);
    checkRecord(removed[0], [], [node]);
  }

  label = "large fragment snapshot";
  {
    const host = document.createElement("div");
    const fragment = document.createDocumentFragment();
    const nodes = Array.from({length: 1002}, () => document.createElement("span"));
    nodes.forEach(node => fragment.appendChild(node));
    const observer = new MutationObserver(() => {});
    observer.observe(fragment, {childList: true});
    observer.observe(host, {childList: true});
    host.appendChild(fragment);
    const records = observer.takeRecords();
    check(records.length === 2, label + ": fragment removal and host insertion records");
    const [removed, added] = records;
    check(removed.target === fragment && added.target === host, label + ": record targets");
    checkRecord(removed, [], nodes);
    checkRecord(added, nodes, []);
    host.replaceChildren();
    const replacement = observer.takeRecords();
    check(replacement.length === 1, label + ": one replacement record");
    checkRecord(replacement[0], [], nodes);
    observer.disconnect();
    checkRecord(added, nodes, []);
    checkRecord(removed, [], nodes);
    checkRecord(replacement[0], [], nodes);
  }

  label = "intrinsic prototype";
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "NodeList");
  const host = document.createElement("div");
  const node = document.createElement("span");
  const observer = new MutationObserver(() => {});
  observer.observe(host, {attributes: true, childList: true});
  let list;
  try {
    Object.defineProperty(globalThis, "NodeList", {
      configurable: true, get() { throw new Error("author NodeList getter must not run"); }
    });
    host.setAttribute("data-test", "value");
    host.appendChild(node);
    const records = observer.takeRecords();
    checkRecord(records[0], [], []);
    checkRecord(records[1], [node], []);
    for (const record of records) {
      for (const value of [record.addedNodes, record.removedNodes]) {
        check(Object.getPrototypeOf(value) === NodeListConstructor.prototype,
              label + ": native prototype survives global replacement");
      }
    }
    list = records[1].addedNodes;
  } finally {
    Object.defineProperty(globalThis, "NodeList", descriptor);
    observer.disconnect();
  }
  const revoked = Proxy.revocable(list, {});
  revoked.revoke();
  let conversions = 0, traps = 0;
  const authorProxy = new Proxy(list, {get() { traps++; }});
  for (const fake of [{}, Object.create(list), authorProxy, revoked.proxy]) {
    let error;
    try { childItem.call(fake, {valueOf() { conversions++; return 0; }}); }
    catch (caught) { error = caught; }
    check(error instanceof child.TypeError, "invalid receiver uses callee TypeError");
  }
  check(conversions === 0 && traps === 0, "brand check precedes conversion and Proxy traps");

  label = "callback delivery";
  await new Promise((resolve, reject) => {
    const observer = new MutationObserver(function (records, deliveredObserver) {
      try {
        check(this === observer && deliveredObserver === observer, label + ": callback receiver");
        check(Array.isArray(records) && records.length === 1, label + ": record sequence");
        checkRecord(records[0], [], [node]);
        resolve();
      } catch (error) { reject(error); }
      finally { observer.disconnect(); }
    });
    observer.observe(host, {childList: true});
    host.removeChild(node);
  });
  globalThis.__mutationRecordNodeListResult = true;
  return true;
})().catch(error => {
  globalThis.__mutationRecordNodeListResult = String(error);
  throw error;
})
