use std::{cell::RefCell, marker::PhantomData, ptr::NonNull, rc::Rc};

use anyhow::{Result, anyhow};

use super::{
    RendererCommandTurnOutput, RendererPageCommand, RendererPageReply,
    RendererRuntimeCommandOutput, owner_local_store::RendererPageLocalEntry, page_vm::PageVm,
};
use crate::script_vm::inspector_main::RendererInspectorMainFirstDispatchGuard;

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

pub(super) fn bind_active_nested_main_page(
    entry: &mut RendererPageLocalEntry,
) -> ActiveNestedMainPageGuard {
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
    let active = ACTIVE_NESTED_MAIN_PAGE
        .try_with(|active| active.borrow().clone())
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("nested Main receiver has no active Page owner stack"))?;

    // Release the per-session lane at the actual agent dispatch boundary, not
    // when the pause loop merely claims the queued command.
    let _post_dispatch_wake = first_dispatch.release_for_dispatch();
    // SAFETY: `bind_active_nested_main_page` installs this pointer immediately
    // around the owner-local Page dispatch that can enter V8. A normal debugger
    // pause synchronously suspends that outer dispatch, and this callback runs
    // on the same owner thread before the guard is dropped. Instrumentation
    // pauses never claim Main work. The nested borrow ends before V8 resumes
    // the outer Page call.
    let page_vm = unsafe { active.page_vm.as_ptr().as_mut() }
        .ok_or_else(|| anyhow!("nested Main Page pointer was unexpectedly null"))?;
    let reply: RendererPageReply = page_vm.dispatch_renderer_page_command(command)?;
    let page_state = active.entry_slot.active_page_state()?;
    RendererCommandTurnOutput::new(
        reply,
        page_state,
        RendererRuntimeCommandOutput::default(),
        None,
        None,
    )
}
