//! Bounded diagnostic of real Page retention using only public runtime APIs.
//! This example is not a performance benchmark or a process-memory hard limit.

use std::{
    io::Write,
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use moli_core::{
    LayoutPolicy, LayoutWorkingSetBudget, LayoutWorkingSetBudgetLimits,
    LayoutWorkingSetBudgetSnapshot, LayoutWorkingSetBudgetTiming,
    page::Page,
    runtime::{Browser, BrowserConfig, NavigationRuntimeConfig, RenderedDomWaitUntil},
};
use serde_json::{Value, json};

#[path = "layout_working_set_pressure/mod.rs"]
mod pressure_scenarios;

#[derive(Parser)]
struct Options {
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(2..=4))]
    pages: u8,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=64))]
    rounds: u16,
}

impl Options {
    fn repeated(&self) -> bool {
        self.pages != 2 || self.rounds != 1
    }

    fn rows(&self) -> Vec<usize> {
        if self.repeated() {
            (0..self.pages)
                .map(|index| 128 + usize::from(index) * 32)
                .collect()
        } else {
            vec![512, 128]
        }
    }
}

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const SOURCE_SHA: &str = match option_env!("MOLI_PRESSURE_PROBE_SOURCE_SHA") {
    Some(sha) => sha,
    None => "unattributed",
};

struct Journal {
    start: Instant,
    sequence: u64,
}

impl Journal {
    fn emit(&mut self, event: &str, phase: &str, data: Value) -> Result<()> {
        let value = json!({
            "sequence": self.sequence,
            "event": event,
            "phase": phase,
            "elapsed_ms": self.start.elapsed().as_secs_f64() * 1000.0,
            "unix_ms": SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
            "data": data,
        });
        self.sequence += 1;
        let stdout = std::io::stdout();
        let mut output = stdout.lock();
        serde_json::to_writer(&mut output, &value)?;
        writeln!(output)?;
        output.flush()?;
        Ok(())
    }

    fn snapshot(&mut self, phase: &str, pool: &LayoutWorkingSetBudget) -> Result<()> {
        self.emit("budget", phase, snapshot_value(&pool.snapshot()))
    }
}

fn snapshot_value(state: &LayoutWorkingSetBudgetSnapshot) -> Value {
    json!({
        "charged_bytes": state.charged_bytes,
        "pressure_target": state.pressure_target,
        "pressure_epoch": state.pressure_epoch,
        "open_pages": state.open_pages,
        "live_leases": state.live_leases,
        "queued_pages": state.queued_pages,
        "ready_bytes": state.ready_bytes,
        "trim_requests": state.trim_requests,
    })
}

fn is_empty(state: &LayoutWorkingSetBudgetSnapshot) -> bool {
    state.charged_bytes == 0
        && state.live_leases == 0
        && state.queued_pages == 0
        && state.ready_bytes == 0
}

async fn wait_budget(
    journal: &mut Journal,
    pool: &LayoutWorkingSetBudget,
    phase: &str,
    predicate: impl Fn(&LayoutWorkingSetBudgetSnapshot) -> bool,
) -> Result<LayoutWorkingSetBudgetSnapshot> {
    journal.emit(
        "wait_begin",
        phase,
        json!({"only_nonadvancing_snapshot": true}),
    )?;
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut interval = tokio::time::interval(Duration::from_millis(25));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = Value::Null;
    loop {
        interval.tick().await;
        // No Page command, geometry getter, pressure setter or manual
        // maintenance call is made inside this wait.
        let state = pool.snapshot();
        let value = snapshot_value(&state);
        if value != last {
            journal.emit("budget", phase, value.clone())?;
            last = value.clone();
        }
        if predicate(&state) {
            journal.emit("wait_complete", phase, value)?;
            return Ok(state);
        }
        if Instant::now() >= deadline {
            bail!("{phase}: budget condition timed out; last={value}");
        }
    }
}

