//! Wrapper-world membership, separate from a concrete Realm's authority.

use std::{
    collections::HashMap,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    JsContextHost, OwnerDispatchScope, PrebootstrappedChildDefaultContext,
    RuntimeObservableContextToken, WindowExecutionContextAccessPolicy,
    WindowExecutionContextIdentity, WindowExecutionContextOwner,
};
use crate::{document_runtime::DomHandle, frame_owner_model::FrameDocumentTaskOwner};

/// An isolated wrapper world can have a distinct Context in several Windows.
/// Names are inspector metadata, and a Context token identifies only one
/// Realm. Neither is a suitable identity for that shared wrapper world.
///
/// Allocation is process-wide because related Pages can share an isolate but
/// have separate Hosts and Host-local Realm-token counters. This identity is
/// never used to authorize a Window operation or to revive a retired Realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct IsolatedWindowWorldId(NonZeroU64);

impl IsolatedWindowWorldId {
    pub(crate) fn allocate() -> anyhow::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let value = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| anyhow::anyhow!("isolated Window world identity exhausted"))?;
        Ok(Self(
            NonZeroU64::new(value).expect("isolated world identities start at one"),
        ))
    }
}

pub(crate) fn install_isolated_window_world_for_context(
    context: v8::Local<'_, v8::Context>,
    world: IsolatedWindowWorldId,
) {
    assert!(
        context.get_slot::<IsolatedWindowWorldId>().is_none(),
        "a Context's wrapper-world identity is immutable"
    );
    let previous = context.set_slot(Rc::new(world));
    debug_assert!(previous.is_none());
}

/// Main-world Contexts carry no isolated-world slot. This is a native slot
/// read, with no property lookup, wrapper creation, or layout side effect.
pub(crate) fn isolated_window_world_for_context(
    context: v8::Local<'_, v8::Context>,
) -> Option<IsolatedWindowWorldId> {
    context
        .get_slot::<IsolatedWindowWorldId>()
        .as_deref()
        .copied()
}

/// Shared by a world's concrete Window Contexts. Inspector names describe the
/// world; only the independently allocated id identifies it.
pub(crate) struct IsolatedWindowWorld {
    pub(crate) id: IsolatedWindowWorldId,
    pub(crate) name: String,
    pub(crate) devtools_session: Option<moli_page_types::DevToolsSessionKey>,
    pub(crate) access_policy: WindowExecutionContextAccessPolicy,
}

impl IsolatedWindowWorld {
    pub(crate) fn new(
        name: &str,
        devtools_session: Option<moli_page_types::DevToolsSessionKey>,
        access_policy: WindowExecutionContextAccessPolicy,
    ) -> anyhow::Result<Rc<Self>> {
        Ok(Rc::new(Self {
            id: IsolatedWindowWorldId::allocate()?,
            name: name.to_owned(),
            devtools_session,
            access_policy,
        }))
    }
}

pub(crate) fn install_isolated_window_world_definition(
    context: v8::Local<'_, v8::Context>,
    world: Rc<IsolatedWindowWorld>,
) {
    install_isolated_window_world_for_context(context, world.id);
    let previous = context.set_slot(world);
    assert!(
        previous.is_none(),
        "a Context has one wrapper-world definition"
    );
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct WorldMemberKey {
    world: IsolatedWindowWorldId,
    owner: WindowExecutionContextOwner,
}

struct WorldMember {
    token: RuntimeObservableContextToken,
    context: Rc<v8::Weak<v8::Context>>,
}

pub(crate) struct PrebootstrappedIsolatedWindowContext {
    pub(crate) world: Rc<IsolatedWindowWorld>,
    pub(crate) frame_id: Option<String>,
    pub(crate) child_handle: Option<DomHandle>,
    pub(crate) document_owner: FrameDocumentTaskOwner,
    pub(crate) runtime: crate::script_vm::WindowRealmRuntime,
}

#[derive(Default)]
struct IsolatedWindowContexts {
    members: HashMap<WorldMemberKey, WorldMember>,
    pending: Vec<PrebootstrappedIsolatedWindowContext>,
}

/// ScriptVm owns this existing prebootstrap residence. The Host keeps only its
/// Weak reference, so Context-owned Hosts cannot strongly root their Contexts.
/// Isolated state is allocated only when an isolated world is actually used.
#[derive(Default)]
pub(crate) struct PrebootstrappedWindowContexts {
    child_defaults: HashMap<DomHandle, PrebootstrappedChildDefaultContext>,
    isolated: Option<Box<IsolatedWindowContexts>>,
}

impl Deref for PrebootstrappedWindowContexts {
    type Target = HashMap<DomHandle, PrebootstrappedChildDefaultContext>;

    fn deref(&self) -> &Self::Target {
        &self.child_defaults
    }
}

impl DerefMut for PrebootstrappedWindowContexts {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child_defaults
    }
}

