use super::*;

#[test]
fn html_media_quote_mod_time_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const absent = (prototype, name) => {
                assert(
                  Object.getOwnPropertyDescriptor(prototype, name) === undefined,
                  `${prototype.constructor.name}.${name} should be absent`
                );
              };

              accessor(HTMLHtmlElement.prototype, "version");
              accessor(HTMLMediaElement.prototype, "preload");
              accessor(HTMLQuoteElement.prototype, "cite");
              accessor(HTMLModElement.prototype, "cite");
              accessor(HTMLModElement.prototype, "dateTime");
              accessor(HTMLTimeElement.prototype, "dateTime");
              for (const name of ["version", "preload", "cite", "dateTime"]) {
                absent(HTMLElement.prototype, name);
              }
              assert(!own(HTMLAudioElement.prototype, "preload"), "audio should inherit preload");
              assert(!own(HTMLVideoElement.prototype, "preload"), "video should inherit preload");

              const html = document.documentElement;
              const audio = document.createElement("audio");
              const video = document.createElement("video");
              const q = document.createElement("q");
              const blockquote = document.createElement("blockquote");
              const ins = document.createElement("ins");
              const del = document.createElement("del");
              const time = document.createElement("time");
              const div = document.createElement("div");
              document.body.append(audio, video, q, blockquote, ins, del, time, div);

              for (const [element, names, label] of [
                [html, ["version"], "html"],
                [audio, ["preload"], "audio"],
                [video, ["preload"], "video"],
                [q, ["cite"], "q"],
                [blockquote, ["cite"], "blockquote"],
                [ins, ["cite", "dateTime"], "ins"],
                [del, ["cite", "dateTime"], "del"],
                [time, ["dateTime"], "time"]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${label}.${name} should not be own before set`);
                }
              }
              for (const name of ["version", "preload", "cite", "dateTime"]) {
                assert(!(name in div), `div.${name} should be absent`);
              }

              html.version = "4.01";
              audio.preload = "metadata";
              video.preload = "none";
              q.cite = "refs/q.html";
              blockquote.cite = "refs/quote.html";
              ins.cite = "refs/ins.html";
              ins.dateTime = "2026-06-19";
              del.cite = "refs/del.html";
              del.dateTime = "2026-06-20";
              time.dateTime = "2026-06-21";

              assert(html.version === "4.01" && html.getAttribute("version") === "4.01", "html version");
              assert(audio.preload === "metadata" && audio.getAttribute("preload") === "metadata", "audio preload");
              assert(video.preload === "none" && video.getAttribute("preload") === "none", "video preload");
              audio.preload = "invalid";
              assert(audio.preload === "auto" && audio.getAttribute("preload") === "invalid", "audio invalid preload");
              assert(q.cite === "https://example.com/base/refs/q.html", "q cite URL");
              assert(blockquote.cite === "https://example.com/base/refs/quote.html", "blockquote cite URL");
              assert(ins.cite === "https://example.com/base/refs/ins.html", "ins cite URL");
              assert(ins.dateTime === "2026-06-19", "ins dateTime");
              assert(del.cite === "https://example.com/base/refs/del.html", "del cite URL");
              assert(del.dateTime === "2026-06-20", "del dateTime");
              assert(time.dateTime === "2026-06-21", "time dateTime");

              for (const [element, names, label] of [
                [html, ["version"], "html"],
                [audio, ["preload"], "audio"],
                [video, ["preload"], "video"],
                [q, ["cite"], "q"],
                [blockquote, ["cite"], "blockquote"],
                [ins, ["cite", "dateTime"], "ins"],
                [del, ["cite", "dateTime"], "del"],
                [time, ["dateTime"], "time"]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${label}.${name} should not be own after set`);
                  assert(delete element[name], `${label}.${name} delete`);
                  assert(!own(element, name), `${label}.${name} should stay inherited`);
                }
              }
              assert(html.version === "4.01", "html version after delete");
              assert(audio.preload === "auto", "audio preload after delete");
              assert(q.cite === "https://example.com/base/refs/q.html", "q cite after delete");
              assert(ins.dateTime === "2026-06-19", "ins dateTime after delete");
              assert(time.dateTime === "2026-06-21", "time dateTime after delete");
              return "ok";
            })()
            "#,
        )
        .expect("HTML/media/quote/mod/time owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn label_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              accessor(HTMLOptGroupElement.prototype, "label");
              accessor(HTMLOptionElement.prototype, "label");
              accessor(HTMLTrackElement.prototype, "label");
              assert(!own(HTMLElement.prototype, "label"), "label should not be on HTMLElement.prototype");
              assert(!("label" in document.createElement("div")), "div should not expose label");
              assert(!("label" in document.createElement("select")), "select should not expose label");

              const optgroup = document.createElement("optgroup");
              const option = document.createElement("option");
              const track = document.createElement("track");
              option.textContent = "Fallback";
              document.body.append(optgroup, option, track);

              for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
                assert(!own(element, "label"), `${tag}.label should not be own before set`);
              }
              assert(optgroup.label === "", "optgroup default label");
              assert(option.label === "Fallback", "option label fallback");
              assert(track.label === "", "track default label");

              optgroup.label = "Group";
              option.label = "Explicit";
              track.label = "English";
              assert(optgroup.label === "Group" && optgroup.getAttribute("label") === "Group", "optgroup label");
              assert(option.label === "Explicit" && option.getAttribute("label") === "Explicit", "option label");
              assert(track.label === "English" && track.getAttribute("label") === "English", "track label");

              for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
                assert(!own(element, "label"), `${tag}.label should not be own after set`);
                assert(delete element.label, `${tag}.label delete`);
                assert(!own(element, "label"), `${tag}.label should stay inherited`);
              }
              assert(optgroup.label === "Group", "optgroup label after delete");
              assert(option.label === "Explicit", "option label after delete");
              assert(track.label === "English", "track label after delete");
              option.removeAttribute("label");
              assert(option.label === "Fallback", "option label fallback after attribute removal");
              return "ok";
            })()
            "#,
        )
        .expect("label owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn frame_legacy_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const frameNames = ["scrolling", "frameBorder", "longDesc", "marginHeight", "marginWidth"];
              for (const name of frameNames) {
                accessor(HTMLFrameElement.prototype, name);
                accessor(HTMLIFrameElement.prototype, name);
                assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }
              accessor(HTMLImageElement.prototype, "longDesc");
              for (const name of ["scrolling", "frameBorder", "marginHeight", "marginWidth"]) {
                assert(!(name in document.createElement("img")), `${name} should not be on img`);
              }

              const frame = document.createElement("frame");
              const iframe = document.createElement("iframe");
              const img = document.createElement("img");
              document.body.append(frame, iframe, img);

              for (const [element, label] of [[frame, "frame"], [iframe, "iframe"]]) {
                for (const name of frameNames) {
                  assert(!own(element, name), `${label}.${name} should not be own before set`);
                }
                element.scrolling = `${label}-scroll`;
                element.frameBorder = `${label}-border`;
                element.longDesc = `https://assets.example/${label}-desc`;
                element.marginHeight = null;
                element.marginWidth = `${label}-width`;
                assert(element.scrolling === `${label}-scroll`, `${label} scrolling`);
                assert(element.getAttribute("scrolling") === `${label}-scroll`, `${label} scrolling attr`);
                assert(element.frameBorder === `${label}-border`, `${label} frameBorder`);
                assert(element.getAttribute("frameborder") === `${label}-border`, `${label} frameBorder attr`);
                assert(element.longDesc === `https://assets.example/${label}-desc`, `${label} longDesc`);
                assert(element.getAttribute("longdesc") === `https://assets.example/${label}-desc`, `${label} longDesc attr`);
                assert(element.marginHeight === "", `${label} marginHeight null`);
                assert(element.getAttribute("marginheight") === "", `${label} marginHeight attr`);
                assert(element.marginWidth === `${label}-width`, `${label} marginWidth`);
                assert(element.getAttribute("marginwidth") === `${label}-width`, `${label} marginWidth attr`);
                for (const name of frameNames) {
                  assert(!own(element, name), `${label}.${name} should not be own after set`);
                  assert(delete element[name], `${label}.${name} delete`);
                  assert(!own(element, name), `${label}.${name} should stay inherited`);
                }
              }

              assert(!own(img, "longDesc"), "img.longDesc should not be own before set");
              img.longDesc = "https://assets.example/image-desc";
              assert(img.longDesc === "https://assets.example/image-desc", "image longDesc");
              assert(img.getAttribute("longdesc") === "https://assets.example/image-desc", "image longDesc attr");
              assert(!own(img, "longDesc"), "img.longDesc should not be own after set");
              assert(delete img.longDesc, "img.longDesc delete");
              assert(!own(img, "longDesc"), "img.longDesc should stay inherited");
              assert(img.longDesc === "https://assets.example/image-desc", "image longDesc after delete");
              return "ok";
            })()
            "#,
        )
        .expect("frame legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn resource_legacy_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const absentFromPlainHtml = (name) => {
                assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              };

              accessor(HTMLAreaElement.prototype, "alt");
              accessor(HTMLImageElement.prototype, "alt");
              accessor(HTMLImageElement.prototype, "useMap");
              accessor(HTMLImageElement.prototype, "srcset");
              accessor(HTMLImageElement.prototype, "lowsrc");
              accessor(HTMLImageElement.prototype, "decoding");
              accessor(HTMLSourceElement.prototype, "srcset");
              accessor(HTMLObjectElement.prototype, "useMap");
              for (const name of ["alt", "useMap", "srcset", "lowsrc", "decoding"]) {
                absentFromPlainHtml(name);
              }

              const area = document.createElement("area");
              const image = document.createElement("img");
              const source = document.createElement("source");
              const object = document.createElement("object");
              document.body.append(area, image, source, object);

              for (const [element, names, label] of [
                [area, ["alt"], "area"],
                [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
                [source, ["srcset"], "source"],
                [object, ["useMap"], "object"]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${label}.${name} should not be own before set`);
                }
              }

              area.alt = "map alt";
              image.alt = "image alt";
              image.useMap = "#main-map";
              image.srcset = "small.png 1x, large.png 2x";
              image.lowsrc = "https://assets.example/low.png";
              image.decoding = "ASYNC";
              source.srcset = "source-small.png 1x";
              object.useMap = "#object-map";

              assert(area.alt === "map alt" && area.getAttribute("alt") === "map alt", "area alt");
              assert(image.alt === "image alt" && image.getAttribute("alt") === "image alt", "image alt");
              assert(image.useMap === "#main-map" && image.getAttribute("usemap") === "#main-map", "image useMap");
              assert(image.srcset === "small.png 1x, large.png 2x", "image srcset");
              assert(image.lowsrc === "https://assets.example/low.png", "image lowsrc");
              assert(image.decoding === "async" && image.getAttribute("decoding") === "ASYNC", "image decoding canonical");
              image.decoding = "invalid";
              assert(image.decoding === "auto", "image decoding invalid");
              assert(source.srcset === "source-small.png 1x" && source.getAttribute("srcset") === "source-small.png 1x", "source srcset");
              assert(object.useMap === "#object-map" && object.getAttribute("usemap") === "#object-map", "object useMap");

              for (const [element, names, label] of [
                [area, ["alt"], "area"],
                [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
                [source, ["srcset"], "source"],
                [object, ["useMap"], "object"]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${label}.${name} should not be own after set`);
                  assert(delete element[name], `${label}.${name} delete`);
                  assert(!own(element, name), `${label}.${name} should stay inherited`);
                }
              }
              assert(image.useMap === "#main-map", "image useMap after delete");
              assert(image.decoding === "auto", "image decoding after delete");
              assert(source.srcset === "source-small.png 1x", "source srcset after delete");
              assert(object.useMap === "#object-map", "object useMap after delete");
              return "ok";
            })()
            "##,
        )
        .expect("resource legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_dimension_and_color_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const ownerChecks = [
                [HTMLBodyElement.prototype, "bgColor"],
                [HTMLTableElement.prototype, "bgColor"],
                [HTMLTableRowElement.prototype, "bgColor"],
                [HTMLTableCellElement.prototype, "bgColor"],
                [HTMLMarqueeElement.prototype, "bgColor"],
                [HTMLTableElement.prototype, "border"],
                [HTMLImageElement.prototype, "border"],
                [HTMLObjectElement.prototype, "border"],
                [HTMLHRElement.prototype, "color"],
                [HTMLFontElement.prototype, "color"],
                [HTMLImageElement.prototype, "hspace"],
                [HTMLImageElement.prototype, "vspace"],
                [HTMLObjectElement.prototype, "hspace"],
                [HTMLObjectElement.prototype, "vspace"],
                [HTMLMarqueeElement.prototype, "hspace"],
                [HTMLMarqueeElement.prototype, "vspace"]
              ];
              for (const [prototype, name] of ownerChecks) accessor(prototype, name);
              for (const name of ["bgColor", "border", "color", "hspace", "vspace"]) {
                assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }

              const table = document.createElement("table");
              const row = document.createElement("tr");
              const cell = document.createElement("td");
              const image = document.createElement("img");
              const object = document.createElement("object");
              const hr = document.createElement("hr");
              const font = document.createElement("font");
              const marquee = document.createElement("marquee");
              document.body.append(table, row, cell, image, object, hr, font, marquee);

              for (const [element, name] of [
                [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
                [table, "border"], [image, "border"], [object, "border"],
                [hr, "color"], [font, "color"],
                [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
                [marquee, "hspace"], [marquee, "vspace"]
              ]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
              }

              table.bgColor = "red";
              row.bgColor = "green";
              cell.bgColor = null;
              marquee.bgColor = "blue";
              table.border = "3";
              image.border = null;
              object.border = null;
              hr.color = "black";
              font.color = null;
              image.hspace = 7;
              image.vspace = 8;
              object.hspace = 9;
              object.vspace = 10;
              marquee.hspace = 11;
              marquee.vspace = 12;

              assert(table.bgColor === "red" && table.getAttribute("bgcolor") === "red", "table bgColor");
              assert(row.bgColor === "green" && row.getAttribute("bgcolor") === "green", "row bgColor");
              assert(cell.bgColor === "" && cell.getAttribute("bgcolor") === "", "cell bgColor null");
              assert(marquee.bgColor === "blue" && marquee.getAttribute("bgcolor") === "blue", "marquee bgColor");
              assert(table.border === "3" && table.getAttribute("border") === "3", "table border");
              assert(image.border === "" && image.getAttribute("border") === "", "image border null");
              assert(object.border === "" && object.getAttribute("border") === "", "object border null");
              assert(hr.color === "black" && hr.getAttribute("color") === "black", "hr color");
              assert(font.color === "" && font.getAttribute("color") === "", "font color null");
              assert(image.hspace === 7 && image.getAttribute("hspace") === "7", "image hspace");
              assert(image.vspace === 8 && image.getAttribute("vspace") === "8", "image vspace");
              assert(object.hspace === 9 && object.getAttribute("hspace") === "9", "object hspace");
              assert(object.vspace === 10 && object.getAttribute("vspace") === "10", "object vspace");
              assert(marquee.hspace === 11 && marquee.getAttribute("hspace") === "11", "marquee hspace");
              assert(marquee.vspace === 12 && marquee.getAttribute("vspace") === "12", "marquee vspace");

              for (const [element, name] of [
                [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
                [table, "border"], [image, "border"], [object, "border"],
                [hr, "color"], [font, "color"],
                [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
                [marquee, "hspace"], [marquee, "vspace"]
              ]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
                assert(delete element[name], `${element.localName}.${name} delete`);
                assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
              }
              assert(table.bgColor === "red", "table bgColor after delete");
              assert(cell.bgColor === "", "cell bgColor after delete");
              assert(font.color === "", "font color after delete");
              assert(image.hspace === 7 && marquee.vspace === 12, "unsigned after delete");
              return "ok";
            })()
            "##,
        )
        .expect("legacy dimension and color owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn body_legacy_accessors_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://body-legacy-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
                assert(!own(document.body, name), `${name} should not be own before set`);
              }
              for (const name of bodyOnlyNames) {
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }

              const handler = () => "body-load";
              document.body.onload = handler;
              assert(window.onload === handler, "body.onload setter syncs window.onload");
              assert(document.body.onload === handler, "body.onload getter reads window.onload");

              document.body.text = "black";
              document.body.link = "#111111";
              document.body.vLink = "#222222";
              document.body.aLink = "#333333";
              document.body.background = "paper.png";
              assert(document.body.text === "black" && document.body.getAttribute("text") === "black", "body text");
              assert(document.body.link === "#111111" && document.body.getAttribute("link") === "#111111", "body link");
              assert(document.body.vLink === "#222222" && document.body.getAttribute("vlink") === "#222222", "body vLink");
              assert(document.body.aLink === "#333333" && document.body.getAttribute("alink") === "#333333", "body aLink");
              assert(document.body.background === "paper.png" && document.body.getAttribute("background") === "paper.png", "body background");

              for (const name of names) {
                assert(!own(document.body, name), `${name} should not be own after set`);
                assert(delete document.body[name], `${name} delete`);
                assert(!own(document.body, name), `${name} should stay inherited`);
              }
              assert(document.body.onload === handler, "body.onload after delete");
              assert(document.body.text === "black", "body text after delete");
              window.onload = null;
              return "ok";
            })()
            "##,
        )
        .expect("body legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn global_event_handler_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://global-event-handler-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
              const submit = accessor(HTMLElement.prototype, "onsubmit");
              const load = accessor(HTMLElement.prototype, "onload");
              assert(throwsTypeError(() => click.get.call(HTMLElement.prototype)),
                "HTMLElement.prototype.onclick receiver brand");
              assert(throwsTypeError(() => submit.get.call(HTMLElement.prototype)),
                "HTMLElement.prototype.onsubmit receiver brand");
              assert(throwsTypeError(() => load.get.call(HTMLElement.prototype)),
                "HTMLElement.prototype.onload receiver brand");
              assert(Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, "onload").get !== load.get,
                "HTMLBodyElement.onload should keep body/window override");

              const div = document.createElement("div");
              const other = document.createElement("div");
              const form = document.createElement("form");
              for (const [element, name] of [[div, "onclick"], [other, "onclick"], [form, "onsubmit"]]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
              }

              function handler() {}
              div.onclick = handler;
              other.onclick = "not a function";
              form.onsubmit = handler;
              assert(div.onclick === handler, "div.onclick handler");
              assert(other.onclick === null, "non-function onclick becomes null");
              assert(form.onsubmit === handler, "form.onsubmit handler");
              for (const [element, name] of [[div, "onclick"], [other, "onclick"], [form, "onsubmit"]]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
                assert(delete element[name], `${element.localName}.${name} delete`);
                assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
              }
              assert(div.onclick === handler, "div.onclick after delete");
              assert(form.onsubmit === handler, "form.onsubmit after delete");

              if (typeof SVGElement === "function") {
                accessor(SVGElement.prototype, "onclick");
                const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
                assert(!own(svg, "onclick"), "svg.onclick should not be own before set");
                svg.onclick = handler;
                assert(svg.onclick === handler, "svg.onclick handler");
                assert(!own(svg, "onclick"), "svg.onclick should not be own after set");
              }

              return "ok";
            })()
            "##,
        )
        .expect("global event handler owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn global_drag_event_handlers_cover_window_document_and_elements() {
    let mut vm = new_parsed_test_vm(
        "https://global-drag-event-handlers.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const names = [
    'ondragstart', 'ondrag', 'ondragover', 'ondragenter',
    'ondragleave', 'ondrop', 'ondragend'
  ];
  const div = document.createElement('div');
  document.body.appendChild(div);
  const describe = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return !!descriptor &&
      typeof descriptor.get === 'function' &&
      typeof descriptor.set === 'function' &&
      descriptor.enumerable && descriptor.configurable;
  };
  const surfaces = names.every(name =>
    name in window && name in document && name in div &&
    describe(window, name) &&
    describe(Document.prototype, name) &&
    describe(HTMLElement.prototype, name) &&
    window[name] === null && document[name] === null && div[name] === null
  );

  const calls = [];
  window.ondrag = event => calls.push(`window:${event.type}`);
  document.ondrag = event => calls.push(`document:${event.type}`);
  div.ondrag = event => calls.push(`element:${event.type}`);
  window.dispatchEvent(new Event('drag'));
  document.dispatchEvent(new Event('drag'));
  div.dispatchEvent(new Event('drag'));

  window.ondrag = {};
  document.ondrag = undefined;
  div.ondrag = null;
  return JSON.stringify({
    surfaces,
    calls,
    cleared: [window.ondrag, document.ondrag, div.ondrag]
  });
})()
"#,
        )
        .expect("GlobalEventHandlers drag surface should evaluate");

    assert_eq!(
        result,
        r#"{"surfaces":true,"calls":["window:drag","document:drag","element:drag"],"cleared":[null,null,null]}"#
    );
}

