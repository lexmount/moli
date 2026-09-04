use super::*;

#[tokio::test]
async fn media_load_honors_media_src_csp() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-csp.test/page.html",
        &loader,
    );
    vm.set_response_content_security_policies(&["media-src 'self'".to_owned()]);
    vm.set_response_content_security_report_only_policies(&["media-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmMediaCspEvents = [];
  document.addEventListener("securitypolicyviolation", event => {
    __lmMediaCspEvents.push(`csp:${event.disposition}:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const video = document.createElement("video");
  video.onloadeddata = () => __lmMediaCspEvents.push("loadeddata");
  video.onerror = () => __lmMediaCspEvents.push("error");
  video.src = "data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx";
  (document.body || document.documentElement || document).appendChild(video);
  globalThis.__lmMediaCspVideo = video;
})()
"#,
    )
    .expect("media CSP setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "blocked media loadstart turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "blocked media error turn").await;

    assert_eq!(
        vm.eval(
            r#"[
  __lmMediaCspEvents.join("|"),
  __lmMediaCspVideo.readyState,
  __lmMediaCspVideo.networkState
].join("|")"#,
        )
        .expect("media CSP events should evaluate"),
        "csp:report:media-src:data|csp:enforce:media-src:data|error|0|3"
    );
}

#[tokio::test]
async fn media_report_only_csp_reports_without_blocking_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-csp-report-only.test/page.html",
        &loader,
    );
    vm.set_response_content_security_report_only_policies(&["media-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmMediaReportOnlyEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmMediaReportOnlyEvents.push(`csp:${event.disposition}:${event.effectiveDirective}`);
  });
  const video = document.createElement('video');
  video.onloadeddata = () => __lmMediaReportOnlyEvents.push('loadeddata');
  video.onerror = () => __lmMediaReportOnlyEvents.push('error');
  video.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
  (document.body || document.documentElement || document).appendChild(video);
  globalThis.__lmMediaReportOnlyVideo = video;
})()
"#,
    )
    .expect("media report-only CSP setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("report-only media {phase} turn"),
        )
        .await;
    }

    assert_eq!(
        vm.eval(
            r#"[
  __lmMediaReportOnlyEvents.join("|"),
  __lmMediaReportOnlyVideo.readyState,
  __lmMediaReportOnlyVideo.networkState
].join("|")"#,
        )
        .expect("media report-only CSP events should evaluate"),
        "csp:report:media-src|loadeddata|4|1"
    );
}

#[tokio::test]
async fn media_invalid_base_url_fails_before_csp_check() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-csp-fallback.test/path/page.html",
        &loader,
    );
    vm.set_response_content_security_policies(&["media-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const base = document.createElement('base');
  base.href = 'about:blank';
  root.prepend(base);

  globalThis.__lmMediaFallbackCspEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmMediaFallbackCspEvents.push(`csp:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const video = document.createElement('video');
  video.onloadeddata = () => __lmMediaFallbackCspEvents.push('loadeddata');
  video.onerror = () => __lmMediaFallbackCspEvents.push('error');
  video.src = 'asset.mp4';
  root.appendChild(video);
  globalThis.__lmMediaFallbackCspVideo = video;
})()
"#,
    )
    .expect("media fallback CSP setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        0,
        "an invalid base URL must fail before CSP queues a violation task"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "invalid-base media loadstart turn",
    )
    .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "invalid-base media error turn")
        .await;

    assert_eq!(
        vm.eval(
            r#"[
  __lmMediaFallbackCspEvents.join("|"),
  __lmMediaFallbackCspVideo.readyState,
  __lmMediaFallbackCspVideo.networkState
].join("|")"#,
        )
        .expect("media fallback CSP events should evaluate"),
        "error|0|3"
    );
}

#[tokio::test]
async fn media_invalid_request_url_fails_before_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-invalid-url.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__lmMediaInvalidUrlEvents = [];
  const video = document.createElement('video');
  video.onloadeddata = () => __lmMediaInvalidUrlEvents.push('loadeddata');
  video.onerror = () => __lmMediaInvalidUrlEvents.push('error');
  video.src = 'http://[';
  (document.body || document.documentElement || document).appendChild(video);
  globalThis.__lmMediaInvalidUrlVideo = video;
})()
"#,
    )
    .expect("invalid media URL setup should evaluate");

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "invalid-URL media loadstart turn",
    )
    .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "invalid-URL media error turn")
        .await;

    assert_eq!(
        vm.eval(
            r#"(() => {
  const firstError = __lmMediaInvalidUrlVideo.error;
  globalThis.__lmMediaInvalidUrlFirstError = firstError;
  const result = [
    __lmMediaInvalidUrlEvents.join("|"),
    __lmMediaInvalidUrlVideo.readyState,
    __lmMediaInvalidUrlVideo.networkState,
    firstError instanceof MediaError,
    firstError && firstError.code,
    firstError && firstError.message.length > 0,
    __lmMediaInvalidUrlVideo.error === firstError
  ].join("|");
  __lmMediaInvalidUrlVideo.load();
  return `${result}|${__lmMediaInvalidUrlVideo.error === null}`;
})()"#,
        )
        .expect("invalid media URL events should evaluate"),
        "error|0|3|true|4|true|true|true"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "reloaded invalid-URL media loadstart turn",
    )
    .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "reloaded invalid-URL media error turn",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"[
  __lmMediaInvalidUrlVideo.error instanceof MediaError,
  __lmMediaInvalidUrlVideo.error.code,
  __lmMediaInvalidUrlVideo.error !== __lmMediaInvalidUrlFirstError
].join("|")"#,
        )
        .expect("reloaded invalid media URL error should evaluate"),
        "true|4|true"
    );
}

#[tokio::test]
async fn media_blocks_http_dangling_markup_without_blocking_safe_url_inputs() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-dangling-markup.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__lmMediaDanglingMarkupEvents = [];
  const probes = [
    ["audio", "blocked", "asset.mp3?\n<", false],
    ["video", "blocked", "asset.mp4?<\tkey", false],
    ["video", "child-source", "asset.mp4?\n<", true],
    ["audio", "data", "data:audio/mp3,bytes\n<", false]
  ];
  for (const [localName, name, source, useChildSource] of probes) {
    const media = document.createElement(localName);
    media.onerror = () => __lmMediaDanglingMarkupEvents.push(`${localName}:${name}:error`);
    media.oncanplay = () => __lmMediaDanglingMarkupEvents.push(`${localName}:${name}:canplay`);
    if (useChildSource) {
      const sourceElement = document.createElement("source");
      sourceElement.src = source;
      media.appendChild(sourceElement);
    } else {
      media.src = source;
    }
    (document.body || document.documentElement || document).appendChild(media);
  }
})()
"#,
    )
    .expect("media dangling markup setup should evaluate");

    for turn in 0..10 {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("dangling-markup media event turn {turn}"),
        )
        .await;
    }

    assert_eq!(
        vm.eval(r#"__lmMediaDanglingMarkupEvents.sort().join("|")"#)
            .expect("media dangling markup events should evaluate"),
        "audio:blocked:error|audio:data:canplay|video:blocked:error|video:child-source:error"
    );
}

#[tokio::test]
async fn media_dangling_markup_still_dispatches_report_only_csp() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-dangling-csp.test/page.html",
        &loader,
    );
    vm.set_response_content_security_report_only_policies(&["media-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmMediaDanglingCspEvents = [];
  document.addEventListener("securitypolicyviolation", event => {
    __lmMediaDanglingCspEvents.push(`csp:${event.disposition}:${event.effectiveDirective}`);
  });
  const video = document.createElement("video");
  video.onerror = () => __lmMediaDanglingCspEvents.push("error");
  video.oncanplay = () => __lmMediaDanglingCspEvents.push("canplay");
  video.src = "asset.mp4?\n<";
  (document.body || document.documentElement || document).appendChild(video);
})()
"#,
    )
    .expect("media dangling markup CSP setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "report-only dangling-markup media loadstart turn",
    )
    .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "report-only dangling-markup media error turn",
    )
    .await;

    assert_eq!(
        vm.eval(r#"__lmMediaDanglingCspEvents.join("|")"#)
            .expect("media dangling markup CSP events should evaluate"),
        "csp:report:media-src|error"
    );
}

#[test]
fn media_play_returns_a_fulfilled_promise() {
    let mut vm = new_storage_test_vm("https://media-play-promise.test/");

    vm.exec(
        r#"
(() => {
  const video = document.createElement("video");
  const events = [];
  video.addEventListener("play", () => events.push("play"));
  video.addEventListener("playing", () => events.push("playing"));
  const promise = video.play();
  globalThis.__mediaPlayPromiseProbe = {
    promiseTag: Object.prototype.toString.call(promise),
    paused: video.paused,
    events: events.join(","),
    outcome: "pending"
  };
  promise.then(value => {
    globalThis.__mediaPlayPromiseProbe.outcome = value === undefined
      ? "fulfilled:undefined"
      : "fulfilled:other";
  });
})()
"#,
        None,
    )
    .expect("media play promise probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__mediaPlayPromiseProbe)")
        .expect("media play promise probe should evaluate");

    assert_eq!(
        result,
        r#"{"promiseTag":"[object Promise]","paused":false,"events":"play,playing","outcome":"fulfilled:undefined"}"#
    );
}

#[test]
fn media_numeric_setters_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://media-numeric-setters-webidl.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  if (!document.body) {
    html.appendChild(document.createElement('body'));
  }
  const video = document.createElement('video');
  let volumeCalls = 0;
  let playbackRateCalls = 0;
  let currentTimeCalls = 0;
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  video.volume = {
    valueOf() {
      volumeCalls += 1;
      return '0.25';
    }
  };
  const volumeObject = `${video.volume}:${volumeCalls}`;
  video.volume = 2;
  const volumeUpperClamp = video.volume;
  video.volume = -1;
  const volumeLowerClamp = video.volume;
  const volumeInfinity = probe(() => { video.volume = Infinity; });
  const volumeSymbol = probe(() => { video.volume = Symbol('volume'); });

  video.playbackRate = {
    valueOf() {
      playbackRateCalls += 1;
      return '1.5';
    }
  };
  const playbackRateObject = `${video.playbackRate}:${playbackRateCalls}`;
  const playbackRateThrowing = probe(() => {
    video.playbackRate = {
      valueOf() {
        throw new RangeError('rate');
      }
    };
  });
  const playbackRateNaN = probe(() => { video.playbackRate = NaN; });

  video.currentTime = {
    valueOf() {
      currentTimeCalls += 1;
      return '3.5';
    }
  };
  const currentTimeObject = `${video.currentTime}:${currentTimeCalls}`;
  video.currentTime = -4;
  const currentTimeLowerClamp = video.currentTime;
  const currentTimeSymbol = probe(() => { video.currentTime = Symbol('time'); });
  const currentTimeInfinity = probe(() => { video.currentTime = Infinity; });

  return [
    volumeObject,
    volumeUpperClamp,
    volumeLowerClamp,
    volumeInfinity,
    volumeSymbol,
    playbackRateObject,
    playbackRateThrowing,
    playbackRateNaN,
    currentTimeObject,
    currentTimeLowerClamp,
    currentTimeSymbol,
    currentTimeInfinity
  ].join('|');
})()
"##,
        )
        .expect("media numeric setters should parse WebIDL values");

    assert_eq!(
        result,
        "0.25:1|1|0|throw:TypeError|throw:TypeError|1.5:1|throw:RangeError|throw:TypeError|3.5:1|0|throw:TypeError|throw:TypeError"
    );
}

