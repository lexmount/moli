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
