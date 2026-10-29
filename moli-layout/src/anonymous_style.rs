use crate::{LayoutBox, ResolvedLayoutStyle};

/// Inheritance input for an anonymous box in the resolver's prepared pass.
/// Owner and style are borrowed from the same parent box. During construction,
/// layout may also pair a newly resolved style with its source before its box
/// exists. Renderers consume this input without selecting another style world
/// or observing the owner's element style again.
#[derive(Clone, Copy, Debug)]
pub struct LayoutAnonymousStyleContext<'a, N> {
    owner: N,
    parent_style: &'a ResolvedLayoutStyle,
}

impl<'a, N> LayoutAnonymousStyleContext<'a, N> {
    pub(crate) const fn new(owner: N, parent_style: &'a ResolvedLayoutStyle) -> Self {
        Self {
            owner,
            parent_style,
        }
    }

    /// Derives both inputs from one parent instead of accepting an unrelated
    /// owner and style. A parent without a source or anonymous owner has no
    /// native inheritance context.
    pub fn from_parent(parent: &'a LayoutBox<N>) -> Option<Self>
    where
        N: Copy,
    {
        Some(Self::new(
            parent.source().or(parent.owner())?,
            parent.style(),
        ))
    }

    pub const fn owner(&self) -> N
    where
        N: Copy,
    {
        self.owner
    }

    pub const fn parent_style(&self) -> &'a ResolvedLayoutStyle {
        self.parent_style
    }
}