async fn wait_for_incumbent_protection(
    journal: &mut Journal,
    operation_completed_at: Instant,
    protection: Duration,
) -> Result<()> {
    let deadline = operation_completed_at
        .checked_add(protection)
        .context("incumbent protection deadline overflow")?;
    let started = Instant::now();
    journal.emit("protection_wait_begin", "incumbent_eligibility", json!({
        "operation_completed_ms": operation_completed_at.saturating_duration_since(journal.start).as_secs_f64() * 1000.0,
        "deadline_ms": deadline.saturating_duration_since(journal.start).as_secs_f64() * 1000.0,
        "protection_ms": protection.as_millis(),
        "remaining_ms": deadline.saturating_duration_since(started).as_secs_f64() * 1000.0,
        "purpose": "wait for the known protection interval, not evidence of a refund",
    }))?;
    tokio::time::timeout(
        STEP_TIMEOUT,
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)),
    )
    .await
    .context("incumbent protection wait timed out")?;
    let completed = Instant::now();
    ensure!(
        completed >= deadline,
        "protection wait completed before its deadline"
    );
    journal.emit("protection_wait_complete", "incumbent_eligibility", json!({
        "elapsed_ms": completed.saturating_duration_since(started).as_secs_f64() * 1000.0,
        "age_since_operation_completed_ms": completed.saturating_duration_since(operation_completed_at).as_secs_f64() * 1000.0,
        "deadline_ms": deadline.saturating_duration_since(journal.start).as_secs_f64() * 1000.0,
        "refund_observed": false,
    }))
}

fn check_original_ready_window<T>(
    journal: &mut Journal,
    phase: &str,
    lower_bound: Instant,
    timing: LayoutWorkingSetBudgetTiming,
    operation: Result<T>,
) -> Result<T> {
    let checked_at = Instant::now();
    let elapsed = checked_at.saturating_duration_since(lower_bound);
    // Only B's subsequent request can create this demand. Its ledger operation
    // observes time after lower_bound; monotonic clamping prevents an earlier
    // sample from moving Ready's deadline before that point plus claim_ttl.
    // A later non-Ready snapshot cannot advance this proof: ledger time is
    // sampled before its lock, and snapshot itself does not observe time.
    let within_window = elapsed < timing.claim_ttl && elapsed < timing.demand_ttl;
    let lower_bound_ms = lower_bound
        .saturating_duration_since(journal.start)
        .as_secs_f64()
        * 1000.0;
    let operation_error = operation.as_ref().err().map(|error| format!("{error:#}"));
    journal.emit("claim_window", phase, json!({
        "status": if within_window { "within_conservative_window" } else { "original_ready_claim_unproven" },
        "ready_entry_lower_bound_ms": lower_bound_ms,
        "conservative_claim_deadline_ms": lower_bound_ms + timing.claim_ttl.as_secs_f64() * 1000.0,
        "conservative_demand_deadline_ms": lower_bound_ms + timing.demand_ttl.as_secs_f64() * 1000.0,
        "checked_at_ms": checked_at.saturating_duration_since(journal.start).as_secs_f64() * 1000.0,
        "elapsed_from_lower_bound_ms": elapsed.as_secs_f64() * 1000.0,
        "claim_ttl_ms": timing.claim_ttl.as_millis(),
        "demand_ttl_ms": timing.demand_ttl.as_millis(),
        "lower_bound_source": "pre_competition_non_ready_snapshot_call_start",
        "exact_ready_deadline_observed": false,
        "operation_error": operation_error,
    }))?;
    ensure!(
        within_window,
        "{phase}: diagnostic could not prove consumption of the original Ready ticket: \
         conservative claim/demand window expired (elapsed={elapsed:?}, timing={timing:?}); \
         ordinary re-admission is not counted as a successful handoff"
    );
    operation
}

fn pressure(
    journal: &mut Journal,
    pool: &LayoutWorkingSetBudget,
    phase: &str,
    target: usize,
) -> Result<()> {
    let change = pool.set_pressure_target(target)?;
    journal.emit(
        "pressure",
        phase,
        json!({
            "target": target, "changed": change.changed, "epoch": change.epoch,
        }),
    )?;
    journal.snapshot(phase, pool)
}

