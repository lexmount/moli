use super::*;
use crate::ensure_v8_for_test as ensure_v8;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn image_bitmap_shared_pipeline_preserves_window_sources_and_rejection_identity() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in ["https://bitmap.test/", "http://bitmap.test/"] {
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            url::Url::parse(url).unwrap(),
            &loader,
        );
        vm.eval(&format!(
            "const runBitmapContract = {}; runBitmapContract().then(result => globalThis.__bitmapResult=result);",
            include_str!("../../worker/thread/tests/image_bitmap.js"),
        )).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while vm.eval("typeof __bitmapResult").unwrap() == "undefined" {
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .unwrap();
            assert!(
                Instant::now() < deadline,
                "Window bitmap contract timed out"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(vm.eval("__bitmapResult.complete").unwrap(), "true");
        assert_eq!(vm.eval("__bitmapResult.total").unwrap(), "29");
        assert_eq!(
            vm.eval("JSON.stringify(__bitmapResult.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn image_bitmap_native_proxy_sources_preserve_brands_and_conversion_order() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://bitmap.test/").unwrap(),
        &loader,
    );
    let image = moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 255]).unwrap();
    let bytes = moli_image::encode_png(&image).unwrap().bytes;
    vm.eval(&format!(
        "globalThis.nativeBlob=new Blob([new Uint8Array({bytes:?})]);"
    ))
    .unwrap();
    vm.eval(r#"
        globalThis.nativeOffscreen=new OffscreenCanvas(1,1);
        const context=nativeOffscreen.getContext('2d');context.fillStyle='red';context.fillRect(0,0,1,1);
        globalThis.nativeBitmap=nativeOffscreen.transferToImageBitmap();context.fillRect(0,0,1,1);
        globalThis.nativeImageData=new ImageData(new Uint8ClampedArray([255,0,0,255]),1,1);
        globalThis.nativeFrame=new VideoFrame(nativeOffscreen,{timestamp:0});
    "#).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("nativeOffscreen", "nativeOffscreenProxy"),
            ("nativeImageData", "nativeImageDataProxy"),
            ("nativeBlob", "nativeBlobProxy"),
            ("nativeBitmap", "nativeBitmapProxy"),
            ("nativeFrame", "nativeFrameProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"
        (async () => {
            for(const source of [nativeOffscreenProxy,nativeImageDataProxy,nativeBlobProxy,nativeBitmapProxy,nativeFrameProxy]) {
                let conversions=0, traps=0;
                const options={get resizeWidth(){conversions++;return 1;}};
                const pending=createImageBitmap(source,options);
                let error;
                try {await createImageBitmap(new Proxy(source,{get(){traps++;}}),options);}
                catch(reason){error=reason;}
                if(!(error instanceof TypeError)||conversions!==1||traps!==0)throw Error('author proxy conversion');
                const bitmap=await pending;
                const context=new OffscreenCanvas(1,1).getContext('2d');context.drawImage(bitmap,0,0);
                if(!(bitmap instanceof ImageBitmap)||Array.from(context.getImageData(0,0,1,1).data).join()!=='255,0,0,255')throw Error('native source pixels');
                bitmap.close();
            }
            nativeFrame.close();nativeBitmap.close();globalThis.__nativeBitmapResult=true;
        })().catch(error=>globalThis.__nativeBitmapResult=String(error));
    "#).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while vm.eval("typeof __nativeBitmapResult").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            Instant::now() < deadline,
            "native proxy bitmap task timed out"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(vm.eval("__nativeBitmapResult").unwrap(), "true");
}
