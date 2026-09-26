use super::*;

#[test]
fn element_get_animations_tracks_script_created_animations() {
    let mut vm = new_parsed_test_vm(
        "https://animation-registry.test/",
        "<!doctype html><body><div id=\"first\"></div><div id=\"second\"></div></body>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const first = document.getElementById("first");
              const second = document.getElementById("second");
              const animation = first.animate(
                [{ opacity: 1 }, { opacity: 0 }],
                1000
              );

              const initial = first.getAnimations();
              const secondInitial = second.getAnimations();
              animation.pause();
              const paused = first.getAnimations();
              animation.cancel();
              const cancelled = first.getAnimations();
              animation.play();
              const replayed = first.getAnimations();
              animation.finish();
              const finished = first.getAnimations();

              const manual = new Animation(
                new KeyframeEffect(first, [{ opacity: 0 }], 1000),
                document.timeline
              );
              manual.play();
              const manuallyPlayed = first.getAnimations();

              return JSON.stringify({
                initial: [initial.length, initial[0] === animation],
                secondInitial: secondInitial.length,
                paused: [paused.length, paused[0] === animation],
                cancelled: cancelled.length,
                replayed: [replayed.length, replayed[0] === animation],
                finished: finished.length,
                manuallyPlayed: [
                  manuallyPlayed.length,
                  manuallyPlayed[0] === manual
                ]
              });
            })()
            "#,
        )
        .expect("Element.getAnimations registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[1,true],"secondInitial":0,"paused":[1,true],"cancelled":0,"replayed":[1,true],"finished":0,"manuallyPlayed":[1,true]}"#
    );
}
#[test]
fn animation_internal_callback_state_ignores_public_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://animations-callback-slots.test/page.html",
        "<!doctype html><body></body>",
    );

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__animationCallbackEvents = [];
              Object.defineProperties(Object.prototype, {
                __moliAnimationPromiseResolve: {
                  configurable: true,
                  value: () => globalThis.__animationCallbackEvents.push("prototype-resolve")
                },
                __moliAnimationMicrotaskAnimation: {
                  configurable: true,
                  value: null
                },
                __moliAnimationMicrotaskToken: {
                  configurable: true,
                  value: -1
                }
              });
              const target = document.body.appendChild(document.createElement("div"));
              const animation = target.animate(
                { backgroundImage: ["url(real.png)", "url(real.png)"] },
                { duration: 1 }
              );
              const ownNames = Object.getOwnPropertyNames(animation)
                .filter(name => name.startsWith("__moliAnimation"))
                .sort();
              Object.defineProperties(animation, {
                __moliAnimationPromiseResolve: {
                  configurable: true,
                  value: () => globalThis.__animationCallbackEvents.push("own-resolve")
                },
                __moliAnimationMicrotaskAnimation: {
                  configurable: true,
                  value: null
                },
                __moliAnimationMicrotaskToken: {
                  configurable: true,
                  value: -1
                }
              });
              animation.finished.then(
                () => {
                  globalThis.__animationCallbackEvents.push(
                    `finished:${animation.playState}:${getComputedStyle(target).backgroundImage}`
                  );
                },
                error => {
                  globalThis.__animationCallbackEvents.push(`rejected:${error && error.name}`);
                }
              );
              return JSON.stringify({ ownNames, events: globalThis.__animationCallbackEvents });
            })()
            "#,
        )
        .expect("Animation internal callback spoofing setup should evaluate");
    assert_eq!(initial, r#"{"ownNames":[],"events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__animationCallbackEvents)")
        .expect("Animation internal callback promise should settle");
    assert_eq!(
        settled,
        r#"["finished:finished:url(\"https://animations-callback-slots.test/real.png\")"]"#
    );
}
#[test]
fn animation_declared_prototype_methods_preserve_descriptors_and_behavior() {
    let mut vm = new_parsed_test_vm(
        "https://animations-declared-methods.test/",
        "<!doctype html><body><div id=\"target\"></div></body>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.getElementById("target");
              const descriptors = [
                [Element.prototype, "animate", 0],
                [Element.prototype, "getAnimations", 0],
                [KeyframeEffect.prototype, "setKeyframes", 1],
                [Animation.prototype, "play", 0],
                [Animation.prototype, "pause", 0],
                [Animation.prototype, "cancel", 0],
                [Animation.prototype, "finish", 0],
                [Animation.prototype, "reverse", 0],
                [Animation.prototype, "commitStyles", 0],
              ].map(([prototype, name, expectedLength]) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  expectedLength,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              });
              const effect = new KeyframeEffect(target, [{ opacity: 0 }], 1000);
              const setKeyframesResult = effect.setKeyframes([{ opacity: 1 }]) === undefined;
              const animation = target.animate({ opacity: [0, 1] }, { duration: 1 });
              const animations = target.getAnimations();
              return JSON.stringify({
                descriptors,
                setKeyframesResult,
                animationInstance: animation instanceof Animation,
                animationState: animation.playState,
                animationsIsArray: Array.isArray(animations),
              });
            })()
            "#,
        )
        .expect("Animation declared prototype method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["animate:function:animate:0:0:true:true:true","getAnimations:function:getAnimations:0:0:true:true:true","setKeyframes:function:setKeyframes:1:1:true:true:true","play:function:play:0:0:true:true:true","pause:function:pause:0:0:true:true:true","cancel:function:cancel:0:0:true:true:true","finish:function:finish:0:0:true:true:true","reverse:function:reverse:0:0:true:true:true","commitStyles:function:commitStyles:0:0:true:true:true"],"setKeyframesResult":true,"animationInstance":true,"animationState":"running","animationsIsArray":true}"#
    );
}
#[test]
fn keyframe_effect_applies_background_image_against_document_base_url() {
    let mut vm = new_parsed_test_vm(
        "https://animations.test/page.html",
        r#"<!doctype html><base href="/non-existent-base/"><body><div id="target"></div></body>"#,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.getElementById("target");
              const keyframe = new KeyframeEffect(target, [
                { backgroundImage: "url(something.png)" },
                { backgroundImage: "url(something.png)" }
              ], 10000);
              const animation = new Animation(keyframe, document.timeline);
              animation.play();
              const running = getComputedStyle(target).backgroundImage;
              animation.cancel();
              const canceled = getComputedStyle(target).backgroundImage;
              return `${running}|${canceled}`;
            })()
            "#,
        )
        .expect("KeyframeEffect background image probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://animations.test/non-existent-base/something.png")|none"#
    );
}
#[test]
fn animation_private_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://animations-declared-slots.test/page.html",
        "<!doctype html><body><div id=\"target\"></div></body>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.getElementById("target");
              const realEffect = new KeyframeEffect(target, [
                { backgroundImage: "url(real.png)" }
              ], 1000);
              const animation = new Animation(realEffect, document.timeline);
              let onfinishCount = 0;
              animation.onfinish = () => {
                onfinishCount += 1;
              };
              const animationOwnSlots = Object.getOwnPropertyNames(animation)
                .filter(name => name.startsWith("__moliAnimation"))
                .sort();
              const keyframeOwnSlots = Object.getOwnPropertyNames(realEffect)
                .filter(name => name.startsWith("__moliKeyframeEffect"))
                .sort();

              const protoEffect = new KeyframeEffect(target, [
                { backgroundImage: "url(proto.png)" }
              ], 1000);
              Animation.prototype.__moliAnimationPlayState = "running";
              Animation.prototype.__moliAnimationEffect = protoEffect;
              Animation.prototype.__moliAnimationTimeline = document.timeline;
              Animation.prototype.__moliAnimationStartTime = 12;
              Animation.prototype.__moliAnimationOnfinish = () => {
                throw new Error("prototype onfinish");
              };
              Animation.prototype.__moliAnimationFinishedResolve = () => {
                throw new Error("prototype resolve");
              };
              Animation.prototype.__moliAnimationFinishToken = 99;
              KeyframeEffect.prototype.__moliKeyframeEffectTarget = target;
              KeyframeEffect.prototype.__moliKeyframeEffectKeyframes = [
                { backgroundImage: "url(proto-keyframe.png)" }
              ];
              Object.defineProperties(animation, {
                __moliAnimationEffect: { value: protoEffect, configurable: true },
                __moliAnimationPlayState: { value: "paused", configurable: true },
                __moliAnimationOnfinish: {
                  value: () => { throw new Error("own onfinish"); },
                  configurable: true
                },
                __moliAnimationFinishedResolve: {
                  value: () => { throw new Error("own resolve"); },
                  configurable: true
                }
              });
              Object.defineProperty(realEffect, "__moliKeyframeEffectKeyframes", {
                value: [{ backgroundImage: "url(raw-own.png)" }],
                configurable: true
              });
              const pollutedAnimationOwnSlots = Object.getOwnPropertyNames(animation)
                .filter(name => name.startsWith("__moliAnimation"))
                .sort();
              const pollutedKeyframeOwnSlots = Object.getOwnPropertyNames(realEffect)
                .filter(name => name.startsWith("__moliKeyframeEffect"))
                .sort();

              animation.play();
              const realAfterPlay = getComputedStyle(target).backgroundImage;
              const realStateAfterPlay = animation.playState;
              let realFinish;
              try {
                animation.finish();
                realFinish = "ok";
              } catch (error) {
                realFinish = `throw:${error && error.message}`;
              }
              const realStateAfterFinish = animation.playState;
              const realOnfinishCount = onfinishCount;
              realEffect.setKeyframes([
                { backgroundImage: "url(updated.png)" }
              ]);
              animation.cancel();
              const realAfterCancel = getComputedStyle(target).backgroundImage;
              animation.play();
              const realAfterSetKeyframes = getComputedStyle(target).backgroundImage;
              target.style.backgroundImage = "";

              const fakeAnimation = Object.create(Animation.prototype);
              const fakeEffect = Object.create(KeyframeEffect.prototype);
              KeyframeEffect.prototype.setKeyframes.call(fakeEffect, [
                { backgroundImage: "url(fake-own.png)" }
              ]);
              const fakeEffectOwnSlots = Object.getOwnPropertyNames(fakeEffect)
                .filter(name => name.startsWith("__moliKeyframeEffect"))
                .sort();

              const before = target.style.backgroundImage || "";
              Animation.prototype.play.call(fakeAnimation);
              const afterPlay = target.style.backgroundImage || "";
              const fakeAnimationOwnSlotsAfterPlay = Object.getOwnPropertyNames(fakeAnimation)
                .filter(name => name.startsWith("__moliAnimation"))
                .sort();
              let finish;
              try {
                Animation.prototype.finish.call(fakeAnimation);
                finish = "ok";
              } catch (error) {
                finish = `throw:${error && error.message}`;
              }
              const afterFinish = target.style.backgroundImage || "";

              return JSON.stringify({
                animationOwnSlots,
                keyframeOwnSlots,
                pollutedAnimationOwnSlots,
                pollutedKeyframeOwnSlots,
                fakeEffectOwnSlots,
                fakeAnimationOwnSlotsAfterPlay,
                finish,
                real: [
                  realStateAfterPlay,
                  realAfterPlay,
                  realFinish,
                  realStateAfterFinish,
                  realOnfinishCount,
                  realAfterCancel,
                  realAfterSetKeyframes
                ].join("|"),
                fake: [
                  before,
                  afterPlay,
                  afterFinish
                ].join("|")
              });
            })()
            "#,
        )
        .expect("Animation declared-slot probe should evaluate");

    assert_eq!(
        result,
        r#"{"animationOwnSlots":[],"keyframeOwnSlots":[],"pollutedAnimationOwnSlots":["__moliAnimationEffect","__moliAnimationFinishedResolve","__moliAnimationOnfinish","__moliAnimationPlayState"],"pollutedKeyframeOwnSlots":["__moliKeyframeEffectKeyframes"],"fakeEffectOwnSlots":[],"fakeAnimationOwnSlotsAfterPlay":[],"finish":"ok","real":"running|url(\"https://animations-declared-slots.test/real.png\")|ok|finished|1|none|url(\"https://animations-declared-slots.test/updated.png\")","fake":"||"}"#
    );
}