impl PrebootstrappedWindowContexts {
    #[cfg(test)]
    pub(crate) fn pending_isolated_count_for_test(&self) -> usize {
        self.isolated
            .as_ref()
            .map_or(0, |state| state.pending.len())
    }

    pub(crate) fn take_pending_isolated(&mut self) -> Vec<PrebootstrappedIsolatedWindowContext> {
        self.isolated
            .as_mut()
            .map(|state| std::mem::take(&mut state.pending))
            .unwrap_or_default()
    }

    pub(crate) fn forget_isolated_context(&mut self, token: RuntimeObservableContextToken) {
        if let Some(state) = self.isolated.as_mut() {
            state.members.retain(|_, member| member.token != token);
            if state.members.is_empty() && state.pending.is_empty() {
                self.isolated = None;
            }
        }
    }
}

impl JsContextHost {
    pub(super) fn isolated_window_contexts_for_security_refresh(
        &self,
    ) -> Vec<(RuntimeObservableContextToken, Rc<v8::Weak<v8::Context>>)> {
        let Some(store) = self
            .child_default_context_bootstrap
            .as_ref()
            .and_then(|config| config.pending_contexts.upgrade())
        else {
            return Vec::new();
        };
        let store = store.borrow();
        store
            .isolated
            .as_ref()
            .map(|state| {
                state
                    .members
                    .values()
                    .map(|member| (member.token, member.context.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub(crate) fn remember_isolated_window_context(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    context: v8::Local<'_, v8::Context>,
) -> anyhow::Result<()> {
    let world = context
        .get_slot::<IsolatedWindowWorld>()
        .ok_or_else(|| anyhow::anyhow!("isolated Context has no wrapper-world definition"))?;
    let identity = unsafe { &*host_ptr }
        .window_execution_context_identity_for_access_check(context)
        .ok_or_else(|| anyhow::anyhow!("isolated Context has no registered Window Realm"))?;
    let store = unsafe { &*host_ptr }
        .child_default_context_bootstrap
        .as_ref()
        .and_then(|config| config.pending_contexts.upgrade())
        .ok_or_else(|| anyhow::anyhow!("Window bootstrap owner was retired"))?;
    let key = WorldMemberKey {
        world: world.id,
        owner: identity.owner(),
    };
    let token = identity.realm_token();
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        context,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().forget_isolated_context(token);
            }
        }),
    );
    store
        .borrow_mut()
        .isolated
        .get_or_insert_with(Default::default)
        .members
        .insert(
            key,
            WorldMember {
                token,
                context: Rc::new(weak),
            },
        );
    Ok(())
}

/// Resolve a target Window's concrete Context in the accessing world's view.
/// This is an object projection, not an authorization to execute a Window API.
/// Creating a missing member requires both exact live Realm identities. A
/// cross-origin WindowProxy can be returned without permission to read its
/// Document; V8's access handlers and event delivery enforce that permission.
/// No Host/store borrow crosses bootstrap or V8 calls.
pub(crate) fn window_context_in_wrapper_world<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Context>,
    accessing: v8::Local<'s, v8::Context>,
) -> anyhow::Result<Option<v8::Local<'s, v8::Context>>> {
    let target_world = isolated_window_world_for_context(target);
    let accessing_world = isolated_window_world_for_context(accessing);
    if target_world == accessing_world {
        return Ok(Some(target));
    }
    let Some(target_host_ptr) = crate::util::context_host_ptr_from_context_slot(target) else {
        return Ok(None);
    };
    let Some(accessing_host_ptr) = crate::util::context_host_ptr_from_context_slot(accessing)
    else {
        return Ok(None);
    };
    let target_identity = unsafe { &*target_host_ptr }
        .window_execution_context_identity_for_access_check(target)
        .ok_or_else(|| anyhow::anyhow!("target Window Realm was retired"))?;
    let accessing_identity = unsafe { &*accessing_host_ptr }
        .window_execution_context_identity_for_access_check(accessing)
        .ok_or_else(|| anyhow::anyhow!("accessing Window Realm was retired"))?;
    anyhow::ensure!(
        unsafe { &*target_host_ptr }.window_execution_context_identity_is_current(target_identity)
            && unsafe { &*accessing_host_ptr }
                .window_execution_context_identity_is_current(accessing_identity),
        "wrapper-world Window Realm was retired"
    );
    if target_host_ptr == accessing_host_ptr
        && target_identity.owner() == accessing_identity.owner()
    {
        return Ok(Some(accessing));
    }
    if accessing_world.is_none() {
        return Ok(unsafe { &*target_host_ptr }
            .window_execution_context(
                scope,
                target_identity.owner(),
                target_identity.dispatch_scope(),
            )
            .map(|(_, context)| context));
    }
    let world = accessing
        .get_slot::<IsolatedWindowWorld>()
        .ok_or_else(|| anyhow::anyhow!("accessing Context has no wrapper-world definition"))?;
    ensure_isolated_window_world_member(scope, target_host_ptr, target_identity, world).map(Some)
}

