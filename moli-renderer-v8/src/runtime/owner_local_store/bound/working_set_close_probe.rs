use super::*;
use crate::native_bridge::{JsContextHost, LayoutPublicationRequest};
use crate::{LayoutWorkingSetBudget, LayoutWorkingSetBudgetSnapshot};
use moli_layout::{LayoutFlushReason, LayoutPassRequest};

type CloseSamples = Vec<(LayoutWorkingSetBudgetSnapshot, Option<(usize, usize)>)>;

/// Keeps a real checked-out entry recoverable if native preparation fails.
/// The probe and this guard always execute on the existing named owner lane.
struct RestoreCheckedOutEntry {
    token: RendererPageToken,
    entry: Option<LivePageEntry>,
}

impl RestoreCheckedOutEntry {
    fn restore(&mut self) {
        if let Some(entry) = self.entry.take() {
            restore_entry_after_command_on_bound_owner_local_store(self.token, entry);
        }
    }
}

impl Drop for RestoreCheckedOutEntry {
    fn drop(&mut self) {
        self.restore();
    }
}

fn sample(
    pool: &LayoutWorkingSetBudget,
    host: Option<&Rc<RefCell<JsContextHost>>>,
) -> (LayoutWorkingSetBudgetSnapshot, Option<(usize, usize)>) {
    let native = host.map(|host| {
        let host = host.borrow();
        (
            host.realtime_layout_working_set_count_for_test(),
            host.realtime_layout_working_set_ticket_count_for_test(),
        )
    });
    (pool.snapshot(), native)
}

fn retained_host(entry: &LivePageEntry) -> Result<Rc<RefCell<JsContextHost>>> {
    entry
        .page_vm()
        .vm()
        .context_host_weak_for_test()
        .upgrade()
        .ok_or_else(|| anyhow!("working-set close probe requires a live Host"))
}

fn close_resident_page(
    token: RendererPageToken,
    pool: &LayoutWorkingSetBudget,
) -> Result<CloseSamples> {
    let host = with_bound_render_runtime_owner_local_store_session(|session| {
        #[cfg(debug_assertions)]
        RendererOwnerLocalStore::ensure_token_thread(&token)?;
        let entry = session
            .store
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::resident_entry)
            .ok_or_else(|| anyhow!("working-set close probe Page is not resident"))?;
        retained_host(entry)
    })?;
    let mut samples = vec![sample(pool, Some(&host))];

    // Close the actual stable Page. Keep an external Host Rc across all of its
    // teardown; that reference must not prolong its computation cache.
    remove_page_on_bound_owner_local_store(token);
    samples.push(sample(pool, Some(&host)));
    drop(host);
    samples.push(sample(pool, None));
    Ok(samples)
}

fn close_checked_out_page(
    token: RendererPageToken,
    pool: &LayoutWorkingSetBudget,
) -> Result<CloseSamples> {
    let mut residence = RestoreCheckedOutEntry {
        token,
        entry: Some(take_entry_for_command_on_bound_owner_local_store(token)?),
    };
    let host = retained_host(
        residence
            .entry
            .as_ref()
            .expect("freshly checked-out entry exists"),
    )?;
    let mut samples = vec![sample(pool, Some(&host))];
    let (publication, pass) = {
        let host = host.borrow();
        let document = host.document_handle();
        let viewport = host.layout_viewport_for_document(document);
        let builds_before = host.layout_pass_observability_for_test().1;
        let mut publication =
            host.prepare_layout_publication(document, viewport, LayoutPublicationRequest::Screen)?;
        ensure!(
            publication.working_sets_for_pass().is_some(),
            "close probe requires the actual Realtime screen transaction"
        );
        let pass = host
            .build_layout_pass_for_document_in_environment(
                document,
                LayoutPassRequest::new(viewport, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )?
            .ok_or_else(|| anyhow!("working-set close probe needs a real layout root"))?;
        ensure!(
            host.layout_pass_observability_for_test().1 > builds_before,
            "working-set close probe must execute the shared layout backend"
        );
        (publication, pass)
    };
    samples.push(sample(pool, Some(&host)));

    // Registration and Document admission are revoked while the real entry is
    // checked out. The publication still owns the cache and its original lease.
    remove_page_on_bound_owner_local_store(token);
    samples.push(sample(pool, Some(&host)));

    // Returning the entry to its retiring stable slot performs actual Page
    // teardown. Keep the native publication, frozen result and Host Rc alive.
    residence.restore();
    samples.push(sample(pool, Some(&host)));

    // Frozen geometry is separate from retained computation state.
    drop(pass);
    samples.push(sample(pool, Some(&host)));
    drop(publication);
    samples.push(sample(pool, Some(&host)));
    drop(host);
    samples.push(sample(pool, None));
    Ok(samples)
}

/// Returns only owned observations; no Host, entry, or V8 value crosses lanes.
pub(in crate::runtime) async fn close_page_working_set_retention_for_test(
    local_executor: JsLocalExecutor,
    token: RendererPageToken,
    pool: LayoutWorkingSetBudget,
    checked_out_publication: bool,
) -> Result<CloseSamples> {
    run_on_bound_owner_local_store_local_task(local_executor, async move {
        if checked_out_publication {
            close_checked_out_page(token, &pool)
        } else {
            close_resident_page(token, &pool)
        }
    })
    .await
}