#[test]
fn global_touch_event_handlers_live_on_mixin_owners() {
    let mut vm = new_parsed_test_vm(
        "https://global-touch-event-handlers.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    vm.set_navigator_overrides_and_sync_surface(&moli_page_types::NavigatorOverrides {
        max_touch_points: Some(5),
        ..Default::default()
    })
    .unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  return frame.contentWindow.eval('(' + function() {
  const names = ['ontouchstart', 'ontouchend', 'ontouchmove', 'ontouchcancel'];
  const owners = [window, HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype, Document.prototype];
  const descriptorIsEventHandler = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return !!descriptor &&
      typeof descriptor.get === 'function' &&
      typeof descriptor.set === 'function' &&
      descriptor.enumerable && descriptor.configurable;
  };
  const ownAccessors = names.every(name =>
    owners.every(owner => descriptorIsEventHandler(owner, name))
  );
  const absentFromElement = names.every(name => !(name in Element.prototype));

  const div = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const calls = [];
  for (const [label, target] of [
    ['window', window],
    ['document', document],
    ['html', div],
    ['svg', svg]
  ]) {
    target.ontouchstart = event => calls.push(`${label}:${event.type}`);
    target.dispatchEvent(new Event('touchstart'));
  }

  return JSON.stringify({ ownAccessors, absentFromElement, calls });
  }.toString() + ')()');
})()
"#,
        )
        .expect("GlobalEventHandlers touch surface should evaluate");

    assert_eq!(
        result,
        r#"{"ownAccessors":true,"absentFromElement":true,"calls":["window:touchstart","document:touchstart","html:touchstart","svg:touchstart"]}"#
    );
}