#[tokio::test]
async fn media_pseudo_classes_update_has_ancestor_styles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-pseudo-classes.test/",
        &loader,
    );

    let result = vm
        .eval(
            r##"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement("html"));
  }
  if (!document.head) {
    document.documentElement.appendChild(document.createElement("head"));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement("body"));
  }
  const style = document.createElement("style");
  style.textContent = `
    #subject {
      background-color: rgb(0, 0, 0);
      border: 2px solid rgb(0, 0, 0);
      color: rgb(0, 0, 0);
      accent-color: rgb(0, 0, 0);
    }
    #subject:has(:muted) { background-color: rgb(255, 0, 0); }
    #subject:has(:playing) { border-color: rgb(0, 128, 0); }
    #subject:has(:paused) { color: rgb(255, 165, 0); }
    #subject:has(:seeking) { accent-color: rgb(0, 0, 255); }
  `;
  document.head.appendChild(style);

  const subject = document.createElement("section");
  subject.id = "subject";
  const video = document.createElement("video");
  subject.appendChild(video);
  document.body.appendChild(subject);

  let seekingEvents = 0;
  let seekedEvents = 0;
  video.addEventListener("seeking", () => {
    seekingEvents += 1;
  });
  video.addEventListener("seeked", () => {
    seekedEvents += 1;
  });
  globalThis.__lmMediaPseudo = { video, subject, counts: { get seekingEvents() { return seekingEvents; }, get seekedEvents() { return seekedEvents; } } };

  const snapshot = () => [
    video.matches(":muted"),
    video.matches(":playing"),
    video.matches(":paused"),
    video.matches(":seeking"),
    subject.matches("#subject:has(:muted)"),
    subject.matches("#subject:has(:playing)"),
    subject.matches("#subject:has(:paused)"),
    subject.matches("#subject:has(:seeking)"),
    getComputedStyle(subject).backgroundColor,
    getComputedStyle(subject).borderColor,
    getComputedStyle(subject).color,
    getComputedStyle(subject).accentColor,
    seekingEvents,
    seekedEvents
  ].join(",");

  const supports = [
    CSS.supports("selector(:muted)"),
    CSS.supports("selector(:playing)"),
    CSS.supports("selector(:paused)"),
    CSS.supports("selector(:seeking)")
  ].join(",");
  const before = snapshot();
  video.muted = true;
  const afterMuted = snapshot();
  video.play();
  const afterPlay = snapshot();
  video.currentTime = 10;
  const afterSeeking = snapshot();
  return [supports, before, afterMuted, afterPlay, afterSeeking].join("|");
})()
"##,
        )
        .expect("media pseudo-class style invalidation probe should evaluate");

    assert_eq!(
        result,
        "true,true,true,true|false,false,true,false,false,false,true,false,rgb(0, 0, 0),rgb(0, 0, 0),rgb(255, 165, 0),rgb(0, 0, 0),0,0|true,false,true,false,true,false,true,false,rgb(255, 0, 0),rgb(0, 0, 0),rgb(255, 165, 0),rgb(0, 0, 0),0,0|true,true,false,false,true,true,false,false,rgb(255, 0, 0),rgb(0, 128, 0),rgb(0, 0, 0),rgb(0, 0, 0),0,0|true,true,false,true,true,true,false,true,rgb(255, 0, 0),rgb(0, 128, 0),rgb(0, 0, 0),rgb(0, 0, 255),0,0"
    );

    assert!(
        !vm.has_ready_timeout(),
        "media seeking events must not create Page timer descriptors"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media seeking event turn").await;

    assert_eq!(
        vm.eval(
            "[__lmMediaPseudo.video.seeking, __lmMediaPseudo.counts.seekingEvents, __lmMediaPseudo.counts.seekedEvents].join(',')"
        )
        .expect("media seeking event turn should evaluate"),
        "true,1,0"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media seeked event turn").await;

    let after_task = vm
        .eval(
            r##"
(() => {
  const { video, subject, counts } = globalThis.__lmMediaPseudo;
  return [
    video.seeking,
    video.matches(":seeking"),
    subject.matches("#subject:has(:seeking)"),
    getComputedStyle(subject).accentColor,
    counts.seekingEvents,
    counts.seekedEvents
  ].join(",");
})()
"##,
        )
        .expect("media seek completion task should evaluate");

    assert_eq!(after_task, "false,false,false,rgb(0, 0, 0),1,1");
}

#[tokio::test]
async fn media_seek_completion_ignores_stale_seek_tokens() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-stale-seek-token.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  if (!document.body) {
    html.appendChild(document.createElement('body'));
  }
  const video = document.createElement('video');
  document.body.appendChild(video);
  let seekingEvents = 0;
  let seekedEvents = 0;
  video.addEventListener('seeking', () => { seekingEvents += 1; });
  video.addEventListener('seeked', () => { seekedEvents += 1; });
  video.currentTime = 1;
  video.currentTime = 2;
  globalThis.__lmStaleSeek = { video, get seekingEvents() { return seekingEvents; }, get seekedEvents() { return seekedEvents; } };
  return [video.seeking, video.currentTime, seekingEvents, seekedEvents].join(',');
})()
"#,
        )
        .expect("stale seek setup should evaluate");

    assert_eq!(result, "true,2,0,0");

    for _ in 0..4 {
        if !vm
            .run_one_media_element_event_executor_turn(&loader)
            .await
            .expect("selected dispatcher should advance stale seek completions")
        {
            break;
        }
    }

    let after_tasks = vm
        .eval(
            r#"
(() => {
  const state = globalThis.__lmStaleSeek;
  return [state.video.seeking, state.video.currentTime, state.seekingEvents, state.seekedEvents].join(',');
})()
"#,
        )
        .expect("stale seek completion result should evaluate");

    assert_eq!(after_tasks, "false,2,2,1");
}

#[tokio::test]
async fn slotted_media_state_invalidates_shadow_stylesheet_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://slotted-media-state-shadow.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('section');
  const shadow = host.attachShadow({ mode: 'open' });
  const style = document.createElement('style');
  style.textContent = `
    ::slotted(video) { color: rgb(255, 0, 0); }
    ::slotted(video:seeking) { color: rgb(0, 0, 255); }
  `;
  shadow.appendChild(style);
  shadow.appendChild(document.createElement('slot'));
  const video = document.createElement('video');
  host.appendChild(video);
  document.body.appendChild(host);
  const before = [video.matches(':seeking'), getComputedStyle(video).color].join(',');
  video.currentTime = 1;
  const during = [video.matches(':seeking'), getComputedStyle(video).color].join(',');
  globalThis.__lmSlottedMedia = video;
  return [before, during].join('|');
})()
"#,
        )
        .expect("slotted media-state setup should evaluate");

    assert_eq!(result, "false,rgb(255, 0, 0)|true,rgb(0, 0, 255)");

    for _ in 0..4 {
        if !vm
            .run_one_media_element_event_executor_turn(&loader)
            .await
            .expect("selected dispatcher should advance slotted media completion")
        {
            break;
        }
    }

    let after = vm
        .eval(
            r#"
(() => {
  const video = globalThis.__lmSlottedMedia;
  return [video.matches(':seeking'), getComputedStyle(video).color].join(',');
})()
"#,
        )
        .expect("slotted media-state result should evaluate");

    assert_eq!(after, "false,rgb(255, 0, 0)");
}

#[test]
fn media_text_track_surface_exposes_reflected_tracks_and_lists() {
    let mut vm = new_storage_test_vm("https://media-text-track-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const manual = video.addTextTrack('subtitles', 'Manual', 'en');
  const track = document.createElement('track');
  track.kind = 'CAPTIONS';
  track.label = 'Caption';
  track.srclang = 'fr';
  video.appendChild(track);
  const elementTrack = track.track;
  const cue = new VTTCue(0, 1, 'caption');
  cue.id = 'cue-id';
  manual.addCue(cue);
  manual.mode = 'showing';
  track.label = 'Caption 2';
  track.srclang = 'fr-CA';

  return [
    track.kind,
    track.getAttribute('kind'),
    track.default,
    track.readyState,
    elementTrack instanceof TextTrack,
    elementTrack.kind,
    elementTrack.label,
    elementTrack.language,
    video.textTracks === video.textTracks,
    video.textTracks.length,
    video.textTracks[0].label,
    video.textTracks[1].label,
    manual.kind,
    manual.label,
    manual.language,
    manual.cues instanceof TextTrackCueList,
    manual.cues.length,
    manual.cues[0] === cue,
    manual.cues.getCueById('cue-id') === cue,
    cue.track === manual
  ].join('|');
})()
"#,
        )
        .expect("media text track surface should evaluate");

    assert_eq!(
        result,
        "captions|CAPTIONS|false|0|true|captions|Caption 2|fr-CA|true|2|Caption 2|Manual|subtitles|Manual|en|true|1|true|true|true"
    );
}

#[test]
fn media_element_prototype_constants_are_declared() {
    let mut vm = new_storage_test_vm("https://media-element-constants-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constants = [
    ["NETWORK_EMPTY", 0],
    ["NETWORK_IDLE", 1],
    ["NETWORK_LOADING", 2],
    ["NETWORK_NO_SOURCE", 3],
    ["HAVE_NOTHING", 0],
    ["HAVE_METADATA", 1],
    ["HAVE_CURRENT_DATA", 2],
    ["HAVE_FUTURE_DATA", 3],
    ["HAVE_ENOUGH_DATA", 4]
  ];
  const descriptorShape = (owner, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      name,
      descriptor && descriptor.value,
      descriptor && descriptor.value === expected,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable,
      descriptor && descriptor.configurable
    ].join(":");
  };
  const constructorOwn = (ctor) =>
    constants
      .map(([name]) => name)
      .filter(name => Object.prototype.hasOwnProperty.call(ctor, name));
  const prototypeKeysContainConstants = (prototype) =>
    Object.keys(prototype).some(name =>
      constants.some(([constant]) => constant === name)
    );
  const video = document.createElement("video");
  const audio = document.createElement("audio");
  return JSON.stringify({
    media: constants.map(([name, value]) =>
      descriptorShape(HTMLMediaElement.prototype, name, value)
    ),
    audio: constants.map(([name, value]) =>
      descriptorShape(HTMLAudioElement.prototype, name, value)
    ),
    video: constants.map(([name, value]) =>
      descriptorShape(HTMLVideoElement.prototype, name, value)
    ),
    constructorOwn: {
      media: constructorOwn(HTMLMediaElement),
      audio: constructorOwn(HTMLAudioElement),
      video: constructorOwn(HTMLVideoElement)
    },
    instanceOwn: {
      audio: constants
        .map(([name]) => name)
        .filter(name => Object.prototype.hasOwnProperty.call(audio, name)),
      video: constants
        .map(([name]) => name)
        .filter(name => Object.prototype.hasOwnProperty.call(video, name))
    },
    keysContainConstants: [
      prototypeKeysContainConstants(HTMLMediaElement.prototype),
      prototypeKeysContainConstants(HTMLAudioElement.prototype),
      prototypeKeysContainConstants(HTMLVideoElement.prototype)
    ],
    inheritedValues: [
      audio.networkState === audio.NETWORK_EMPTY,
      video.networkState === video.NETWORK_EMPTY,
      audio.readyState === audio.HAVE_NOTHING,
      video.readyState === video.HAVE_NOTHING
    ]
  });
})()
"#,
        )
        .expect("media element constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"media":["NETWORK_EMPTY:0:true:true:false:false","NETWORK_IDLE:1:true:true:false:false","NETWORK_LOADING:2:true:true:false:false","NETWORK_NO_SOURCE:3:true:true:false:false","HAVE_NOTHING:0:true:true:false:false","HAVE_METADATA:1:true:true:false:false","HAVE_CURRENT_DATA:2:true:true:false:false","HAVE_FUTURE_DATA:3:true:true:false:false","HAVE_ENOUGH_DATA:4:true:true:false:false"],"audio":["NETWORK_EMPTY:0:true:true:false:false","NETWORK_IDLE:1:true:true:false:false","NETWORK_LOADING:2:true:true:false:false","NETWORK_NO_SOURCE:3:true:true:false:false","HAVE_NOTHING:0:true:true:false:false","HAVE_METADATA:1:true:true:false:false","HAVE_CURRENT_DATA:2:true:true:false:false","HAVE_FUTURE_DATA:3:true:true:false:false","HAVE_ENOUGH_DATA:4:true:true:false:false"],"video":["NETWORK_EMPTY:0:true:true:false:false","NETWORK_IDLE:1:true:true:false:false","NETWORK_LOADING:2:true:true:false:false","NETWORK_NO_SOURCE:3:true:true:false:false","HAVE_NOTHING:0:true:true:false:false","HAVE_METADATA:1:true:true:false:false","HAVE_CURRENT_DATA:2:true:true:false:false","HAVE_FUTURE_DATA:3:true:true:false:false","HAVE_ENOUGH_DATA:4:true:true:false:false"],"constructorOwn":{"media":["NETWORK_EMPTY","NETWORK_IDLE","NETWORK_LOADING","NETWORK_NO_SOURCE","HAVE_NOTHING","HAVE_METADATA","HAVE_CURRENT_DATA","HAVE_FUTURE_DATA","HAVE_ENOUGH_DATA"],"audio":["NETWORK_EMPTY","NETWORK_IDLE","NETWORK_LOADING","NETWORK_NO_SOURCE","HAVE_NOTHING","HAVE_METADATA","HAVE_CURRENT_DATA","HAVE_FUTURE_DATA","HAVE_ENOUGH_DATA"],"video":["NETWORK_EMPTY","NETWORK_IDLE","NETWORK_LOADING","NETWORK_NO_SOURCE","HAVE_NOTHING","HAVE_METADATA","HAVE_CURRENT_DATA","HAVE_FUTURE_DATA","HAVE_ENOUGH_DATA"]},"instanceOwn":{"audio":[],"video":[]},"keysContainConstants":[true,true,true],"inheritedValues":[true,true,true,true]}"#
    );
}

#[test]
fn media_error_constants_are_declared_on_constructor_and_prototype() {
    let mut vm = new_storage_test_vm("https://media-error-constants-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constants = [
    ["MEDIA_ERR_ABORTED", 1],
    ["MEDIA_ERR_NETWORK", 2],
    ["MEDIA_ERR_DECODE", 3],
    ["MEDIA_ERR_SRC_NOT_SUPPORTED", 4]
  ];
  const descriptorShape = (owner, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      name,
      descriptor && descriptor.value,
      descriptor && descriptor.value === expected,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable,
      descriptor && descriptor.configurable
    ].join(":");
  };
  const keysContainConstants = owner =>
    Object.keys(owner).some(name =>
      constants.some(([constant]) => constant === name)
    );
  return JSON.stringify({
    constructor: constants.map(([name, value]) =>
      descriptorShape(MediaError, name, value)
    ),
    prototype: constants.map(([name, value]) =>
      descriptorShape(MediaError.prototype, name, value)
    ),
    keysContainConstants: [
      keysContainConstants(MediaError),
      keysContainConstants(MediaError.prototype)
    ],
    constructorMatchesPrototype: constants.every(([name]) =>
      MediaError[name] === MediaError.prototype[name]
    )
  });
})()
"#,
        )
        .expect("MediaError constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["MEDIA_ERR_ABORTED:1:true:true:false:false","MEDIA_ERR_NETWORK:2:true:true:false:false","MEDIA_ERR_DECODE:3:true:true:false:false","MEDIA_ERR_SRC_NOT_SUPPORTED:4:true:true:false:false"],"prototype":["MEDIA_ERR_ABORTED:1:true:true:false:false","MEDIA_ERR_NETWORK:2:true:true:false:false","MEDIA_ERR_DECODE:3:true:true:false:false","MEDIA_ERR_SRC_NOT_SUPPORTED:4:true:true:false:false"],"keysContainConstants":[true,true],"constructorMatchesPrototype":true}"#
    );
}

