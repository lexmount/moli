use super::*;

fn sensor_permissions(
    states: [&str; 3],
) -> Vec<crate::protocol_types::PermissionOverrideRegistration> {
    ["accelerometer", "gyroscope", "magnetometer"]
        .into_iter()
        .zip(states)
        .map(
            |(name, state)| crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::json!({"name": name}),
                setting: state.to_owned(),
                origin: None,
                embedded_origin: None,
            },
        )
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn device_sensor_permissions_use_their_own_fifo_and_one_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sensor.test/fifo")?);
        page.vm_mut().set_permission_overrides(&sensor_permissions(["granted", "granted", "denied"]));
        page.vm_mut().eval(r#"
          globalThis.trace=[];
          const revoked=Proxy.revocable({},{});revoked.revoke();
          DeviceMotionEvent.requestPermission.call(revoked.proxy, {toString(){throw Error('unused');}}).then(value=>{trace.push('motion:'+value);Promise.resolve().then(()=>trace.push('microtask'));});
          DeviceOrientationEvent.requestPermission.call(revoked.proxy, undefined).then(value=>trace.push('relative:'+value));
          DeviceOrientationEvent.requestPermission(true).then(value=>trace.push('absolute:'+value));
          'queued'
        "#)?;
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "");
        assert!(!page.vm().has_ready_timeout(), "Sensor permission requests must not enter the timer source");
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MiscPlatformApi, &loader).await?);
        for expected in ["motion:granted|microtask", "motion:granted|microtask|relative:granted", "motion:granted|microtask|relative:granted|absolute:denied"] {
            assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor, &loader).await?);
            assert_eq!(page.vm_mut().eval("trace.join('|')")?, expected);
        }
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn device_sensor_retired_document_tasks_do_not_settle_or_checkpoint_replacements() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sensor.test/retired")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];
          for(const ctor of [DeviceMotionEvent,DeviceOrientationEvent]) {
            ctor.requestPermission().then(()=>trace.push('retired resolve'),()=>trace.push('retired reject'));
          }
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        for _ in 0..2 {
            let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor).expect("retired sensor tasks retain their exact FIFO entries");
            page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.join('|')")?, "");
        page.vm_mut().eval_without_microtask_checkpoint_for_test(
            "DeviceMotionEvent.requestPermission().catch(()=>trace.push('replacement reject'));'queued'",
        )?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor, &loader).await?);
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.join('|')")?, "replacement microtask|replacement reject");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn device_sensor_static_methods_ignore_this_and_preserve_callee_promise_and_error_realms() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sensor.test/realms")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.frame=document.createElement('iframe');document.body.append(frame);
          globalThis.child=frame.contentWindow;
          const argument={valueOf(){throw Error('must not convert');},toString(){throw Error('must not convert');}};
          const revoked=Proxy.revocable({},{});revoked.revoke();
          for(const ctor of [child.DeviceMotionEvent, child.DeviceOrientationEvent]) {
            const promise=ctor.requestPermission.call(revoked.proxy,argument);
            if(!(promise instanceof child.Promise)||promise instanceof Promise)throw Error('incorrect Promise realm');
            promise.then(()=>trace.push('unexpected grant'),error=>trace.push(String(error instanceof child.DOMException && !(error instanceof DOMException) && error.name==='NotAllowedError')));
          }
          'queued'
        "#)?;
        for _ in 0..2 {
            assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor, &loader).await?);
        }
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "true|true");
        page.vm_mut().eval("child.DeviceMotionEvent.requestPermission().then(()=>trace.push('removed resolve'),()=>trace.push('removed reject'));frame.remove();'removed'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DeviceSensor, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "true|true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