async fn geometry(
    journal: &mut Journal,
    page: &mut Page,
    phase: &str,
    fixture: (&str, usize),
    width: usize,
    setup: bool,
    mutate: bool,
) -> Result<()> {
    let (label, rows) = fixture;
    let mut body = String::new();
    if setup {
        body.push_str(&format!(
            "document.body.style.margin='0';document.body.innerHTML=\
             '<div id=target style=\"width:120px;height:20px\"></div>' + \
             '<div class=item style=\"width:16px;height:1px\"></div>'.repeat({rows});"
        ));
    }
    if mutate {
        body.push_str(&format!(
            "document.getElementById('target').style.width='{width}px';"
        ));
    }
    body.push_str(
        "const target=document.getElementById('target');const immediate=target.offsetWidth;",
    );
    if setup || mutate {
        // These are finite setup/drain callbacks, before the snapshot-only
        // maintenance wait. They do not change layout inputs or warm a cache.
        body.push_str(
            "await new Promise(resolve=>requestAnimationFrame(()=>\
             requestAnimationFrame(()=>setTimeout(resolve,0))));",
        );
    }
    body.push_str(
        "return JSON.stringify({immediate,width:target.offsetWidth,height:target.offsetHeight,\
         rows:document.querySelectorAll('.item').length,\
         viewport:[innerWidth,innerHeight,devicePixelRatio]});",
    );
    let expression = format!("(async()=>{{{body}}})()");
    journal.emit(
        "operation",
        phase,
        json!({
            "page": label, "page_id": page.page_id(),
            "owner": format!("{:?}", page.renderer_owner_local_host_id()),
            "setup": setup, "mutate": mutate, "expression": expression,
        }),
    )?;
    let payload = tokio::time::timeout(
        STEP_TIMEOUT,
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await
    .context("geometry operation timed out")??;
    journal.emit(
        "evaluation",
        phase,
        json!({"page": label, "payload": payload}),
    )?;
    // The public Page API returns the RemoteObject itself. JavaScript throws
    // and rejected awaited promises are mapped to {"exception": description};
    // transport/Inspector errors are already returned through Result above.
    ensure!(
        payload.get("exception").is_none(),
        "JavaScript exception: {payload}"
    );
    ensure!(
        payload["type"] == "string",
        "expected a string RemoteObject: {payload}"
    );
    let text = payload
        .get("value")
        .and_then(Value::as_str)
        .context("evaluation did not return the expected JSON string")?;
    let value: Value = serde_json::from_str(text)?;
    ensure!(
        value["immediate"] == width && value["width"] == width,
        "{phase}/{label}: width mismatch: {value}"
    );
    ensure!(
        value["height"] == 20 && value["rows"] == rows,
        "{phase}/{label}: fixture mismatch: {value}"
    );
    journal.emit("geometry", phase, json!({"page": label, "value": value}))
}

async fn phases(
    journal: &mut Journal,
    pool: &LayoutWorkingSetBudget,
    timing: LayoutWorkingSetBudgetTiming,
    first: &mut Page,
    second: &mut Page,
) -> Result<()> {
    let capacity = pool.limits().browser_bytes;
    geometry(journal, first, "cold_setup", ("A", 512), 120, true, false).await?;
    geometry(journal, second, "cold_setup", ("B", 128), 120, true, false).await?;
    ensure!(
        is_empty(&pool.snapshot()),
        "zero-pressure setup retained state"
    );

    pressure(journal, pool, "warm_restore", capacity)?;
    geometry(journal, first, "warm_A", ("A", 512), 121, false, true).await?;
    let first_warm = pool.snapshot();
    journal.snapshot("warm_A", pool)?;
    ensure!(
        first_warm.open_pages == 2 && first_warm.live_leases == 1,
        "expected only A retained: {first_warm:?}"
    );
    geometry(journal, second, "warm_B", ("B", 128), 121, false, true).await?;
    let both = pool.snapshot();
    journal.snapshot("warm_B", pool)?;
    ensure!(
        both.live_leases == 2 && both.queued_pages == 0,
        "two warm Pages: {both:?}"
    );
    let second_charge = both
        .charged_bytes
        .checked_sub(first_warm.charged_bytes)
        .context("warming B reduced stable A charge")?;
    ensure!(second_charge > 1, "B must have measurable retained charge");

    // Reset actual retained payloads, preserving both Pages and their frozen
    // geometry. This gives the competition phase one known incumbent.
    pressure(journal, pool, "competition_reset", 0)?;
    wait_budget(journal, pool, "competition_reset_refund", is_empty).await?;
    pressure(journal, pool, "competition_restore", capacity)?;
    geometry(journal, first, "incumbent_A", ("A", 512), 122, false, true).await?;
    let incumbent_completed_at = Instant::now();
    let incumbent = pool.snapshot();
    ensure!(
        incumbent.live_leases == 1 && incumbent.queued_pages == 0,
        "competition needs exactly one incumbent: {incumbent:?}"
    );
    let target = incumbent
        .charged_bytes
        .checked_add(second_charge / 2)
        .context("competition target overflow")?;
    ensure!(
        target <= capacity && second_charge <= target,
        "one whole candidate must fit the target"
    );
    pressure(journal, pool, "competition_target", target)?;
    journal.emit("calibration", "competition", json!({
        "incumbent_charge": incumbent.charged_bytes,
        "B_charge_when_uncontended": second_charge,
        "target": target,
        "attribution": "aggregate delta while only B changed; not a public per-Page lease census",
    }))?;
    // Deliberately establish eligibility before opening the conservative claim
    // window. This does not run maintenance or assume any payload was released.
    wait_for_incumbent_protection(journal, incumbent_completed_at, timing.protection).await?;
    // Keep this initial lower bound even when the first wait snapshot is Ready.
    // A later non-Ready snapshot cannot safely move the ledger's clock floor.
    let before_b_snapshot_started = Instant::now();
    let before_b = pool.snapshot();
    ensure!(
        before_b.open_pages == 2
            && before_b.live_leases == 1
            && before_b.charged_bytes == incumbent.charged_bytes
            && before_b.queued_pages == 0
            && before_b.ready_bytes == 0
            && before_b.pressure_target == target
            && before_b.charged_bytes <= target,
        "A must remain the sole incumbent with no queued/Ready demand: {before_b:?}"
    );
    journal.emit("claim_window_initial_bound", "before_B_denied", json!({
        "snapshot_started_ms": before_b_snapshot_started.saturating_duration_since(journal.start).as_secs_f64() * 1000.0,
        "snapshot": snapshot_value(&before_b),
    }))?;
    let denied = geometry(journal, second, "B_denied", ("B", 128), 122, false, true).await;
    check_original_ready_window(
        journal,
        "B_request_complete",
        before_b_snapshot_started,
        timing,
        denied,
    )?;
    let queued = pool.snapshot();
    journal.emit("budget", "B_denied", snapshot_value(&queued))?;
    let queued = check_original_ready_window(
        journal,
        "B_queued_snapshot",
        before_b_snapshot_started,
        timing,
        Ok(queued),
    )?;
    ensure!(
        queued.queued_pages == 1 && queued.live_leases <= 1,
        "one real demand must remain queued: {queued:?}"
    );
    let ready = wait_budget(journal, pool, "maintenance_handoff", |s| {
        s.open_pages == 2
            && s.charged_bytes == 0
            && s.live_leases == 0
            && s.queued_pages == 1
            && s.ready_bytes > 0
    })
    .await;
    let ready = check_original_ready_window(
        journal,
        "ready_observed",
        before_b_snapshot_started,
        timing,
        ready,
    )?;
    ensure!(
        ready.ready_bytes <= target
            && incumbent
                .charged_bytes
                .checked_add(ready.ready_bytes)
                .is_some_and(|sum| sum > target),
        "observed candidate must fit alone and compete with A: {ready:?}"
    );
    ensure!(
        ready.trim_requests > incumbent.trim_requests,
        "handoff did not record a trim request"
    );

    let clean_a = geometry(
        journal,
        first,
        "clean_after_refund",
        ("A", 512),
        122,
        false,
        false,
    )
    .await;
    check_original_ready_window(
        journal,
        "clean_A_after_refund",
        before_b_snapshot_started,
        timing,
        clean_a,
    )?;
    let clean_b = geometry(
        journal,
        second,
        "clean_after_refund",
        ("B", 128),
        122,
        false,
        false,
    )
    .await;
    check_original_ready_window(
        journal,
        "clean_B_after_refund",
        before_b_snapshot_started,
        timing,
        clean_b,
    )?;
    let clean = pool.snapshot();
    journal.emit("budget", "clean_after_refund", snapshot_value(&clean))?;
    let clean = check_original_ready_window(
        journal,
        "clean_snapshot",
        before_b_snapshot_started,
        timing,
        Ok(clean),
    )?;
    ensure!(
        clean.charged_bytes == 0 && clean.live_leases == 0 && clean.queued_pages == 1,
        "clean reads refilled storage or lost the live claim: {clean:?}"
    );
    check_original_ready_window(
        journal,
        "before_B_actual_claim",
        before_b_snapshot_started,
        timing,
        Ok(()),
    )?;
    let claim = geometry(
        journal,
        second,
        "B_actual_claim",
        ("B", 128),
        123,
        false,
        true,
    )
    .await;
    check_original_ready_window(
        journal,
        "B_actual_claim_complete",
        before_b_snapshot_started,
        timing,
        claim,
    )?;
    let claimed = pool.snapshot();
    journal.snapshot("B_actual_claim", pool)?;
    ensure!(
        claimed.live_leases == 1
            && claimed.charged_bytes > 0
            && claimed.charged_bytes <= target
            && claimed.queued_pages == 0
            && claimed.ready_bytes == 0,
        "actual B demand did not claim: {claimed:?}"
    );

    pressure(journal, pool, "hard_zero", 0)?;
    wait_budget(journal, pool, "hard_zero_refund", is_empty).await?;
    geometry(journal, first, "zero_clean", ("A", 512), 122, false, false).await?;
    geometry(journal, second, "zero_clean", ("B", 128), 123, false, false).await?;
    ensure!(
        is_empty(&pool.snapshot()),
        "zero pressure was not persistent"
    );
    pressure(journal, pool, "recovery", capacity)?;
    geometry(
        journal,
        first,
        "recovery_clean",
        ("A", 512),
        122,
        false,
        false,
    )
    .await?;
    geometry(
        journal,
        second,
        "recovery_clean",
        ("B", 128),
        123,
        false,
        false,
    )
    .await?;
    ensure!(
        is_empty(&pool.snapshot()),
        "restoring the target eagerly refilled storage"
    );
    geometry(
        journal,
        first,
        "recovery_actual_A",
        ("A", 512),
        123,
        false,
        true,
    )
    .await?;
    geometry(
        journal,
        second,
        "recovery_actual_B",
        ("B", 128),
        124,
        false,
        true,
    )
    .await?;
    let recovered = pool.snapshot();
    journal.snapshot("recovery_actual_B", pool)?;
    ensure!(
        recovered.live_leases == 2
            && recovered.queued_pages == 0
            && recovered.charged_bytes > 0
            && recovered.charged_bytes <= capacity,
        "both Pages must retain again after actual changes: {recovered:?}"
    );
    Ok(())
}

async fn run(journal: &mut Journal, options: &Options) -> Result<()> {
    ensure!(
        SOURCE_SHA.len() == 40 && SOURCE_SHA.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "build this diagnostic with MOLI_PRESSURE_PROBE_SOURCE_SHA set to the clean source commit"
    );
    let limits = LayoutWorkingSetBudgetLimits::default();
    let timing = LayoutWorkingSetBudgetTiming::default();
    let pool = LayoutWorkingSetBudget::new(limits, timing)?;
    let config = BrowserConfig::default()
        .with_layout_policy(LayoutPolicy::Realtime)
        .with_layout_working_set_budget(pool.clone());
    let carried = NavigationRuntimeConfig::from(&config);
    ensure!(
        carried
            .layout_working_set_budget()
            .is_some_and(|p| p.shares_state_with(&pool)),
        "runtime config lost the explicit pool"
    );
    pressure(journal, &pool, "initial_zero", 0)?;
    let browser = Browser::new(config)?;
    ensure!(
        browser
            .config()
            .layout_working_set_budget()
            .is_some_and(|p| p.shares_state_with(&pool)),
        "Browser did not preserve the requested pool"
    );
    let mut pages = Vec::with_capacity(usize::from(options.pages));
    let attempt: Result<()> = async {
        for index in 0..options.pages {
            let label = if options.repeated() {
                format!("P{index}")
            } else {
                ["A", "B"][usize::from(index)].to_owned()
            };
            let page = tokio::time::timeout(
                STEP_TIMEOUT,
                browser.fetch_with_wait_until(
                    "about:blank",
                    RenderedDomWaitUntil::Load,
                    STEP_TIMEOUT,
                ),
            )
            .await
            .context("Page creation timed out")??;
            journal.emit(
                "page_created",
                "setup",
                json!({
                    "page": label, "page_id": page.page_id(),
                    "owner": format!("{:?}", page.renderer_owner_local_host_id()),
                }),
            )?;
            pages.push(page);
        }
        if options.repeated() {
            pressure_scenarios::repeated(journal, &pool, timing, &mut pages, options).await
        } else {
            let (first, second) = pages.split_at_mut(1);
            phases(journal, &pool, timing, &mut first[0], &mut second[0]).await
        }
    }
    .await;
    if let Err(error) = &attempt {
        journal.emit(
            "failure",
            "scenario",
            json!({"error": format!("{error:#}")}),
        )?;
        journal.snapshot("failure", &pool)?;
    }
    let mut close_errors = Vec::new();
    for (index, page) in pages.into_iter().enumerate() {
        let closed = tokio::time::timeout(STEP_TIMEOUT, page.close_async()).await;
        match closed {
            Ok(Ok(())) => journal.emit("page_closed", "cleanup", json!({"index": index}))?,
            other => {
                let error = format!("Page {index}: {other:?}");
                journal.emit("page_close_error", "cleanup", json!({"error": error}))?;
                close_errors.push(error);
            }
        }
        journal.snapshot("after_page_close", &pool)?;
    }
    let released = wait_budget(journal, &pool, "all_pages_closed", |s| {
        s.open_pages == 0 && is_empty(s)
    })
    .await;
    if let Err(error) = &released {
        journal.emit("failure", "cleanup", json!({"error": format!("{error:#}")}))?;
    }
    drop(browser);
    journal.snapshot("browser_dropped", &pool)?;
    attempt?;
    ensure!(
        close_errors.is_empty(),
        "Page close failures: {close_errors:?}"
    );
    released?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let options = Options::parse();
    let mut journal = Journal {
        start: Instant::now(),
        sequence: 0,
    };
    let result = async {
        let limits = LayoutWorkingSetBudgetLimits::default();
        let timing = LayoutWorkingSetBudgetTiming::default();
        journal.emit("start", "metadata", json!({
            "schema": if options.repeated() { 2 } else { 1 }, "source_sha": SOURCE_SHA, "pid": std::process::id(),
            "profile": "release", "debug_assertions": cfg!(debug_assertions),
            "formal_cost_sample": false, "profile_enabled": false,
            "pages": options.pages, "rounds": options.rounds,
            "scenario": if options.repeated() { "repeated_multi_page" } else { "single_handoff" },
            "fixture_rows": options.rows(), "fixture_text": "none",
            "limits": {"document": limits.document_bytes, "page": limits.page_bytes, "browser": limits.browser_bytes},
            "timing_ms": {"protection": timing.protection.as_millis(), "idle": timing.idle_after.as_millis(),
                "claim": timing.claim_ttl.as_millis(), "demand": timing.demand_ttl.as_millis()},
            "host": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
            "geometry_source_build_frame_counters": "not exposed by public release API",
            "per_page_fifo_snapshot": "not exposed; actor attribution follows the controlled operation sequence",
        }))?;
        ensure!(!cfg!(debug_assertions), "use the release diagnostic binary");
        run(&mut journal, &options).await
    }.await;
    let data = match &result {
        Ok(()) => json!({"ok": true}),
        Err(error) => json!({"ok": false, "error": format!("{error:#}")}),
    };
    let reported = journal.emit("result", "terminal", data);
    if result.is_ok() && reported.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