#[test]
fn desktop_touch_feature_detection_does_not_hide_event_constructors() {
    let mut vm = new_parsed_test_vm(
        "https://desktop-touch.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const names = ['ontouchstart', 'ontouchend', 'ontouchmove', 'ontouchcancel'];
        const owners = [window, Document.prototype, HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype];
        return names.every(name => owners.every(owner => !(name in owner))) &&
            new TouchEvent('touchstart') instanceof TouchEvent && typeof Touch === 'function';
    })()"#).unwrap(), "true");
}

#[test]
fn media_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
              assert(!("media" in document.createElement("div")), "media should not be on div");

              const link = document.createElement("link");
              const source = document.createElement("source");
              const style = document.createElement("style");
              const meta = document.createElement("meta");
              document.head.append(link, style, meta);
              document.body.append(source);

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
        .expect("media owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_media_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://media-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const audio = document.createElement("audio");
  const video = document.createElement("video");
  const div = document.createElement("div");
  const img = document.createElement("img");
  const source = document.createElement("source");
  const text = document.createTextNode("x");
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
        .expect("HTML media element receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn specialized_structural_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name, hasSetter = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const templateNames = [
                ["content", false],
                ["shadowRootMode", true],
                ["shadowRootDelegatesFocus", true],
                ["shadowRootClonable", true],
                ["shadowRootSerializable", true],
                ["shadowRootCustomElementRegistry", true],
                ["shadowRootSlotAssignment", true],
                ["shadowRootAdoptedStyleSheets", true]
              ];
              for (const [name, hasSetter] of templateNames) {
                accessor(HTMLTemplateElement.prototype, name, hasSetter);
              }
              const template = document.createElement("template");
              for (const [name] of templateNames) {
                assert(!own(template, name), `${name} should not be own on template`);
              }
              template.innerHTML = "<span>inside</span>";
              template.shadowRootMode = "open";
              template.shadowRootDelegatesFocus = true;
              template.shadowRootClonable = true;
              template.shadowRootSerializable = true;
              template.shadowRootSlotAssignment = "manual";
              template.shadowRootAdoptedStyleSheets = "[]";
              assert(template.content.firstChild.localName === "span", "template content behavior");
              assert(template.shadowRootMode === "open", "template shadowRootMode behavior");
              assert(template.shadowRootDelegatesFocus === true, "template delegates behavior");
              assert(template.shadowRootClonable === true, "template clonable behavior");
              assert(template.shadowRootSerializable === true, "template serializable behavior");
              assert(template.shadowRootSlotAssignment === "manual", "template slotAssignment behavior");
              assert(template.shadowRootAdoptedStyleSheets === "[]", "template adopted sheets behavior");

              const shadowRootNames = [
                ["host", false],
                ["mode", false],
                ["delegatesFocus", false],
                ["slotAssignment", false],
                ["clonable", false],
                ["serializable", false],
                ["referenceTarget", true],
                ["activeElement", false]
              ];
              for (const [name, hasSetter] of shadowRootNames) {
                accessor(ShadowRoot.prototype, name, hasSetter);
              }
              const shadowHost = document.createElement("section");
              document.body.append(shadowHost);
              const shadowRoot = shadowHost.attachShadow({
                mode: "open",
                delegatesFocus: true,
                slotAssignment: "manual",
                clonable: true,
                serializable: true,
                referenceTarget: "target-id"
              });
              for (const [name] of shadowRootNames) {
                assert(!own(shadowRoot, name), `${name} should not be own on shadow root`);
              }
              assert(shadowRoot.host === shadowHost, "shadow root host behavior");
              assert(shadowRoot.mode === "open", "shadow root mode behavior");
              assert(shadowRoot.delegatesFocus === true, "shadow root delegates behavior");
              assert(shadowRoot.slotAssignment === "manual", "shadow root slotAssignment behavior");
              assert(shadowRoot.clonable === true, "shadow root clonable behavior");
              assert(shadowRoot.serializable === true, "shadow root serializable behavior");
              assert(shadowRoot.referenceTarget === "target-id", "shadow root referenceTarget init");
              shadowRoot.referenceTarget = null;
              assert(shadowRoot.referenceTarget === null, "shadow root referenceTarget null setter");
              shadowRoot.referenceTarget = true;
              assert(shadowRoot.referenceTarget === "true", "shadow root referenceTarget string setter");
              assert(shadowRoot.activeElement === null, "shadow root activeElement default");

              const tableNames = [
                ["caption", true],
                ["tHead", true],
                ["tFoot", true],
                ["rows", false],
                ["tBodies", false]
              ];
              for (const [name, hasSetter] of tableNames) {
                accessor(HTMLTableElement.prototype, name, hasSetter);
              }
              const table = document.createElement("table");
              table.innerHTML = "<caption>old</caption><thead></thead><tbody><tr></tr></tbody><tfoot></tfoot>";
              for (const [name] of tableNames) {
                assert(!own(table, name), `${name} should not be own on table`);
              }
              assert(table.caption.textContent === "old", "table caption getter");
              assert(table.tHead.localName === "thead", "table tHead getter");
              assert(table.tFoot.localName === "tfoot", "table tFoot getter");
              assert(table.rows.length === 1 && table.tBodies.length === 1, "table collections");
              const caption = document.createElement("caption");
              caption.textContent = "new";
              table.caption = caption;
              assert(table.caption === caption && table.firstElementChild === caption, "table caption setter");

              for (const [prototype, name, hasSetter] of [
                [HTMLTableSectionElement.prototype, "rows", false],
                [HTMLTableRowElement.prototype, "rowIndex", false],
                [HTMLTableRowElement.prototype, "sectionRowIndex", false],
                [HTMLTableRowElement.prototype, "cells", false],
                [HTMLTableCellElement.prototype, "colSpan", true],
                [HTMLTableCellElement.prototype, "rowSpan", true],
                [HTMLTableCellElement.prototype, "cellIndex", false]
              ]) {
                accessor(prototype, name, hasSetter);
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
              }
              const tableProbe = document.createElement("table");
              const section = document.createElement("tbody");
              const row = document.createElement("tr");
              const cell = document.createElement("td");
              tableProbe.append(section);
              section.append(row);
              row.append(cell);
              for (const [element, names] of [
                [section, ["rows"]],
                [row, ["rowIndex", "sectionRowIndex", "cells"]],
                [cell, ["colSpan", "rowSpan", "cellIndex"]]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${name} should not be own before access`);
                }
              }
              assert(section.rows.length === 1, "section rows");
              assert(row.rowIndex === 0, "rowIndex");
              assert(row.sectionRowIndex === 0, "sectionRowIndex");
              assert(row.cells.length === 1, "row cells");
              assert(cell.cellIndex === 0, "cellIndex");
              cell.colSpan = 7;
              cell.rowSpan = 0;
              assert(cell.colSpan === 7 && cell.getAttribute("colspan") === "7", "colSpan behavior");
              assert(cell.rowSpan === 0 && cell.getAttribute("rowspan") === "0", "rowSpan behavior");
              for (const [element, names] of [
                [section, ["rows"]],
                [row, ["rowIndex", "sectionRowIndex", "cells"]],
                [cell, ["colSpan", "rowSpan", "cellIndex"]]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${name} should not be own after access`);
                  assert(delete element[name], `${name} delete`);
                  assert(!own(element, name), `${name} should stay inherited`);
                }
              }

              const simpleCases = [
                [HTMLLIElement.prototype, "value", document.createElement("li"), 7, "7", "value"],
                [HTMLOListElement.prototype, "start", document.createElement("ol"), 3, "3", "start"],
                [HTMLOListElement.prototype, "reversed", document.createElement("ol"), true, "", "reversed"],
                [HTMLOListElement.prototype, "type", document.createElement("ol"), "A", "A", "type"],
                [HTMLOptGroupElement.prototype, "disabled", document.createElement("optgroup"), true, "", "disabled"],
                [HTMLDetailsElement.prototype, "open", document.createElement("details"), true, "", "open"],
                [HTMLMetaElement.prototype, "content", document.createElement("meta"), "width=device-width", "width=device-width", "content"],
                [HTMLMetaElement.prototype, "httpEquiv", document.createElement("meta"), "refresh", "refresh", "http-equiv"],
                [HTMLTitleElement.prototype, "text", document.createElement("title"), "Page Title", "Page Title", null]
              ];
              for (const [prototype, name, element, value, expected, attribute] of simpleCases) {
                accessor(prototype, name);
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
                assert(!own(element, name), `${name} should not be own before set`);
                element[name] = value;
                assert(element[name] === value || element[name] === expected, `${name} getter`);
                if (attribute === null) {
                  assert(element.textContent === expected, `${name} text content`);
                } else {
                  assert(element.getAttribute(attribute) === expected, `${name} attribute`);
                }
                assert(!own(element, name), `${name} should not be own after set`);
                assert(delete element[name], `${name} delete`);
                assert(!own(element, name), `${name} should stay inherited`);
                assert(element[name] === value || element[name] === expected, `${name} after delete`);
              }
              return "ok";
            })()
            "#,
        )
        .expect("specialized structural accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn title_text_uses_only_direct_text_node_children() {
    let mut vm = new_parsed_test_vm(
        "https://title-text-children.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const title = document.createElement("title");
  title.append(
    document.createComment("COMMENT"),
    document.createTextNode("DIRECT"),
    Object.assign(document.createElement("span"), { textContent: "NESTED" })
  );
  if (title.text !== "DIRECT") throw new Error(`title.text: ${title.text}`);
  if (title.textContent !== "DIRECTNESTED") throw new Error(`title.textContent: ${title.textContent}`);

  title.text = "replacement";
  if (title.childNodes.length !== 1) throw new Error("title.text setter child count");
  if (title.firstChild.nodeType !== Node.TEXT_NODE) throw new Error("title.text setter child type");
  if (title.text !== "replacement") throw new Error("title.text setter value");
  return "ok";
})()
"#,
        )
        .expect("title.text should use child text content rather than descendant text content");

    assert_eq!(result, "ok");
}