#[test]
fn media_error_instance_attributes_are_readonly_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://media-error-attributes-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const shape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(MediaError.prototype, name);
    return [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  let codeBrand;
  let messageBrand;
  try { Reflect.get(MediaError.prototype, "code", {}); } catch (error) { codeBrand = error.name; }
  try { Reflect.get(MediaError.prototype, "message", {}); } catch (error) { messageBrand = error.name; }
  return [shape("code"), shape("message"), codeBrand, messageBrand].join("|");
})()
"#,
        )
        .expect("MediaError attribute descriptor probe should evaluate");

    assert_eq!(
        result,
        "function:true:true:true|function:true:true:true|TypeError|TypeError"
    );
}

#[test]
fn html_track_element_constants_are_declared_on_constructor_and_prototype() {
    let mut vm = new_storage_test_vm("https://html-track-constants-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constants = [
    ["NONE", 0],
    ["LOADING", 1],
    ["LOADED", 2],
    ["ERROR", 3]
  ];
  const descriptorShape = (owner, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      name,
      descriptor && descriptor.value,
      descriptor && descriptor.value === expected,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable,
      descriptor && descriptor.configurable
    ].join(":");
  };
  const track = document.createElement("track");
  return JSON.stringify({
    constructor: constants.map(([name, value]) =>
      descriptorShape(HTMLTrackElement, name, value)
    ),
    prototype: constants.map(([name, value]) =>
      descriptorShape(HTMLTrackElement.prototype, name, value)
    ),
    instanceOwn: constants
      .map(([name]) => name)
      .filter(name => Object.prototype.hasOwnProperty.call(track, name)),
    keysContainConstants: Object.keys(HTMLTrackElement).some(name =>
      constants.some(([constant]) => constant === name)
    ) || Object.keys(HTMLTrackElement.prototype).some(name =>
      constants.some(([constant]) => constant === name)
    ),
    readyStateMatchesNone: track.readyState === HTMLTrackElement.NONE
  });
})()
"#,
        )
        .expect("HTMLTrackElement constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["NONE:0:true:true:false:false","LOADING:1:true:true:false:false","LOADED:2:true:true:false:false","ERROR:3:true:true:false:false"],"prototype":["NONE:0:true:true:false:false","LOADING:1:true:true:false:false","LOADED:2:true:true:false:false","ERROR:3:true:true:false:false"],"instanceOwn":[],"keysContainConstants":true,"readyStateMatchesNone":true}"#
    );
}

#[test]
fn media_text_track_surface_is_declared_on_interface_prototypes() {
    let mut vm = new_storage_test_vm("https://media-text-track-declared-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const trackElement = document.createElement('track');
  trackElement.id = 'caption-track';
  video.appendChild(trackElement);
  const elementTrack = trackElement.track;
  const manual = video.addTextTrack('subtitles', 'Manual', 'en');
  manual.mode = 'hidden';
  const cue = new VTTCue(0, 1, 'caption');
  cue.id = 'cue-id';
  manual.addCue(cue);
  const second = new VTTCue(1, 2, 'second');
  second.id = 'second';
  manual.addCue(second);
  const list = video.textTracks;
  const cues = manual.cues;
  const ownFrom = (object, names) =>
    Object.getOwnPropertyNames(object).filter(name => names.includes(name));
  const accessorDescriptor = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    return {
      enumerable: descriptor.enumerable,
      configurable: descriptor.configurable,
      getter: `${descriptor.get.name}:${descriptor.get.length}`,
      setter: descriptor.set ? `${descriptor.set.name}:${descriptor.set.length}` : null
    };
  };
  const methodDescriptor = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    return {
      enumerable: descriptor.enumerable,
      writable: descriptor.writable,
      configurable: descriptor.configurable,
      name: descriptor.value.name,
      length: descriptor.value.length
    };
  };
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const listSurfaceNames = ['length', 'onaddtrack', 'onremovetrack', 'getTrackById'];
  const cueListSurfaceNames = ['length', 'getCueById'];
  const trackSurfaceNames = [
    'kind', 'label', 'language', 'id', 'mode', 'cues', 'activeCues',
    'oncuechange', 'addCue', 'removeCue'
  ];
  const secondLookupBeforeRemove = cues.getCueById('second') === second;
  manual.removeCue(second);
  const mode = Object.getOwnPropertyDescriptor(TextTrack.prototype, 'mode');
  mode.set.call(manual, 'showing');

  return JSON.stringify({
    own: {
      list: ownFrom(list, listSurfaceNames),
      cues: ownFrom(cues, cueListSurfaceNames),
      track: ownFrom(manual, trackSurfaceNames)
    },
    prototypes: {
      list: ownFrom(TextTrackList.prototype, listSurfaceNames),
      cues: ownFrom(TextTrackCueList.prototype, cueListSurfaceNames),
      track: ownFrom(TextTrack.prototype, trackSurfaceNames)
    },
    chains: [
      Object.getPrototypeOf(TextTrackList.prototype) === EventTarget.prototype,
      Object.getPrototypeOf(TextTrack.prototype) === EventTarget.prototype
    ],
    accessors: {
      listLength: accessorDescriptor(TextTrackList.prototype, 'length'),
      onaddtrack: accessorDescriptor(TextTrackList.prototype, 'onaddtrack'),
      cueListLength: accessorDescriptor(TextTrackCueList.prototype, 'length'),
      mode: accessorDescriptor(TextTrack.prototype, 'mode'),
      oncuechange: accessorDescriptor(TextTrack.prototype, 'oncuechange')
    },
    methods: {
      getTrackById: methodDescriptor(TextTrackList.prototype, 'getTrackById'),
      getCueById: methodDescriptor(TextTrackCueList.prototype, 'getCueById'),
      addCue: methodDescriptor(TextTrack.prototype, 'addCue'),
      removeCue: methodDescriptor(TextTrack.prototype, 'removeCue')
    },
    borrowed: [
      Object.getOwnPropertyDescriptor(TextTrackList.prototype, 'length').get.call(list),
      Object.getOwnPropertyDescriptor(TextTrackCueList.prototype, 'length').get.call(cues),
      mode.get.call(manual)
    ],
    forged: [
      probe(() => mode.get.call({})),
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackList.prototype, 'length').get.call({})
      ),
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackCueList.prototype, 'getCueById')
          .value.call({}, 'cue-id')
      )
    ],
    crossBrand: [
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackList.prototype, 'length').get.call(cues)
      ),
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackCueList.prototype, 'length').get.call(list)
      ),
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackList.prototype, 'getTrackById')
          .value.call(cues, 'cue-id')
      ),
      probe(() =>
        Object.getOwnPropertyDescriptor(TextTrackCueList.prototype, 'getCueById')
          .value.call(list, 'caption-track')
      )
    ],
    getTrackByIdBehavior: [
      list.getTrackById('caption-track') === elementTrack,
      list.getTrackById('missing') === null
    ],
    getCueByIdBehavior: [
      cues.getCueById('cue-id') === cue,
      secondLookupBeforeRemove,
      cues.getCueById('second') === null,
      second.track === null
    ]
  });
})()
"#,
        )
        .expect("media text track declared surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":{"list":[],"cues":[],"track":[]},"prototypes":{"list":["length","onaddtrack","onremovetrack","getTrackById"],"cues":["length","getCueById"],"track":["kind","id","label","language","mode","cues","activeCues","oncuechange","addCue","removeCue"]},"chains":[true,true],"accessors":{"listLength":{"enumerable":true,"configurable":true,"getter":"get length:0","setter":null},"onaddtrack":{"enumerable":true,"configurable":true,"getter":"get onaddtrack:0","setter":"set onaddtrack:1"},"cueListLength":{"enumerable":true,"configurable":true,"getter":"get length:0","setter":null},"mode":{"enumerable":true,"configurable":true,"getter":"get mode:0","setter":"set mode:1"},"oncuechange":{"enumerable":true,"configurable":true,"getter":"get oncuechange:0","setter":"set oncuechange:1"}},"methods":{"getTrackById":{"enumerable":true,"writable":true,"configurable":true,"name":"getTrackById","length":1},"getCueById":{"enumerable":true,"writable":true,"configurable":true,"name":"getCueById","length":1},"addCue":{"enumerable":true,"writable":true,"configurable":true,"name":"addCue","length":1},"removeCue":{"enumerable":true,"writable":true,"configurable":true,"name":"removeCue","length":1}},"borrowed":[2,1,"showing"],"forged":["TypeError","TypeError","TypeError"],"crossBrand":["TypeError","TypeError","TypeError","TypeError"],"getTrackByIdBehavior":[true,true],"getCueByIdBehavior":[true,true,true,true]}"#
    );
}

#[test]
fn track_event_track_uses_prototype_private_slot() {
    let mut vm = new_storage_test_vm("https://track-event-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const first = video.addTextTrack('subtitles', 'First', 'en');
  const second = video.addTextTrack('captions', 'Second', 'en');
  const event = new TrackEvent('addtrack', { track: first });
  const empty = new TrackEvent('addtrack');
  const descriptor = Object.getOwnPropertyDescriptor(TrackEvent.prototype, 'track');
  const ownBefore = Object.getOwnPropertyNames(event).filter(name => name === 'track');
  event.track = second;
  const fake = { track: second };

  return JSON.stringify({
    hasGetter: typeof descriptor.get === 'function',
    getterName: descriptor.get.name,
    getterLength: descriptor.get.length,
    hasSetter: 'set' in descriptor && descriptor.set === undefined,
    enumerable: descriptor.enumerable,
    configurable: descriptor.configurable,
    ownBefore,
    trackAfterAssignment: event.track === first,
    assignmentCreatedOwn: Object.prototype.hasOwnProperty.call(event, 'track'),
    emptyTrackIsNull: empty.track === null,
    fakeTrackIsNull: descriptor.get.call(fake) === null
  });
})()
"#,
        )
        .expect("TrackEvent private slot probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasGetter":true,"getterName":"get track","getterLength":0,"hasSetter":true,"enumerable":true,"configurable":true,"ownBefore":[],"trackAfterAssignment":true,"assignmentCreatedOwn":false,"emptyTrackIsNull":true,"fakeTrackIsNull":true}"#
    );
}

#[test]
fn media_text_track_reflection_and_cue_list_indexing_match_wpt_edges() {
    let mut vm = new_storage_test_vm("https://media-text-track-wpt-edges.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const emptySrcTrack = document.createElement('track');
  emptySrcTrack.src = '';
  const emptyTrackSrc = `${emptySrcTrack.src}:${emptySrcTrack.getAttribute('src')}`;

  const track = document.createElement('track');
  track.label = 'bar';
  track.srclang = 'en';
  video.appendChild(track);
  const elementTrack = track.track;
  track.removeAttribute('label');
  track.removeAttribute('srclang');
  const reflected = `${elementTrack.label}:${elementTrack.language}`;

  const manual = video.addTextTrack('subtitles');
  const cues = manual.cues;
  let strictCreate = 'no-throw';
  try {
    Function('cues', '"use strict"; cues[0] = "x";')(cues);
  } catch (error) {
    strictCreate = error.name;
  }
  cues[0] = 'x';
  const emptyIndex = cues[0] === undefined;

  const first = new VTTCue(0, 1, 'first');
  first.id = 'first';
  const second = new VTTCue(1, 2, 'second');
  second.id = 'second';
  manual.addCue(first);
  manual.addCue(second);
  second.startTime = 0;
  const tieOrder = `${cues[0].id}:${cues[1].id}`;
  let strictSet = 'no-throw';
  try {
    Function('cues', '"use strict"; cues[0] = "x";')(cues);
  } catch (error) {
    strictSet = error.name;
  }
  cues[0] = 'x';
  const retained = cues[0] === second;
  manual.removeCue(second);
  manual.removeCue(first);
  const removedIndex = cues[0] === undefined;

  return [emptyTrackSrc, reflected, strictCreate, emptyIndex, tieOrder, strictSet, retained, removedIndex].join('|');
})()
"#,
        )
        .expect("media text track WPT edge probe should evaluate");

    assert_eq!(
        result,
        "https://media-text-track-wpt-edges.test/:|:|TypeError|true|second:first|TypeError|true|true"
    );
}
#[tokio::test]
async fn media_text_track_data_src_loads_after_media_parent_and_clears_cues() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-text-track-loading.test/",
        &loader,
    );

    let before = vm
        .eval(
            r#"
(() => {
  const source = 'data:text/vtt,WEBVTT%0A%0A2%0A00%3A00%3A02.000%20--%3E%2000%3A00%3A03.000%0Atwo%0A%0A1%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Aone';
  const video = document.createElement('video');
  const track = document.createElement('track');
  track.src = source;
  track.default = true;
  const beforeParent = track.readyState;
  video.appendChild(track);
  const beforePreferenceTask = track.track.cues === null;
  video.play();
  globalThis.__lmTrackLoadProbe = { track };
  return [beforeParent, beforePreferenceTask, track.readyState].join('|');
})()
"#,
        )
        .expect("media text track data source setup should evaluate");

    assert_eq!(before, "0|true|0");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("already-applied default-mode task should settle"),
        "track insertion should queue one coalesced default-mode task"
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track terminal networking turn")
    );

    let after_first_load = vm
        .eval(
            r#"
(() => {
  const track = globalThis.__lmTrackLoadProbe.track;
  const cues = track.track.cues;
  const loaded = `${track.readyState}:${cues.length}:${cues[0].id}:${cues[1].id}`;
  const cue = new VTTCue(1.5, 2, 'middle');
  cue.id = 'middle';
  track.track.addCue(cue);
  const sorted = `${cues.length}:${cues[1].id}`;
  track.src = 'data:text/vtt,WEBVTT';
  globalThis.__lmTrackLoadProbe.cues = cues;
  return [loaded, sorted, cues.length, track.readyState, track.track.cues === cues].join('|');
})()
"#,
        )
        .expect("media text track data source should load and clear cues");

    assert_eq!(after_first_load, "2:2:1:2|3:middle|0|0|true");

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track reload-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track reload terminal networking turn")
    );

    let after_second_load = vm
        .eval(
            r#"
(() => {
  const track = globalThis.__lmTrackLoadProbe.track;
  const cues = globalThis.__lmTrackLoadProbe.cues;
  return [track.readyState, cues.length, track.track.cues === cues].join('|');
})()
"#,
        )
        .expect("media text track empty source should finish loading");

    assert_eq!(after_second_load, "2|0|true");

    let before_missing_source = vm
        .eval(
            r#"
(() => {
  const track = globalThis.__lmTrackLoadProbe.track;
  globalThis.__lmTrackLoadProbe.errors = 0;
  track.addEventListener('error', () => globalThis.__lmTrackLoadProbe.errors += 1);
  track.removeAttribute('src');
  return `${track.readyState}:${globalThis.__lmTrackLoadProbe.errors}`;
})()
"#,
        )
        .expect("removing the text-track source should queue failure");
    assert_eq!(before_missing_source, "0:0");

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("missing text-track source start should settle")
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackLoad,
            &loader,
        )
        .await
        .expect("missing text-track source failure should settle")
    );
    assert_eq!(
        vm.eval(
            "`${globalThis.__lmTrackLoadProbe.track.readyState}:${globalThis.__lmTrackLoadProbe.errors}`"
        )
        .expect("missing text-track source state should evaluate"),
        "3:1"
    );
}

