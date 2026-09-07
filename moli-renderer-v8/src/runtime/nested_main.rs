use std::{cell::RefCell, marker::PhantomData, ptr::NonNull, rc::Rc};

use anyhow::{Result, anyhow};

use super::{
    RendererCommandTurnOutput, RendererPageCommand, RendererPageReply,
    RendererRuntimeCommandOutput,
    owner_local_store::{
        LivePageEntry, LivePageEntryCheckoutError,
        checkout_entry_for_owner_turn_on_bound_owner_local_store,
        restore_entry_after_command_on_bound_owner_local_store,
    },
    page_vm::PageVm,
};
use crate::devtools::ingress::main::RendererInspectorMainFirstDispatchGuard;

#[derive(Clone)]
struct ActiveNestedMainPage {
    page_vm: NonNull<PageVm>,
    entry_slot: super::RendererPageSlotHandle,
}

thread_local! {
    static ACTIVE_NESTED_MAIN_PAGE: RefCell<Option<ActiveNestedMainPage>> = const {
        RefCell::new(None)
    };
}

/// Dynamic owner-stack binding used by Chromium-style nested Main dispatch.
///
/// V8 invokes its pause loop synchronously on the renderer owner thread. The
/// outer Page command remains suspended on that same stack, so the pointer is
/// valid only for this guard's lifetime and is never sent to another thread.
pub(super) struct ActiveNestedMainPageGuard {
    previous: Option<ActiveNestedMainPage>,
    _owner_local: PhantomData<Rc<()>>,
}

impl Drop for ActiveNestedMainPageGuard {
    fn drop(&mut self) {
        ACTIVE_NESTED_MAIN_PAGE.with(|active| {
            *active.borrow_mut() = self.previous.take();
        });
    }
}

pub(super) fn bind_active_nested_main_page(entry: &mut LivePageEntry) -> ActiveNestedMainPageGuard {
    let active_page = ActiveNestedMainPage {
        page_vm: NonNull::from(entry.page_vm_mut()),
        entry_slot: entry.slot.clone(),
    };
    let previous = ACTIVE_NESTED_MAIN_PAGE.with(|active| active.borrow_mut().replace(active_page));
    ActiveNestedMainPageGuard {
        previous,
        _owner_local: PhantomData,
    }
}

pub(crate) fn dispatch_nested_main_page_command(
    command: RendererPageCommand,
    first_dispatch: RendererInspectorMainFirstDispatchGuard,
) -> Result<RendererCommandTurnOutput> {
    let active = ACTIVE_NESTED_MAIN_PAGE
        .try_with(|active| active.borrow().clone())
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("nested Main receiver has no active Page owner stack"))?;

    // SAFETY: `bind_active_nested_main_page` installs this pointer immediately
    // around an owner-local command or scheduled turn that retains its live
    // PageVm and can enter V8. A normal debugger
    // pause synchronously suspends that outer dispatch, and this callback runs
    // on the same owner thread before the guard is dropped. Instrumentation
    // pauses never claim Main work. The nested borrow ends before V8 resumes
    // the outer Page call.
    let page_vm = unsafe { active.page_vm.as_ptr().as_mut() }
        .ok_or_else(|| anyhow!("nested Main Page pointer was unexpectedly null"))?;
    let reply: RendererPageReply = page_vm.dispatch_renderer_page_command(command)?;
    let page_state = active.entry_slot.active_page_state()?;
    let output = RendererCommandTurnOutput::new(
        reply,
        page_state,
        RendererRuntimeCommandOutput::default(),
        None,
        None,
    )?;
    // Chromium's synchronous non-V8 agent dispatch sends its response before
    // returning to the nested Main receiver. Moli decodes the typed reply in
    // the protocol actor, so retain the receiver slot with the immutable
    // result. Consuming or abandoning that result releases fail-open; the
    // actor cannot observe a later renderer publication before it has handled
    // this handoff.
    Ok(output.hold_until_protocol_handoff(first_dispatch))
}

/// Finalizes one frontend session through the IO receiver. An interrupt uses
/// the active Page stack; an idle-owner wake checks out the same exact Page.
pub(crate) fn detach_session_from_page(
    token: super::RendererPageToken,
    inspector_session_id: Option<&str>,
    fetch_subresource_interception: Option<(
        bool,
        Option<moli_page_types::SubresourceResourceType>,
    )>,
) -> Result<bool> {
    let active = ACTIVE_NESTED_MAIN_PAGE
        .try_with(|active| active.borrow().clone())
        .ok()
        .flatten();

    if let Some(active) = active {
        anyhow::ensure!(
            active.entry_slot.page_id() == token.page_id(),
            "session detach interrupt targeted a different active Page"
        );
        // SAFETY: this uses the same dynamic Page binding as nested Main
        // dispatch. A V8 interrupt runs synchronously on the owner thread
        // while the outer Page call is suspended, and this borrow ends before
        // that call resumes.
        let page_vm = unsafe { active.page_vm.as_ptr().as_mut() }
            .ok_or_else(|| anyhow!("active Page pointer was unexpectedly null"))?;
        if let Some((enabled, resource_type)) = fetch_subresource_interception {
            page_vm.set_fetch_subresource_interception(enabled, resource_type);
        }
        return Ok(page_vm.detach_runtime_inspector_session(inspector_session_id));
    }

    let mut entry = match checkout_entry_for_owner_turn_on_bound_owner_local_store(token) {
        Ok(entry) => entry,
        Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => {
            return Ok(false);
        }
        Err(LivePageEntryCheckoutError::Busy) => {
            return Err(anyhow!(
                "renderer Page remained checked out without an active IO interrupt stack"
            ));
        }
    };
    let page_vm = entry.page_vm_mut();
    if let Some((enabled, resource_type)) = fetch_subresource_interception {
        page_vm.set_fetch_subresource_interception(enabled, resource_type);
    }
    let detached = page_vm.detach_runtime_inspector_session(inspector_session_id);
    restore_entry_after_command_on_bound_owner_local_store(token, entry);
    Ok(detached)
}
