use super::*;

#[test]
fn detached_marquee_numeric_attributes_follow_legacy_reflection() {
    let mut vm = new_storage_test_vm("https://detached-marquee-numeric.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const marquee = doc.createElement("marquee");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throws = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  };

  for (const name of ["loop", "scrollAmount", "scrollDelay"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLMarqueeElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable && descriptor.configurable, `${name} descriptor flags`);
    assert(!Object.prototype.hasOwnProperty.call(HTMLElement.prototype, name), `${name} owner`);
    assert(throws(() => descriptor.get.call(doc.createElement("div"))) === "TypeError", `${name} getter brand`);
    assert(throws(() => descriptor.set.call(doc.createElement("div"), 2)) === "TypeError", `${name} setter brand`);
  }

  for (const [raw, expected] of [
    [null, -1],
    ["a1", -1],
    ["-2", -1],
    ["0", -1],
    ["2", 2],
    [" 5 trailing", 5],
    ["2147483648", -1],
    ["\u000b7", -1]
  ]) {
    if (raw === null) marquee.removeAttribute("loop");
    else marquee.setAttribute("loop", raw);
    assert(marquee.loop === expected, `loop ${raw}`);
  }

  marquee.loop = 4;
  assert(marquee.loop === 4 && marquee.getAttribute("loop") === "4", "loop positive setter");
  marquee.loop = -1;
  assert(marquee.loop === -1 && marquee.getAttribute("loop") === "-1", "loop -1 setter");
  marquee.setAttribute("loop", "3");
  assert(throws(() => { marquee.loop = 0; }) === "IndexSizeError", "loop zero setter");
  assert(marquee.getAttribute("loop") === "3", "loop zero preserves attribute");
  assert(throws(() => { marquee.loop = -2; }) === "IndexSizeError", "loop negative setter");
  assert(marquee.getAttribute("loop") === "3", "loop negative preserves attribute");
  assert(throws(() => { marquee.loop = Symbol("loop"); }) === "TypeError", "loop symbol setter");
  assert(marquee.getAttribute("loop") === "3", "loop symbol preserves attribute");

  for (const [name, attribute, defaultValue, cases] of [
    ["scrollAmount", "scrollamount", 6, [[null, 6], ["aa", 6], ["-1", 6], ["0", 0], ["10", 10], [" +7tail", 7], ["2147483648", 6]]],
    ["scrollDelay", "scrolldelay", 85, [[null, 85], ["aa", 85], ["-1", 85], ["1", 1], ["100", 100], ["2147483648", 85]]]
  ]) {
    for (const [raw, expected] of cases) {
      if (raw === null) marquee.removeAttribute(attribute);
      else marquee.setAttribute(attribute, raw);
      assert(marquee[name] === expected, `${name} ${raw}`);
    }
    marquee[name] = 12;
    assert(marquee[name] === 12 && marquee.getAttribute(attribute) === "12", `${name} setter`);
    marquee[name] = -1;
    assert(marquee[name] === defaultValue && marquee.getAttribute(attribute) === String(defaultValue), `${name} wrapped setter`);
    marquee.setAttribute(attribute, "14");
    assert(throws(() => { marquee[name] = Symbol(name); }) === "TypeError", `${name} symbol setter`);
    assert(marquee.getAttribute(attribute) === "14", `${name} symbol preserves attribute`);
  }
  return "ok";
})()
"##,
        )
        .expect("detached marquee numeric reflection should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_body_legacy_accessors_use_owner_prototype() {
    let mut vm = new_storage_test_vm("https://detached-body-legacy-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const names = ["onload", "text", "link", "vLink", "aLink", "background"];
  const bodyOnlyNames = ["text", "link", "vLink", "aLink", "background"];
  for (const name of names) {
    accessor(HTMLBodyElement.prototype, name);
  }
  for (const name of bodyOnlyNames) {
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!(name in document.createElement("div")), `${name} should not be on div`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [body, label, usesWindow] of [
    [detachedDoc.body, "windowless", false],
    [document.createElement("body"), "created", true]
  ]) {
    for (const name of names) {
      assert(!own(body, name), `${label}.${name} should not be own before set`);
    }
    const handler = () => `${label}-load`;
    const priorWindowHandler = () => `${label}-prior-window-load`;
    window.onload = priorWindowHandler;
    body.onload = handler;
    assert(
      window.onload === (usesWindow ? handler : priorWindowHandler),
      `${label}.onload setter follows owner browsing context`
    );
    assert(
      body.onload === (usesWindow ? handler : null),
      `${label}.onload getter follows owner browsing context`
    );
    body.text = `${label}-text`;
    body.link = `${label}-link`;
    body.vLink = `${label}-vlink`;
    body.aLink = `${label}-alink`;
    body.background = `${label}-background`;
    assert(body.text === `${label}-text` && body.getAttribute("text") === `${label}-text`, `${label}.text`);
    assert(body.link === `${label}-link` && body.getAttribute("link") === `${label}-link`, `${label}.link`);
    assert(body.vLink === `${label}-vlink` && body.getAttribute("vlink") === `${label}-vlink`, `${label}.vLink`);
    assert(body.aLink === `${label}-alink` && body.getAttribute("alink") === `${label}-alink`, `${label}.aLink`);
    assert(body.background === `${label}-background` && body.getAttribute("background") === `${label}-background`, `${label}.background`);
    for (const name of ["text", "link", "vLink", "aLink"]) {
      body[name] = null;
      const attribute = name.toLowerCase();
      assert(body[name] === "", `${label}.${name} null getter`);
      assert(body.getAttribute(attribute) === "", `${label}.${name} null attribute`);
    }
    for (const name of names) {
      assert(!own(body, name), `${label}.${name} should not be own after set`);
      assert(delete body[name], `${label}.${name} delete`);
      assert(!own(body, name), `${label}.${name} should stay inherited`);
    }
    assert(body.onload === (usesWindow ? handler : null), `${label}.onload after delete`);
    assert(body.text === "", `${label}.text after delete`);
  }
  for (const name of ["text", "link", "vLink", "aLink"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, name);
    for (const receiver of [document.createElement("div"), {}]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "wrong")), `${name} setter receiver`);
    }
  }
  window.onload = null;
  return "ok";
})()
"#,
        )
        .expect("detached body legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_global_event_handler_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-global-event-handlers.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const click = accessor(HTMLElement.prototype, "onclick");
  accessor(HTMLElement.prototype, "onsubmit");
  assert(
    throwsTypeError(() => HTMLElement.prototype.onclick),
    "HTMLElement.prototype.onclick receiver brand"
  );
  assert(Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, "onload").get !==
    Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onload").get,
    "HTMLBodyElement.onload should keep body/window override");

  const detachedDoc = document.implementation.createHTMLDocument("");
  const detachedDiv = detachedDoc.createElement("div");
  const detachedForm = detachedDoc.createElement("form");
  const createdDiv = document.createElement("div");
  function handler() {}

  for (const [element, name, label] of [
    [detachedDiv, "onclick", "detachedDiv"],
    [detachedForm, "onsubmit", "detachedForm"],
    [createdDiv, "onclick", "createdDiv"],
  ]) {
    assert(!own(element, name), `${label}.${name} should not be own before set`);
    element[name] = handler;
    assert(element[name] === handler, `${label}.${name} handler`);
    assert(!own(element, name), `${label}.${name} should not be own after set`);
    assert(delete element[name], `${label}.${name} delete`);
    assert(!own(element, name), `${label}.${name} should stay inherited`);
    assert(element[name] === handler, `${label}.${name} after delete`);
  }

  assert(throwsTypeError(() => click.get.call({})), "forged getter receiver brand");
  return "ok";
})()
"#,
        )
        .expect("detached global event handler owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_media_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-media-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const link = doc.createElement("link");
  const source = doc.createElement("source");
  const style = doc.createElement("style");
  const meta = doc.createElement("meta");
  const div = doc.createElement("div");
  doc.head.append(link, style, meta);
  doc.body.append(source, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLLinkElement.prototype, "media");
  accessor(HTMLSourceElement.prototype, "media");
  accessor(HTMLStyleElement.prototype, "media");
  accessor(HTMLMetaElement.prototype, "media");
  assert(!own(HTMLElement.prototype, "media"), "media should not be on HTMLElement.prototype");
  assert(!("media" in div), "media should not be on div");

  for (const [element, label] of [
    [link, "link"],
    [source, "source"],
    [style, "style"],
    [meta, "meta"]
  ]) {
    assert(!own(element, "media"), `${label}.media should not be own before set`);
    element.media = `${label}-media`;
    assert(element.media === `${label}-media`, `${label}.media getter`);
    assert(element.getAttribute("media") === `${label}-media`, `${label}.media attr`);
    assert(!own(element, "media"), `${label}.media should not be own after set`);
    assert(delete element.media, `${label}.media delete`);
    assert(!own(element, "media"), `${label}.media should stay inherited`);
    assert(element.media === `${label}-media`, `${label}.media after delete`);
  }
  return "ok";
})()
"##,
        )
        .expect("detached media owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_element_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-html-media-element-prototypes.test/base/");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter shape`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const mediaWritable = [
    "src", "volume", "muted", "defaultMuted", "playbackRate", "currentTime",
    "autoplay", "controls", "loop"
  ];
  const mediaReadonly = [
    "paused", "duration", "ended", "seeking", "readyState", "networkState", "textTracks"
  ];
  const videoWritable = ["poster", "width", "height", "playsInline"];
  const videoReadonly = ["videoWidth", "videoHeight"];

  for (const name of mediaWritable) accessor(HTMLMediaElement.prototype, name, true);
  for (const name of mediaReadonly) accessor(HTMLMediaElement.prototype, name, false);
  for (const name of [...mediaWritable, ...mediaReadonly]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!own(HTMLAudioElement.prototype, name), `${name} should not duplicate on audio`);
    assert(!own(HTMLVideoElement.prototype, name), `${name} should not duplicate on video`);
  }
  for (const name of videoWritable) accessor(HTMLVideoElement.prototype, name, true);
  for (const name of videoReadonly) accessor(HTMLVideoElement.prototype, name, false);
  for (const name of [...videoWritable, ...videoReadonly]) {
    assert(!own(HTMLMediaElement.prototype, name), `${name} should not live on HTMLMediaElement`);
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!own(HTMLAudioElement.prototype, name), `${name} should not live on audio`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  const mediaCases = [
    [document.createElement("audio"), "live-audio"],
    [detachedDoc.createElement("audio"), "detached-audio"],
    [document.createElement("video"), "live-video"],
    [detachedDoc.createElement("video"), "detached-video"]
  ];

  for (const [element, label] of mediaCases) {
    for (const name of [...mediaWritable, ...mediaReadonly]) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    assert(element.paused === true, `${label}.paused default`);
    assert(element.volume === 1, `${label}.volume default`);
    assert(element.muted === false, `${label}.muted default`);
    assert(Number.isNaN(element.duration), `${label}.duration default`);
    assert(element.ended === false, `${label}.ended default`);
    assert(typeof element.seeking === "boolean", `${label}.seeking default`);
    assert(element.readyState === element.HAVE_NOTHING, `${label}.readyState default`);
    assert(element.networkState === element.NETWORK_EMPTY, `${label}.networkState default`);
    assert(element.textTracks === element.textTracks, `${label}.textTracks cache`);

    element.src = `${label}.mp4`;
    element.volume = 0.25;
    element.muted = true;
    element.defaultMuted = true;
    element.playbackRate = 1.5;
    element.currentTime = 12.25;
    element.autoplay = true;
    element.controls = true;
    element.loop = true;

    assert(element.src.includes(`${label}.mp4`), `${label}.src getter`);
    assert(Math.abs(element.volume - 0.25) < 0.0001, `${label}.volume set`);
    assert(element.muted === true, `${label}.muted set`);
    assert(element.defaultMuted === true && element.hasAttribute("muted"), `${label}.defaultMuted set`);
    assert(Math.abs(element.playbackRate - 1.5) < 0.0001, `${label}.playbackRate set`);
    assert(Math.abs(element.currentTime - 12.25) < 0.0001, `${label}.currentTime set`);
    assert(element.autoplay === true && element.hasAttribute("autoplay"), `${label}.autoplay set`);
    assert(element.controls === true && element.hasAttribute("controls"), `${label}.controls set`);
    assert(element.loop === true && element.hasAttribute("loop"), `${label}.loop set`);

    for (const name of [...mediaWritable, ...mediaReadonly]) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(element.src.includes(`${label}.mp4`), `${label}.src after delete`);
    assert(element.muted === true, `${label}.muted after delete`);
    assert(element.defaultMuted === true, `${label}.defaultMuted after delete`);
  }

  for (const [video, label] of [
    [document.createElement("video"), "live-video-only"],
    [detachedDoc.createElement("video"), "detached-video-only"]
  ]) {
    for (const name of [...videoWritable, ...videoReadonly]) {
      assert(!own(video, name), `${label}.${name} should not be own before set`);
    }
    video.poster = `${label}.png`;
    video.width = 320;
    video.height = 180;
    video.playsInline = true;
    assert(video.poster.includes(`${label}.png`), `${label}.poster getter`);
    assert(video.width === 320 && video.getAttribute("width") === "320", `${label}.width set`);
    assert(video.height === 180 && video.getAttribute("height") === "180", `${label}.height set`);
    assert(video.playsInline === true && video.hasAttribute("playsinline"), `${label}.playsInline set`);
    assert(video.videoWidth === 0, `${label}.videoWidth default`);
    assert(video.videoHeight === 0, `${label}.videoHeight default`);
    for (const name of [...videoWritable, ...videoReadonly]) {
      assert(!own(video, name), `${label}.${name} should not be own after set`);
      assert(delete video[name], `${label}.${name} delete`);
      assert(!own(video, name), `${label}.${name} should stay inherited`);
    }
    assert(video.width === 320 && video.height === 180, `${label}.dimensions after delete`);
    assert(video.playsInline === true, `${label}.playsInline after delete`);
  }

  return "ok";
})()
"##,
        )
        .expect("detached HTML media element owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-media-receiver-brand.test/base/");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const doc = document.implementation.createHTMLDocument("");
  const audio = doc.createElement("audio");
  const video = doc.createElement("video");
  const div = doc.createElement("div");
  const img = doc.createElement("img");
  const source = doc.createElement("source");
  const text = doc.createTextNode("x");
  const mediaBadReceivers = [{}, text, div, img, source];
  const videoBadReceivers = [{}, text, div, img, source, audio];

  const mediaValues = {
    crossOrigin: "anonymous",
    loading: "lazy",
    preload: "metadata",
    src: "clip.mp4",
    volume: 0.25,
    muted: true,
    defaultMuted: true,
    playbackRate: 1.5,
    currentTime: 2,
    autoplay: true,
    controls: true,
    loop: true
  };
  const mediaNames = [
    "crossOrigin", "loading", "preload", "src", "volume", "muted", "defaultMuted",
    "playbackRate", "currentTime", "paused", "duration", "ended", "seeking",
    "readyState", "networkState", "textTracks", "autoplay", "controls", "loop"
  ];
  for (const name of mediaNames) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(audio) !== "undefined", `${name} audio getter`);
    assert(typeof descriptor.get.call(video) !== "undefined", `${name} video getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(audio, mediaValues[name]);
      descriptor.set.call(video, mediaValues[name]);
      assert(!Object.prototype.hasOwnProperty.call(audio, name), `${name} audio inherited`);
      assert(!Object.prototype.hasOwnProperty.call(video, name), `${name} video inherited`);
    }
    for (const receiver of mediaBadReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, mediaValues[name])), `${name} setter receiver`);
      }
    }
  }

  const mediaMethods = {
    play: [],
    pause: [],
    load: [],
    canPlayType: ["audio/mpeg"],
    addTextTrack: ["subtitles"]
  };
  for (const [name, args] of Object.entries(mediaMethods)) {
    const method = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, name).value;
    assert(typeof method === "function", `${name} method`);
    method.call(audio, ...args);
    method.call(video, ...args);
    for (const receiver of mediaBadReceivers) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
  }

  const videoValues = {
    poster: "poster.png",
    width: 320,
    height: 180,
    playsInline: true
  };
  for (const name of ["poster", "width", "height", "playsInline", "videoWidth", "videoHeight"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLVideoElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(video) !== "undefined", `${name} getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(video, videoValues[name]);
      assert(!Object.prototype.hasOwnProperty.call(video, name), `${name} inherited`);
    }
    for (const receiver of videoBadReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, videoValues[name])), `${name} setter receiver`);
      }
    }
  }
  return "ok";
})()
"##,
        )
        .expect("detached HTML media element receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_text_reflection_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-text-reflection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const cases = [
    ["a", HTMLAnchorElement.prototype, "anchor old", "anchor new"],
    ["title", HTMLTitleElement.prototype, "title old", "title new"],
    ["option", HTMLOptionElement.prototype, "option old", "option new"]
  ];
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  for (const [tag, prototype, oldText, newText] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "text");
    assert(!!descriptor, `${tag} text descriptor missing`);
    assert(typeof descriptor.get === "function", `${tag} text getter`);
    assert(typeof descriptor.set === "function", `${tag} text setter`);
    assert(descriptor.enumerable === true, `${tag} text enumerable`);
    assert(descriptor.configurable === true, `${tag} text configurable`);

    const element = doc.createElement(tag);
    element.textContent = oldText;
    assert(!own(element, "text"), `${tag} text should not be own initially`);
    assert(element.text === oldText, `${tag} text getter`);
    element.text = newText;
    assert(element.textContent === newText, `${tag} text setter content`);
    assert(element.text === newText, `${tag} text setter getter`);
    assert(!own(element, "text"), `${tag} text should stay inherited after set`);
    assert(delete element.text, `${tag} delete text`);
    assert(!own(element, "text"), `${tag} text should stay inherited after delete`);
    element.text = oldText;
    assert(element.text === oldText, `${tag} text after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached text reflection prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_specialized_url_resource_properties_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-specialized-url-resource.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const anchor = doc.createElement("a");
  const area = doc.createElement("area");
  const image = doc.createElement("img");
  const iframe = doc.createElement("iframe");
  doc.body.append(anchor, area, image, iframe);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const anchorNames = [
    "href",
    "protocol",
    "host",
    "hostname",
    "port",
    "pathname",
    "search",
    "hash"
  ];
  method(HTMLAnchorElement.prototype, "toString", 0);
  assert(!own(anchor, "toString"), "anchor toString should not be own");
  for (const name of anchorNames) {
    accessor(HTMLAnchorElement.prototype, name, true);
    assert(!own(anchor, name), `anchor ${name} should not be own`);
    accessor(HTMLAreaElement.prototype, name, true);
    assert(!own(area, name), `area ${name} should not be own`);
  }
  for (const name of ["src", "srcset"]) {
    accessor(HTMLImageElement.prototype, name, true);
    assert(!own(image, name), `image ${name} should not be own`);
  }
  method(HTMLImageElement.prototype, "decode", 0);
  assert(!own(image, "decode"), "image decode should not be own");
  accessor(HTMLIFrameElement.prototype, "src", true);
  accessor(HTMLIFrameElement.prototype, "srcdoc", true);
  accessor(HTMLIFrameElement.prototype, "contentDocument", false);
  accessor(HTMLIFrameElement.prototype, "contentWindow", false);
  for (const name of ["src", "srcdoc", "contentDocument", "contentWindow"]) {
    assert(!own(iframe, name), `iframe ${name} should not be own`);
  }

  anchor.href = "https://old.test/base/path?x=1#old";
  anchor.protocol = "http";
  anchor.host = "example.test:8080";
  anchor.pathname = "next";
  anchor.search = "q=2";
  anchor.hash = "done";
  assert(anchor.href === "http://example.test:8080/next?q=2#done", "anchor href mutation");
  assert(anchor.protocol === "http:", "anchor protocol");
  assert(anchor.host === "example.test:8080", "anchor host");
  assert(anchor.hostname === "example.test", "anchor hostname");
  assert(anchor.port === "8080", "anchor port");
  assert(anchor.pathname === "/next", "anchor pathname");
  assert(anchor.search === "?q=2", "anchor search");
  assert(anchor.hash === "#done", "anchor hash");
  for (const name of anchorNames) {
    assert(delete anchor[name], `delete anchor ${name}`);
    assert(!own(anchor, name), `anchor ${name} should stay inherited`);
  }
  assert(anchor.href === "http://example.test:8080/next?q=2#done", "anchor href after delete");
  assert(anchor.toString() === anchor.href, "anchor toString after delete");
  assert(HTMLAnchorElement.prototype.toString.call(anchor) === anchor.href, "anchor toString descriptor call");
  assert(delete anchor.toString, "delete anchor toString");
  assert(!own(anchor, "toString"), "anchor toString should stay inherited");
  assert(anchor.toString() === anchor.href, "anchor toString after toString delete");
  area.href = "https://area.test/map";
  assert(area.href === "https://area.test/map", "area href reflection");
  assert(delete area.href, "delete area href");
  assert(!own(area, "href"), "area href should stay inherited");
  assert(area.href === "https://area.test/map", "area href after delete");

  image.src = "https://cdn.test/image.png";
  image.srcset = "small.png 1x, large.png 2x";
  assert(image.src === "https://cdn.test/image.png", "image src reflection");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset reflection");
  for (const name of ["src", "srcset"]) {
    assert(delete image[name], `delete image ${name}`);
    assert(!own(image, name), `image ${name} should stay inherited`);
  }
  assert(image.src === "https://cdn.test/image.png", "image src after delete");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset after delete");
  assert(typeof image.decode().then === "function", "image decode behavior");

  iframe.src = "https://frame.test/initial.html";
  assert(iframe.src === "https://frame.test/initial.html", "iframe src reflection");
  iframe.srcdoc = "<!doctype html><body><p id='marker'>first</p></body>";
  const firstDocument = iframe.contentDocument;
  const firstWindow = iframe.contentWindow;
  assert(firstDocument.body.textContent.trim() === "first", "first srcdoc document");
  assert(firstWindow.document === firstDocument, "first contentWindow document");
  iframe.srcdoc = "<!doctype html><body><p id='marker'>second</p></body>";
  const secondDocument = iframe.contentDocument;
  assert(secondDocument.body.textContent.trim() === "second", "srcdoc clears cached document");
  assert(secondDocument !== firstDocument, "srcdoc replacement materializes new document");
  for (const name of ["src", "srcdoc", "contentDocument", "contentWindow"]) {
    assert(delete iframe[name], `delete iframe ${name}`);
    assert(!own(iframe, name), `iframe ${name} should stay inherited`);
  }
  assert(iframe.contentDocument.body.textContent.trim() === "second", "iframe after delete");

  return "ok";
})()
"##,
        )
        .expect("detached specialized URL/resource prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn hyperlink_stringifiers_use_native_href_and_enforce_owner_brand() {
    let mut vm = new_storage_test_vm("https://hyperlink-stringifier.test/base/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const detachedDocument = document.implementation.createHTMLDocument("");
  const cases = [
    [HTMLAnchorElement.prototype, "a", document.createElement("a"), detachedDocument.createElement("a")],
    [HTMLAreaElement.prototype, "area", document.createElement("area"), detachedDocument.createElement("area")]
  ];

  for (const [prototype, label, live, detached] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "toString");
    assert(!!descriptor, `${label} toString descriptor`);
    assert(typeof descriptor.value === "function", `${label} toString method`);
    assert(descriptor.value.length === 0, `${label} toString length`);

    for (const [element, suffix] of [[live, "live"], [detached, "detached"]]) {
      const expected = `https://example.test/${label}/${suffix}`;
      element.setAttribute("href", expected);
      assert(descriptor.value.call(element) === expected, `${label} ${suffix} value`);
      Object.defineProperty(element, "href", {
        configurable: true,
        get() { throw new Error("stringifier read the JavaScript href property"); }
      });
      assert(descriptor.value.call(element) === expected, `${label} ${suffix} shadowed href`);
    }
  }

  const anchorToString = HTMLAnchorElement.prototype.toString;
  const areaToString = HTMLAreaElement.prototype.toString;
  const invalidReceivers = [null, undefined, {}, window, document.createElement("div")];
  for (const receiver of invalidReceivers) {
    assert(throwsTypeError(() => anchorToString.call(receiver)), "anchor invalid receiver");
    assert(throwsTypeError(() => areaToString.call(receiver)), "area invalid receiver");
  }
  assert(throwsTypeError(() => anchorToString.call(document.createElement("area"))), "anchor rejects area");
  assert(throwsTypeError(() => areaToString.call(document.createElement("a"))), "area rejects anchor");
  return "ok";
})()
"#,
        )
        .expect("hyperlink stringifiers should enforce their owner interface");

    assert_eq!(result, "ok");
}