#[tokio::test]
async fn media_text_track_load_honors_media_src_csp() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-track-csp.test/",
        &loader,
    );
    vm.set_response_content_security_policies(&["media-src 'self'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmTrackCspEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmTrackCspEvents.push(`csp:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const video = document.createElement('video');
  const track = document.createElement('track');
  track.default = true;
  track.addEventListener('load', () => __lmTrackCspEvents.push('load'));
  track.addEventListener('error', () => __lmTrackCspEvents.push('error'));
  video.appendChild(track);
  (document.body || document.documentElement || document).appendChild(video);
  track.src = 'data:text/vtt,WEBVTT';
  globalThis.__lmTrackCspElement = track;
})()
"#,
    )
    .expect("text track CSP setup should evaluate");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("text-track default-mode task should settle"),
        "track insertion should queue one default-mode task"
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track CSP load-start networking turn"),
        "track source should queue one typed load-start task"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackLoad,
            &loader,
        )
        .await
        .expect("text-track CSP failure terminal DOM-manipulation turn"),
        "blocked track source should queue one typed failure terminal task"
    );

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  events: __lmTrackCspEvents.sort(),
  readyState: __lmTrackCspElement.readyState,
  cueCount: __lmTrackCspElement.track.cues.length,
})"#,
        )
        .expect("text track CSP result should evaluate"),
        r#"{"events":["csp:media-src:data","error"],"readyState":3,"cueCount":0}"#
    );
}

#[tokio::test]
async fn media_text_track_list_addtrack_events_are_queued() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-text-track-addtrack.test/",
        &loader,
    );

    let before = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const trackElement = document.createElement('track');
  video.appendChild(trackElement);
  const first = trackElement.track;
  const list = video.textTracks;
  const events = [];
  list.onaddtrack = event => {
    events.push([
      event.target === list,
      event instanceof TrackEvent,
      event.track === (events.length === 0 ? first : video.textTracks[1])
    ].join(':'));
    if (events.length === 1) {
      video.addTextTrack('captions', 'Caption Track', 'en');
    }
  };
  globalThis.__lmTrackListAddTrackProbe = { video, first, list, events };
  return [list.length, events.length].join('|');
})()
"#,
        )
        .expect("media text track addtrack setup should evaluate");

    assert_eq!(before, "1|0");

    run_next_page_media_element_event_for_test(&mut vm, &loader, "first addtrack event turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "second addtrack event turn")
        .await;

    let after = vm
        .eval(
            r#"
(() => {
  const probe = globalThis.__lmTrackListAddTrackProbe;
  return [probe.list.length, probe.events.join('|')].join('|');
})()
"#,
        )
        .expect("media text track addtrack events should fire");

    assert_eq!(after, "2|true:true:true|true:true:true");
}
#[tokio::test]
async fn media_active_cues_update_after_media_load_and_play() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-active-cues.test/",
        &loader,
    );

    let before = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const video = document.createElement('video');
  const track = video.addTextTrack('subtitles');
  const first = new VTTCue(0, 1, 'first');
  const second = new VTTCue(1, 2, 'second');
  track.addCue(first);
  track.addCue(second);
  track.mode = 'showing';
  const active = track.activeCues;
  globalThis.__lmActiveCueProbe = { video, track, active, loaded: false };
  video.onloadeddata = () => {
    const beforePlay = `${active.length}:${video.readyState}`;
    video.play();
    const afterPlay = `${active.length}:${active[0] && active[0].text}`;
    const third = new VTTCue(0, 2, 'third');
    track.addCue(third);
    globalThis.__lmActiveCueProbe.result = [
      beforePlay,
      afterPlay,
      `${active.length}:${active[0].text}:${active[1].text}`
    ].join('|');
    globalThis.__lmActiveCueProbe.loaded = true;
  };
  document.body.appendChild(video);
  video.src = 'data:video/webm;base64,AA==';
  return `${active.length}:${globalThis.__lmActiveCueProbe.loaded}`;
})()
"#,
        )
        .expect("media active cue setup should evaluate");

    assert_eq!(before, "0:false");

    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("active-cue media {phase} turn"),
        )
        .await;
    }

    let after = vm
        .eval(
            r#"
[
  globalThis.__lmActiveCueProbe.loaded,
  globalThis.__lmActiveCueProbe.result
].join('|')
"#,
        )
        .expect("media active cue result should evaluate");

    assert_eq!(after, "true|0:2|1:first|2:third:first");
}
#[tokio::test]
async fn default_track_inserted_while_playing_refreshes_active_cues() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://default-track-active-cues.test/",
        &loader,
    );

    let before = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const video = document.createElement('video');
  document.body.appendChild(video);
  video.play();
  const trackElement = document.createElement('track');
  trackElement.default = true;
  trackElement.src = 'data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Aactive';
  video.appendChild(trackElement);
  globalThis.__lmDefaultTrackProbe = {
    track: trackElement.track,
    beforeMode: trackElement.track.mode,
    beforeActive: trackElement.track.activeCues
  };
  return `${globalThis.__lmDefaultTrackProbe.beforeMode}:${globalThis.__lmDefaultTrackProbe.beforeActive}`;
})()
"#,
        )
        .expect("default text track setup should evaluate");

    assert_eq!(before, "disabled:null");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default-mode DOM-manipulation task should settle"),
        "dynamic default track should queue one typed mode-selection task"
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default text-track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default text-track terminal networking turn")
    );

    let after = vm
        .eval(
            r#"
[
  globalThis.__lmDefaultTrackProbe.track.mode,
  globalThis.__lmDefaultTrackProbe.track.activeCues.length,
  globalThis.__lmDefaultTrackProbe.track.activeCues[0].text
].join('|')
"#,
        )
        .expect("default text track active cue result should evaluate");

    assert_eq!(after, "showing|1|active");
}
#[test]
fn vtt_cue_get_cue_as_html_returns_text_fragment() {
    let mut vm = new_storage_test_vm("https://vtt-cue-html.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const empty = new VTTCue(0, 1, '');
  const emptyFragment = empty.getCueAsHTML();
  const text = new VTTCue(0, 1, 'hello');
  const textFragment = text.getCueAsHTML();
  return [
    emptyFragment instanceof DocumentFragment,
    emptyFragment.childNodes.length,
    emptyFragment.childNodes[0] instanceof Text,
    emptyFragment.childNodes[0].data,
    textFragment.textContent,
    textFragment.childNodes.length
  ].join('|');
})()
"#,
        )
        .expect("VTTCue getCueAsHTML probe should evaluate");

    assert_eq!(result, "true|1|true||hello|1");
}
#[test]
fn vtt_cue_get_cue_as_html_builds_supported_markup_fragment() {
    let mut vm = new_storage_test_vm("https://vtt-cue-markup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const cue = new VTTCue(0, 1, 'A <b>bold</b> <i.larger>italic</i> <ruby>Bear<rt>bear</rt></ruby> <c.red.upper>class</c> <v.blue Speaker>voice</v>');
  const fragment = cue.getCueAsHTML();
  const b = fragment.querySelector('b');
  const i = fragment.querySelector('i');
  const ruby = fragment.querySelector('ruby');
  const rt = fragment.querySelector('rt');
  const spans = fragment.querySelectorAll('span');
  return [
    fragment.textContent,
    b && b.textContent,
    i && i.className,
    ruby && ruby.textContent,
    rt && rt.textContent,
    spans[0] && spans[0].className,
    spans[1] && spans[1].className,
    spans[1] && spans[1].title
  ].join('|');
})()
"#,
        )
        .expect("VTTCue getCueAsHTML markup probe should evaluate");

    assert_eq!(
        result,
        "A bold italic Bearbear class voice|bold|larger|Bearbear|bear|red upper|blue|Speaker"
    );
}

