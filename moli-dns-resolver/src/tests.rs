use std::{
    net::IpAddr,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{DnsCachePartition, DnsLookupResult, DnsResolverService, DnsTarget};

fn test_service(
    worker_count: usize,
    lookup: impl Fn(&DnsTarget) -> DnsLookupResult + Send + Sync + 'static,
) -> DnsResolverService {
    DnsResolverService::new(
        NonZeroUsize::new(worker_count).expect("test worker count is non-zero"),
        Duration::from_secs(60),
        Arc::new(lookup),
    )
    .expect("test DNS resolver service should start")
}

fn test_result() -> DnsLookupResult {
    Ok(Arc::from([IpAddr::from([127, 0, 0, 1])]))
}

#[test]
fn identical_in_flight_lookups_are_coalesced() {
    let (lookup_started_tx, lookup_started_rx) = crossbeam_channel::bounded(1);
    let (release_tx, release_rx) = crossbeam_channel::bounded(1);
    let service = test_service(1, move |_| {
        lookup_started_tx
            .send(())
            .expect("lookup start should be observed");
        release_rx.recv().expect("lookup should be released");
        test_result()
    });
    let partition = DnsCachePartition::fresh();
    let target = DnsTarget::new("coalesced.test", 443);
    let (completion_tx, completion_rx) = crossbeam_channel::unbounded();

    for _ in 0..2 {
        let completion_tx = completion_tx.clone();
        service.resolve(partition, target.clone(), move |result| {
            completion_tx
                .send(result)
                .expect("completion should be observed");
        });
    }

    lookup_started_rx
        .recv()
        .expect("one lookup should start for the shared key");
    assert!(lookup_started_rx.try_recv().is_err());
    release_tx.send(()).expect("lookup should be released");
    for _ in 0..2 {
        assert_eq!(
            completion_rx
                .recv()
                .expect("coalesced completion should arrive")
                .expect("coalesced lookup should succeed")
                .as_ref(),
            &[IpAddr::from([127, 0, 0, 1])]
        );
    }
}

#[test]
fn worker_pool_bounds_parallel_system_lookups() {
    let (lookup_started_tx, lookup_started_rx) = crossbeam_channel::unbounded();
    let (release_tx, release_rx) = crossbeam_channel::unbounded();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let service = test_service(2, {
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        move |_| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            lookup_started_tx
                .send(())
                .expect("lookup start should be observed");
            release_rx.recv().expect("lookup should be released");
            active.fetch_sub(1, Ordering::SeqCst);
            test_result()
        }
    });
    let partition = DnsCachePartition::fresh();
    let (completion_tx, completion_rx) = crossbeam_channel::unbounded();

    for index in 0..4 {
        let completion_tx = completion_tx.clone();
        service.resolve(
            partition,
            DnsTarget::new(format!("host-{index}.test"), 443),
            move |result| {
                completion_tx
                    .send(result)
                    .expect("completion should be observed");
            },
        );
    }

    lookup_started_rx.recv().expect("first lookup should start");
    lookup_started_rx
        .recv()
        .expect("second lookup should start");
    assert!(lookup_started_rx.try_recv().is_err());
    release_tx.send(()).expect("first lookup should finish");
    release_tx.send(()).expect("second lookup should finish");
    lookup_started_rx.recv().expect("third lookup should start");
    lookup_started_rx
        .recv()
        .expect("fourth lookup should start");
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    release_tx.send(()).expect("third lookup should finish");
    release_tx.send(()).expect("fourth lookup should finish");
    for _ in 0..4 {
        completion_rx
            .recv()
            .expect("bounded lookup completion should arrive")
            .expect("bounded lookup should succeed");
    }
}

#[test]
fn positive_answers_are_cached_within_one_partition() {
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let service = test_service(1, {
        let lookup_count = Arc::clone(&lookup_count);
        move |_| {
            lookup_count.fetch_add(1, Ordering::SeqCst);
            test_result()
        }
    });
    let partition = DnsCachePartition::fresh();
    let target = DnsTarget::new("cached.test", 443);
    let (completion_tx, completion_rx) = crossbeam_channel::unbounded();

    for _ in 0..2 {
        let completion_tx = completion_tx.clone();
        service.resolve(partition, target.clone(), move |result| {
            completion_tx
                .send(result)
                .expect("completion should be observed");
        });
        completion_rx
            .recv()
            .expect("cached completion should arrive")
            .expect("cached lookup should succeed");
    }

    assert_eq!(lookup_count.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_answers_do_not_cross_runtime_partitions() {
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let service = test_service(1, {
        let lookup_count = Arc::clone(&lookup_count);
        move |_| {
            lookup_count.fetch_add(1, Ordering::SeqCst);
            test_result()
        }
    });
    let target = DnsTarget::new("partitioned.test", 443);
    let (completion_tx, completion_rx) = crossbeam_channel::unbounded();

    for partition in [DnsCachePartition::fresh(), DnsCachePartition::fresh()] {
        let completion_tx = completion_tx.clone();
        service.resolve(partition, target.clone(), move |result| {
            completion_tx
                .send(result)
                .expect("completion should be observed");
        });
        completion_rx
            .recv()
            .expect("partitioned completion should arrive")
            .expect("partitioned lookup should succeed");
    }

    assert_eq!(lookup_count.load(Ordering::SeqCst), 2);
}

#[test]
fn reserved_invalid_names_fail_synchronously_without_consuming_a_worker() {
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let service = test_service(1, {
        let lookup_count = Arc::clone(&lookup_count);
        move |_| {
            lookup_count.fetch_add(1, Ordering::SeqCst);
            test_result()
        }
    });
    let (completion_tx, completion_rx) = crossbeam_channel::bounded(1);

    service.resolve(
        DnsCachePartition::fresh(),
        DnsTarget::new("Moli-Failure.INVALID.", 80),
        move |result| {
            completion_tx
                .send(result)
                .expect("reserved-name completion should be observed");
        },
    );

    let error = completion_rx
        .try_recv()
        .expect("reserved-name failure should complete on the caller")
        .expect_err("a reserved .invalid name cannot resolve");
    assert!(error.contains("reserved .invalid domain"));
    assert_eq!(lookup_count.load(Ordering::SeqCst), 0);
}
