globalThis.targetResolutionLog = [];
globalThis.targetResolutionFrames = [];
const targetNames = ["base-destination", "form-destination", "submitter-destination",
  "child-base", "source-document", "raw<name", "raw\nname"];
const hrefBase = document.createElement("base");
hrefBase.href = "https://base-must-not-select-target.test/";
const targetBase = document.createElement("base");
targetBase.id = "targetBase";
targetBase.target = "base-destination";
document.head.append(hrefBase, targetBase);
for (const name of targetNames) {
  const frame = document.createElement("iframe");
  frame.name = name;
  frame.srcdoc = name === "source-document"
    ? "<!doctype html><head><base href='https://child-base.test/'><base id=targetBase target=child-base></head><body>"
    : "<!doctype html><body>";
  document.body.append(frame);
  targetResolutionFrames.push(frame);
}
globalThis.installTargetResolutionListeners = () => {
  for (const owner of [window, ...targetResolutionFrames.map(f => f.contentWindow)]) {
    owner.navigation.onnavigate = event => {
      targetResolutionLog.push(owner === window ? "source" : owner.name);
      event.preventDefault();
    };
  }
};
globalThis.submitTargetResolutionCase = (child, kind, target, override, mutation) => {
  targetResolutionLog.length = 0;
  const owner = child
    ? targetResolutionFrames.find(f => f.name === "source-document").contentDocument
    : document;
  owner.getElementById("resolution-form")?.remove();
  owner.getElementById("resolution-link")?.remove();
  owner.getElementById("extraTargetBase")?.remove();
  const base = owner.getElementById("targetBase");
  base.target = child ? "child-base" : "base-destination";
  if (mutation === "first-base-empty") {
    base.target = "";
    const later = owner.createElement("base");
    later.id = "extraTargetBase";
    later.target = "form-destination";
    owner.head.append(later);
  }
  const link = kind === "a" || kind === "area";
  const element = owner.createElement(link ? kind : "form");
  element.id = link ? "resolution-link" : "resolution-form";
  element[link ? "href" : "action"] = "https://target-resolution.test/submitted";
  if (target !== null) element.setAttribute("target", target);
  let control;
  if (!link && kind !== "form") {
    control = owner.createElement(kind);
    control.type = "submit";
    if (override !== null) control.setAttribute("formtarget", override);
    element.append(control);
  }
  const mutate = () => {
    if (mutation === "base-change") base.target = "submitter-destination";
    if (mutation === "target-change") element.target = "submitter-destination";
    if (mutation === "submitter-empty") control.setAttribute("formtarget", "");
  };
  element.addEventListener(link ? "click" : "formdata", mutate);
  owner.body.append(element);
  if (link) element.click();
  else if (kind === "form") element.submit();
  else control.click();
};