#[test]
fn vtt_cue_surface_is_declared_on_interface_prototypes() {
    let mut vm = new_storage_test_vm("https://vtt-cue-declared-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const cue = new VTTCue(1, 2, 'hello');
  const expectedNames = [
    'addEventListener',
    'removeEventListener',
    'dispatchEvent',
    'startTime',
    'endTime',
    'text',
    'id',
    'vertical',
    'snapToLines',
    'line',
    'position',
    'size',
    'align',
    'getCueAsHTML',
    'pauseOnExit',
    'onenter',
    'onexit',
    'track'
  ];
  const descriptor = (object, name) => {
    const desc = Object.getOwnPropertyDescriptor(object, name);
    if (Object.prototype.hasOwnProperty.call(desc, 'value')) {
      return {
        enumerable: desc.enumerable,
        writable: desc.writable,
        configurable: desc.configurable,
        value: desc.value,
        name: typeof desc.value === 'function' ? desc.value.name : undefined,
        length: typeof desc.value === 'function' ? desc.value.length : undefined
      };
    }
    return {
      enumerable: desc.enumerable,
      configurable: desc.configurable,
      hasGetter: typeof desc.get === 'function',
      hasSetter: typeof desc.set === 'function'
    };
  };
  cue.onenter = () => {};
  const onenterAssigned = typeof cue.onenter === 'function';
  cue.onenter = undefined;
  const onenterUndefined = cue.onenter === null;
  const startTime = Object.getOwnPropertyDescriptor(TextTrackCue.prototype, 'startTime');
  const text = Object.getOwnPropertyDescriptor(VTTCue.prototype, 'text');
  const getCueAsHTML =
    Object.getOwnPropertyDescriptor(VTTCue.prototype, 'getCueAsHTML').value;
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  startTime.set.call(cue, 1.5);
  return JSON.stringify({
    ownNames: Object.getOwnPropertyNames(cue).filter(name => expectedNames.includes(name)),
    ownKeys: Object.keys(cue).filter(name => expectedNames.includes(name)),
    internalNames: Object.getOwnPropertyNames(cue)
      .filter(name => name.startsWith('__moli')).sort(),
    textTrackCueNames: Object.getOwnPropertyNames(TextTrackCue.prototype)
      .filter(name => expectedNames.includes(name)),
    vttCueNames: Object.getOwnPropertyNames(VTTCue.prototype)
      .filter(name => expectedNames.includes(name)),
    chains: [
      Object.getPrototypeOf(VTTCue.prototype) === TextTrackCue.prototype,
      Object.getPrototypeOf(TextTrackCue.prototype) === EventTarget.prototype,
      cue instanceof EventTarget
    ],
    startTime: descriptor(TextTrackCue.prototype, 'startTime'),
    text: descriptor(VTTCue.prototype, 'text'),
    getCueAsHTML: descriptor(VTTCue.prototype, 'getCueAsHTML'),
    pauseOnExit: descriptor(TextTrackCue.prototype, 'pauseOnExit'),
    onenter: descriptor(TextTrackCue.prototype, 'onenter'),
    track: descriptor(TextTrackCue.prototype, 'track'),
    borrowedStartTime: startTime.get.call(cue),
    forged: [
      probe(() => startTime.get.call({})),
      probe(() => text.set.call({}, 'forged')),
      probe(() => getCueAsHTML.call({}))
    ],
    defaults: [
      cue.startTime,
      cue.endTime,
      cue.text,
      cue.id,
      cue.vertical,
      cue.snapToLines,
      cue.line,
      cue.position,
      cue.size,
      cue.align,
      cue.pauseOnExit,
      cue.onexit === null,
      cue.track === null,
      cue.getCueAsHTML().textContent
    ],
    onenterAssigned,
    onenterUndefined
  });
})()
"#,
        )
        .expect("VTTCue declared surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownNames":[],"ownKeys":[],"internalNames":[],"textTrackCueNames":["track","id","startTime","endTime","pauseOnExit","onenter","onexit"],"vttCueNames":["vertical","snapToLines","line","position","size","align","getCueAsHTML","text"],"chains":[true,true,true],"startTime":{"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"text":{"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"getCueAsHTML":{"enumerable":true,"writable":true,"configurable":true,"name":"getCueAsHTML","length":0},"pauseOnExit":{"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"onenter":{"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"track":{"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"borrowedStartTime":1.5,"forged":["TypeError","TypeError","TypeError"],"defaults":[1.5,2,"hello","","",true,"auto","auto",100,"center",false,true,true,"hello"],"onenterAssigned":true,"onenterUndefined":true}"#
    );
}

#[test]
fn script_type_and_language_classification_matches_wpt_matrix() {
    let mut vm = new_storage_test_vm("https://script-type-language.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  const placeholder = document.createElement('div');
  document.documentElement.appendChild(placeholder);
  window.ran = false;
  const probe = (attr, value, expected) => {
    window.ran = false;
    const script = document.createElement('script');
    script.setAttribute(attr, value);
    script.textContent = 'window.ran = true;';
    placeholder.appendChild(script);
    if (window.ran !== expected) {
      failures.push(`${attr}=${JSON.stringify(value)}:${window.ran}->${expected}`);
    }
  };
  const typeShouldRun = value => probe('type', value, true);
  const typeShouldNotRun = value => probe('type', value, false);
  const languageShouldRun = value => probe('language', value, true);
  const languageShouldNotRun = value => probe('language', value, false);

  const application = ['ecmascript', 'javascript', 'x-ecmascript', 'x-javascript'];
  const text = [
    'ecmascript', 'javascript', 'javascript1.0', 'javascript1.1',
    'javascript1.2', 'javascript1.3', 'javascript1.4', 'javascript1.5',
    'jscript', 'livescript', 'x-ecmascript', 'x-javascript'
  ];
  const legacyTypes = ['javascript1.6', 'javascript1.7', 'javascript1.8', 'javascript1.9'];
  const spaces = [' ', '\t', '\n', '\r', '\f'];

  typeShouldRun('');
  typeShouldNotRun(' ');
  application.map(t => 'application/' + t).forEach(typeShouldRun);
  application.map(t => ('application/' + t).toUpperCase()).forEach(typeShouldRun);
  spaces.forEach(s => {
    application.map(t => 'application/' + t + s).forEach(typeShouldRun);
    application.map(t => s + 'application/' + t).forEach(typeShouldRun);
  });
  application.map(t => 'application/' + t + '\0').forEach(typeShouldNotRun);
  application.map(t => 'application/' + t + '\0foo').forEach(typeShouldNotRun);
  text.map(t => 'text/' + t).forEach(typeShouldRun);
  text.map(t => ('text/' + t).toUpperCase()).forEach(typeShouldRun);
  legacyTypes.map(t => 'text/' + t).forEach(typeShouldNotRun);
  spaces.forEach(s => {
    text.map(t => 'text/' + t + s).forEach(typeShouldRun);
    text.map(t => s + 'text/' + t).forEach(typeShouldRun);
  });
  text.map(t => 'text/' + t + '\0').forEach(typeShouldNotRun);
  text.map(t => 'text/' + t + '\0foo').forEach(typeShouldNotRun);
  text.forEach(typeShouldNotRun);
  legacyTypes.forEach(typeShouldNotRun);

  languageShouldRun('');
  languageShouldNotRun(' ');
  text.forEach(languageShouldRun);
  text.map(t => t.toUpperCase()).forEach(languageShouldRun);
  legacyTypes.forEach(languageShouldNotRun);
  spaces.forEach(s => {
    text.map(t => t + s).forEach(languageShouldNotRun);
    text.map(t => s + t).forEach(languageShouldNotRun);
  });
  text.map(t => t + 'xyz').forEach(languageShouldNotRun);
  text.map(t => 'xyz' + t).forEach(languageShouldNotRun);
  text.map(t => t + '\0').forEach(languageShouldNotRun);
  text.map(t => t + '\0foo').forEach(languageShouldNotRun);

  return failures.join('\n');
})()
"#,
        )
        .expect("script type/language probe should evaluate");

    assert_eq!(result, "");
}
#[test]
fn vtt_cue_accessors_follow_webidl_and_resort_track_cues() {
    let mut vm = new_storage_test_vm("https://vtt-cue-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const video = document.createElement('video');
  const track = video.addTextTrack('subtitles');
  const cue = new VTTCue(1, 2, 'one');
  const later = new VTTCue(3, 4, 'two');
  const emptyIdCue = new VTTCue(5, 6, 'empty');
  cue.id = null;
  cue.pauseOnExit = 'yes';
  const idAndPause = `${cue.id}:${cue.pauseOnExit}`;
  cue.pauseOnExit = null;
  const pauseAfterNull = cue.pauseOnExit;
  const startNaN = probe(() => { cue.startTime = NaN; });
  const endPositiveInfinity = probe(() => { cue.endTime = Infinity; });
  const endNegativeInfinity = probe(() => { cue.endTime = -Infinity; });
  cue.position = 25;
  const positionNumber = cue.position;
  cue.position = 'auto';
  const positionAuto = cue.position;
  let entered = false;
  cue.onenter = () => { entered = true; };
  cue.dispatchEvent(new Event('enter'));
  cue.onenter = undefined;
  const onenterUndefined = cue.onenter === null;
  let listenerEntered = false;
  const listener = () => { listenerEntered = true; };
  cue.addEventListener('enter', listener, false);
  cue.dispatchEvent(new Event('enter'));
  cue.removeEventListener('enter', listener, false);
  listenerEntered = false;
  cue.dispatchEvent(new Event('enter'));
  const listenerRemoved = !listenerEntered;
  track.addCue(cue);
  track.addCue(later);
  track.addCue(emptyIdCue);
  later.startTime = 0.5;
  const order = track.cues[0] === later && track.cues[1] === cue;
  const emptyIdLookup = track.cues.getCueById('') === null;
  return [
    idAndPause,
    pauseAfterNull,
    startNaN,
    endPositiveInfinity,
    cue.endTime,
    endNegativeInfinity,
    positionNumber,
    positionAuto,
    entered,
    onenterUndefined,
    listenerRemoved,
    order,
    emptyIdLookup
  ].join('|');
})()
"#,
        )
        .expect("VTTCue accessors should evaluate");

    assert_eq!(
        result,
        "null:true|false|throw:TypeError|throw:TypeError|2|throw:TypeError|25|auto|true|true|true|true|true"
    );
}
#[test]
fn text_track_id_and_cue_order_follow_wpt_edges() {
    let mut vm = new_storage_test_vm("https://text-track-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const video = document.createElement('video');
  const trackElement = document.createElement('track');
  trackElement.id = 'LoremIpsum';
  video.appendChild(trackElement);
  const elementTrack = trackElement.track;
  elementTrack.id = 'newvalue';
  const idReadonly = `${elementTrack.id}:${video.textTracks.getTrackById('LoremIpsum') === elementTrack}`;

  const concat = cues => Array.prototype.reduce.call(cues, (acc, cue) => acc + cue.text, '');
  const track = video.addTextTrack('subtitles');
  track.addCue(new VTTCue(2, 5, '1'));
  track.addCue(new VTTCue(2, 5, '2'));
  track.addCue(new VTTCue(2, 5, '3'));
  const initial = concat(track.cues);
  track.cues[0].startTime = 4;
  const movedLast = concat(track.cues);
  track.cues[2].startTime = 2;
  const restoredInsertionOrder = concat(track.cues);
  track.cues[2].endTime = 9;
  const endMovedFirst = concat(track.cues);
  return [idReadonly, initial, movedLast, restoredInsertionOrder, endMovedFirst].join('|');
})()
"#,
        )
        .expect("TextTrack id and cue order probe should evaluate");

    assert_eq!(result, "LoremIpsum:true|123|231|123|312");
}
#[test]
fn media_source_constructor_preserves_prototype_chain_and_static_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(MediaSource, "isTypeSupported");
              return JSON.stringify({
              ctorType: typeof MediaSource,
              staticType: typeof MediaSource.isTypeSupported,
              staticName: MediaSource.isTypeSupported.name,
              staticLength: MediaSource.isTypeSupported.length,
              staticDescriptor: [
                !!descriptor,
                descriptor && descriptor.enumerable,
                descriptor && descriptor.writable,
                descriptor && descriptor.configurable
              ],
              keysContainStatic: Object.keys(MediaSource).includes("isTypeSupported"),
              instanceOf: (() => {
                const mediaSource = new MediaSource();
                return mediaSource instanceof MediaSource;
              })(),
              prototypeMatches: (() => {
                const mediaSource = new MediaSource();
                return Object.getPrototypeOf(mediaSource) === MediaSource.prototype;
              })(),
              constructorName: (() => {
                const mediaSource = new MediaSource();
                return mediaSource.constructor && mediaSource.constructor.name;
              })(),
              ownKeys: (() => Object.keys(new MediaSource()))()
            });
            })()
            "#,
        )
        .expect("MediaSource constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","staticType":"function","staticName":"isTypeSupported","staticLength":1,"staticDescriptor":[true,true,true,true],"keysContainStatic":true,"instanceOf":true,"prototypeMatches":true,"constructorName":"MediaSource","ownKeys":[]}"#
    );
}
#[test]
fn media_source_is_type_supported_applies_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://media-source-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => MediaSource.isTypeSupported()));
  values.push(probe(() => MediaSource.isTypeSupported(Symbol('type'))));
  values.push(probe(() => MediaSource.isTypeSupported({ toString() { throw new RangeError('boom'); } })));
  values.push(probe(() => MediaSource.isTypeSupported(null)));

  let calls = 0;
  values.push(probe(() => MediaSource.isTypeSupported({
    toString() {
      calls += 1;
      return 'video/mp4; codecs="avc1.42E01E"';
    },
  })));
  values.push(String(calls));
  return values.join('|');
})()
"#,
        )
        .expect("MediaSource.isTypeSupported WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:RangeError|false|true|1"
    );
}
#[test]
fn zhihu_probe_media_devices_surface_exposes_promise_methods() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
            (() => {
              globalThis.__mediaDevicesProbe = "pending";
              const devices = navigator.mediaDevices;
              const proto = Object.getPrototypeOf(devices);
              const fakeDevices = Object.create(MediaDevices.prototype);
              const summarizeMethodDescriptor = name => {
                const descriptor = Object.getOwnPropertyDescriptor(devices, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const methodOutcome = async (method, receiver, ...args) => {
                let value;
                try {
                  value = devices[method].call(receiver, ...args);
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
              const enumeratePromise = devices.enumerateDevices();
              const userMediaPromise = devices.getUserMedia({ audio: true }).catch(() => null);
              return Promise.all([
                methodOutcome("enumerateDevices", fakeDevices),
                methodOutcome("getUserMedia", fakeDevices, { audio: true })
              ]).then((fakeOutcomes) => {
                globalThis.__mediaDevicesProbe = JSON.stringify({
                  ctorType: typeof MediaDevices,
                  ctorName: devices.constructor && devices.constructor.name,
                  tag: Object.prototype.toString.call(devices),
                  protoCtor: proto && proto.constructor && proto.constructor.name,
                  enumerateDevicesType: typeof devices.enumerateDevices,
                  enumerateDevicesName: devices.enumerateDevices && devices.enumerateDevices.name,
                  enumerateDevicesLength: devices.enumerateDevices && devices.enumerateDevices.length,
                  enumerateDevicesDescriptor: summarizeMethodDescriptor("enumerateDevices"),
                  enumerateDevicesPromiseTag: Object.prototype.toString.call(enumeratePromise),
                  enumerateDevicesThenType: typeof enumeratePromise.then,
                  getUserMediaType: typeof devices.getUserMedia,
                  getUserMediaName: devices.getUserMedia && devices.getUserMedia.name,
                  getUserMediaLength: devices.getUserMedia && devices.getUserMedia.length,
                  getUserMediaDescriptor: summarizeMethodDescriptor("getUserMedia"),
                  getUserMediaPromiseTag: Object.prototype.toString.call(userMediaPromise),
                  getUserMediaThenType: typeof userMediaPromise.then,
                  fakeOutcomes: fakeOutcomes.join("|")
                });
              });
            })()
            "#,
        None,
    )
    .expect("mediaDevices probe should schedule");

    let result = vm
        .eval("String(globalThis.__mediaDevicesProbe)")
        .expect("mediaDevices probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","ctorName":"MediaDevices","tag":"[object MediaDevices]","protoCtor":"MediaDevices","enumerateDevicesType":"function","enumerateDevicesName":"enumerateDevices","enumerateDevicesLength":0,"enumerateDevicesDescriptor":"true:function:enumerateDevices:0:true:true:true","enumerateDevicesPromiseTag":"[object Promise]","enumerateDevicesThenType":"function","getUserMediaType":"function","getUserMediaName":"getUserMedia","getUserMediaLength":1,"getUserMediaDescriptor":"true:function:getUserMedia:1:true:true:true","getUserMediaPromiseTag":"[object Promise]","getUserMediaThenType":"function","fakeOutcomes":"rejected:true:TypeError|rejected:true:TypeError"}"#
    );
}
#[test]
fn zhihu_probe_offline_audio_context_supports_fingerprintjs2_audio_flow() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        (() => {
          const Ctor = window.OfflineAudioContext || window.webkitOfflineAudioContext;
          if (!Ctor) {
            globalThis.__audioProbe = { available: false };
            return;
          }
          const ctx = new Ctor(1, 44100, 44100);
          const osc = ctx.createOscillator();
          osc.type = "triangle";
          osc.frequency.setValueAtTime(10000, ctx.currentTime);
          const comp = ctx.createDynamicsCompressor();
          for (const [name, value] of [["threshold", -50], ["knee", 40], ["ratio", 12], ["reduction", -20], ["attack", 0], ["release", 0.25]]) {
            if (comp[name] !== undefined && typeof comp[name].setValueAtTime === "function") {
              comp[name].setValueAtTime(value, ctx.currentTime);
            }
          }
          osc.connect(comp);
          comp.connect(ctx.destination);
          osc.start(0);
          globalThis.__audioProbe = {
            available: true,
            offlineType: typeof window.OfflineAudioContext,
            webkitType: typeof window.webkitOfflineAudioContext,
            sharedCtor: window.OfflineAudioContext === window.webkitOfflineAudioContext,
            done: false,
            sum: null,
            promiseTag: null,
            ctxCtor: ctx.constructor && ctx.constructor.name,
            oscCtor: osc.constructor && osc.constructor.name,
            compCtor: comp.constructor && comp.constructor.name,
            destinationCtor: ctx.destination && ctx.destination.constructor && ctx.destination.constructor.name,
            reductionType: typeof comp.reduction,
            frequencyValue: osc.frequency && osc.frequency.value,
            thresholdValue: comp.threshold && comp.threshold.value,
            length: ctx.length,
            sampleRate: ctx.sampleRate,
            stateBefore: ctx.state,
          };
          ctx.oncomplete = (event) => {
            const data = event.renderedBuffer.getChannelData(0);
            globalThis.__audioProbe.done = true;
            globalThis.__audioProbe.sum = data.slice(4500, 5000).reduce((acc, value) => acc + Math.abs(value), 0);
            globalThis.__audioProbe.bufferCtor = event.renderedBuffer.constructor && event.renderedBuffer.constructor.name;
            globalThis.__audioProbe.stateAfter = ctx.state;
          };
          const rendering = ctx.startRendering();
          globalThis.__audioProbe.promiseTag = Object.prototype.toString.call(rendering);
        })()
        "#,
        None,
    )
    .expect("offline audio probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__audioProbe)")
        .expect("offline audio probe should be readable");

    let value: serde_json::Value =
        serde_json::from_str(&result).expect("offline audio probe should return valid json");
    assert_eq!(value["available"], true);
    assert_eq!(value["offlineType"], "function");
    assert_eq!(value["webkitType"], "undefined");
    assert_eq!(value["sharedCtor"], false);
    assert_eq!(value["done"], true);
    assert_eq!(value["promiseTag"], "[object Promise]");
    assert_eq!(value["ctxCtor"], "OfflineAudioContext");
    assert_eq!(value["oscCtor"], "OscillatorNode");
    assert_eq!(value["compCtor"], "DynamicsCompressorNode");
    assert_eq!(value["destinationCtor"], "AudioDestinationNode");
    assert_eq!(value["bufferCtor"], "AudioBuffer");
    assert_eq!(value["reductionType"], "number");
    assert_eq!(value["length"], 44100);
    assert_eq!(value["sampleRate"], 44100.0);
    assert_eq!(value["stateBefore"], "suspended");
    assert_eq!(value["stateAfter"], "closed");
    let sum = value["sum"]
        .as_f64()
        .expect("offline audio sum should be numeric");
    assert!(
        (sum - 124.04347527516074).abs() < 0.001,
        "unexpected offline audio fingerprint sum: {sum}"
    );
}

