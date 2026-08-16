// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt::Debug, hash::Hash, sync::Arc};

use taffy::Size;

use crate::{
    LayoutAnonymousReason, LayoutBoxId, LayoutBoxKind, LayoutDisplay, LayoutElementCategory,
    LayoutElementSemantics, LayoutError, LayoutFormControlKind, LayoutInputControlKind,
    LayoutWorld, ResolvedLayoutStyle, replaced::ReplacedContext,
};

pub(crate) fn prepare_form_controls<N>(world: &mut LayoutWorld<N>) -> Result<(), LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let controls = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter_map(|id| {
            form_control_text(&world.boxes[id.index()].element_semantics).map(|text| (id, text))
        })
        .filter(|(_, text)| !text.is_empty())
        .collect::<Vec<_>>();
    for (control, text) in controls {
        let Some(owner) = world.boxes[control.index()].source else {
            continue;
        };
        let owner_label = world.boxes[control.index()].source_label.clone();
        let wrapper_style = ResolvedLayoutStyle::anonymous_from(
            &world.boxes[control.index()].style,
            LayoutDisplay::Block,
        );
        let text_style = ResolvedLayoutStyle::text_leaf_from(&wrapper_style);
        let mut wrapper = LayoutWorld::new_box(
            None,
            Some(owner),
            None,
            format!("form-content({owner_label})"),
            Some(owner_label.clone()),
            None,
            Some(LayoutAnonymousReason::FormControlContent),
            LayoutBoxKind::AnonymousBlock,
            wrapper_style,
            None,
        );
        wrapper.inline_formatting_context = true;
        let wrapper_id = world.allocate(wrapper);
        let text_id = world.allocate(LayoutWorld::new_box(
            None,
            Some(owner),
            None,
            format!("form-content({owner_label})::text"),
            Some(owner_label),
            None,
            Some(LayoutAnonymousReason::FormControlContent),
            LayoutBoxKind::Text,
            text_style,
            Some(Arc::from(text)),
        ));
        world.append_synthesized_child(wrapper_id, text_id)?;
        world.append_synthesized_child(control, wrapper_id)?;
    }
    Ok(())
}

fn form_control_text(semantics: &Option<LayoutElementSemantics>) -> Option<String> {
    let semantics = semantics.as_ref()?;
    let LayoutElementCategory::FormControl(kind) = semantics.category else {
        return None;
    };
    if !semantics.is_replaced() {
        return None;
    }
    let data = semantics
        .metadata
        .form_control
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let text = match kind {
        LayoutFormControlKind::Input(input) => match input {
            LayoutInputControlKind::Button => data.value.to_string(),
            LayoutInputControlKind::Reset => {
                if data.value.is_empty() {
                    "Reset".to_owned()
                } else {
                    data.value.to_string()
                }
            }
            LayoutInputControlKind::Submit => {
                if data.value.is_empty() {
                    "Submit".to_owned()
                } else {
                    data.value.to_string()
                }
            }
            LayoutInputControlKind::File => {
                if data.value.is_empty() {
                    "Choose File".to_owned()
                } else {
                    data.value.to_string()
                }
            }
            LayoutInputControlKind::Checkbox
            | LayoutInputControlKind::Color
            | LayoutInputControlKind::Hidden
            | LayoutInputControlKind::Image
            | LayoutInputControlKind::Radio
            | LayoutInputControlKind::Range => String::new(),
            LayoutInputControlKind::Date
            | LayoutInputControlKind::DateTimeLocal
            | LayoutInputControlKind::Email
            | LayoutInputControlKind::Month
            | LayoutInputControlKind::Number
            | LayoutInputControlKind::Password
            | LayoutInputControlKind::Search
            | LayoutInputControlKind::Telephone
            | LayoutInputControlKind::Text
            | LayoutInputControlKind::Time
            | LayoutInputControlKind::Url
            | LayoutInputControlKind::Week => {
                if data.value.is_empty() {
                    data.placeholder.to_string()
                } else {
                    data.value.to_string()
                }
            }
        },
        LayoutFormControlKind::TextArea | LayoutFormControlKind::Select => {
            if data.value.is_empty() {
                data.placeholder.to_string()
            } else {
                data.value.to_string()
            }
        }
        LayoutFormControlKind::Button
        | LayoutFormControlKind::Option
        | LayoutFormControlKind::OptionGroup
        | LayoutFormControlKind::FieldSet
        | LayoutFormControlKind::Legend
        | LayoutFormControlKind::Output
        | LayoutFormControlKind::Progress
        | LayoutFormControlKind::Meter => String::new(),
    };
    Some(text)
}