#[test]
fn fetch_priority_reflection_is_shared_by_supported_elements() {
    let mut vm = new_parsed_test_vm(
        "https://fetch-priority-reflection.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const svgNamespace = "http://www.w3.org/2000/svg";
  const cases = [
    [HTMLImageElement.prototype, document.createElement("img"), "img"],
    [HTMLLinkElement.prototype, document.createElement("link"), "link"],
    [HTMLScriptElement.prototype, document.createElement("script"), "script"],
    [SVGImageElement.prototype, document.createElementNS(svgNamespace, "image"), "svg image"],
    [SVGScriptElement.prototype, document.createElementNS(svgNamespace, "script"), "svg script"]
  ];

  for (const [prototype, element, label] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "fetchPriority");
    assert(!!descriptor, `${label} descriptor`);
    assert(typeof descriptor.get === "function", `${label} getter`);
    assert(typeof descriptor.set === "function", `${label} setter`);
    assert(descriptor.get.call(element) === "auto", `${label} missing default`);

    element.setAttribute("fetchpriority", "HIGH");
    assert(descriptor.get.call(element) === "high", `${label} high canonicalization`);
    descriptor.set.call(element, "LOW");
    assert(element.getAttribute("fetchpriority") === "LOW", `${label} setter reflection`);
    assert(descriptor.get.call(element) === "low", `${label} low canonicalization`);
    descriptor.set.call(element, "invalid");
    assert(element.getAttribute("fetchpriority") === "invalid", `${label} invalid reflection`);
    assert(descriptor.get.call(element) === "auto", `${label} invalid default`);
    assert(throwsTypeError(() => descriptor.set.call(element, Symbol())), `${label} symbol setter`);

    const invalidReceivers = [
      {},
      document.createTextNode("x"),
      document.createElement("div"),
      ...cases.filter(([, candidate]) => candidate !== element).map(([, candidate]) => candidate)
    ];
    for (const receiver of invalidReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${label} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "high")), `${label} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("fetchPriority should reflect the shared limited-known-value enumeration");

    assert_eq!(result, "ok");
}

#[test]
fn simple_specialized_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://simple-specialized-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const li = document.createElement("li");
  const ol = document.createElement("ol");
  const optgroup = document.createElement("optgroup");
  const details = document.createElement("details");
  const meta = document.createElement("meta");
  const title = document.createElement("title");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  const elements = [li, ol, optgroup, details, meta, title, div];

  const cases = [
    [HTMLLIElement.prototype, "value", li, 7],
    [HTMLOListElement.prototype, "start", ol, 3],
    [HTMLOListElement.prototype, "reversed", ol, true],
    [HTMLOListElement.prototype, "type", ol, "A"],
    [HTMLOptGroupElement.prototype, "disabled", optgroup, true],
    [HTMLDetailsElement.prototype, "open", details, true],
    [HTMLMetaElement.prototype, "content", meta, "width=device-width"],
    [HTMLMetaElement.prototype, "httpEquiv", meta, "refresh"],
    [HTMLTitleElement.prototype, "text", title, "Page Title"]
  ];

  for (const [prototype, name, element, value] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(typeof descriptor.get.call(element) !== "undefined", `${name} valid getter`);
    descriptor.set.call(element, value);
    assert(!own(element, name), `${name} should stay inherited`);

    for (const receiver of [{}, text, ...elements.filter(candidate => candidate !== element)]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("simple specialized receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_table_structural_members_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://table-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const table = document.createElement("table");
  const caption = document.createElement("caption");
  const thead = document.createElement("thead");
  const tfoot = document.createElement("tfoot");
  const tbody = document.createElement("tbody");
  const row = document.createElement("tr");
  const td = document.createElement("td");
  const th = document.createElement("th");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  table.append(caption, thead, tbody, tfoot);
  tbody.append(row);
  row.append(td, th);

  const tableBad = [{}, text, div, tbody, row, td, th];
  const sectionBad = [{}, text, div, table, row, td, th];
  const rowBad = [{}, text, div, table, tbody, td, th];
  const cellBad = [{}, text, div, table, tbody, row];

  for (const name of ["caption", "tHead", "tFoot", "rows", "tBodies"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(table) !== "undefined", `${name} valid getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(table, null);
    }
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, null)), `${name} setter receiver`);
      }
    }
  }

  const tableMethods = {
    createCaption: [],
    deleteCaption: [],
    createTHead: [],
    deleteTHead: [],
    createTFoot: [],
    deleteTFoot: [],
    createTBody: [],
    insertRow: [-1],
    deleteRow: [-1]
  };
  const methodTable = document.createElement("table");
  for (const [name, args] of Object.entries(tableMethods)) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name).value;
    assert(typeof method === "function", `${name} method`);
    method.call(methodTable, ...args);
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
  }

  const sectionRows = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, "rows");
  assert(sectionRows.get.call(tbody).length === 1, "section rows valid getter");
  for (const receiver of sectionBad) {
    assert(throwsTypeError(() => sectionRows.get.call(receiver)), "section rows receiver");
  }
  for (const [name, args] of [["insertRow", [-1]], ["deleteRow", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, name).value;
    const section = document.createElement("tbody");
    method.call(section, ...args);
    for (const receiver of sectionBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const name of ["rowIndex", "sectionRowIndex", "cells"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name);
    assert(typeof descriptor.get.call(row) !== "undefined", `${name} valid getter`);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} receiver`);
    }
  }
  for (const [name, args] of [["insertCell", [-1]], ["deleteCell", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name).value;
    const methodRow = document.createElement("tr");
    method.call(methodRow, ...args);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const cell of [td, th]) {
    for (const name of ["colSpan", "rowSpan", "cellIndex"]) {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLTableCellElement.prototype, name);
      assert(typeof descriptor.get.call(cell) !== "undefined", `${name} valid getter`);
      if (typeof descriptor.set === "function") {
        descriptor.set.call(cell, 2);
      }
      for (const receiver of cellBad) {
        assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
        if (typeof descriptor.set === "function") {
          assert(throwsTypeError(() => descriptor.set.call(receiver, 2)), `${name} setter receiver`);
        }
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("HTML table structural receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn table_head_placement_uses_element_children_as_the_reference() {
    let mut vm = new_parsed_test_vm(
        "https://table-head-placement.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const buildTable = () => {
    const table = document.createElement("table");
    table.append(
      document.createTextNode("leading"),
      document.createElement("caption"),
      document.createComment("between"),
      document.createElement("colgroup"),
      document.createTextNode("before body"),
      document.createElement("tbody")
    );
    return table;
  };

  const createdTable = buildTable();
  const createdHead = createdTable.createTHead();

  const assignedTable = buildTable();
  const assignedHead = document.createElement("thead");
  assignedTable.tHead = assignedHead;

  return [
    [...createdTable.children].map(element => element.localName).join(","),
    createdHead.previousElementSibling.localName,
    createdHead.nextElementSibling.localName,
    [...assignedTable.children].map(element => element.localName).join(","),
    assignedHead.previousElementSibling.localName,
    assignedHead.nextElementSibling.localName
  ].join("|");
})()
"#,
        )
        .expect("table head placement probe should evaluate");

    assert_eq!(
        result,
        "caption,colgroup,thead,tbody|colgroup|tbody|caption,colgroup,thead,tbody|colgroup|tbody"
    );
}

#[test]
fn document_storage_access_api_minimal_surface_matches_idlharness() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        globalThis.__storageAccessApiProbe = "pending";
        (async () => {
          const has = Document.prototype.hasStorageAccess;
          const request = Document.prototype.requestStorageAccess;
          const hasDesc = Object.getOwnPropertyDescriptor(Document.prototype, "hasStorageAccess");
          const requestDesc = Object.getOwnPropertyDescriptor(Document.prototype, "requestStorageAccess");
          const promiseOutcome = async (fn, receiver) => {
            let value;
            try {
              value = fn.call(receiver);
            } catch (error) {
              return `throw:${error && error.name}`;
            }
            const isPromise = value instanceof Promise;
            try {
              await value;
              return `resolved:${isPromise}`;
            } catch (error) {
              return `rejected:${isPromise}:${error && error.name}`;
            }
          };
          const hasAccess = await document.hasStorageAccess();
          const requestResult = await document.requestStorageAccess();
          return {
            hasType: typeof has,
            hasName: has.name,
            hasLength: has.length,
            hasWritable: !!hasDesc.writable,
            hasEnumerable: !!hasDesc.enumerable,
            hasConfigurable: !!hasDesc.configurable,
            requestType: typeof request,
            requestName: request.name,
            requestLength: request.length,
            requestWritable: !!requestDesc.writable,
            requestEnumerable: !!requestDesc.enumerable,
            requestConfigurable: !!requestDesc.configurable,
            hasAccess,
            requestUndefined: requestResult === undefined,
            nullReceiver: await promiseOutcome(has, null),
            objectReceiver: await promiseOutcome(request, {})
          };
        })().then(
          value => { globalThis.__storageAccessApiProbe = JSON.stringify(value); },
          error => { globalThis.__storageAccessApiProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage access api probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageAccessApiProbe)")
        .expect("storage access api probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasType":"function","hasName":"hasStorageAccess","hasLength":0,"hasWritable":true,"hasEnumerable":true,"hasConfigurable":true,"requestType":"function","requestName":"requestStorageAccess","requestLength":0,"requestWritable":true,"requestEnumerable":true,"requestConfigurable":true,"hasAccess":true,"requestUndefined":true,"nullReceiver":"rejected:true:TypeError","objectReceiver":"rejected:true:TypeError"}"#
    );
}