#[test]
fn detached_canvas_image_state_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-canvas-image-resource.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLCanvasElement.prototype, "width", true);
  accessor(HTMLCanvasElement.prototype, "height", true);
  accessor(HTMLImageElement.prototype, "width", true);
  accessor(HTMLImageElement.prototype, "height", true);
  accessor(HTMLImageElement.prototype, "naturalWidth", false);
  accessor(HTMLImageElement.prototype, "naturalHeight", false);
  accessor(HTMLImageElement.prototype, "isMap", true);
  accessor(HTMLImageElement.prototype, "complete", false);
  accessor(HTMLImageElement.prototype, "currentSrc", false);
  for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  const parsed = new DOMParser().parseFromString(
    "<!doctype html><html><body><canvas></canvas><img></body></html>",
    "text/html"
  );
  const liveCanvas = document.createElement("canvas");
  const detachedCanvas = parsed.querySelector("canvas");
  const liveImage = document.createElement("img");
  const detachedImage = parsed.querySelector("img");
  const canvasWidth = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "width");
  const canvasHeight = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "height");

  for (const canvas of [liveCanvas, detachedCanvas]) {
    for (const name of ["width", "height"]) {
      assert(!own(canvas, name), `canvas ${name} should not be own before set`);
    }
    canvasWidth.set.call(canvas, 640);
    canvasHeight.set.call(canvas, 480);
    assert(canvasWidth.get.call(canvas) === 640, "canvas width value");
    assert(canvasHeight.get.call(canvas) === 480, "canvas height value");
    assert(canvas.getAttribute("width") === "640", "canvas width attr");
    assert(canvas.getAttribute("height") === "480", "canvas height attr");
    assert(!own(canvas, "width"), "canvas width should stay inherited after set");
    assert(!own(canvas, "height"), "canvas height should stay inherited after set");
    const context = HTMLCanvasElement.prototype.getContext.call(canvas, "2d");
    assert(Object.prototype.toString.call(context) === "[object CanvasRenderingContext2D]", "canvas context");
    assert(HTMLCanvasElement.prototype.toDataURL.call(canvas).startsWith("data:image/png;base64,"), "canvas data URL");
    const offscreen = HTMLCanvasElement.prototype.transferControlToOffscreen.call(canvas);
    assert(offscreen instanceof OffscreenCanvas, "canvas offscreen instance");
    assert(offscreen.width === 640, "canvas offscreen width");
    assert(offscreen.height === 480, "canvas offscreen height");
    assert(delete canvas.width, "canvas width delete");
    assert(delete canvas.height, "canvas height delete");
    assert(canvas.width === 640, "canvas width after delete");
    assert(canvas.height === 480, "canvas height after delete");
  }

  for (const image of [liveImage, detachedImage]) {
    for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
      assert(!own(image, name), `image ${name} should not be own before set`);
    }
    image.width = 33;
    image.height = 44;
    image.isMap = true;
    assert(image.width === 33, "image width value");
    assert(image.height === 44, "image height value");
    assert(image.naturalWidth === 0, "image naturalWidth default");
    assert(image.naturalHeight === 0, "image naturalHeight default");
    assert(image.isMap === true, "image isMap value");
    assert(image.complete === true, "image complete without source");
    assert(image.currentSrc === "", "image currentSrc without source");
    assert(image.getAttribute("width") === "33", "image width attr");
    assert(image.getAttribute("height") === "44", "image height attr");
    assert(image.getAttribute("ismap") === "", "image ismap attr");
    for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
      assert(!own(image, name), `image ${name} should stay inherited after mutation`);
      assert(delete image[name], `image ${name} delete`);
      assert(!own(image, name), `image ${name} should stay inherited after delete`);
    }
    assert(image.width === 33, "image width after delete");
    assert(image.height === 44, "image height after delete");
    assert(image.isMap === true, "image isMap after delete");
  }
  return "ok";
})()
"#,
        )
        .expect("canvas and image state prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_resource_template_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-resource-template.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error.name === "TypeError";
    }
  };
  const accessor = (prototype, name, hasSetter = true) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const name of ["type", "media", "blocking", "disabled"]) {
    accessor(HTMLStyleElement.prototype, name);
  }
  accessor(HTMLLinkElement.prototype, "disabled");
  accessor(HTMLIFrameElement.prototype, "csp");
  for (const name of ["integrity", "rev", "type"]) {
    accessor(HTMLLinkElement.prototype, name);
  }
  accessor(HTMLIFrameElement.prototype, "sandbox");
  accessor(HTMLIFrameElement.prototype, "allowFullscreen");
  for (const name of ["default", "kind", "src", "srclang", "label"]) {
    accessor(HTMLTrackElement.prototype, name);
  }
  accessor(HTMLTrackElement.prototype, "readyState", false);
  accessor(HTMLTrackElement.prototype, "track", false);

  const div = document.createElement("div");
  for (const name of ["blocking", "csp", "sandbox", "allowFullscreen", "default", "srclang", "readyState", "track"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }
  const cspDescriptor = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, "csp");
  assert(throwsTypeError(() => cspDescriptor.get.call(div)), "iframe csp getter brand");
  assert(throwsTypeError(() => cspDescriptor.set.call(div, "script-src 'none'")), "iframe csp setter brand");
  assert(!div.hasAttribute("csp"), "iframe csp setter must not mutate an incompatible receiver");

  const detachedDocument = document.implementation.createHTMLDocument("");
  const styleElements = [document.createElement("style"), detachedDocument.createElement("style")];
  const linkElements = [document.createElement("link"), detachedDocument.createElement("link")];
  const iframeElements = [document.createElement("iframe"), detachedDocument.createElement("iframe")];
  const trackElements = [document.createElement("track"), detachedDocument.createElement("track")];

  for (const style of styleElements) {
    for (const name of ["type", "media", "blocking", "disabled"]) {
      assert(!own(style, name), `style.${name} should not be own before set`);
    }
    assert(style.type === "", "style type default");
    style.type = "text/less";
    style.media = "print";
    style.blocking = "render";
    style.disabled = true;
    assert(style.type === "text/less" && style.getAttribute("type") === "text/less", "style type");
    assert(style.media === "print" && style.getAttribute("media") === "print", "style media");
    assert(style.blocking === "render" && style.getAttribute("blocking") === "render", "style blocking");
    assert(typeof style.disabled === "boolean", "style disabled boolean");
    for (const name of ["type", "media", "blocking", "disabled"]) {
      assert(!own(style, name), `style.${name} should stay inherited after set`);
      assert(delete style[name], `style.${name} delete`);
      assert(!own(style, name), `style.${name} should stay inherited after delete`);
    }
    assert(style.type === "text/less", "style type after delete");
    assert(style.media === "print", "style media after delete");
    assert(style.blocking === "render", "style blocking after delete");
  }

  for (const link of linkElements) {
    for (const name of ["disabled", "integrity", "rev", "type"]) {
      assert(!own(link, name), `link.${name} should not be own before set`);
    }
    assert(link.integrity === "" && link.rev === "" && link.type === "", "link string defaults");
    link.integrity = "sha256-test";
    link.rev = "made";
    link.type = "text/css";
    assert(link.integrity === "sha256-test" && link.getAttribute("integrity") === "sha256-test", "link integrity");
    assert(link.rev === "made" && link.getAttribute("rev") === "made", "link rev");
    assert(link.type === "text/css" && link.getAttribute("type") === "text/css", "link type");
    link.disabled = true;
    assert(link.disabled === true, "link disabled true");
    assert(link.getAttribute("disabled") === "", "link disabled attr");
    for (const name of ["disabled", "integrity", "rev", "type"]) {
      assert(!own(link, name), `link.${name} should stay inherited after set`);
    }
    link.disabled = false;
    assert(link.disabled === false, "link disabled false");
    assert(link.getAttribute("disabled") === null, "link disabled attr removed");
    assert(!own(link, "disabled"), "link.disabled should stay inherited after false");
    for (const name of ["disabled", "integrity", "rev", "type"]) {
      assert(delete link[name], `link.${name} delete`);
      assert(!own(link, name), `link.${name} should stay inherited after delete`);
    }
    assert(link.integrity === "sha256-test" && link.rev === "made" && link.type === "text/css", "link strings after delete");
  }

  for (const iframe of iframeElements) {
    for (const name of ["csp", "sandbox", "allowFullscreen"]) {
      assert(!own(iframe, name), `iframe.${name} should not be own before set`);
    }
    assert(iframe.csp === "", "iframe csp default");
    iframe.csp = 123456;
    const sandbox = iframe.sandbox;
    assert(Object.prototype.toString.call(sandbox) === "[object DOMTokenList]", "iframe sandbox type");
    assert(sandbox === iframe.sandbox, "iframe sandbox SameObject");
    iframe.sandbox = "allow-scripts";
    iframe.allowFullscreen = true;
    assert(iframe.csp === "123456" && iframe.getAttribute("csp") === "123456", "iframe csp");
    assert(sandbox.value === "allow-scripts" && iframe.getAttribute("sandbox") === "allow-scripts", "iframe sandbox");
    assert(sandbox.supports("ALLOW-SCRIPTS"), "iframe sandbox supported token");
    assert(iframe.allowFullscreen === true && iframe.getAttribute("allowfullscreen") === "", "iframe allowFullscreen");
    for (const name of ["csp", "sandbox", "allowFullscreen"]) {
      assert(!own(iframe, name), `iframe.${name} should stay inherited after set`);
      assert(delete iframe[name], `iframe.${name} delete`);
      assert(!own(iframe, name), `iframe.${name} should stay inherited after delete`);
    }
    iframe.setAttribute("csp", "default-src 'self'");
    assert(iframe.csp === "default-src 'self'", "iframe csp after delete");
    assert(iframe.sandbox === sandbox && sandbox.value === "allow-scripts", "iframe sandbox after delete");
    assert(iframe.allowFullscreen === true, "iframe allowFullscreen after delete");
  }

  for (const track of trackElements) {
    for (const name of ["default", "kind", "src", "srclang", "label", "readyState", "track"]) {
      assert(!own(track, name), `track.${name} should not be own before set`);
    }
    track.default = true;
    track.kind = "CAPTIONS";
    track.src = "captions.vtt";
    track.srclang = "en";
    track.label = "English";
    assert(track.default === true && track.getAttribute("default") === "", "track default");
    assert(track.kind === "captions" && track.getAttribute("kind") === "CAPTIONS", "track kind");
    assert(track.src.includes("captions.vtt") && track.getAttribute("src") === "captions.vtt", "track src");
    assert(track.srclang === "en" && track.getAttribute("srclang") === "en", "track srclang");
    assert(track.label === "English" && track.getAttribute("label") === "English", "track label");
    assert(track.readyState === 0, "track readyState default");
    assert(track.track && track.track.kind === "captions", "track TextTrack kind");
    for (const name of ["default", "kind", "src", "srclang", "label", "readyState", "track"]) {
      assert(!own(track, name), `track.${name} should stay inherited after set`);
      assert(delete track[name], `track.${name} delete`);
      assert(!own(track, name), `track.${name} should stay inherited after delete`);
    }
    assert(track.default === true, "track default after delete");
    assert(track.kind === "captions", "track kind after delete");
    assert(track.readyState === 0, "track readyState after delete");
    assert(track.track && track.track.kind === "captions", "track TextTrack after delete");
  }
  return "ok";
})()
"##,
        )
        .expect("detached resource template owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_node_tree_accessors_use_standard_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-node-tree-accessors.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const first = doc.createElement("section");
  const spacer = doc.createTextNode("gap");
  const second = doc.createElement("article");
  const text = doc.createTextNode("alpha");
  const doctype = doc.implementation.createDocumentType("html", "", "");
  first.appendChild(text);
  doc.body.append(first, spacer, second);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, setter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === setter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const nodeNames = [
    "nodeType",
    "nodeName",
    "parentNode",
    "parentElement",
    "ownerDocument",
    "childNodes",
    "firstChild",
    "lastChild",
    "previousSibling",
    "nextSibling",
    "isConnected"
  ];
  for (const name of nodeNames) {
    accessor(Node.prototype, name, "undefined");
    for (const object of [doc, doctype, doc.body, first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const parentNames = ["children", "firstElementChild", "lastElementChild", "childElementCount"];
  for (const name of parentNames) {
    accessor(Element.prototype, name, "undefined");
    for (const object of [doc, doc.body, first]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const siblingNames = ["previousElementSibling", "nextElementSibling"];
  for (const name of siblingNames) {
    accessor(Element.prototype, name, "undefined");
    accessor(CharacterData.prototype, name, "undefined");
    for (const object of [first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const nodeValueDescriptor = accessor(Node.prototype, "nodeValue", "function");
  const textContentDescriptor = accessor(Node.prototype, "textContent", "function");
  for (const name of ["nodeValue", "textContent"]) {
    for (const object of [doc, doctype, doc.body, first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }
  assert(nodeValueDescriptor.get.call(text) === "alpha", "nodeValue prototype getter");
  assert(textContentDescriptor.get.call(first) === "alpha", "textContent prototype getter");

  assert(first.nodeType === 1, "element nodeType");
  assert(first.nodeName === "SECTION", "element nodeName");
  assert(text.nodeType === 3, "text nodeType");
  assert(text.nodeName === "#text", "text nodeName");
  assert(doctype.nodeType === 10, "doctype nodeType");
  assert(doctype.nodeName === "html", "doctype nodeName");
  assert(doctype.nodeValue === null, "doctype nodeValue");
  assert(doctype.parentNode === null, "doctype parentNode");
  assert(doctype.ownerDocument === doc, "doctype ownerDocument");
  assert(doc.ownerDocument === null, "document ownerDocument");
  assert(first.ownerDocument === doc, "element ownerDocument");
  assert(first.parentNode === doc.body, "parentNode");
  assert(first.parentElement === doc.body, "parentElement");
  assert(first.childNodes.length === 1, "childNodes length");
  assert(first.firstChild === text, "firstChild");
  assert(first.lastChild === text, "lastChild");
  assert(first.nextSibling === spacer, "nextSibling");
  assert(spacer.previousSibling === first, "previousSibling");
  assert(doc.body.children.length === 2, "children length");
  assert(doc.body.firstElementChild === first, "firstElementChild");
  assert(doc.body.lastElementChild === second, "lastElementChild");
  assert(doc.body.childElementCount === 2, "childElementCount");
  assert(spacer.previousElementSibling === first, "previousElementSibling");
  assert(spacer.nextElementSibling === second, "nextElementSibling");
  nodeValueDescriptor.set.call(text, "beta");
  assert(text.nodeValue === "beta", "nodeValue prototype setter");
  assert(first.textContent === "beta", "textContent after nodeValue setter");
  nodeValueDescriptor.set.call(text, null);
  assert(text.nodeValue === "", "nodeValue null setter");
  textContentDescriptor.set.call(first, "gamma");
  assert(first.textContent === "gamma", "textContent prototype setter");
  assert(first.childNodes.length === 1, "textContent replacement child count");
  assert(first.firstChild.nodeType === 3, "textContent replacement child type");
  assert(!own(first, "textContent"), "textContent should stay inherited after set");
  assert(!own(first.firstChild, "nodeValue"), "nodeValue should stay inherited after replacement");
  nodeValueDescriptor.set.call(first, "ignored");
  assert(first.nodeValue === null, "element nodeValue setter ignored");
  textContentDescriptor.set.call(doctype, "ignored");
  textContentDescriptor.set.call(doc, "ignored");
  assert(doctype.textContent === null, "doctype textContent setter ignored");
  assert(doc.textContent === null, "document textContent setter ignored");
  assert(delete first.nodeType, "delete inherited nodeType");
  assert(first.nodeType === 1, "nodeType after delete");

  return "ok";
})()
"##,
        )
        .expect("detached Node and DOM mixin prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_pointer_capture_methods_use_element_prototype_no_frame_behavior() {
    let mut vm = new_storage_test_vm("https://detached-pointer-capture.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const target = doc.createElement("div");
  doc.body.appendChild(target);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const method = (name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} value`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.value;
  };
  const outcome = (callback) => {
    try {
      const value = callback();
      return `OK:${value === undefined ? "undefined" : String(value)}`;
    } catch (error) {
      return `ERR:${error.name}:${error.code || ""}`;
    }
  };

  const set = method("setPointerCapture", 1);
  const release = method("releasePointerCapture", 1);
  const has = method("hasPointerCapture", 1);
  for (const name of ["setPointerCapture", "releasePointerCapture", "hasPointerCapture"]) {
    assert(!own(target, name), `${name} should not be own`);
    assert(!own(doc.body, name), `${name} should not be own on body`);
  }

  return [
    outcome(() => target.setPointerCapture(1)),
    outcome(() => target.releasePointerCapture(1)),
    outcome(() => target.hasPointerCapture(1)),
    outcome(() => set.call(target, 1)),
    outcome(() => release.call(target, 1)),
    outcome(() => has.call(target, 1))
  ].join("|");
})()
"#,
        )
        .expect("detached pointer capture prototype methods should evaluate");

    assert_eq!(
        result,
        "OK:undefined|OK:undefined|OK:false|OK:undefined|OK:undefined|OK:false"
    );
}

#[test]
fn detached_label_accessors_use_html_label_element_prototype() {
    let mut vm = new_storage_test_vm("https://detached-label-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const explicitLabel = doc.createElement("label");
  const explicitInput = doc.createElement("input");
  const implicitLabel = doc.createElement("label");
  const implicitInput = doc.createElement("textarea");
  explicitInput.id = "target";
  explicitLabel.htmlFor = "target";
  implicitLabel.append("implicit", implicitInput);
  form.append(explicitLabel, explicitInput, implicitLabel);
  doc.body.append(form);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLLabelElement.prototype, "htmlFor", true);
  accessor(HTMLLabelElement.prototype, "control", false);
  accessor(HTMLLabelElement.prototype, "form", false);
  for (const label of [explicitLabel, implicitLabel]) {
    assert(!own(label, "htmlFor"), "htmlFor should not be own");
    assert(!own(label, "control"), "control should not be own");
    assert(!own(label, "form"), "form should not be own");
  }

  assert(explicitLabel.htmlFor === "target", "htmlFor reflection");
  assert(explicitLabel.control === explicitInput, "explicit control");
  assert(implicitLabel.control === implicitInput, "implicit control");
  assert(explicitLabel.form === form, "explicit form");
  assert(implicitLabel.form === form, "implicit form");
  assert(delete explicitLabel.htmlFor, "delete htmlFor");
  assert(delete explicitLabel.control, "delete control");
  assert(delete explicitLabel.form, "delete form");
  explicitLabel.htmlFor = "target";
  explicitLabel.control = null;
  explicitLabel.form = null;
  assert(!own(explicitLabel, "htmlFor"), "htmlFor should stay inherited");
  assert(!own(explicitLabel, "control"), "control should stay inherited");
  assert(!own(explicitLabel, "form"), "form should stay inherited");
  assert(explicitLabel.htmlFor === "target", "htmlFor after assignment");
  assert(explicitLabel.control === explicitInput, "control after assignment");
  assert(explicitLabel.form === form, "form after assignment");
  return "ok";
})()
"#,
        )
        .expect("detached label prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_document_title_getter_and_setter_walk_full_tree() {
    let mut vm = new_storage_test_vm("https://detached-title.test/");
    let result = vm
        .eval(
            r#"
(() => {
  // createHTMLDocument flows through the detached_surface accessor path that
  // exposes a real `title` getter/setter (unlike DOMParser snapshots, which
  // expose `title` as a plain own-property and are tracked separately).
  const doc = document.implementation.createHTMLDocument('ORIG');
  const out = [];
  out.push('initial=' + doc.title);
  doc.title = 'UPDATED';
  out.push('updated=' + doc.title);
  // Append a <title> directly under <body>; the head-side title still wins
  // because it is first in tree order.
  const bodyTitle = doc.createElement('title');
  bodyTitle.appendChild(doc.createTextNode('FROM_BODY'));
  doc.body.appendChild(bodyTitle);
  out.push('headStillWins=' + doc.title);
  // Remove the head; the body-side <title> is now the first title in tree
  // order, so the setter should overwrite that existing element.
  const head = doc.getElementsByTagName('head')[0];
  if (head) head.parentNode.removeChild(head);
  doc.title = 'REPLACED_BODY';
  out.push('replacedBody=' + doc.title);
  // Now drop every title element. Setter has no title and no head → no-op.
  const titles = Array.from(doc.getElementsByTagName('title'));
  for (const t of titles) t.parentNode.removeChild(t);
  doc.title = 'SHOULD_NOT_APPLY';
  out.push('afterHeadGone=' + doc.title);
  return out.join('|');
})()
"#,
        )
        .expect("detached document.title spec behavior should evaluate");
    assert_eq!(
        result,
        "initial=ORIG|updated=UPDATED|headStillWins=UPDATED|replacedBody=REPLACED_BODY|afterHeadGone="
    );
}

#[test]
fn detached_native_remove_dispatches_through_runtime_pipeline() {
    let mut vm = new_storage_test_vm("https://detached-remove-runtime-pipeline.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  window.detachedRemovePipelineEvents = [];
  class DetachedRemovePipelineElement extends HTMLElement {
    disconnectedCallback() {
      window.detachedRemovePipelineEvents.push([
        this.isConnected,
        this.parentNode === null,
        doc.body.childNodes.length
      ].join(":"));
    }
  }
  customElements.define("detached-remove-pipeline", DetachedRemovePipelineElement);
  const element = document.createElement("detached-remove-pipeline");
  doc.body.appendChild(element);
  window.detachedRemovePipelineEvents.length = 0;
  doc.body.removeChild(element);
  return [
    JSON.stringify(window.detachedRemovePipelineEvents),
    element.parentNode === null,
    doc.body.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("detached native remove runtime pipeline timing should evaluate");

    assert_eq!(result, r#"["false:true:0"]|true|0"#);
}

#[test]
fn detached_document_state_accessors_are_declared_on_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-document-state-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString("<html><body></body></html>", "text/html");
  const xml = parser.parseFromString("<root></root>", "application/xml");
  const htmlProto = Object.getPrototypeOf(html);
  const xmlProto = Object.getPrototypeOf(xml);
  const descriptorOwner = (object, name) => {
    for (let current = object; current; current = Object.getPrototypeOf(current)) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        return current;
      }
    }
    return null;
  };
  const shape = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);
    return [
      typeof descriptor.get,
      descriptor.get.name,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(",");
  };
  const valueShape = (object, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    return [
      descriptor.value === expected,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ].join(",");
  };
  const htmlKeysBefore = Object.keys(html)
    .filter(name => name === "implementation" || name === "fonts" || name === "location")
    .join(",");
  const xmlKeysBefore = Object.keys(xml)
    .filter(name => name === "implementation" || name === "fonts" || name === "location")
    .join(",");
  const htmlFontsShape = shape(htmlProto, "fonts");
  const xmlFontsShape = shape(xmlProto, "fonts");
  const htmlImplementationShape = shape(htmlProto, "implementation");
  const xmlImplementationShape = shape(xmlProto, "implementation");
  const htmlLocationShape = valueShape(html, "location", null);
  const xmlLocationShape = valueShape(xml, "location", null);
  const htmlImplementationCacheBefore = Object.getOwnPropertyDescriptor(html, "implementation") === undefined;
  const xmlImplementationCacheBefore = Object.getOwnPropertyDescriptor(xml, "implementation") === undefined;
  const htmlImplementation = html.implementation;
  const xmlImplementation = xml.implementation;
  const htmlFonts = html.fonts;
  const xmlFonts = xml.fonts;
  const htmlImplementationCache = Object.getOwnPropertyDescriptor(html, "implementation") === undefined;
  const xmlImplementationCache = Object.getOwnPropertyDescriptor(xml, "implementation") === undefined;

  html.implementation = { marker: "html" };
  xml.implementation = { marker: "xml" };
  html.fonts = { marker: "html-fonts" };
  xml.fonts = { marker: "xml-fonts" };
  html.location = { marker: "html-location" };
  xml.location = { marker: "xml-location" };
  const prototypeSurface = [
    Object.prototype.hasOwnProperty.call(html, "createElement"),
    Object.prototype.hasOwnProperty.call(html, "querySelector"),
    Object.prototype.hasOwnProperty.call(html, "getElementById"),
    Object.prototype.hasOwnProperty.call(html, "fonts"),
    Object.prototype.hasOwnProperty.call(html, "implementation"),
    Object.prototype.hasOwnProperty.call(htmlProto, "createElement"),
    Object.prototype.hasOwnProperty.call(htmlProto, "querySelector"),
    Object.prototype.hasOwnProperty.call(htmlProto, "fonts"),
    descriptorOwner(htmlProto, "fonts") === Document.prototype,
    descriptorOwner(htmlProto, "implementation") === Document.prototype,
    htmlProto === HTMLDocument.prototype,
    xmlProto === Document.prototype
  ].join(",");

  return [
    htmlFontsShape,
    xmlFontsShape,
    htmlImplementationShape,
    xmlImplementationShape,
    htmlLocationShape,
    xmlLocationShape,
    htmlKeysBefore,
    xmlKeysBefore,
    htmlImplementationCacheBefore,
    xmlImplementationCacheBefore,
    htmlImplementationCache,
    xmlImplementationCache,
    html.implementation === htmlImplementation,
    xml.implementation === xmlImplementation,
    html.fonts === htmlFonts,
    xml.fonts === xmlFonts,
    html.location === null,
    xml.location === null,
    htmlImplementation.createDocumentType("html", "", "").ownerDocument === html,
    xmlImplementation.createDocumentType("html", "", "").ownerDocument === xml,
    Object.prototype.toString.call(htmlFonts),
    Object.prototype.toString.call(xmlFonts),
    prototypeSurface
  ].join("|");
})()
"#,
        )
        .expect("detached document state accessor descriptor probe should evaluate");

    assert_eq!(
        result,
        "function,get fonts,true,true,true|function,get fonts,true,true,true|function,get implementation,true,true,true|function,get implementation,true,true,true|true,false,false,true|true,false,false,true|||true|true|true|true|true|true|true|true|true|true|true|true|[object FontFaceSet]|[object FontFaceSet]|false,false,false,false,false,false,false,false,true,true,true,true"
    );
}

#[test]
fn detached_document_creation_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-document-creation-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString("<html><body></body></html>", "text/html");
  const xml = parser.parseFromString("<root></root>", "application/xml");
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  const htmlElement = Document.prototype.createElement.call(html, "section");
  const htmlElementNs = Document.prototype.createElementNS.call(
    html,
    "http://www.w3.org/1999/xhtml",
    "x:article"
  );
  const xmlElement = Document.prototype.createElement.call(xml, "Mixed");
  const xmlElementNs = Document.prototype.createElementNS.call(xml, "urn:test", "p:item");
  const text = Document.prototype.createTextNode.call(html, "txt");
  const comment = Document.prototype.createComment.call(html, "note");
  const fragment = Document.prototype.createDocumentFragment.call(html);
  const pi = Document.prototype.createProcessingInstruction.call(xml, "pi", "data");
  const attr = Document.prototype.createAttribute.call(html, "DATA-X");
  const nsAttr = Document.prototype.createAttributeNS.call(xml, "urn:test", "p:flag");
  const imported = Document.prototype.importNode.call(html, xmlElementNs, false);
  const adoptedSource = Document.prototype.createElement.call(xml, "adopted");
  const adopted = Document.prototype.adoptNode.call(html, adoptedSource);

  fragment.append(text, comment);

  return JSON.stringify({
    htmlElement: [
      htmlElement.ownerDocument === html,
      htmlElement.tagName,
      htmlElement instanceof HTMLElement,
      own(htmlElement, "tagName")
    ].join(","),
    htmlElementNs: [
      htmlElementNs.ownerDocument === html,
      htmlElementNs.prefix,
      htmlElementNs.localName,
      htmlElementNs.namespaceURI
    ].join(","),
    xmlElement: [
      xmlElement.ownerDocument === xml,
      xmlElement.localName,
      String(xmlElement.namespaceURI)
    ].join(","),
    xmlElementNs: [
      xmlElementNs.ownerDocument === xml,
      xmlElementNs.prefix,
      xmlElementNs.localName,
      xmlElementNs.namespaceURI
    ].join(","),
    characterNodes: [
      text.ownerDocument === html,
      text.data,
      comment.ownerDocument === html,
      comment.data,
      fragment.ownerDocument === html,
      fragment.childNodes.length,
      pi.ownerDocument === xml,
      pi.target
    ].join(","),
    attrs: [
      attr.ownerDocument === html,
      attr.name,
      nsAttr.ownerDocument === xml,
      nsAttr.prefix,
      nsAttr.localName,
      nsAttr.namespaceURI
    ].join(","),
    importAdopt: [
      imported.ownerDocument === html,
      imported.prefix,
      imported.localName,
      adopted === adoptedSource,
      adopted.ownerDocument === html
    ].join(","),
    documentOwn: [
      own(html, "createElement"),
      own(html, "createTextNode"),
      own(html, "createAttribute"),
      own(html, "importNode"),
      own(html, "adoptNode")
    ].join(",")
  });
})()
"#,
        )
        .expect("detached Document prototype creation brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"htmlElement":"true,SECTION,true,false","htmlElementNs":"true,x,article,http://www.w3.org/1999/xhtml","xmlElement":"true,Mixed,null","xmlElementNs":"true,p,item,urn:test","characterNodes":"true,txt,true,note,true,2,true,pi","attrs":"true,data-x,true,p,flag,urn:test","importAdopt":"true,p,item,true,true","documentOwn":"false,false,false,false,false"}"#
    );
}

#[test]
fn hyperlink_protocol_is_colon_when_href_cannot_be_parsed() {
    let mut vm = new_storage_test_vm("https://hyperlink-invalid-url.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const base = document.createElement("base");
  base.href = "about:blank";
  (document.head || document.documentElement || document).appendChild(base);
  const inputs = [
    "",
    "javascript://:443",
    "javascript://test:test",
    "javascript://[:1]",
    "mailto://:443",
    "mailto://test:test",
    "mailto://[:1]"
  ];

  for (const tag of ["a", "area"]) {
    const element = document.createElement(tag);
    for (const input of inputs) {
      element.setAttribute("href", input);
      if (element.href !== input) {
        throw new Error(`${tag} should preserve the unparsable href ${input}`);
      }
      if (element.protocol !== ":") {
        throw new Error(`${tag} should expose ':' for the unparsable href ${input}`);
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("unparsable hyperlink protocol probe should evaluate");

    assert_eq!(result, "ok");
}
