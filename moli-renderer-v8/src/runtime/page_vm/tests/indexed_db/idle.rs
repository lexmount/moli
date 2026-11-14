use super::*;
use crate::runtime::{PageVmIdleWaitAdvance, PageVmIdleWaitState};
use moli_indexeddb::sqlite::service::{Command, DatabaseClient};
use std::sync::mpsc;

fn storage_key(page: &mut PageVm) -> Vec<u8> {
    page.vm_mut()
        .with_default_context_scope_for_test(|_, host| {
            Ok(unsafe { &mut *host }
                .top_document_storage_context()
                .storage_key()
                .serialized_storage_key()
                .into_bytes())
        })
        .expect("Window storage key")
}

async fn assert_idle_pending(page: &mut PageVm) -> anyhow::Result<()> {
    assert!(matches!(
        page.advance_idle_wait_turn(PageVmIdleWaitState::EventLoop, Duration::from_secs(1),)
            .await?,
        PageVmIdleWaitAdvance::Waiting { .. } | PageVmIdleWaitAdvance::Progressed { .. }
    ));
    Ok(())
}

async fn drain(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    wake: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) -> anyhow::Result<()> {
    while run_selected_indexed_db_task_for_test(page, loader, wake)
        .await?
        .is_some()
    {}
    Ok(())
}

fn block_database_owner(client: &DatabaseClient) -> anyhow::Result<mpsc::Sender<()>> {
    let (entered, arrival) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let store = client
        .committed()
        .expect("committed metadata")
        .stores
        .values()
        .next()
        .expect("kv store")
        .id;
    // Native completions execute inside the serial database owner turn. Hold
    // this receipt so the next renderer request cannot complete by chance.
    client.submit(
        Command::Count {
            transaction: moli_indexeddb::TransactionHandle::from_raw(0),
            store,
            source: Default::default(),
            range: Default::default(),
        },
        move |_| {
            let _ = entered.send(());
            // Dropping the sender also releases the gate if an assertion fails.
            let _ = resume.recv_timeout(Duration::from_secs(10));
        },
    )?;
    arrival.recv_timeout(Duration::from_secs(5))?;
    Ok(release)
}

#[tokio::test(flavor = "current_thread")]
async fn idle_waits_for_in_flight_sql_and_transaction_terminal_microtasks() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let (mut page, _resource_source, mut wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://idle-indexed-db.test/")?,
        );
        install_indexed_db_manager(&mut page, &manager);
        page.vm_mut().eval(
            r#"
            globalThis.events = [];
            const open = indexedDB.open('idle-io', 1);
            open.onupgradeneeded = () => open.result.createObjectStore('kv');
            open.onsuccess = () => { globalThis.db = open.result; };
            'queued'
        "#,
        )?;
        drain(&mut page, &loader, &mut wake).await?;
        assert_eq!(page.vm_mut().eval("String(db.version)")?, "1");
        let service = manager.lock().sqlite_service();
        let (completed, completion) = mpsc::channel();
        let client = service.resolve(storage_key(&mut page), "idle-io".into(), move |reply| {
            let _ = completed.send(reply);
        })?;
        completion.recv_timeout(Duration::from_secs(5))?.result?;
        let release = block_database_owner(&client)?;
        page.vm_mut().eval(
            r#"
            globalThis.tx = db.transaction('kv', 'readwrite');
            const request = tx.objectStore('kv').put('value', 1);
            request.onsuccess = () => {
                events.push('request');
                queueMicrotask(() => events.push('request-microtask'));
            };
            tx.oncomplete = () => {
                events.push('complete');
                queueMicrotask(() => events.push('complete-microtask'));
            };
            'queued'
        "#,
        )?;
        while page
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IndexedDbTask,
                &loader,
            )
            .await?
        {}
        assert!(page.vm_mut().has_in_flight_indexed_db_tasks_for_test()?);
        assert_idle_pending(&mut page).await?;
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)")?, "[]");

        release.send(())?;
        drain(&mut page, &loader, &mut wake).await?;
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            r#"["request","request-microtask","complete","complete-microtask"]"#
        );
        assert!(
            matches!(
                page.advance_idle_wait_turn(
                    PageVmIdleWaitState::EventLoop,
                    Duration::from_secs(1),
                )
                .await?,
                PageVmIdleWaitAdvance::Completed
            ),
            "an open db and retained finished tx do not keep idle pending"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("idle waits for native SQL completion and transaction callbacks");
}

#[tokio::test(flavor = "current_thread")]
async fn idle_waits_for_a_factory_request_without_ready_sql_work() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let (mut page, _resource_source, mut wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://idle-indexed-db-factory.test/")?,
        );
        install_indexed_db_manager(&mut page, &manager);
        let service = manager.lock().sqlite_service();
        // Reserve the connection FIFO head without opening a database. The
        // page's request has neither a ready task nor a submitted SQL command.
        let lease = service.queue_connection_request_named(
            &storage_key(&mut page),
            &"blocked-idle".into(),
            Arc::new(|| {}),
        )?;
        assert!(matches!(
            page.advance_idle_wait_turn(PageVmIdleWaitState::EventLoop, Duration::from_secs(1),)
                .await?,
            PageVmIdleWaitAdvance::Completed
        ));
        schedule_open(&mut page, "blocked-idle", "events");
        // Run request acceptance; the resulting factory task is then blocked
        // by the native FIFO lease, independently of SQL completion readiness.
        while page
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IndexedDbTask,
                &loader,
            )
            .await?
        {}
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IndexedDbTask,
            )
            .is_none()
        );
        assert!(!page.vm_mut().has_in_flight_indexed_db_tasks_for_test()?);
        assert_idle_pending(&mut page).await?;
        drop(lease);
        drain(&mut page, &loader, &mut wake).await?;
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            r#"["upgrade","microtask","success"]"#
        );
        assert!(matches!(
            page.advance_idle_wait_turn(PageVmIdleWaitState::EventLoop, Duration::from_secs(1),)
                .await?,
            PageVmIdleWaitAdvance::Completed
        ));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a blocked IndexedDB factory request retains idle without SQL readiness");
}
