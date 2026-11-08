use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};
use tokio::sync::oneshot;

#[test]
fn one_async_worker_keeps_running_while_sql_is_blocked() {
    let mut runtime = Runtime::new().unwrap();
    let handle = runtime.handle();
    let (entered, waiting) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let worker = handle.clone();
    handle
        .spawn(async move {
            worker
                .blocking(move || {
                    let _ = entered.send(thread::current().id());
                    wait.recv().unwrap();
                })
                .await
                .unwrap();
        })
        .unwrap();
    let sql_thread = handle.wait(async move { waiting.await.unwrap() }).unwrap();
    let async_thread = handle.wait(async { thread::current().id() }).unwrap();
    assert_ne!(async_thread, sql_thread);
    for _ in 0..8 {
        assert_eq!(
            handle.wait(async { thread::current().id() }).unwrap(),
            async_thread
        );
    }
    release.send(()).unwrap();
    runtime.shutdown().unwrap();
    assert!(matches!(handle.spawn(async {}), Err(RuntimeError::Closed)));
}

#[test]
fn real_connection_moves_between_blocking_workers() {
    let mut runtime = Runtime::new().unwrap();
    let handle = runtime.handle();
    let (connection, ready) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let first = handle.clone();
    handle
        .spawn(async move {
            first
                .blocking(move || {
                    let database = crate::open_private_memory(Default::default()).unwrap();
                    database
                        .execute_batch(
                            "CREATE TABLE values_(n INTEGER); INSERT INTO values_ VALUES(42)",
                        )
                        .unwrap();
                    connection.send((database, thread::current().id())).unwrap();
                    wait.recv().unwrap();
                })
                .await
                .unwrap();
        })
        .unwrap();
    let second = handle.clone();
    let value = handle
        .wait(async move {
            let (database, first_thread) = ready.await.unwrap();
            second
                .blocking(move || {
                    assert_ne!(thread::current().id(), first_thread);
                    let value: i64 = database
                        .query_row("SELECT n FROM values_", [], |row| row.get(0))
                        .unwrap();
                    database.close().unwrap();
                    value
                })
                .await
                .unwrap()
        })
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(value, 42);
    runtime.shutdown().unwrap();
}

#[test]
fn shutdown_joins_accepted_tasks_without_consuming_results() {
    let mut runtime = Runtime::new().unwrap();
    let completed = Arc::new(AtomicUsize::new(0));
    for _ in 0..32 {
        let handle = runtime.handle();
        let count = completed.clone();
        runtime
            .handle()
            .spawn(async move {
                handle
                    .blocking(move || {
                        count.fetch_add(1, Ordering::Relaxed);
                    })
                    .await
                    .unwrap();
            })
            .unwrap();
    }
    runtime.shutdown().unwrap();
    assert_eq!(completed.load(Ordering::Relaxed), 32);
}

#[tokio::test]
async fn native_service_can_be_dropped_by_an_async_host() {
    let runtime = Runtime::new().unwrap();
    let handle = runtime.handle();
    let worker = handle.clone();
    handle
        .spawn(async move {
            worker
                .blocking(|| {
                    crate::open_private_memory(Default::default())
                        .unwrap()
                        .close()
                        .unwrap();
                })
                .await
                .unwrap();
        })
        .unwrap();
    drop(runtime);
    assert!(matches!(handle.spawn(async {}), Err(RuntimeError::Closed)));
}

#[test]
fn synchronous_self_wait_is_rejected_on_async_and_blocking_workers() {
    let mut runtime = Runtime::new().unwrap();
    let handle = runtime.handle();
    let own = handle.clone();
    let worker = handle.clone();
    handle
        .wait(async move {
            assert!(matches!(own.wait(async {}), Err(RuntimeError::OwnRuntime)));
            worker
                .blocking(move || {
                    assert!(matches!(own.wait(async {}), Err(RuntimeError::OwnRuntime)));
                })
                .await
                .unwrap();
        })
        .unwrap();
    runtime.shutdown().unwrap();
}

#[test]
fn task_panic_is_reported_after_other_accepted_tasks_finish() {
    let mut runtime = Runtime::new().unwrap();
    let completed = Arc::new(AtomicUsize::new(0));
    runtime
        .handle()
        .spawn(async { panic!("test task failure") })
        .unwrap();
    let count = completed.clone();
    runtime
        .handle()
        .spawn(async move {
            tokio::task::yield_now().await;
            count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
    assert!(matches!(runtime.shutdown(), Err(RuntimeError::Task(_))));
    assert_eq!(completed.load(Ordering::Relaxed), 1);
}

#[test]
fn shutdown_from_the_hosts_only_blocking_worker_does_not_wait_for_another_worker() {
    for multi_thread in [false, true] {
        let mut builder = if multi_thread {
            Builder::new_multi_thread()
        } else {
            Builder::new_current_thread()
        };
        let host = builder
            .worker_threads(1)
            .max_blocking_threads(1)
            .build()
            .unwrap();
        host.block_on(async {
            tokio::task::spawn_blocking(|| {
                let mut runtime = Runtime::new().unwrap();
                let handle = runtime.handle();
                let worker = handle.clone();
                handle
                    .wait(async move {
                        worker
                            .blocking(|| {
                                crate::open_private_memory(Default::default())
                                    .unwrap()
                                    .close()
                                    .unwrap()
                            })
                            .await
                            .unwrap();
                    })
                    .unwrap();
                runtime.shutdown().unwrap();
                assert!(matches!(handle.spawn(async {}), Err(RuntimeError::Closed)));
            })
            .await
            .unwrap();
        });
    }
}
