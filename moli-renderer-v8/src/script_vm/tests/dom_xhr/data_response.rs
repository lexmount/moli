use super::*;

async fn check_data_response_modes(worker: bool) {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader("https://data-response.test/", &loader);
    let probe = r#"(async () => {
  const cases = [
    ['data:,local%20body#fragment', 'text/plain;charset=US-ASCII', [108,111,99,97,108,32,98,111,100,121]],
    ['data:application/octet-stream;base64,AP+A', 'application/octet-stream', [0,255,128]],
    ['data:text/plain;charset=utf-8,%E4%B8%AD', 'text/plain;charset=utf-8', [228,184,173]]
  ];
  let count = 0;
  for (const [url,mime,bytes] of cases) {
    if (new URL(url).origin !== 'null') throw new Error('data URL origin must remain opaque');
    for (const mode of ['cors','same-origin','no-cors']) {
      for (const redirect of ['follow','manual','error']) {
        for (const method of ['GET','HEAD','POST']) {
          for (const input of ['url','request']) {
            const label = [url,mode,redirect,method,input].join('/');
            const assert = (condition,message) => { if (!condition) throw new Error(label+': '+message); };
            const init = {mode,redirect,method};
            const response = input === 'url'
              ? await fetch(url,init)
              : await fetch(new Request(url,{mode,method,redirect:'follow'}).clone(),{redirect});
            assert(response instanceof Response,'Response');
            assert(response.type === 'basic','type='+response.type);
            assert(response.status === 200 && response.statusText === 'OK' && response.ok,'status');
            assert(response.url === url.split('#')[0] && !response.redirected,'URL and redirected');
            assert(response.headers.get('Content-Type') === mime,'Content-Type='+response.headers.get('Content-Type'));
            assert(response.headers.get('Content-Length') === null,'no synthesized Content-Length');
            assert(!response.bodyUsed,'body initially unused');
            assert((response.body === null) === (method === 'HEAD'),'null body for HEAD');
            let headerError;
            try { response.headers.set('X-Author','changed'); } catch(error) { headerError = error; }
            assert(headerError instanceof TypeError,'immutable fetch headers');
            const clone = response.clone();
            assert(clone.type === 'basic' && clone.status === 200 && clone.url === response.url,'clone surface');
            assert(clone.headers.get('Content-Type') === mime,'clone Content-Type');
            const expected = method === 'HEAD' ? [] : bytes;
            const actual = Array.from(new Uint8Array(await response.arrayBuffer()));
            const cloned = Array.from(new Uint8Array(await clone.arrayBuffer()));
            assert(JSON.stringify(actual) === JSON.stringify(expected),'body bytes '+actual);
            assert(JSON.stringify(cloned) === JSON.stringify(expected),'clone bytes '+cloned);
            assert(response.bodyUsed === (method !== 'HEAD') && clone.bodyUsed === (method !== 'HEAD'),'bodyUsed');
            count++;
          }
        }
      }
    }
  }
  return String(count);
})()"#;
    let script = if worker {
        let worker_source = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            "globalThis.dataResponseResult = 'pending'; const worker = new Worker('data:text/javascript,' + encodeURIComponent({})); worker.onmessage = event => {{ dataResponseResult = event.data; }}; worker.onerror = event => {{ dataResponseResult = event.message; event.preventDefault(); }};",
            serde_json::to_string(&worker_source).unwrap()
        )
    } else {
        format!(
            "globalThis.dataResponseResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ dataResponseResult = value; }}, error => {{ dataResponseResult = String(error.stack || error); }});"
        )
    };
    vm.eval(&script).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while vm.eval("dataResponseResult === 'pending'").unwrap() == "true" {
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                .await
                .unwrap();
        }
    })
    .await
    .expect("data URL fetch matrix should settle locally");
    assert_eq!(vm.eval("dataResponseResult").unwrap(), "162");
}

#[tokio::test]
async fn data_response_preserves_basic_filter_in_window() {
    check_data_response_modes(false).await;
}

#[tokio::test]
async fn data_response_preserves_basic_filter_in_opaque_origin_worker() {
    check_data_response_modes(true).await;
}
