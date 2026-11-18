use super::*;

#[test]
fn stylesheet_constructor_derives_options_and_uses_native_media_list_state() {
    let mut vm = new_storage_html_test_vm("https://stylesheet-options.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/stylesheet-init-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                const trace = [], sentinel = {};
                let error;
                try { new CSSStyleSheet({
                    get alternate() { trace.push('alternate'); },
                    get baseURL() { trace.push('baseURL'); throw sentinel; },
                    get disabled() { trace.push('disabled'); },
                    get media() { trace.push('media'); }
                }); } catch (caught) { error = caught; }
                if (error !== sentinel || trace.join() !== 'alternate,baseURL') return false;
                const media = new CSSStyleSheet({media: 'screen'}).media;
                Object.defineProperty(media, '0', {get() { throw Error('author index'); }});
                return new CSSStyleSheet({media}).media.mediaText === 'screen';
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn stylesheet_resource_timing_distinguishes_links_from_imports() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/css\r\nCache-Control: no-store\r\n",
        |_, request| match request.target.as_str() {
            "/outer.css" => "@import '/inner.css'; p { color: blue; }".to_owned(),
            "/inner.css" => "p { color: red; }".to_owned(),
            path => panic!("unexpected stylesheet request {path}"),
        },
    )
    .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let document = server.base_url().join("document.html").unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(document.as_str(), &loader);
    page.eval(
        r#"
        const sheet = document.createElement('link');
        sheet.rel = 'stylesheet';
        sheet.href = '/outer.css';
        (document.head || document.documentElement).appendChild(sheet);
    "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "['/outer.css', '/inner.css'].map(path => performance.getEntriesByName(new URL(path, location.href).href)[0]?.initiatorType).join(',')",
        "link,css",
        "linked root and imported stylesheet timing",
    ).await;
    assert_eq!(server.finish_targets().await, ["/outer.css", "/inner.css"]);
}
