use super::*;
use crate::LayoutWorkingSetBudgetSnapshot;

fn assert_refunded(snapshot: &LayoutWorkingSetBudgetSnapshot) {
    assert_eq!(snapshot.open_pages, 0);
    assert_eq!(snapshot.live_leases, 0);
    assert_eq!(snapshot.charged_bytes, 0);
    assert_eq!(snapshot.queued_pages, 0);
    assert_eq!(snapshot.ready_bytes, 0);
    assert!(snapshot.pages.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_page_close_clears_cached_payload_before_external_host_drop() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&runtime, &loader).await;
    assert_eq!(
        evaluate(&page, "initialWidth").await,
        serde_json::json!(120)
    );
    let registered = account(&pool, &page);
    assert!(registered.open);
    assert_eq!(registered.documents, 1);
    assert!(registered.charged_bytes > 0);

    let samples = RendererPageTestingHandle::new_for_testing(&page)
        .close_working_set_retention_probe_async(pool.clone(), false)
        .await
        .unwrap();
    // The probe already removed the owner slot. A normal close acknowledgment
    // still cleans up this external handle and is idempotent for the budget.
    page.close_async().await.unwrap();

    let [before, closed_with_host, host_dropped] = samples.as_slice() else {
        panic!("resident close probe must report all three lifetime boundaries");
    };
    assert_eq!(before.0.open_pages, 1);
    assert_eq!(before.0.live_leases, 1);
    assert!(before.0.charged_bytes > 0);
    assert_eq!(before.1, Some((1, 1)));
    assert_eq!(before.0.pages[0].registration, registered.registration);

    assert_refunded(&closed_with_host.0);
    assert_eq!(
        closed_with_host.1,
        Some((0, 0)),
        "an externally retained Host survives while Page teardown clears its cache and tickets"
    );
    assert_refunded(&host_dropped.0);
    assert_eq!(host_dropped.1, None);
    assert_refunded(&pool.snapshot());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_page_close_revokes_before_inflight_working_set_payload_is_dropped() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&runtime, &loader).await;
    assert_eq!(
        evaluate(&page, "initialWidth").await,
        serde_json::json!(120)
    );
    let registered = account(&pool, &page);
    assert!(registered.open);
    assert_eq!(registered.documents, 1);
    assert!(registered.charged_bytes > 0);

    let samples = RendererPageTestingHandle::new_for_testing(&page)
        .close_working_set_retention_probe_async(pool.clone(), true)
        .await
        .unwrap();
    page.close_async().await.unwrap();

    let [
        before,
        built,
        revoked,
        torn_down,
        geometry_dropped,
        payload_dropped,
        host_dropped,
    ] = samples.as_slice()
    else {
        panic!("checked-out close probe must report all seven lifetime boundaries");
    };
    assert_eq!(before.0.open_pages, 1);
    assert_eq!(before.0.live_leases, 1);
    assert!(before.0.charged_bytes > 0);
    assert_eq!(before.1, Some((1, 1)));
    assert_eq!(before.0.pages[0].registration, registered.registration);
    let original_charge = before.0.charged_bytes;

    assert_eq!(built.0.open_pages, 1);
    assert_eq!(built.0.live_leases, 1);
    assert_eq!(built.0.charged_bytes, original_charge);
    assert_eq!(built.0.pages[0].documents, 1);
    assert_eq!(
        built.1,
        Some((0, 1)),
        "the real screen transaction owns the cache while its Host keeps the ticket registry"
    );

    for sample in [revoked, torn_down, geometry_dropped] {
        let snapshot = &sample.0;
        assert_eq!(
            snapshot.open_pages, 0,
            "stable Page registration is revoked"
        );
        assert_eq!(snapshot.live_leases, 1);
        assert_eq!(snapshot.charged_bytes, original_charge);
        assert_eq!(snapshot.queued_pages, 0);
        assert_eq!(snapshot.ready_bytes, 0);
        let closed = snapshot
            .pages
            .iter()
            .find(|entry| entry.registration == registered.registration)
            .expect("a closed Page remains accounted while its payload is alive");
        assert_eq!(closed.token, registered.token);
        assert!(!closed.open);
        assert_eq!(
            closed.documents, 0,
            "close revokes exact Document tickets immediately"
        );
        assert!(closed.demand.is_none());
        assert_eq!(closed.charged_bytes, original_charge);
    }
    assert_eq!(
        revoked.1,
        Some((0, 1)),
        "the checked-out Host may still hold a local reference to a revoked ticket"
    );
    assert_eq!(
        torn_down.1,
        Some((0, 0)),
        "restoring the retiring entry performs real Page teardown while publication remains alive"
    );
    assert_eq!(geometry_dropped.1, Some((0, 0)));

    assert_refunded(&payload_dropped.0);
    assert_eq!(
        payload_dropped.1,
        Some((0, 0)),
        "dropping actual retained buffers refunds the lease before the external Host is released"
    );
    assert_refunded(&host_dropped.0);
    assert_eq!(host_dropped.1, None);
    assert_refunded(&pool.snapshot());
}