pub(crate) fn form_control_context(
    semantics: &LayoutElementSemantics,
    font_size: f32,
    line_height: f32,
) -> Option<ReplacedContext> {
    let LayoutElementCategory::FormControl(kind) = semantics.category else {
        return None;
    };
    let data = semantics
        .metadata
        .form_control
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let character_width = (font_size * 0.6).max(1.0);
    let single_line_height = line_height.max(font_size);
    let text_width = |characters: usize| characters as f32 * character_width;
    let value_characters = data.value.chars().count();
    let size = match kind {
        LayoutFormControlKind::Input(input) => match input {
            LayoutInputControlKind::Checkbox | LayoutInputControlKind::Radio => Size {
                width: 13.0,
                height: 13.0,
            },
            LayoutInputControlKind::Color => Size {
                width: 44.0,
                height: 23.0,
            },
            LayoutInputControlKind::Range => Size {
                width: 129.0,
                height: 20.0,
            },
            LayoutInputControlKind::Button
            | LayoutInputControlKind::Reset
            | LayoutInputControlKind::Submit => {
                let default_label = match input {
                    LayoutInputControlKind::Reset => "Reset",
                    LayoutInputControlKind::Submit => "Submit",
                    LayoutInputControlKind::Button => "",
                    _ => unreachable!(),
                };
                Size {
                    width: text_width(value_characters.max(default_label.len())).max(40.0),
                    height: single_line_height,
                }
            }
            LayoutInputControlKind::Image => Size {
                width: 300.0,
                height: 150.0,
            },
            LayoutInputControlKind::Hidden => Size::ZERO,
            LayoutInputControlKind::Date
            | LayoutInputControlKind::DateTimeLocal
            | LayoutInputControlKind::Month
            | LayoutInputControlKind::Time
            | LayoutInputControlKind::Week => Size {
                width: text_width(data.size.unwrap_or(14) as usize),
                height: single_line_height,
            },
            LayoutInputControlKind::File => Size {
                width: text_width(data.size.unwrap_or(20) as usize) + 72.0,
                height: single_line_height,
            },
            LayoutInputControlKind::Number => Size {
                width: text_width(data.size.unwrap_or(20) as usize) + 16.0,
                height: single_line_height,
            },
            LayoutInputControlKind::Email
            | LayoutInputControlKind::Password
            | LayoutInputControlKind::Search
            | LayoutInputControlKind::Telephone
            | LayoutInputControlKind::Text
            | LayoutInputControlKind::Url => Size {
                width: text_width(data.size.unwrap_or(20) as usize),
                height: single_line_height,
            },
        },
        LayoutFormControlKind::TextArea => Size {
            // Blink reserves a narrow editing/scrollbar gutter even when
            // author CSS removes the physical border and padding.
            width: text_width(usize::from(data.columns)) + 15.0,
            height: f32::from(data.rows) * line_height.max(font_size),
        },
        LayoutFormControlKind::Select => {
            let rows = data
                .size
                .unwrap_or(if data.multiple { 4 } else { 1 })
                .max(1);
            let listbox = data.multiple || rows > 1;
            let characters = data
                .maximum_option_characters
                .max(u16::try_from(data.value.chars().count()).unwrap_or(u16::MAX))
                .max(1);
            Size {
                width: text_width(usize::from(characters)) + if listbox { 4.0 } else { 18.0 },
                height: f32::from(rows)
                    * (line_height.max(font_size) + if listbox { 5.0 } else { 0.0 }),
            }
        }
        LayoutFormControlKind::Progress | LayoutFormControlKind::Meter => Size {
            width: 160.0,
            height: 16.0,
        },
        // These controls retain normal-flow children and are not replaced
        // leaves. Returning a context is harmless for callers that ask but
        // construction will not route them through replaced measurement.
        LayoutFormControlKind::Button
        | LayoutFormControlKind::Option
        | LayoutFormControlKind::OptionGroup
        | LayoutFormControlKind::FieldSet
        | LayoutFormControlKind::Legend
        | LayoutFormControlKind::Output => Size {
            width: text_width(value_characters).max(40.0),
            height: single_line_height,
        },
    };
    Some(ReplacedContext::form_control(size))
}
