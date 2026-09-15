use super::*;

#[test]
fn base_uri_csp_updates_document_and_relative_urls_at_href_mutation() {
    let mut vm = new_storage_test_vm("https://base-csp.test/path/page.html");
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    let result = vm
        .eval(
            r#"
(() => {
  const first = document.createElement('base');
  first.href = '/one/';
  document.head.append(first);
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "base-uri 'none'";
  document.head.append(meta);
  const second = document.createElement('base');
  second.href = '/two/';
  document.head.append(second);
  const frozen = document.baseURI;
  first.href = '/changed/';
  const link = document.createElement('a');
  link.href = 'asset';
  return JSON.stringify([frozen, document.baseURI, link.href]);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["https://base-csp.test/one/","https://base-csp.test/path/page.html","https://base-csp.test/path/asset"]"#
    );
}

#[test]
fn base_uri_csp_delivers_meta_after_fragment_base_insertion_steps() {
    for meta_first in [false, true] {
        let mut vm = new_storage_test_vm("https://base-csp.test/path/page.html");
        vm.document_runtime
            .dom_host_mut()
            .reset_html_document_shell();
        vm.eval(if meta_first {
            "globalThis.metaFirst = true"
        } else {
            "globalThis.metaFirst = false"
        })
        .unwrap();
        let result = vm
            .eval(
                r#"
(() => {
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "base-uri 'none'";
  const base = document.createElement('base');
  base.href = '/one/';
  const fragment = document.createDocumentFragment();
  fragment.append(...(metaFirst ? [meta, base] : [base, meta]));
  document.head.append(fragment);
  const before = document.baseURI;
  meta.remove();
  base.href = '/two/';
  return JSON.stringify([before, document.baseURI]);
})()
"#,
            )
            .unwrap();
        assert_eq!(
            result, r#"["https://base-csp.test/one/","https://base-csp.test/path/page.html"]"#,
            "meta_first={meta_first}"
        );
    }
}

#[test]
fn base_uri_csp_ignores_meta_outside_document_head() {
    let mut vm = new_storage_test_vm("https://base-csp.test/path/page.html");
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    let result = vm
        .eval(
            r#"
(() => {
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "base-uri 'none'";
  document.body.append(meta);
  const base = document.createElement('base');
  base.href = '/one/';
  document.head.append(base);
  const before = document.baseURI;
  document.head.append(meta);
  base.href = '/two/';
  return JSON.stringify([before, document.baseURI]);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["https://base-csp.test/one/","https://base-csp.test/path/page.html"]"#
    );
}

#[test]
fn base_uri_csp_keeps_the_frozen_fallback_across_history_url_changes() {
    let mut vm = new_storage_test_vm("https://base-csp.test/path/page.html");
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    let result = vm
        .eval(
            r#"
(() => {
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "base-uri 'none'";
  document.head.append(meta);
  const base = document.createElement('base');
  base.href = '/blocked/';
  document.head.append(base);
  history.pushState(null, '', '/new/page.html');
  const frozen = document.baseURI;
  const link = document.createElement('a');
  link.href = 'asset';
  return JSON.stringify([frozen, link.href, document.URL]);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["https://base-csp.test/path/page.html","https://base-csp.test/path/asset","https://base-csp.test/new/page.html"]"#
    );
}

fn run_large_stack_dom_test<F>(thread_name: &'static str, test: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(test)
        .expect("large-stack DOM test thread should spawn")
        .join()
        .expect("large-stack DOM test thread should finish");
}

mod detached_adoption;
mod detached_documents;
mod dom_parser;
mod element_construction;
mod elements_and_events;
mod files_and_transfer;
mod pointer_and_mouse;
mod ranges;
mod rendered_text;
mod style_and_geometry;
mod tree_mutations;
mod xpath_and_selectors;