#[test]
fn offline_audio_context_short_buffers_expose_nonzero_samples() {
    let mut vm = new_storage_test_vm("https://short-audio-fingerprint.test/");

    vm.exec(
        r#"
        globalThis.__shortAudioProbe = { done: false };
        const ctx = new OfflineAudioContext(1, 500, 44100);
        ctx.oncomplete = event => {
          const data = event.renderedBuffer.getChannelData(0);
          const nonzero = Array.from(data).filter(value => value !== 0);
          globalThis.__shortAudioProbe = {
            done: true,
            length: data.length,
            nonzero: nonzero.length,
            sum: nonzero.reduce((acc, value) => acc + Math.abs(value))
          };
        };
        ctx.startRendering();
        "#,
        None,
    )
    .expect("short offline audio probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__shortAudioProbe)")
        .expect("short offline audio probe should be readable");
    let value: serde_json::Value =
        serde_json::from_str(&result).expect("short offline audio probe should return valid json");
    assert_eq!(value["done"], true);
    assert_eq!(value["length"], 500);
    assert!(
        value["nonzero"].as_u64().unwrap_or_default() > 0,
        "short offline audio probe should expose non-zero samples: {result}"
    );
    assert!(
        value["sum"].as_f64().unwrap_or_default() > 0.0,
        "short offline audio probe should produce a positive sample sum: {result}"
    );
}

#[test]
fn offline_audio_context_updates_compressor_reduction_on_complete() {
    let mut vm = new_storage_test_vm("https://compressor-reduction.test/");

    vm.exec(
        r#"
        const ctx = new OfflineAudioContext(1, 500, 44100);
        const osc = ctx.createOscillator();
        const comp = ctx.createDynamicsCompressor();
        osc.connect(comp);
        comp.connect(ctx.destination);
        globalThis.__compressorReductionProbe = {
          before: comp.reduction,
          afterStart: null,
          oncomplete: null,
          afterPromise: null
        };
        ctx.oncomplete = () => {
          globalThis.__compressorReductionProbe.oncomplete = comp.reduction;
        };
        const rendering = ctx.startRendering();
        globalThis.__compressorReductionProbe.afterStart = comp.reduction;
        rendering.then(() => {
          globalThis.__compressorReductionProbe.afterPromise = comp.reduction;
        });
        "#,
        None,
    )
    .expect("compressor reduction timing probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__compressorReductionProbe)")
        .expect("compressor reduction timing probe should be readable");
    let value: serde_json::Value =
        serde_json::from_str(&result).expect("compressor reduction probe should return valid json");
    assert_eq!(value["before"], 0);
    assert_eq!(value["afterStart"], 0);
    assert!(
        value["oncomplete"].as_f64().unwrap_or_default() < 0.0,
        "compressor reduction should update before oncomplete: {result}"
    );
    assert!(
        value["afterPromise"].as_f64().unwrap_or_default() < 0.0,
        "compressor reduction should stay updated after rendering promise settles: {result}"
    );
}

#[test]
fn zhihu_probe_offline_audio_oncomplete_runs_after_start_rendering_returns() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        globalThis.__audioPhases = [];
        const Ctor = window.OfflineAudioContext || window.webkitOfflineAudioContext;
        const ctx = new Ctor(1, 44100, 44100);
        const osc = ctx.createOscillator();
        const comp = ctx.createDynamicsCompressor();
        osc.connect(comp);
        comp.connect(ctx.destination);
        ctx.startRendering();
        globalThis.__audioPhases.push("after-start");
        ctx.oncomplete = () => globalThis.__audioPhases.push("complete");
        globalThis.__audioPhases.push("after-handler");
        "#,
        None,
    )
    .expect("offline audio async ordering probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__audioPhases)")
        .expect("offline audio phases should be readable");
    assert_eq!(result, r#"["after-start","after-handler","complete"]"#);
}

#[test]
fn web_audio_declared_objects_keep_brands_and_hidden_properties() {
    let mut vm = new_storage_test_vm("https://audio-declared-objects.test/");

    vm.exec(
        r#"
(() => {
  const ctx = new OfflineAudioContext(1, 32, 8000);
  const osc = ctx.createOscillator();
  const comp = ctx.createDynamicsCompressor();
  globalThis.__webAudioDeclaredProbe = { completion: null };
  ctx.oncomplete = event => {
    globalThis.__webAudioDeclaredProbe.completion = {
      tag: Object.prototype.toString.call(event),
      ctor: event.constructor && event.constructor.name,
      type: event.type,
      renderedBufferTag: Object.prototype.toString.call(event.renderedBuffer),
      renderedBufferCtor: event.renderedBuffer.constructor && event.renderedBuffer.constructor.name,
      renderedBufferLength: event.renderedBuffer.length,
      renderedBufferSampleRate: event.renderedBuffer.sampleRate,
      renderedBufferDuration: event.renderedBuffer.duration,
      renderedBufferEnumerable: Object.prototype.propertyIsEnumerable.call(event, "renderedBuffer"),
      targetIsContext: event.target === ctx,
      currentTargetIsContext: event.currentTarget === ctx
    };
  };
  const rendering = ctx.startRendering();
  osc.frequency.setValueAtTime(123, 0);
  Object.assign(globalThis.__webAudioDeclaredProbe, {
    ctxTag: Object.prototype.toString.call(ctx),
    ctxCtor: ctx.constructor && ctx.constructor.name,
    ctxLength: ctx.length,
    ctxSampleRate: ctx.sampleRate,
    ctxState: ctx.state,
    lengthEnumerable: Object.prototype.propertyIsEnumerable.call(ctx, "length"),
    destinationTag: Object.prototype.toString.call(ctx.destination),
    destinationCtor: ctx.destination.constructor && ctx.destination.constructor.name,
    destinationKeys: Object.keys(ctx.destination).join(","),
    oscTag: Object.prototype.toString.call(osc),
    oscCtor: osc.constructor && osc.constructor.name,
    oscKeys: Object.keys(osc).join(","),
    oscType: osc.type,
    frequencyTag: Object.prototype.toString.call(osc.frequency),
    frequencyCtor: osc.frequency.constructor && osc.frequency.constructor.name,
    frequencyValue: osc.frequency.value,
    frequencyEnumerable: Object.prototype.propertyIsEnumerable.call(osc, "frequency"),
    setValueAtTimeType: typeof osc.frequency.setValueAtTime,
    compTag: Object.prototype.toString.call(comp),
    compCtor: comp.constructor && comp.constructor.name,
    compKeys: Object.keys(comp).join(","),
    thresholdTag: Object.prototype.toString.call(comp.threshold),
    reduction: comp.reduction,
    reductionOwn: Object.prototype.hasOwnProperty.call(comp, "reduction"),
    promiseTag: Object.prototype.toString.call(rendering)
  });
})()
"#,
        None,
    )
    .expect("web audio declared object probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__webAudioDeclaredProbe)")
        .expect("web audio declared object probe should evaluate");

    assert_eq!(
        result,
        r#"{"completion":{"tag":"[object Object]","ctor":"Object","type":"complete","renderedBufferTag":"[object AudioBuffer]","renderedBufferCtor":"AudioBuffer","renderedBufferLength":32,"renderedBufferSampleRate":8000,"renderedBufferDuration":0.004,"renderedBufferEnumerable":false,"targetIsContext":true,"currentTargetIsContext":true},"ctxTag":"[object OfflineAudioContext]","ctxCtor":"OfflineAudioContext","ctxLength":32,"ctxSampleRate":8000,"ctxState":"closed","lengthEnumerable":false,"destinationTag":"[object AudioDestinationNode]","destinationCtor":"AudioDestinationNode","destinationKeys":"","oscTag":"[object OscillatorNode]","oscCtor":"OscillatorNode","oscKeys":"","oscType":"sine","frequencyTag":"[object AudioParam]","frequencyCtor":"AudioParam","frequencyValue":123,"frequencyEnumerable":false,"setValueAtTimeType":"function","compTag":"[object DynamicsCompressorNode]","compCtor":"DynamicsCompressorNode","compKeys":"","thresholdTag":"[object AudioParam]","reduction":0,"reductionOwn":false,"promiseTag":"[object Promise]"}"#
    );
}

#[test]
fn web_audio_declared_fixed_own_methods_keep_descriptors() {
    let mut vm = new_storage_test_vm("https://audio-declared-own-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ctx = new OfflineAudioContext(1, 32, 8000);
  const osc = ctx.createOscillator();
  const comp = ctx.createDynamicsCompressor();
  const analyser = ctx.createAnalyser();
  const describe = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    if (!descriptor) {
      return `${name}:missing`;
    }
    return [
      name,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable,
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length
    ].join(":");
  };

  return JSON.stringify({
    ctxKeys: Object.keys(ctx).join(","),
    oscKeys: Object.keys(osc).join(","),
    compKeys: Object.keys(comp).join(","),
    analyserKeys: Object.keys(analyser).join(","),
    paramKeys: Object.keys(osc.frequency).join(","),
    ctx: ["addEventListener", "removeEventListener", "dispatchEvent"].map(name => describe(ctx, name)),
    osc: ["connect", "disconnect", "start"].map(name => describe(osc, name)),
    comp: ["connect", "disconnect"].map(name => describe(comp, name)),
    analyser: [
      "connect",
      "disconnect",
      "getFloatFrequencyData",
      "getFloatTimeDomainData",
      "getByteFrequencyData",
      "getByteTimeDomainData"
    ].map(name => describe(analyser, name)),
    param: describe(osc.frequency, "setValueAtTime")
  });
})()
"#,
        )
        .expect("web audio declared own method descriptors should evaluate");

    assert_eq!(
        result,
        r#"{"ctxKeys":"addEventListener,removeEventListener,dispatchEvent","oscKeys":"","compKeys":"","analyserKeys":"","paramKeys":"","ctx":["addEventListener:true:true:true:function:addEventListener:0","removeEventListener:true:true:true:function:removeEventListener:0","dispatchEvent:true:true:true:function:dispatchEvent:0"],"osc":["connect:false:true:true:function:connect:1","disconnect:false:true:true:function:disconnect:0","start:false:true:true:function:start:1"],"comp":["connect:false:true:true:function:connect:1","disconnect:false:true:true:function:disconnect:0"],"analyser":["connect:false:true:true:function:connect:1","disconnect:false:true:true:function:disconnect:0","getFloatFrequencyData:false:true:true:function:getFloatFrequencyData:1","getFloatTimeDomainData:false:true:true:function:getFloatTimeDomainData:1","getByteFrequencyData:false:true:true:function:getByteFrequencyData:1","getByteTimeDomainData:false:true:true:function:getByteTimeDomainData:1"],"param":"setValueAtTime:false:true:true:function:setValueAtTime:2"}"#
    );
}

