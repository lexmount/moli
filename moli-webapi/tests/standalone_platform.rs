mod support;

use moli_v8_test_util::ensure_v8;
use moli_webapi::{
    external, interfaces, resize_observer_entry, url_api, url_search_params, video_codecs,
    webgl_values, worker_location,
};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn with_platform(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    let url = url_api::build_url_constructor_template(scope);
    finish_template(scope, interfaces::URL::DESCRIPTOR, url);
    let params = url_search_params::build_url_search_params_constructor_template(scope);
    finish_template(scope, interfaces::URLSearchParams::DESCRIPTOR, params);
    for interface in [
        interfaces::External::DESCRIPTOR,
        interfaces::WorkerLocation::DESCRIPTOR,
        interfaces::VideoDecoder::DESCRIPTOR,
        interfaces::VideoEncoder::DESCRIPTOR,
        interfaces::ResizeObserverEntry::DESCRIPTOR,
        interfaces::ResizeObserverSize::DESCRIPTOR,
        interfaces::WebGLActiveInfo::DESCRIPTOR,
        interfaces::WebGLShaderPrecisionFormat::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        external::install_external_template_bindings(scope, template, interface.name());
        video_codecs::install_video_codec_template_bindings(scope, template, interface.name());
        resize_observer_entry::install_resize_observer_entry_template_bindings(
            scope,
            template,
            interface.name(),
        );
        webgl_values::install_value_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    test(scope);
}

#[test]
fn standalone_url_records_and_search_params_share_live_state() {
    with_platform(|scope| {
        eval(
            scope,
            r#"
        const url = new URL('../path?a=1&a=2', 'https://example.test/base/');
        const params = url.searchParams;
        if (url.href !== 'https://example.test/path?a=1&a=2') throw 'resolution';
        const it = params.entries();
        if (it.next().value.join() !== 'a,1') throw 'iterator';
        url.search = '?z=3&b=2';
        if (params !== url.searchParams || params.get('z') !== '3') throw 'live owner';
        if (it.next().value.join() !== 'b,2') throw 'live iterator';
        params.append('z', '4'); params.sort();
        if (url.search !== '?b=2&z=3&z=4') throw 'update owner';
        const own = new URLSearchParams([['x', '\ud800']]);
        if (own.toString() !== 'x=%EF%BF%BD') throw 'USVString';
        let n=0; own.forEach(() => n++); if (n!==1) throw 'callback';
        const sentinel={};
        try { own.forEach(() => {throw sentinel}); throw 'missing exception'; }
        catch(e) { if(e!==sentinel) throw e; }
        "#,
        );
    });
}

#[test]
fn standalone_url_parse_keeps_intrinsics_and_conversion_reentrancy() {
    with_platform(|scope| {
        eval(
            scope,
            r#"
        const OriginalURL = URL, parse=URL.parse;
        const params = new URLSearchParams('x=1');
        globalThis.URL=function(){throw 'author constructor'};
        const parsed = parse('https://example.test/path?x=1');
        if (!(parsed instanceof OriginalURL) || parsed.searchParams.get('x')!=='1') throw 'intrinsic';
        if (parse('!')!==null) throw 'invalid URL';
        if (OriginalURL.canParse('https://example.test/', '!')) throw 'invalid base';
        parsed.pathname={toString(){parsed.hash='kept';return '/new'}};
        if(parsed.pathname!=='/new' || parsed.hash!=='#kept') throw 'reentrancy';
        const iterator=params.entries();
        if(Object.getPrototypeOf(iterator).hasOwnProperty('constructor')) throw 'iterator constructor';
        "#,
        );
    });
}

#[test]
fn standalone_worker_location_uses_captured_script_url() {
    with_platform(|scope| {
        let global = scope.get_current_context().global(scope);
        let prototype =
            moli_v8_util::global_constructor_prototype(scope, "WorkerLocation").unwrap();
        worker_location::install_worker_location_prototype_bindings(scope, prototype).unwrap();
        worker_location::install_worker_location_value(
            scope,
            global,
            &url::Url::parse("https://user:pw@example.test:8443/path?q=1#h").unwrap(),
        )
        .unwrap();
        eval(
            scope,
            r#"
        if(!(location instanceof WorkerLocation)) throw 'brand';
        if(location.host!=='example.test:8443' || location.origin!=='https://example.test:8443' || location.search!=='?q=1' || location.hash!=='#h') throw 'components';
        const before=String(location); location.href='https://other.test/';
        if(String(location)!==before) throw 'read-only';
        "#,
        );
    });
}

#[test]
fn standalone_video_configuration_snapshots_and_rejections() {
    with_platform(|scope| {
        eval(
            scope,
            r#"
        globalThis.result='pending';
        VideoDecoder.isConfigSupported({codec:'vp8',codedWidth:2,codedHeight:3}).then(r=>{
          if(r.supported!==false || r.config.codec!=='vp8' || r.config.codedHeight!==3) throw 'snapshot';
          return VideoEncoder.isConfigSupported({codec:'',width:2,height:3}).then(()=>{throw 'expected rejection'},e=>{if(!(e instanceof TypeError)) throw e});
        }).then(()=>result='pass',e=>result=String(e));
        "#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "result").to_rust_string_lossy(scope), "pass");
    });
}

#[test]
fn standalone_resize_values_keep_frozen_sequences_and_target_identity() {
    with_platform(|scope| {
        let target = v8::Object::new(scope);
        publish(scope, "target", target);
        let value = resize_observer_entry::ResizeObserverEntryData {
            target: target.into(),
            content_rect: (1., 2., 3., 4.),
            content_box_size: (3., 4.),
            border_box_size: (5., 6.),
            device_pixel_content_box_size: (6., 8.),
        }
        .into_object(scope);
        publish(scope, "entry", value);
        eval(
            scope,
            r#"
        if(!(entry instanceof ResizeObserverEntry) || entry.target!==target) throw 'identity';
        if(!Object.isFrozen(entry.borderBoxSize) || !(entry.borderBoxSize[0] instanceof ResizeObserverSize)) throw 'sequence';
        if(entry.borderBoxSize[0].inlineSize!==5 || entry.devicePixelContentBoxSize[0].blockSize!==8) throw 'values';
        "#,
        );
    });
}

#[test]
fn standalone_external_preserves_native_noop_members() {
    with_platform(|scope| {
        let value = external::build_external_object(scope).unwrap();
        publish(scope, "external", value);
        eval(
            scope,
            r#"
        if(!(external instanceof External) || external.AddSearchProvider('x')!==undefined || external.IsSearchProviderInstalled('x')!==undefined) throw 'External';
        try { External.prototype.AddSearchProvider.call({}); throw 'brand accepted'; }
        catch(e) { if(!(e instanceof TypeError)) throw e; }
        "#,
        );
    });
}
