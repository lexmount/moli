use moli_layout::LayoutPseudo;
use style::values::{computed::UserSelect, specified::ui::CursorKind};

use super::super::ScriptVm;
use crate::native_bridge::element::{
    InputSurfaceHit, StyleObservation, contenteditable_editing_host, is_text_control_user_editable,
};

impl ScriptVm {
    pub(crate) fn refresh_viewport_cursor(
        &self,
        (x, y): (f64, f64),
    ) -> anyhow::Result<(CursorKind, bool)> {
        let hit = self.mouse_input_surface_hit(x, y, "mousemove")?;
        let hover_target = hit.cursor_source.map(|(source, _)| source).or(hit.target);
        let hover_changed = self
            ._context_host
            .borrow_mut()
            .set_hovered_element_for_input(hover_target);
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
        let (source, pseudo) = hit.cursor_source.unwrap_or((target, None));
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
            CursorKind::Auto => {
                let selectable_text = hit.is_text && {
                    let mut select = style.get_ui().user_select;
                    // Generated ::before and ::after text resolves user-select:auto to none.
                    if select == UserSelect::Auto
                        && matches!(pseudo, Some(LayoutPseudo::Before | LayoutPseudo::After))
                    {
                        select = UserSelect::None;
                    }
                    // Other auto values follow the flat-tree parent.
                    let mut parent = if pseudo.is_some() {
                        Some(style_owner)
                    } else {
                        dom.flat_tree_parent(style_owner)
                    };
                    while select == UserSelect::Auto {
                        let Some(element) = parent.filter(|&node| dom.element(node).is_some())
                        else {
                            break;
                        };
                        let Some(style) = styles.read(element).computed_values() else {
                            break;
                        };
                        select = style.get_ui().user_select;
                        parent = dom.flat_tree_parent(element);
                    }
                    select != UserSelect::None
                };
                if selectable_text
                    || is_text_control_user_editable(&host, target)
                    || contenteditable_editing_host(&host, target).is_some()
                {
                    text_cursor
                } else {
                    CursorKind::Default
                }
            }
            keyword => keyword,
        }
    }
}