#[test]
fn web_audio_private_backing_slots_ignore_public_spoofing() {
    let mut vm = new_storage_test_vm("https://audio-private-backing-slots.test/");

    vm.exec(
        r#"
(() => {
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith("__moliOfflineAudio") || name === "__moliDynamicsCompressorReduction")
    .sort()
    .join(",");
  const ctx = new OfflineAudioContext(1, 32, 8000);
  const comp = ctx.createDynamicsCompressor();
  const ctxOwnInternalBefore = internalNames(ctx);
  const compOwnInternalBefore = internalNames(comp);
  ctx.__moliOfflineAudioLength = 999;
  ctx.__moliOfflineAudioSampleRate = 123;
  ctx.__moliOfflineAudioChannelCount = 7;
  ctx.__moliOfflineAudioCompressors = [];
  Object.prototype.__moliOfflineAudioCompleteContext = { spoof: "prototype-context" };
  Object.prototype.__moliOfflineAudioCompleteBuffer = { spoof: "prototype-buffer" };
  ctx.__moliOfflineAudioCompleteContext = { spoof: "context" };
  ctx.__moliOfflineAudioCompleteBuffer = { spoof: "buffer" };
  comp.__moliDynamicsCompressorReduction = 99;
  globalThis.__webAudioPrivateProbe = {
    ctxOwnInternalBefore,
    compOwnInternalBefore,
    ctxOwnInternalAfterSpoof: internalNames(ctx),
    reductionBefore: comp.reduction,
    complete: null
  };
  ctx.oncomplete = event => {
    const buffer = event.renderedBuffer;
    const bufferOwnInternalBefore = internalNames(buffer);
    buffer.__moliOfflineAudioBuffer = { length: 1, 0: 999 };
    const data = buffer.getChannelData(0);
    globalThis.__webAudioPrivateProbe.complete = {
      bufferOwnInternalBefore,
      bufferOwnInternalAfterSpoof: internalNames(buffer),
      targetStable: event.target === ctx,
      currentTargetStable: event.currentTarget === ctx,
      renderedBufferStable: buffer instanceof AudioBuffer,
      bufferLength: buffer.length,
      bufferSampleRate: buffer.sampleRate,
      dataTag: Object.prototype.toString.call(data),
      dataLength: data.length,
      reductionAfter: comp.reduction
    };
  };
  ctx.startRendering();
})()
"#,
        None,
    )
    .expect("web audio private backing slot probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__webAudioPrivateProbe)")
        .expect("web audio private backing slot probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctxOwnInternalBefore":"","compOwnInternalBefore":"","ctxOwnInternalAfterSpoof":"__moliOfflineAudioChannelCount,__moliOfflineAudioCompleteBuffer,__moliOfflineAudioCompleteContext,__moliOfflineAudioCompressors,__moliOfflineAudioLength,__moliOfflineAudioSampleRate","reductionBefore":0,"complete":{"bufferOwnInternalBefore":"","bufferOwnInternalAfterSpoof":"__moliOfflineAudioBuffer","targetStable":true,"currentTargetStable":true,"renderedBufferStable":true,"bufferLength":32,"bufferSampleRate":8000,"dataTag":"[object Float32Array]","dataLength":32,"reductionAfter":-82.26815795898438}}"#
    );
}

#[test]
fn offline_audio_context_analyser_supports_probe_data_methods() {
    let mut vm = new_storage_test_vm("https://audio-analyser-probe.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ctx = new OfflineAudioContext(1, 64, 44100);
  const osc = ctx.createOscillator();
  const analyser = ctx.createAnalyser();
  osc.connect(analyser);
  analyser.connect(ctx.destination);

  const floats = new Float32Array(4);
  const time = new Float32Array(4);
  const bytes = new Uint8Array(4);
  const byteTime = new Uint8Array(4);
  analyser.getFloatFrequencyData(floats);
  analyser.getFloatTimeDomainData(time);
  analyser.getByteFrequencyData(bytes);
  analyser.getByteTimeDomainData(byteTime);

  return JSON.stringify({
    ctorType: typeof AnalyserNode,
    tag: Object.prototype.toString.call(analyser),
    ctor: analyser.constructor && analyser.constructor.name,
    fftSize: analyser.fftSize,
    frequencyBinCount: analyser.frequencyBinCount,
    minDecibels: analyser.minDecibels,
    maxDecibels: analyser.maxDecibels,
    smoothingTimeConstant: analyser.smoothingTimeConstant,
    connectResultCtor: analyser.connect(ctx.destination).constructor.name,
    methods: [
      typeof ctx.createAnalyser,
      typeof analyser.getFloatFrequencyData,
      typeof analyser.getByteFrequencyData,
      typeof analyser.getFloatTimeDomainData,
      typeof analyser.getByteTimeDomainData
    ],
    floats: Array.from(floats),
    time: Array.from(time),
    bytes: Array.from(bytes),
    byteTime: Array.from(byteTime)
  });
})()
"#,
        )
        .expect("offline audio analyser probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctorType":"function","tag":"[object AnalyserNode]","ctor":"AnalyserNode","fftSize":2048,"frequencyBinCount":1024,"minDecibels":-100,"maxDecibels":-30,"smoothingTimeConstant":0.8,"connectResultCtor":"AudioDestinationNode","methods":["function","function","function","function","function"],"floats":[-90.25955200195312,-90.22233581542969,-90.11856842041016,-89.96821594238281],"time":[0,0,0,0],"bytes":[0,0,0,0],"byteTime":[128,128,128,128]}"#
    );
}

#[test]
fn audio_context_exposes_realtime_worklet_surface() {
    let mut vm = new_storage_test_vm("https://audio-worklet.test/");

    let result = vm
        .eval(
            r#"
        (() => {
          const context = new AudioContext();
          return JSON.stringify({
            contextType: typeof AudioContext,
            webkitSame: webkitAudioContext === AudioContext,
            contextTag: Object.prototype.toString.call(context),
            state: context.state,
            sampleRate: context.sampleRate,
            destinationTag: Object.prototype.toString.call(context.destination),
            workletTag: Object.prototype.toString.call(context.audioWorklet),
            addModuleType: typeof context.audioWorklet.addModule,
            nodeType: typeof AudioWorkletNode
          });
        })()
        "#,
        )
        .expect("audio worklet surface should evaluate");
    assert_eq!(
        result,
        r#"{"contextType":"function","webkitSame":true,"contextTag":"[object AudioContext]","state":"running","sampleRate":44100,"destinationTag":"[object AudioDestinationNode]","workletTag":"[object AudioWorklet]","addModuleType":"function","nodeType":"function"}"#
    );
}

#[test]
fn web_audio_internal_maps_and_errors_ignore_public_primordial_overrides() {
    let mut vm = new_storage_test_vm("https://audio-primordial-overrides.test/");

    vm.exec(
        r#"
(() => {
  const IntrinsicMap = Map;
  const IntrinsicError = Error;
  const IntrinsicTypeError = TypeError;
  globalThis.Map = function() {
    throw new IntrinsicError("WebAudio must not construct the public Map");
  };
  globalThis.Error = function() {
    throw new IntrinsicError("WebAudio must not construct the public Error");
  };
  globalThis.TypeError = function() {
    throw new IntrinsicError("WebAudio must not construct the public TypeError");
  };

  const context = new AudioContext();
  const rejected = context.audioWorklet.addModule.call({}, "data:text/javascript,");
  const closed = context.close();
  globalThis.__webAudioPrimordialProbe = {
    contextTag: Object.prototype.toString.call(context),
    closedTag: Object.prototype.toString.call(closed),
    rejection: "pending"
  };
  rejected.catch(error => {
    globalThis.__webAudioPrimordialProbe.rejection =
      `${error.name}:${error instanceof IntrinsicTypeError}`;
  });

  globalThis.Map = IntrinsicMap;
  globalThis.Error = IntrinsicError;
  globalThis.TypeError = IntrinsicTypeError;
})()
"#,
        None,
    )
    .expect("WebAudio primordial override probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__webAudioPrimordialProbe)")
        .expect("WebAudio primordial override probe should evaluate");
    assert_eq!(
        result,
        r#"{"contextTag":"[object AudioContext]","closedTag":"[object Promise]","rejection":"TypeError:true"}"#
    );
}

#[test]
fn navigator_get_autoplay_policy_accepts_declared_overloads() {
    let mut vm = new_storage_test_vm("https://autoplay-policy.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  };
  const audioContext = new AudioContext();
  return [
    typeof navigator.getAutoplayPolicy,
    navigator.getAutoplayPolicy.length,
    navigator.getAutoplayPolicy("mediaelement"),
    navigator.getAutoplayPolicy("audiocontext"),
    navigator.getAutoplayPolicy(document.createElement("audio")),
    navigator.getAutoplayPolicy(document.createElement("video")),
    navigator.getAutoplayPolicy(audioContext),
    errorName(() => navigator.getAutoplayPolicy()),
    errorName(() => navigator.getAutoplayPolicy("invalid")),
    errorName(() => navigator.getAutoplayPolicy({})),
    errorName(() => navigator.getAutoplayPolicy(Object.create(AudioContext.prototype))),
    errorName(() => navigator.getAutoplayPolicy.call({}, "mediaelement"))
  ].join("|");
})()
"#,
        )
        .expect("navigator autoplay policy probe should evaluate");

    assert_eq!(
        result,
        "function|1|allowed|allowed|allowed|allowed|allowed|TypeError|TypeError|TypeError|TypeError|TypeError"
    );
}

#[test]
fn audio_worklet_context_store_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://audio-worklet-private-context.test/");

    vm.exec(
        r#"
(() => {
  const context = new AudioContext();
  const worklet = context.audioWorklet;
  const internal = "__moliContext";
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === internal)
    .join(",");
  const initialNames = internalNames(worklet);
  Object.getPrototypeOf(worklet)[internal] = { spoof: "prototype" };
  worklet[internal] = { spoof: "own" };
  const fake = Object.create(worklet);
  const fakeResult = worklet.addModule.call(fake, "data:text/javascript,");
  globalThis.__audioWorkletContextProbe = {
    initialNames,
    spoofedNames: internalNames(worklet),
    publicSpoof: worklet[internal] && worklet[internal].spoof,
    fakePromiseTag: Object.prototype.toString.call(fakeResult),
    fakeOutcome: "pending"
  };
  fakeResult.then(
    () => { globalThis.__audioWorkletContextProbe.fakeOutcome = "resolved"; },
    error => { globalThis.__audioWorkletContextProbe.fakeOutcome = error && error.name; }
  );
})()
"#,
        None,
    )
    .expect("audio worklet private context probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__audioWorkletContextProbe)")
        .expect("audio worklet private context probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialNames":"","spoofedNames":"__moliContext","publicSpoof":"own","fakePromiseTag":"[object Promise]","fakeOutcome":"TypeError"}"#
    );
}

#[test]
fn web_audio_numeric_entrypoints_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        (() => {
          const results = {};
          function record(name, callback) {
            try {
              callback();
              results[name] = "no throw";
            } catch (error) {
              results[name] = error && error.name;
            }
          }

          record("ctorMissing", () => new OfflineAudioContext(1, 16));
          record("ctorSymbol", () => new OfflineAudioContext(Symbol("channels"), 16, 44100));

          const ctx = new OfflineAudioContext("1.9", "16.9", "44100");
          results.length = ctx.length;
          results.sampleRate = ctx.sampleRate;

          const osc = ctx.createOscillator();
          record("setMissingTime", () => osc.frequency.setValueAtTime(1));
          record("setSymbol", () => osc.frequency.setValueAtTime(Symbol("value"), 0));
          osc.frequency.setValueAtTime("123.5", ctx.currentTime);
          results.frequency = osc.frequency.value;

          ctx.oncomplete = (event) => {
            const buffer = event.renderedBuffer;
            record("channelMissing", () => buffer.getChannelData());
            record("channelSymbol", () => buffer.getChannelData(Symbol("channel")));
            record("channelRange", () => buffer.getChannelData(1));
            results.channel0 = Object.prototype.toString.call(buffer.getChannelData("0.9"));
          };
          ctx.startRendering();
          globalThis.__audioWebIdlProbe = results;
        })()
        "#,
        None,
    )
    .expect("audio WebIDL argument probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__audioWebIdlProbe)")
        .expect("audio WebIDL argument probe should be readable");

    assert_eq!(
        result,
        r#"{"ctorMissing":"TypeError","ctorSymbol":"TypeError","length":16,"sampleRate":44100,"setMissingTime":"TypeError","setSymbol":"TypeError","frequency":123.5,"channelMissing":"TypeError","channelSymbol":"TypeError","channelRange":"RangeError","channel0":"[object Float32Array]"}"#
    );
}
#[test]
fn zhihu_probe_match_media_orientation_matches_chromium_headless_profile() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              landscape: matchMedia("(orientation: landscape)").matches,
              portrait: matchMedia("(orientation: portrait)").matches,
              screenLandscape: matchMedia("screen and (orientation: landscape)").matches,
              screenPortrait: matchMedia("screen and (orientation: portrait)").matches,
              mediaLandscape: matchMedia("(orientation: landscape)").media,
              mediaPortrait: matchMedia("(orientation: portrait)").media
            })
            "#,
        )
        .expect("orientation matchMedia probe should evaluate");

    assert_eq!(
        result,
        r#"{"landscape":true,"portrait":false,"screenLandscape":true,"screenPortrait":false,"mediaLandscape":"(orientation: landscape)","mediaPortrait":"(orientation: portrait)"}"#
    );
}
#[test]
fn match_media_uses_desktop_viewport_and_input_capabilities() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              minWidth: matchMedia("(min-width: 768px)").matches,
              exactWidth: matchMedia("(width: 1920px)").matches,
              maxWidth: matchMedia("(max-width: 768px)").matches,
              minHeight: matchMedia("(min-height: 720px)").matches,
              exactHeight: matchMedia("(height: 1080px)").matches,
              minDeviceWidth: matchMedia("(min-device-width: 1px)").matches,
              exactDeviceWidth: matchMedia("(device-width: 1920px)").matches,
              maxDeviceWidth: matchMedia("(max-device-width: 1px)").matches,
              minDeviceHeight: matchMedia("(min-device-height: 1px)").matches,
              exactDeviceHeight: matchMedia("(device-height: 1080px)").matches,
              pointer: matchMedia("(pointer)").matches,
              pointerFine: matchMedia("(pointer: fine)").matches,
              pointerCoarse: matchMedia("(pointer: coarse)").matches,
              hover: matchMedia("(hover)").matches,
              hoverHover: matchMedia("(hover: hover)").matches,
              hoverNone: matchMedia("(hover: none)").matches,
              anyPointerFine: matchMedia("(any-pointer: fine)").matches,
              anyHoverHover: matchMedia("(any-hover: hover)").matches
            })
            "#,
        )
        .expect("desktop matchMedia probe should evaluate");

    assert_eq!(
        result,
        r#"{"minWidth":true,"exactWidth":true,"maxWidth":false,"minHeight":true,"exactHeight":true,"minDeviceWidth":true,"exactDeviceWidth":true,"maxDeviceWidth":false,"minDeviceHeight":true,"exactDeviceHeight":true,"pointer":true,"pointerFine":true,"pointerCoarse":false,"hover":true,"hoverHover":true,"hoverNone":false,"anyPointerFine":true,"anyHoverHover":true}"#
    );
}

