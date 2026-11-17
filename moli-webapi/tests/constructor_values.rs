mod support;
use moli_v8_test_util::ensure_v8;
use moli_v8_util::{set_private_value, v8str};
use moli_webapi::{credentials, interfaces, navigator_ua_data, text_encoder, trusted_values};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

#[test]
fn websocket_error_values_validate_close_info_without_a_transport_host() {
    with_constructors(|scope| {
        eval(
            scope,
            r#"(() => {
          const assert = (ok) => { if (!ok) throw new Error('WebSocketError'); };
          const error = new WebSocketError('message\ud800', {reason: 'done'});
          assert(error instanceof DOMException && error instanceof WebSocketError);
          assert(error.message === 'message\ud800' && error.closeCode === 1000 && error.reason === 'done');
          const empty = new WebSocketError();
          assert(empty.closeCode === null && empty.reason === '');
          for (const [options, name] of [[{closeCode: 1001}, 'InvalidAccessError'], [{reason: '€'.repeat(42)}, 'SyntaxError']]) {
            try { new WebSocketError('', options); throw new Error('accepted'); }
            catch (error) { assert(error instanceof DOMException && error.name === name); }
          }
          const order = [];
          new WebSocketError({toString() { order.push('message'); return ''; }}, {
            get closeCode() { order.push('code'); return 3001; },
            get reason() { order.push('reason'); return 'done'; }
          });
          assert(order.join() === 'message,code,reason');
          return true;
        })()"#,
        );
    });
}

