use moli_layout::LayoutPseudo;
use style::values::{computed::UserSelect, specified::ui::CursorKind};

use super::super::ScriptVm;
use crate::document_runtime::DomHandle;
use crate::native_bridge::element::{
    InputSurfaceHit, StyleObservation, contenteditable_editing_host, is_text_control_user_editable,
};

impl ScriptVm {
    pub(crate) fn refresh_viewport_pointer(
        &self,
        (x, y): (f64, f64),
    ) -> anyhow::Result<(CursorKind, bool)> {
        let hit = self.mouse_input_surface_hit(x, y, "mousemove")?;
        let hover_changed = self
            ._context_host
            .borrow_mut()
            .set_hovered_element_for_input(hit.hover_source());
        Ok((self.cursor_for_viewport_hit(hit), hover_changed))
    }

    pub(crate) fn clear_viewport_hover(&self) {
        self._context_host
            .borrow_mut()
            .set_hovered_element_for_input(None);
    }

    fn cursor_for_viewport_hit(&self, hit: InputSurfaceHit) -> CursorKind {
        let Some(target) = hit.target else {
            // Native scrollbars and the viewport outside the document use the arrow.
            return CursorKind::Default;
        };
        let host = self._context_host.borrow();
        let dom = host.dom_host();
        let (source, pseudo) = hit
            .painted_source
            .map_or((target, None), |source| (source.node, source.pseudo));
        let style_owner = if dom.element(source).is_some() {
            source
        } else {
            dom.flat_tree_parent(source).unwrap_or(target)
        };
        let mut styles = StyleObservation::new(&host);
        let read = styles.read(style_owner);
        let Some(style) = pseudo
            .and_then(|pseudo| read.pseudo_computed_values(pseudo.label().trim_start_matches(':')))
            .or_else(|| read.computed_values())
        else {
            return CursorKind::Default;
        };
        let text_cursor = if style.writing_mode.is_vertical() {
            CursorKind::VerticalText
        } else {
            CursorKind::Text
        };
        match style.get_inherited_ui().cursor.keyword {
            CursorKind::Text => text_cursor,
            CursorKind::Auto
                if auto_cursor_uses_text(
                    hit,
                    style_owner,
                    style.get_ui().user_select,
                    &mut styles,
                ) =>
            {
                text_cursor
            }
            CursorKind::Auto => CursorKind::Default,
            keyword => keyword,
        }
    }
}

fn auto_cursor_uses_text(
    hit: InputSurfaceHit,
    style_owner: DomHandle,
    mut select: UserSelect,
    styles: &mut StyleObservation<'_>,
) -> bool {
    let host = styles.runtime();
    let dom = host.dom_host();
    let selectable_text = hit.painted_source.is_some_and(|source| {
        // Generated ::before and ::after text resolves user-select:auto to none.
        if !source.is_text
            || (select == UserSelect::Auto
                && matches!(
                    source.pseudo,
                    Some(LayoutPseudo::Before | LayoutPseudo::After)
                ))
        {
            return false;
        }
        let mut parent = if source.pseudo.is_some() {
            Some(style_owner)
        } else {
            dom.flat_tree_parent(style_owner)
        };
        while select == UserSelect::Auto {
            let Some(element) = parent.filter(|&node| dom.element(node).is_some()) else {
                break;
            };
            let Some(style) = styles.read(element).computed_values() else {
                break;
            };
            select = style.get_ui().user_select;
            parent = dom.flat_tree_parent(element);
        }
        select != UserSelect::None
    });
    selectable_text
        || hit.target.is_some_and(|target| {
            is_text_control_user_editable(host, target)
                || contenteditable_editing_host(host, target).is_some()
        })
}
