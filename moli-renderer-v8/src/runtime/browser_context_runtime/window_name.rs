use std::sync::Arc;

use parking_lot::Mutex;

/// The name belongs to the browsing context, not to any of its Documents.
/// Auxiliary Window references and the browser's named-target lookup retain
/// this same cell, so a script rename is immediately visible to both.
#[derive(Clone, Debug, Default)]
pub struct RendererBrowsingContextName(Arc<Mutex<WindowNameState>>);

/// Identity of top-level contexts that can find each other by name. Session
/// history retains this identity so traversing back can restore the group.
#[derive(Clone, Debug, Default)]
pub struct RendererBrowsingContextGroup(Arc<()>);

impl PartialEq for RendererBrowsingContextGroup {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererBrowsingContextGroup {}

#[derive(Debug, Default)]
struct WindowNameState {
    name: String,
    related_pages: RendererBrowsingContextGroup,
}

impl RendererBrowsingContextName {
    /// A new auxiliary context belongs to its opener's browsing context group.
    /// Ordinary top-level contexts and noopener windows start separate groups.
    pub fn new_auxiliary(opener: &Self) -> Self {
        Self(Arc::new(Mutex::new(WindowNameState {
            related_pages: opener.0.lock().related_pages.clone(),
            ..Default::default()
        })))
    }

    pub(crate) fn is_related_to(&self, other: &Self) -> bool {
        self.group() == other.group()
    }

    pub fn group(&self) -> RendererBrowsingContextGroup {
        self.0.lock().related_pages.clone()
    }

    pub fn set_group(&self, group: RendererBrowsingContextGroup) {
        self.0.lock().related_pages = group;
    }

    pub fn get(&self) -> String {
        self.0.lock().name.clone()
    }

    pub fn set(&self, name: String) {
        self.0.lock().name = name;
    }
}

impl PartialEq for RendererBrowsingContextName {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererBrowsingContextName {}
