use super::*;
use std::time::{Duration, Instant};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

#[test]
fn image_bitmap_clone_rejects_malformed_pixel_records_before_allocation() {
    struct Delegate;
    impl v8::ValueSerializerImpl for Delegate {
        fn throw_data_clone_error<'s>(
            &self,
            _scope: &mut v8::PinScope<'s, '_>,
            _message: v8::Local<'s, v8::String>,
        ) {
            panic!("raw record setup must not throw");
        }
    }
    impl v8::ValueDeserializerImpl for Delegate {}
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-clone.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let context = scope.get_current_context();
        for header in [
            vec![],
            vec![0, 1, 0, 0, 0, 0],
            vec![1, 1, 2, 0, 0, 4],
            vec![1, 1, 0, 2, 0, 4],
            vec![1, 1, 0, 0, 2, 4],
            vec![1, 1, 0, 0, 0, 3],
            vec![1, 1, 0, 0, 1, 4],
            vec![u32::MAX, u32::MAX, 0, 0, 1, 0],
            vec![1, 1, 0, 0, 0, 4],
        ] {
            let serializer = v8::ValueSerializer::new(scope, Box::new(Delegate));
            serializer.write_header();
            for value in &header {
                serializer.write_uint32(*value);
            }
            let bytes = serializer.release();
            let deserializer = v8::ValueDeserializer::new(scope, Box::new(Delegate), &bytes);
            assert_eq!(deserializer.read_header(context), Some(true));
            assert!(
                crate::context_bootstrap::ImageBitmapClonePayload::read(&deserializer).is_none(),
                "{header:?}"
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn image_bitmap_clone_payload_and_transfer_lists_use_native_identity_and_intrinsic_prototypes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-clone.test/");
    vm.eval("const canvas=new OffscreenCanvas(2,1);canvas.getContext('2d');globalThis.bitmap=canvas.transferToImageBitmap()")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "bitmap");
        let bitmap =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, bitmap, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let payload =
            crate::context_bootstrap::image_bitmap_clone_payload_from_object(scope, proxy.into())
                .unwrap();
        let copy = crate::context_bootstrap::build_image_bitmap_from_clone_payload(scope, payload)
            .unwrap();
        let key = crate::util::v8str(scope, "nativeBitmapCopy");
        assert_eq!(
            global.create_data_property(scope, key.into(), copy.into()),
            Some(true)
        );
        let key = crate::util::v8str(scope, "bitmapProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        let reads=0,error;
        try{structuredClone({get value(){reads++;}},{transfer:[bitmapProxy,bitmap]});}
        catch(caught){error=caught;}
        return error instanceof DOMException&&error.name==='DataCloneError'&&reads===0&&bitmap.width===2;
    })()"#).unwrap(),"true");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const copy=structuredClone(bitmap);
        return copy.width===2&&bitmapProxy.width===2&&copy instanceof ImageBitmap&&nativeBitmapCopy.width===2;
    })()"#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval(r#"(() => {
        const prototype=ImageBitmap.prototype;
        globalThis.ImageBitmap=function(){throw Error('author constructor');};
        const copy=structuredClone(bitmap,{transfer:[bitmapProxy]});
        return copy.width===2&&Object.getPrototypeOf(copy)===prototype&&bitmap.width===0&&bitmapProxy.width===0;
    })()"#).unwrap(),"true");
}

#[tokio::test(flavor = "current_thread")]
async fn image_bitmap_clone_and_transfer_preserve_pixels_lifetime_and_receiving_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-clone.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        r#"(async () => {{
            const probe={}, realms=[globalThis,document.getElementById('child').contentWindow], checks=[];
            for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)
                checks.push(...await probe(realms[owner],realms[callee],owner+'-'+callee));
            globalThis.__bitmapClones=checks;
        }})().catch(error=>globalThis.__bitmapClones=String(error));"#,
        include_str!("image_bitmap_clone.js")
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while vm.eval("typeof __bitmapClones").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(Instant::now() < deadline, "bitmap clone contract timed out");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        vm.eval("Array.isArray(__bitmapClones)").unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__bitmapClones)").unwrap()
    );
    assert_eq!(vm.eval("__bitmapClones.length").unwrap(), "112");
    assert_eq!(
        vm.eval("JSON.stringify(__bitmapClones.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
