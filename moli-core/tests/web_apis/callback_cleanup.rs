use super::event_dispatch::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn posted_message_callbacks_checkpoint_before_the_next_listener() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = r#"
      const log = [];
      let dispatched;
      addEventListener('nested', () => {
        log.push('nested 1');
        Promise.resolve().then(() => log.push('nested 1 microtask'));
      });
      addEventListener('nested', () => {
        log.push('nested 2');
        Promise.resolve().then(() => log.push('nested 2 microtask'));
      });
      addEventListener('message', event => {
        dispatched = event;
        log.push('first');
        Promise.resolve().then(() => {
          log.push(['first microtask', event.currentTarget === self, event.eventPhase === 2,
            typeof document === 'undefined' || window.event === event]);
          dispatchEvent(new Event('nested'));
          log.push('after nested');
        });
      });
      addEventListener('message', event => {
        log.push('second');
        Promise.resolve().then(() => {
          event.stopImmediatePropagation();
          log.push('second microtask');
          setTimeout(() => {
            log.push(['after dispatch', dispatched.currentTarget === null, dispatched.eventPhase === 0,
              typeof document === 'undefined' || window.event === undefined]);
            finish(log);
          }, 0);
        });
      });
      addEventListener('message', () => log.push('unexpected third'));
      if (typeof document !== 'undefined') postMessage('go', '*');
    "#;
    let mut results = Vec::new();
    for target in ["window", "child", "worker"] {
        results.push((target, run_probe(&browser, &server, target, source).await?));
    }
    server.shutdown().await;
    for (target, observed) in results {
        assert_eq!(
            observed,
            serde_json::json!([
                "first",
                ["first microtask", true, true, true],
                "nested 1",
                "nested 2",
                "after nested",
                "nested 1 microtask",
                "nested 2 microtask",
                "second",
                "second microtask",
                ["after dispatch", true, true, true]
            ]),
            "target={target}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_microtasks_observe_once_and_listener_removal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = r#"
      const log = [];
      const removed = () => log.push('unexpected removed listener');
      addEventListener('message', () => {
        log.push('once');
        Promise.resolve().then(() => {
          log.push('once microtask');
          removeEventListener('message', removed);
          dispatchEvent(new Event('message'));
        });
      }, {once: true});
      addEventListener('message', removed);
      addEventListener('message', () => {
        log.push('retained');
        setTimeout(() => finish(log), 0);
      });
      if (typeof document !== 'undefined') postMessage('go', '*');
    "#;
    let mut results = Vec::new();
    for target in ["window", "child", "worker"] {
        results.push((target, run_probe(&browser, &server, target, source).await?));
    }
    server.shutdown().await;
    for (target, observed) in results {
        assert_eq!(
            observed,
            serde_json::json!(["once", "once microtask", "retained", "retained"]),
            "target={target}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_and_script_exceptions_keep_their_distinct_cleanup_boundaries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut results = Vec::new();
    for target in ["window", "child", "worker"] {
        for from_callback in [false, true] {
            let source = format!(
                r#"
                  const log = [];
                  onerror = () => {{
                    log.push('error 1');
                    Promise.resolve().then(() => log.push('error 1 microtask'));
                    return true;
                  }};
                  addEventListener('error', () => {{
                    log.push('error 2');
                    Promise.resolve().then(() => log.push('error 2 microtask'));
                    setTimeout(() => finish(log), 0);
                  }});
                  const fail = () => {{
                    log.push('body');
                    Promise.resolve().then(() => log.push('body microtask'));
                    throw new Error('cleanup boundary');
                  }};
                  if ({from_callback}) {{
                    addEventListener('message', fail);
                    if (typeof document !== 'undefined') postMessage('go', '*');
                  }} else {{
                    fail();
                  }}
                "#
            );
            results.push((
                target,
                from_callback,
                run_probe(&browser, &server, target, &source).await?,
            ));
        }
    }
    server.shutdown().await;
    for (target, from_callback, observed) in results {
        let expected = if from_callback {
            serde_json::json!([
                "body",
                "body microtask",
                "error 1",
                "error 1 microtask",
                "error 2",
                "error 2 microtask"
            ])
        } else {
            serde_json::json!([
                "body",
                "error 1",
                "error 2",
                "body microtask",
                "error 1 microtask",
                "error 2 microtask"
            ])
        };
        assert_eq!(
            observed, expected,
            "target={target}, from_callback={from_callback}"
        );
    }
    Ok(())
}
