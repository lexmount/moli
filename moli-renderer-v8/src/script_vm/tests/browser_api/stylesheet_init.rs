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

#[test]
fn data_stylesheet_import_honors_css_charset_in_cssom() {
    let mut vm = new_storage_html_test_vm("https://stylesheet-encoding.test/");
    let result = vm.eval(r#"
        (() => {
            const sheet = document.createElement('style');
            sheet.textContent = '@import url("data:text/css,%40charset%20%22windows-1252%22%3B%20span%7Bcontent%3A%22%80%22%7D");';
            document.head.appendChild(sheet);
            const importedRule = sheet.sheet.cssRules[0].styleSheet.cssRules[0];
            return importedRule.style.getPropertyValue('content');
        })()
    "#).unwrap();
    assert_eq!(result, r#""€""#);
}