pub(crate) fn window_in_wrapper_world<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    accessing: v8::Local<'s, v8::Context>,
) -> anyhow::Result<Option<v8::Local<'s, v8::Object>>> {
    let Some(context) = target.get_creation_context(scope) else {
        return Ok(None);
    };
    if !target.strict_equals(context.global(scope).into()) {
        return Ok(None);
    }
    Ok(window_context_in_wrapper_world(scope, context, accessing)?
        .map(|context| context.global(scope)))
}

pub(crate) enum WindowEventAccess {
    Unmanaged,
    Retired,
    Denied,
    Allowed,
}

pub(crate) fn window_event_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    accessing: v8::Local<'s, v8::Context>,
) -> WindowEventAccess {
    let Some(target_context) = target.get_creation_context(scope) else {
        return WindowEventAccess::Unmanaged;
    };
    if !target.strict_equals(target_context.global(scope).into()) {
        return WindowEventAccess::Unmanaged;
    }
    let Some(target_host) = crate::util::context_host_ptr_from_context_slot(target_context) else {
        return WindowEventAccess::Unmanaged;
    };
    let Some(accessing_host) = crate::util::context_host_ptr_from_context_slot(accessing) else {
        return WindowEventAccess::Unmanaged;
    };
    let Some(target_identity) =
        unsafe { &*target_host }.window_execution_context_identity_for_access_check(target_context)
    else {
        return WindowEventAccess::Retired;
    };
    let Some(accessing_identity) =
        unsafe { &*accessing_host }.window_execution_context_identity_for_access_check(accessing)
    else {
        return WindowEventAccess::Retired;
    };
    if !unsafe { &*target_host }.window_execution_context_identity_is_current(target_identity)
        || !unsafe { &*accessing_host }
            .window_execution_context_identity_is_current(accessing_identity)
    {
        return WindowEventAccess::Retired;
    }
    if let Some(id) = isolated_window_world_for_context(accessing) {
        // Blink's ShouldAllowAccessToV8Context requires the target Context to
        // be in the listener's world, then permits a non-main-world view. This
        // does not permit ordinary cross-origin Window property access.
        return if accessing
            .get_slot::<IsolatedWindowWorld>()
            .is_some_and(|world| world.id == id)
        {
            WindowEventAccess::Allowed
        } else {
            WindowEventAccess::Denied
        };
    }
    let allowed = if target_host == accessing_host {
        unsafe { &*target_host }
            .window_execution_context_can_access(accessing_identity, target_identity)
    } else {
        unsafe { &*accessing_host }.window_execution_context_can_access_other_host(
            accessing_identity,
            unsafe { &*target_host },
            target_identity,
        )
    };
    if allowed {
        WindowEventAccess::Allowed
    } else {
        WindowEventAccess::Denied
    }
}