fn with_constructors(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    let encoder = v8::FunctionTemplate::new(scope, text_encoder::text_encoder_constructor_callback);
    text_encoder::install(scope, encoder);
    finish_template(scope, interfaces::TextEncoder::DESCRIPTOR, encoder);
    for interface in [
        interfaces::NavigatorUAData::DESCRIPTOR,
        interfaces::Credential::DESCRIPTOR,
        interfaces::PublicKeyCredential::DESCRIPTOR,
        interfaces::AuthenticatorResponse::DESCRIPTOR,
        interfaces::AuthenticatorAttestationResponse::DESCRIPTOR,
        interfaces::AuthenticatorAssertionResponse::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        if interface.name() == "NavigatorUAData" {
            navigator_ua_data::install(scope, template);
        }
        credentials::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    let global = scope.get_current_context().global(scope);
    trusted_values::install_trusted_script_code_like_constructor(scope, global).unwrap();
    for kind in trusted_values::TRUSTED_TYPE_KINDS {
        let binding = trusted_values::build_trusted_type_constructor(scope, kind).unwrap();
        kind.interface().register(scope).unwrap();
        set_private_value(
            scope,
            global,
            kind.prototype_slot(),
            binding.prototype.into(),
        );
        set_private_value(
            scope,
            global,
            kind.constructor_slot(),
            binding.constructor.into(),
        );
        assert_eq!(
            global.set(
                scope,
                v8str(scope, kind.constructor_name()).into(),
                binding.constructor.into()
            ),
            Some(true)
        );
    }
    test(scope);
}
#[test]
fn standalone_text_encoder_preserves_utf16_read_count_and_detached_view_checks() {
    with_constructors(|scope| {
        eval(
            scope,
            r#"
        const encoder=new TextEncoder(); const dest=new Uint8Array(5);
        const result=encoder.encodeInto('A😀B',dest);
        if(encoder.encoding!=='utf-8' || result.read!==3 || result.written!==5 || [...dest].join()!=='65,240,159,152,128') throw 'encodeInto';
        if([...encoder.encode('\ud800')].join()!=='239,191,189') throw 'USVString';
        try{TextEncoder.prototype.encode.call(Object.create(TextEncoder.prototype),'x');throw 'forged receiver'}catch(e){if(!(e instanceof TypeError)) throw e;}
        const sentinel={};try{encoder.encode({toString(){throw sentinel}});throw 'conversion'}catch(e){if(e!==sentinel)throw e;}
        "#,
        );
    });
}
#[test]
fn standalone_trusted_values_preserve_native_brand_utf16_and_private_prototypes() {
    with_constructors(|scope| {
        eval(
            scope,
            "globalThis.NativeHTML=TrustedHTML;globalThis.TrustedHTML=function(){throw 'author constructor'};",
        );
        let html = trusted_values::build_trusted_type_object(
            scope,
            trusted_values::TrustedTypeKind::Html,
            &[0xd800, 60, 120, 62],
        )
        .unwrap();
        publish(scope, "html", html);
        eval(
            scope,
            "if(!(html instanceof NativeHTML) || html.toString().charCodeAt(0)!==0xd800 || html.toJSON()!==html.toString()) throw 'trusted value';try{NativeHTML.prototype.toString.call({});throw 'forged'}catch(e){if(!(e instanceof TypeError))throw e;}",
        );
        let text = trusted_values::trusted_type_string(
            scope,
            html.into(),
            trusted_values::TrustedTypeKind::Html,
        )
        .unwrap();
        assert!(text.ends_with("<x>"));
    });
}
#[test]
fn standalone_ua_data_uses_captured_identity_and_rejects_original_conversion_exception() {
    with_constructors(|scope| {
        let identity = moli_browser_profile::BrowserIdentityProfile::default();
        let ua = navigator_ua_data::build_navigator_ua_data_object(scope, &identity);
        publish(scope, "ua", ua);
        eval(
            scope,
            "globalThis.uaResult='pending';const snapshot=ua.toJSON();if(snapshot.platform!==ua.platform || snapshot.mobile!==ua.mobile || snapshot.brands.length!==ua.brands.length)throw 'UA snapshot';const sentinel={};ua.getHighEntropyValues({[Symbol.iterator](){throw sentinel}}).then(()=>uaResult='accepted',e=>uaResult=e===sentinel?'pass':String(e));",
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "uaResult").to_rust_string_lossy(scope), "pass");
    });
}
#[test]
fn standalone_credential_json_rules_do_not_require_an_authenticator_host() {
    with_constructors(|scope| {
        eval(
            scope,
            r#"
        const options=PublicKeyCredential.parseRequestOptionsFromJSON({challenge:'AQID',allowCredentials:[{id:'_w',type:'public-key'}]});
        if([...new Uint8Array(options.challenge)].join()!=='1,2,3' || new Uint8Array(options.allowCredentials[0].id)[0]!==255) throw 'JSON conversion';
        try{PublicKeyCredential.parseRequestOptionsFromJSON({challenge:'***'});throw 'base64 accepted'}catch(e){if(e.name!=='EncodingError')throw e;}
        const parse=PublicKeyCredential.parseRequestOptionsFromJSON, NativeException=DOMException, NativeTypeError=TypeError;
        try {
          globalThis.DOMException=globalThis.TypeError=function(){throw 'author exception constructor'};
          try{parse({challenge:'='});throw 'base64 accepted'}catch(e){if(!(e instanceof NativeException) || e.name!=='EncodingError')throw e;}
          try{parse({});throw 'missing challenge accepted'}catch(e){if(!(e instanceof NativeTypeError))throw e;}
        } finally {globalThis.DOMException=NativeException;globalThis.TypeError=NativeTypeError;}
        globalThis.capability='pending';PublicKeyCredential.getClientCapabilities().then(r=>capability=Object.values(r).every(v=>v===false)?'pass':'unexpected');
        "#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(
            eval(scope, "capability").to_rust_string_lossy(scope),
            "pass"
        );
    });
}

#[test]
fn standalone_base64_and_css_escape_preserve_conversion_and_utf16_without_window() {
    with_constructors(|scope| {
        let atob = v8::Function::new(scope, moli_webapi::base64::window_atob_callback).unwrap();
        let btoa = v8::Function::new(scope, moli_webapi::base64::window_btoa_callback).unwrap();
        let escape =
            v8::Function::new(scope, moli_webapi::css_escape::css_escape_callback).unwrap();
        for (name, function) in [("atob", atob), ("btoa", btoa), ("escapeCSS", escape)] {
            let global = scope.get_current_context().global(scope);
            global
                .set(
                    scope,
                    v8::String::new(scope, name).unwrap().into(),
                    function.into(),
                )
                .unwrap();
        }
        eval(
            scope,
            r#"if(atob(' Y\tQ\n==')!=='a' || btoa('a')!=='YQ==')throw 'base64'; if(escapeCSS('1a')!=='\\31 a' || escapeCSS('\ud800').charCodeAt(0)!==0xd800)throw 'CSS.escape';try{btoa('\u0100');throw 'non latin1 accepted'}catch(e){if(e.name!=='InvalidCharacterError')throw e;}const sentinel={};try{escapeCSS({toString(){throw sentinel}});throw 'conversion'}catch(e){if(e!==sentinel)throw e;}"#,
        );
    });
}
