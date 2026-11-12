use super::{Attribute, HtmlSerializationPolicy, NativeNodeId};

pub(super) enum EmbeddedAttribute {
    Srcdoc,
    FrameUrl,
}

impl EmbeddedAttribute {
    pub(super) fn for_attribute(attribute: &Attribute) -> Option<Self> {
        if !attribute.namespace().is_empty() {
            return None;
        }
        match attribute.local_name() {
            "srcdoc" => Some(Self::Srcdoc),
            "data-moli-frame-url" => Some(Self::FrameUrl),
            _ => None,
        }
    }
}

/// A borrowed view; only the index must survive an embedded Document traversal.
pub(super) struct EffectiveAttributes<'a> {
    owner: NativeNodeId,
    original: &'a [Attribute],
    overrides: &'a [Attribute],
    filter: Option<&'a dyn HtmlSerializationPolicy>,
}

impl<'a> EffectiveAttributes<'a> {
    pub(super) fn new(
        owner: NativeNodeId,
        original: &'a [Attribute],
        overrides: &'a [Attribute],
        filter: Option<&'a dyn HtmlSerializationPolicy>,
    ) -> Self {
        Self {
            owner,
            original,
            overrides,
            filter,
        }
    }

    #[inline]
    pub(super) fn next(&self, index: &mut usize, embedded: bool) -> Option<&'a Attribute> {
        loop {
            let position = *index;
            let attribute = if let Some(original) = self.original.get(position) {
                *index += 1;
                self.overrides
                    .iter()
                    .find(|a| same_key(a, original))
                    .unwrap_or(original)
            } else {
                let added = self.overrides.get(position - self.original.len())?;
                *index += 1;
                if self.original.iter().any(|a| same_key(a, added)) {
                    continue;
                }
                added
            };
            // Live frame output always replaces these values, even when a
            // source-attribute filter would exclude them. Ordinary attributes
            // are filtered after overrides in both traversal paths.
            if embedded && EmbeddedAttribute::for_attribute(attribute).is_some()
                || self
                    .filter
                    .is_none_or(|p| p.include_attribute(self.owner, attribute))
            {
                return Some(attribute);
            }
        }
    }
}

fn same_key(a: &Attribute, b: &Attribute) -> bool {
    a.namespace() == b.namespace() && a.local_name() == b.local_name()
}