fn ensure_isolated_window_world_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowExecutionContextIdentity,
    world: Rc<IsolatedWindowWorld>,
) -> anyhow::Result<v8::Local<'s, v8::Context>> {
    let config = unsafe { &*host_ptr }
        .child_default_context_bootstrap
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Window bootstrap is unavailable"))?;
    let store = config
        .pending_contexts
        .upgrade()
        .ok_or_else(|| anyhow::anyhow!("Window bootstrap owner was retired"))?;
    let key = WorldMemberKey {
        world: world.id,
        owner: target.owner(),
    };
    let candidate = store
        .borrow()
        .isolated
        .as_ref()
        .and_then(|state| state.members.get(&key))
        .map(|member| (member.token, member.context.clone()));
    if let Some((token, weak)) = candidate
        && let Some(context) = weak.to_local(scope)
        && crate::util::context_host_ptr_from_context_slot(context) == Some(host_ptr)
        && let Some(identity) =
            unsafe { &*host_ptr }.window_execution_context_identity_for_access_check(context)
        && identity.realm_token() == token
        && identity.owner() == target.owner()
        && identity.dispatch_scope() == target.dispatch_scope()
        && unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
    {
        return Ok(context);
    }
    let (child_handle, document_owner, frame_id) = match target.dispatch_scope() {
        OwnerDispatchScope::Top => (
            None,
            unsafe { &*host_ptr }.current_main_document_task_owner(),
            None,
        ),
        OwnerDispatchScope::Child(handle) => (
            Some(handle),
            unsafe { &*host_ptr }.current_child_document_task_owner(handle),
            unsafe { &*host_ptr }
                .frame_owner_frame_id_for_child_handle(handle)
                .map(|id| id.0),
        ),
        OwnerDispatchScope::LightweightPopup(_) => {
            anyhow::bail!("popup has no independent Window Realm")
        }
    };
    let document_owner =
        document_owner.ok_or_else(|| anyhow::anyhow!("target Window has no current Document"))?;
    anyhow::ensure!(
        target.owner() == WindowExecutionContextOwner::Frame(document_owner.local_window_id),
        "target Window changed before world bootstrap"
    );
    let host = config
        .host
        .upgrade()
        .ok_or_else(|| anyhow::anyhow!("Window Host was retired"))?;
    let template = unsafe { &*host_ptr }
        .bridge
        .bindings
        .window_global_template(scope);
    let indexed_db = unsafe { &*host_ptr }.indexed_db_manager.clone();
    let storage = unsafe { &*host_ptr }.storage_bucket_store.clone();
    let runtime = crate::script_vm::bootstrap_isolated_window_context_in_scope(
        scope,
        template,
        host,
        config.resource_owner_id,
        &config.promise_reject_dispatch,
        indexed_db,
        Some(storage),
        child_handle,
        document_owner,
        world.clone(),
    )?;
    let context = v8::Local::new(scope, &runtime.context);
    store
        .borrow_mut()
        .isolated
        .get_or_insert_with(Default::default)
        .pending
        .push(PrebootstrappedIsolatedWindowContext {
            world,
            frame_id,
            child_handle,
            document_owner,
            runtime,
        });
    Ok(context)
}