fn viewport_surface_800_600_on_1920_1080_screen() -> crate::protocol_types::ViewportSurface {
    crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,
    }
}

#[test]
fn match_media_uses_renderer_viewport_surface_for_viewport_and_screen_queries() {
    let mut vm = new_storage_test_vm("https://match-media-viewport-surface.test/");
    vm.set_viewport_surface(Some(viewport_surface_800_600_on_1920_1080_screen()))
        .expect("viewport surface should update");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              width: matchMedia("(width: 800px)").matches,
              notDefaultWidth: matchMedia("(width: 1920px)").matches,
              height: matchMedia("(height: 600px)").matches,
              deviceWidth: matchMedia("(device-width: 1920px)").matches,
              notViewportDeviceWidth: matchMedia("(device-width: 800px)").matches,
              deviceHeight: matchMedia("(device-height: 1080px)").matches,
              combined: matchMedia("(width: 800px) and (device-width: 1920px)").matches,
              wrongCombined: matchMedia("(width: 800px) and (device-width: 800px)").matches
            })
            "#,
        )
        .expect("viewport surface matchMedia probe should evaluate");

    assert_eq!(
        result,
        r#"{"width":true,"notDefaultWidth":false,"height":true,"deviceWidth":true,"notViewportDeviceWidth":false,"deviceHeight":true,"combined":true,"wrongCombined":false}"#
    );
}

#[test]
fn match_media_viewport_surface_change_dispatches_change_event() {
    let mut vm = new_storage_test_vm("https://match-media-viewport-change.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const mql = matchMedia("(width: 800px) and (device-width: 1920px)");
  globalThis.__viewportMql = mql;
  globalThis.__viewportMqlEvents = [];
  mql.addEventListener("change", event => {
    globalThis.__viewportMqlEvents.push({
      type: event.type,
      media: event.media,
      matches: event.matches,
      targetIsMql: event.target === mql,
      currentTargetIsMql: event.currentTarget === mql
    });
  });
  return `${mql.matches}|${mql.media}`;
})()
"#,
        )
        .expect("viewport matchMedia change setup should evaluate");

    assert_eq!(initial, "false|(width: 800px) and (device-width: 1920px)");

    vm.set_viewport_surface(Some(viewport_surface_800_600_on_1920_1080_screen()))
        .expect("viewport surface should update");

    let result = vm
        .eval(
            r#"
JSON.stringify({
  matches: globalThis.__viewportMql.matches,
  events: globalThis.__viewportMqlEvents
})
"#,
        )
        .expect("viewport matchMedia change result should evaluate");

    assert_eq!(
        result,
        r#"{"matches":true,"events":[{"type":"change","media":"(width: 800px) and (device-width: 1920px)","matches":true,"targetIsMql":true,"currentTargetIsMql":true}]}"#
    );
}

#[test]
fn match_media_change_event_uses_event_prototype_and_declared_properties() {
    let mut vm = new_storage_test_vm("https://match-media-change-event.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const mql = matchMedia("(prefers-color-scheme: dark)");
  globalThis.__mqlChangeEvents = [];
  mql.addEventListener("change", event => {
    globalThis.__mqlChangeEvents.push({
      tag: Object.prototype.toString.call(event),
      ctor: event.constructor && event.constructor.name,
      protoCtor: Object.getPrototypeOf(event)?.constructor?.name ?? null,
      keys: Object.keys(event).join(","),
      type: event.type,
      media: event.media,
      matches: event.matches,
      mediaEnumerable: Object.prototype.propertyIsEnumerable.call(event, "media"),
      matchesEnumerable: Object.prototype.propertyIsEnumerable.call(event, "matches"),
      targetIsMql: event.target === mql,
      currentTargetIsMql: event.currentTarget === mql,
      bubbles: event.bubbles,
      cancelable: event.cancelable
    });
  });
  return `${mql.matches}|${mql.media}`;
})()
"#,
        )
        .expect("matchMedia change event setup should evaluate");

    assert_eq!(initial, "false|(prefers-color-scheme: dark)");

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });

    let result = vm
        .eval("JSON.stringify(globalThis.__mqlChangeEvents)")
        .expect("matchMedia change event probe should evaluate");

    assert_eq!(
        result,
        r#"[{"tag":"[object Event]","ctor":"Event","protoCtor":"Event","keys":"type,target,srcElement,currentTarget,defaultPrevented,bubbles,cancelable,isTrusted,composed,eventPhase","type":"change","media":"(prefers-color-scheme: dark)","matches":true,"mediaEnumerable":false,"matchesEnumerable":false,"targetIsMql":true,"currentTargetIsMql":true,"bubbles":false,"cancelable":false}]"#
    );
}

#[test]
fn match_media_declared_slots_ignore_string_property_spoofing() {
    let mut vm = new_storage_test_vm("https://match-media-declared-slots.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const mql = matchMedia("(prefers-color-scheme: dark)");
  globalThis.__mqlSlotProbe = [];
  const ownSlots = Object.getOwnPropertyNames(mql)
    .filter(name => name.startsWith("__moliMediaQueryList"))
    .sort();
  function descriptorShape(name) {
    const descriptor = Object.getOwnPropertyDescriptor(MediaQueryList.prototype, name);
    return [
      typeof descriptor.get,
      descriptor.get.name,
      descriptor.get.length,
      typeof descriptor.set,
      descriptor.set ? descriptor.set.name : "",
      descriptor.set ? descriptor.set.length : -1,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(mql, name)
    ].join(":");
  }

  MediaQueryList.prototype.__moliMediaQueryListMedia = "(prefers-color-scheme: light)";
  MediaQueryList.prototype.__moliMediaQueryListMatches = true;
  MediaQueryList.prototype.__moliMediaQueryListOnchange = () => {
    __mqlSlotProbe.push("proto-onchange");
  };
  MediaQueryList.prototype.__moliMediaQueryListListeners = {
    change: [() => __mqlSlotProbe.push("proto-listener")]
  };
  Object.assign(mql, {
    __moliMediaQueryListMedia: "(prefers-color-scheme: light)",
    __moliMediaQueryListMatches: true,
    __moliMediaQueryListOnchange: () => {
      __mqlSlotProbe.push("own-onchange");
    },
    __moliMediaQueryListListeners: {
      change: [() => __mqlSlotProbe.push("own-listener")]
    }
  });

  const before = [
    mql.media,
    mql.matches,
    mql.onchange === null
  ].join("|");
  mql.onchange = () => {
    __mqlSlotProbe.push("stale-handler");
  };
  mql.onchange = event => {
    __mqlSlotProbe.push(`handler:${event.media}:${event.matches}:${event.target === mql}`);
  };
  mql.addEventListener("change", event => {
    __mqlSlotProbe.push(`listener:${event.media}:${event.matches}`);
  });
  mql.addEventListener("change", event => {
    __mqlSlotProbe.push(`once:${event.media}:${event.matches}`);
  }, { once: true });
  const removed = event => {
    __mqlSlotProbe.push(`removed:${event.media}:${event.matches}`);
  };
  mql.addEventListener("change", removed);
  mql.removeEventListener("change", removed);
  const afterSet = [
    typeof mql.onchange,
    mql.onchange === mql.__moliMediaQueryListOnchange
  ].join(":");

  return JSON.stringify({
    ownSlots,
    before,
    afterSet,
    descriptors: {
      media: descriptorShape("media"),
      matches: descriptorShape("matches"),
      onchange: descriptorShape("onchange")
    }
  });
})()
"#,
        )
        .expect("matchMedia declared slot spoofing setup should evaluate");

    assert_eq!(
        initial,
        r#"{"ownSlots":[],"before":"(prefers-color-scheme: dark)|false|true","afterSet":"function:false","descriptors":{"media":"function:get media:0:undefined::-1:true:true:false","matches":"function:get matches:0:undefined::-1:true:true:false","onchange":"function:get onchange:0:function:set onchange:1:true:true:false"}}"#
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });

    let result = vm
        .eval("JSON.stringify(globalThis.__mqlSlotProbe)")
        .expect("matchMedia declared slot spoofing events should evaluate");

    assert_eq!(
        result,
        r#"["handler:(prefers-color-scheme: dark):true:true","listener:(prefers-color-scheme: dark):true","once:(prefers-color-scheme: dark):true"]"#
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("light".to_owned()),
        ..Default::default()
    });

    let result = vm
        .eval("JSON.stringify(globalThis.__mqlSlotProbe)")
        .expect("matchMedia declared slot spoofing second event should evaluate");

    assert_eq!(
        result,
        r#"["handler:(prefers-color-scheme: dark):true:true","listener:(prefers-color-scheme: dark):true","once:(prefers-color-scheme: dark):true","handler:(prefers-color-scheme: dark):false:true","listener:(prefers-color-scheme: dark):false"]"#
    );
}

#[test]
fn media_query_list_listeners_use_event_listener_callback_interface_semantics() {
    let mut vm = new_storage_test_vm("https://mql-callback-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const mql = matchMedia("(min-width: 0px)");
  const calls = [];

  let operationGets = 0;
  const objectListener = {};
  Object.defineProperty(objectListener, "handleEvent", {
    configurable: true,
    get() {
      operationGets++;
      return function(event) {
        calls.push(
          `object:${this === objectListener}:${event.currentTarget === mql}:${window.event === event}`
        );
      };
    }
  });
  mql.addListener(objectListener);
  // The legacy and EventTarget APIs are aliases. A duplicate registration must
  // neither add a second call nor replace the original registration options.
  mql.addEventListener("change", objectListener, { once: true });

  let callableOperationGets = 0;
  function callable(event) {
    "use strict";
    calls.push(`callable:${this === mql}:${event.currentTarget === mql}`);
  }
  Object.defineProperty(callable, "handleEvent", {
    get() {
      callableOperationGets++;
      throw new Error("the callable branch must not resolve handleEvent");
    }
  });
  mql.addListener(callable);

  const removedBeforeVisit = () => calls.push("removed");
  const late = () => calls.push("late");
  const mutator = () => {
    calls.push("mutator");
    mql.removeListener(removedBeforeVisit);
    mql.addListener(late);
  };
  mql.addListener(mutator);
  mql.addListener(removedBeforeVisit);

  let onceCalls = 0;
  const once = () => {
    onceCalls++;
    // `once` is removed before callback entry, so re-registration is a fresh
    // listener and must survive this dispatch.
    mql.addEventListener("change", once, { once: true });
  };
  mql.addEventListener("change", once, { once: true });

  mql.dispatchEvent(new Event("change"));
  mql.dispatchEvent(new Event("change"));

  mql.removeEventListener("change", objectListener);
  mql.removeListener(callable);
  mql.dispatchEvent(new Event("change"));

  return JSON.stringify({
    calls,
    operationGets,
    callableOperationGets,
    onceCalls
  });
})()
"#,
        )
        .expect("MediaQueryList callback-interface semantics should evaluate");

    assert_eq!(
        result,
        r#"{"calls":["object:true:true:true","callable:true:true","mutator","object:true:true:true","callable:true:true","mutator","late","mutator","late"],"operationGets":2,"callableOperationGets":0,"onceCalls":3}"#
    );
}

#[test]
fn media_query_list_listener_uses_callback_realm_and_exact_window_lifetime() {
    let mut vm = new_parsed_test_vm(
        "https://mql-callback-realm.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__mqlCallbackRealmFrame = iframe;
})()
"#,
    )
    .expect("cross-Realm MediaQueryList listener setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = globalThis.__mqlCallbackRealmFrame;
  const other = iframe.contentWindow;
  const mql = matchMedia("(min-width: 0px)");
  globalThis.__mqlCallbackRealmTarget = mql;
  globalThis.__mqlCallbackExpectedRealm = other;
  globalThis.__mqlCallbackRealmFacts = [];

  const callback = other.Function(
    "event",
    `"use strict";
     parent.__mqlCallbackRealmFacts.push([
       this === parent.__mqlCallbackRealmTarget,
       globalThis === parent.__mqlCallbackExpectedRealm,
       window.event === event,
       event.currentTarget === parent.__mqlCallbackRealmTarget
     ]);`
  );
  mql.addListener(callback);
  mql.dispatchEvent(new Event("change"));

  let reported = null;
  const missingOperation = new other.Object();
  const onError = event => {
    reported = {
      relevantTypeError:
        event.error instanceof other.TypeError &&
        !(event.error instanceof TypeError),
      targetIsCallbackWindow: event.currentTarget === other
    };
    event.preventDefault();
  };
  other.addEventListener("error", onError);
  mql.addListener(missingOperation);
  mql.dispatchEvent(new Event("change"));
  other.removeEventListener("error", onError);

  iframe.remove();
  mql.dispatchEvent(new Event("change"));

  return JSON.stringify({
    facts: globalThis.__mqlCallbackRealmFacts,
    reported,
    childDetached: iframe.contentWindow === null
  });
})()
"#,
        )
        .expect("cross-Realm MediaQueryList listener invocation should evaluate");

    assert_eq!(
        result,
        r#"{"facts":[[true,true,true,true],[true,true,true,true]],"reported":{"relevantTypeError":true,"targetIsCallbackWindow":true},"childDetached":true}"#
    );
}
