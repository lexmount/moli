use super::*;
use v8::ValueSerializerHelper;

#[test]
fn image_data_constructors_and_clone_graphs_preserve_native_state_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://image-data-native.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "const imageDataProbe={};const imageDataRealms=[globalThis,document.getElementById('child').contentWindow];\
         globalThis.imageDataChecks=[];\
         for(let source=0;source<2;source++)for(let callee=0;callee<2;callee++)\
         imageDataChecks.push(...imageDataProbe(imageDataRealms[source],imageDataRealms[callee],source+'-'+callee));",
        include_str!("image_data_native.js"),
    )).unwrap();
    assert_eq!(vm.eval("imageDataChecks.length").unwrap(), "544");
    assert_eq!(
        vm.eval("JSON.stringify(imageDataChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn image_data_accessors_accept_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://image-data-proxy.test/");
    vm.eval("globalThis.imageDataNative=new ImageData(new Float16Array([0.91748046875,0.2003173828125,0.1385498046875,1]),1,1,{colorSpace:'display-p3',pixelFormat:'rgba-float16'});").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "imageDataNative");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
        let key = crate::util::v8str(scope, "imageDataProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        for(const name of ['width','height','data','colorSpace','pixelFormat']) {
            const get=Object.getOwnPropertyDescriptor(ImageData.prototype,name).get;
            if(get.call(imageDataProxy)!==get.call(imageDataNative))throw Error(name+' native identity');
            let rejected=false;
            try{get.call(new Proxy(imageDataProxy,{}));}catch(error){rejected=error instanceof TypeError;}
            if(!rejected)throw Error(name+' author proxy');
        }
        const context=new OffscreenCanvas(1,1).getContext('2d');
        context.putImageData(imageDataProxy,0,0);
        return Array.from(context.getImageData(0,0,1,1).data).join();
    })()"#).unwrap(), "255,0,0,255");
}

#[test]
fn image_data_indexeddb_preserves_typed_arrays_and_shared_graph_references() {
    let mut vm = new_storage_page_task_executor_test_vm("https://image-data-storage.test/");
    vm.eval(r#"(() => {
        globalThis.imageDataStorage='pending';
        const rows=[];
        for(const [ArrayType,pixelFormat] of [[Uint8ClampedArray,'rgba-unorm8'],[Float16Array,'rgba-float16']]) {
            const buffer=new ArrayBuffer(ArrayType.BYTES_PER_ELEMENT*12);
            const data=new ArrayType(buffer,ArrayType.BYTES_PER_ELEMENT*4,4);
            data.set([1,0,1,1]);
            const image=new ImageData(data,1,1,{pixelFormat,colorSpace:'display-p3'});
            rows.push({image,again:image,data,buffer});
        }
        const request=indexedDB.open('image-data-graph-'+Math.random(),1);
        const fail=event=>{imageDataStorage='error:'+event.target.error?.name;};
        request.onerror=fail;
        request.onupgradeneeded=()=>request.result.createObjectStore('values');
        request.onsuccess=()=>{
            const db=request.result,write=db.transaction('values','readwrite');
            write.onerror=fail;write.objectStore('values').put(rows,'pixels');
            rows[0].data[0]=99;rows[1].data[0]=-99;
            write.oncomplete=()=>{
                const read=db.transaction('values').objectStore('values').get('pixels');read.onerror=fail;
                read.onsuccess=()=>{
                    try {
                        imageDataStorage=read.result.length===2&&read.result.every((row,index)=>
                            row.image instanceof ImageData&&row.image===row.again&&row.image.data===row.data&&row.data.buffer===row.buffer&&
                            row.data.byteOffset===4*(index?2:1)&&row.buffer.byteLength===12*(index?2:1)&&
                            row.image.pixelFormat===(index?'rgba-float16':'rgba-unorm8')&&row.image.colorSpace==='display-p3'&&
                            Array.from(row.data).join()==='1,0,1,1'&&row.data instanceof (index?Float16Array:Uint8ClampedArray));
                    }finally{db.close();}
                };
            };
        };
    })()"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(imageDataStorage)")
            .unwrap(),
        "true"
    );
}

struct LegacyImageDataWriter;

impl v8::ValueSerializerImpl for LegacyImageDataWriter {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }

    fn has_custom_host_object(&self, _isolate: &v8::Isolate) -> bool {
        true
    }

    fn is_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<bool> {
        Some(crate::web_api_interfaces::ImageData::is_instance(
            scope, object,
        ))
    }

    fn write_host_object<'s>(
        &self,
        _scope: &mut v8::PinScope<'s, '_>,
        _object: v8::Local<'s, v8::Object>,
        serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        // The pre-graph record consists of tag, dimensions, color-space bytes
        // and RGBA8 bytes. Keep this independent of the current writer.
        for value in [2, 1, 2, 10] {
            serializer.write_uint32(value);
        }
        serializer.write_raw_bytes(b"display-p3");
        serializer.write_uint32(8);
        serializer.write_raw_bytes(&[1, 2, 3, 4, 5, 6, 7, 8]);
        Some(true)
    }
}

#[test]
fn image_data_legacy_serialized_records_remain_readable() {
    let mut vm = new_storage_page_task_executor_test_vm("https://image-data-legacy.test/");
    vm.eval("globalThis.legacyImageDataSource=new ImageData(1,2)")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let context = scope.get_current_context();
        let global = context.global(scope);
        let key = crate::util::v8str(scope, "legacyImageDataSource");
        let source = global.get(scope, key.into()).unwrap();
        let serializer = v8::ValueSerializer::new(scope, Box::new(LegacyImageDataWriter));
        serializer.write_header();
        assert_eq!(serializer.write_value(context, source), Some(true));
        let mut payload = crate::structured_clone::V8StructuredClonePayload::default();
        payload.base.wire_bytes = serializer.release();
        drop(serializer);
        let result = crate::structured_clone::deserialize_from_wire(scope, &payload).unwrap();
        let key = crate::util::v8str(scope, "legacyImageDataResult");
        assert_eq!(
            global.create_data_property(scope, key.into(), result),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("[legacyImageDataResult instanceof ImageData,legacyImageDataResult.width,legacyImageDataResult.height,legacyImageDataResult.colorSpace,legacyImageDataResult.pixelFormat,Array.from(legacyImageDataResult.data)].join('|')").unwrap(), "true|1|2|display-p3|rgba-unorm8|1,2,3,4,5,6,7,8");
}
