//! Repeated contention with one whole retained payload fitting at a time.
//!
//! Each handoff has one waiter, so public aggregate snapshots can attribute
//! the grant to its only possible recipient. This does not expose FIFO IDs or
//! establish a latency guarantee for arbitrarily busy owners.

use super::*;

async fn read_page(
    journal: &mut Journal,
    page: &mut Page,
    phase: &str,
    index: usize,
    rows: &[usize],
    widths: &mut [usize],
    mutate: bool,
) -> Result<()> {
    if mutate {
        widths[index] += 1;
    }
    geometry(
        journal,
        page,
        phase,
        (&format!("P{index}"), rows[index]),
        widths[index],
        false,
        mutate,
    )
    .await
}

pub(super) async fn repeated(
    journal: &mut Journal,
    pool: &LayoutWorkingSetBudget,
    timing: LayoutWorkingSetBudgetTiming,
    pages: &mut [Page],
    options: &Options,
) -> Result<()> {
    let rows = options.rows();
    let mut widths = vec![120; pages.len()];
    let mut grants = vec![0usize; pages.len()];
    let capacity = pool.limits().browser_bytes;
    for (index, page) in pages.iter_mut().enumerate() {
        geometry(
            journal,
            page,
            "multi_setup",
            (&format!("P{index}"), rows[index]),
            widths[index],
            true,
            false,
        )
        .await?;
    }
    ensure!(
        is_empty(&pool.snapshot()),
        "setup retained under zero pressure"
    );

    // Derive the competition target from actual leases, rather than assuming
    // that a node count predicts retained bytes. All Pages are otherwise idle.
    pressure(journal, pool, "multi_calibrate", capacity)?;
    let mut charges = Vec::with_capacity(pages.len());
    let mut previous = 0;
    for (index, page) in pages.iter_mut().enumerate() {
        read_page(
            journal,
            page,
            "multi_calibrate",
            index,
            &rows,
            &mut widths,
            true,
        )
        .await?;
        let state = pool.snapshot();
        ensure!(
            state.live_leases == index + 1 && state.queued_pages == 0,
            "calibration: {state:?}"
        );
        let charge = state
            .charged_bytes
            .checked_sub(previous)
            .context("calibration charge decreased")?;
        ensure!(charge > 0, "empty Page charge");
        charges.push(charge);
        previous = state.charged_bytes;
        journal.snapshot("multi_calibrate", pool)?;
    }
    let target = *charges.iter().max().context("no Pages")?;
    ensure!(
        charges
            .iter()
            .min()
            .unwrap()
            .checked_mul(2)
            .is_some_and(|two| two > target),
        "competition must fit any one Page and no two: {charges:?}"
    );
    journal.emit("multi_calibration", "multi_calibrate", json!({
        "charges": charges, "target": target,
        "attribution": "aggregate deltas with only one Page changing; no public per-Page lease census",
    }))?;

    for round in 0..usize::from(options.rounds) {
        journal.emit("round_begin", "multi_round", json!({"round": round}))?;
        pressure(journal, pool, "multi_reset", 0)?;
        wait_budget(journal, pool, "multi_reset_refund", is_empty).await?;
        pressure(journal, pool, "multi_competition", target)?;
        let mut incumbent = round % pages.len();
        read_page(
            journal,
            &mut pages[incumbent],
            "multi_incumbent",
            incumbent,
            &rows,
            &mut widths,
            true,
        )
        .await?;
        let mut completed_at = Instant::now();
        let mut state = pool.snapshot();
        ensure!(
            state.live_leases == 1
                && state.queued_pages == 0
                && state.charged_bytes > 0
                && state.charged_bytes <= target,
            "initial incumbent: {state:?}"
        );
        grants[incumbent] += 1;
        journal.emit(
            "grant",
            "multi_incumbent",
            json!({"round": round, "page": incumbent, "grants": grants}),
        )?;

        for offset in 1..pages.len() {
            let candidate = (round + offset) % pages.len();
            wait_for_incumbent_protection(journal, completed_at, timing.protection).await?;
            let bound = Instant::now();
            let before = pool.snapshot();
            ensure!(
                before.open_pages == pages.len()
                    && before.live_leases == 1
                    && before.charged_bytes == state.charged_bytes
                    && before.queued_pages == 0
                    && before.ready_bytes == 0
                    && before.pressure_target == target,
                "known sole incumbent before competition: {before:?}"
            );
            journal.emit("handoff_begin", "multi_handoff", json!({
                "round": round, "from": incumbent, "to": candidate,
                "lower_bound_ms": bound.saturating_duration_since(journal.start).as_secs_f64()*1000.0,
            }))?;
            let request = read_page(
                journal,
                &mut pages[candidate],
                "multi_denied",
                candidate,
                &rows,
                &mut widths,
                true,
            )
            .await;
            check_original_ready_window(journal, "multi_request_complete", bound, timing, request)?;
            let queued = pool.snapshot();
            journal.emit("budget", "multi_denied", snapshot_value(&queued))?;
            ensure!(
                queued.queued_pages == 1 && queued.live_leases <= 1,
                "missing unique waiter: {queued:?}"
            );
            let ready = wait_budget(journal, pool, "multi_owner_handoff", |s| {
                s.open_pages == pages.len()
                    && s.live_leases == 0
                    && s.charged_bytes == 0
                    && s.queued_pages == 1
                    && s.ready_bytes > 0
            })
            .await;
            let ready = check_original_ready_window(journal, "multi_ready", bound, timing, ready)?;
            ensure!(
                ready.ready_bytes <= target
                    && state
                        .charged_bytes
                        .checked_add(ready.ready_bytes)
                        .is_some_and(|sum| sum > target)
                    && ready.trim_requests > state.trim_requests,
                "invalid whole-payload handoff: {ready:?}"
            );
            for index in [incumbent, candidate] {
                let read = read_page(
                    journal,
                    &mut pages[index],
                    "multi_clean",
                    index,
                    &rows,
                    &mut widths,
                    false,
                )
                .await;
                check_original_ready_window(journal, "multi_clean_read", bound, timing, read)?;
            }
            let clean = pool.snapshot();
            ensure!(
                clean.charged_bytes == 0
                    && clean.live_leases == 0
                    && clean.queued_pages == 1
                    && clean.ready_bytes == ready.ready_bytes,
                "clean geometry changed storage or the pending grant: {clean:?}"
            );
            let claim = read_page(
                journal,
                &mut pages[candidate],
                "multi_claim",
                candidate,
                &rows,
                &mut widths,
                true,
            )
            .await;
            check_original_ready_window(journal, "multi_original_claim", bound, timing, claim)?;
            completed_at = Instant::now();
            state = pool.snapshot();
            journal.snapshot("multi_claim", pool)?;
            ensure!(
                state.live_leases == 1
                    && state.charged_bytes > 0
                    && state.charged_bytes <= target
                    && state.queued_pages == 0
                    && state.ready_bytes == 0,
                "candidate did not consume its grant: {state:?}"
            );
            grants[candidate] += 1;
            journal.emit(
                "grant",
                "multi_claim",
                json!({"round": round, "page": candidate, "grants": grants}),
            )?;
            incumbent = candidate;
        }

        pressure(journal, pool, "multi_hard_zero", 0)?;
        wait_budget(journal, pool, "multi_hard_zero_refund", is_empty).await?;
        for (index, page) in pages.iter_mut().enumerate() {
            read_page(
                journal,
                page,
                "multi_zero_clean",
                index,
                &rows,
                &mut widths,
                false,
            )
            .await?;
        }
        ensure!(
            is_empty(&pool.snapshot()),
            "zero-pressure reads warmed storage"
        );
        pressure(journal, pool, "multi_restore", capacity)?;
        for (index, page) in pages.iter_mut().enumerate() {
            read_page(
                journal,
                page,
                "multi_restore_clean",
                index,
                &rows,
                &mut widths,
                false,
            )
            .await?;
        }
        ensure!(
            is_empty(&pool.snapshot()),
            "restoration eagerly warmed storage"
        );
        for (index, page) in pages.iter_mut().enumerate() {
            read_page(
                journal,
                page,
                "multi_rewarm",
                index,
                &rows,
                &mut widths,
                true,
            )
            .await?;
        }
        let recovered = pool.snapshot();
        journal.snapshot("multi_rewarm", pool)?;
        ensure!(
            recovered.live_leases == pages.len()
                && recovered.queued_pages == 0
                && recovered.charged_bytes <= capacity,
            "not all Pages recovered: {recovered:?}"
        );
        ensure!(
            grants.iter().all(|count| *count == round + 1),
            "unequal service: {grants:?}"
        );
        journal.emit(
            "round_complete",
            "multi_round",
            json!({"round": round, "grants": grants, "snapshot": snapshot_value(&recovered)}),
        )?;
    }
    Ok(())
}
