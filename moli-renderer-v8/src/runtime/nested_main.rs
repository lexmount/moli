use std::{cell::RefCell, marker::PhantomData, ptr::NonNull, rc::Rc};

use anyhow::{Result, anyhow};

use super::{
    RendererCommandTurnOutput, RendererPageCommand, RendererPageReply,
    RendererRuntimeCommandOutput, owner_local_store::LivePageEntry, page_vm::PageVm,
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
    mut first_dispatch: RendererInspectorMainFirstDispatchGuard,
) -> Result<RendererCommandTurnOutput> {
    assert_eq!(
        command.nested_dispatch(),
        super::RendererDevToolsMainNestedDispatch::PageAgent,
        "nested Page dispatch must not enter V8 through the suspended Page owner"
    );
    let active = ACTIVE_NESTED_MAIN_PAGE
        .try_with(|active| active.borrow().clone())
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("nested Main receiver has no active Page owner stack"))?;

    // SAFETY: `bind_active_nested_main_page` installs this pointer immediately
    // around the owner-local Page dispatch that can enter V8. A normal debugger
    // pause synchronously suspends that outer dispatch, and this callback runs
    // on the same owner thread before the guard is dropped. Instrumentation
    // pauses never claim Main work. The nested borrow ends before V8 resumes
    // the outer Page call.
    let page_vm = unsafe { active.page_vm.as_ptr().as_mut() }
        .ok_or_else(|| anyhow!("nested Main Page pointer was unexpectedly null"))?;
    first_dispatch.release();
    let reply: RendererPageReply = page_vm.dispatch_renderer_page_command(command)?;
    // The enclosing turn is suspended inside V8, so its owner-slot snapshot
    // still describes the state before the pause. Capture the live VM for this
    // nested handler without committing/replacing the enclosing owner turn.
    let previous = active.entry_slot.active_page_state()?;
    let capture = page_vm.capture_nested_page_state()?;
    let page_state = super::RendererPageState::from_vm_state_capture(
        previous.requested_url.clone(),
        previous.navigation_initiator_url.clone(),
        previous.navigation_redirected,
        previous.navigation_redirect_count,
        previous.status,
        previous.headers.clone(),
        capture,
    );
    let predecessor = page_vm.publish_nested_command_output_prefix();
    let output = RendererCommandTurnOutput::new(
        reply,
        page_state,
        RendererRuntimeCommandOutput::default(),
        None,
        predecessor,
    )?;
    // Ready frontend output is already in the journal. A typed internal reply
    // is only a Browser continuation value, so neither needs a receipt gate.
    Ok(output)
}
