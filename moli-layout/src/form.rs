// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{borrow::Cow, fmt::Debug, hash::Hash, ops::Range, sync::Arc};

use taffy::Size;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    LayoutAnonymousReason, LayoutAnonymousStyleContext, LayoutBoxKind, LayoutDisplay,
    LayoutElementCategory, LayoutElementSemantics, LayoutError, LayoutFormControlKind,
    LayoutInputControlKind, LayoutStyleResolver, LayoutTextSelection, LayoutWorld,
    ResolvedLayoutStyle, TextTransform, TextTransformState, replaced::ReplacedContext,
};

pub(crate) fn prepare_form_controls<N, R>(
    world: &mut LayoutWorld<N>,
    styles: &mut R,
) -> Result<(), LayoutError>
where
    N: Copy + Debug + Eq + Hash,
    R: LayoutStyleResolver<N>,
{
    world.password_offsets.clear();
    let controls = (0..world.boxes.len())
        .map(|index| world.boxes.id_at(index))
        .filter_map(|id| {
            let control = &world.boxes[id];
            let mut password_offsets = PasswordOffsetMap::default();
            form_control_texts(
                &control.element_semantics,
                control.style.text_transform(),
                control.text_selection,
                &mut password_offsets,
            )
            .map(|(texts, placeholder, selection)| {
                (id, texts, placeholder, selection, password_offsets)
            })
        })
        .filter(|(_, texts, _, selection, _)| !texts.is_empty() || selection.is_some())
        .collect::<Vec<_>>();
    for (control, mut texts, placeholder, selection, password_offsets) in controls {
        let Some(owner) = world.boxes[control].source else {
            continue;
        };
        if !password_offsets.0.is_empty() {
            world.password_offsets.insert(owner, password_offsets);
        }
        if world.boxes[control]
            .element_semantics
            .as_ref()
            .is_some_and(LayoutElementSemantics::is_select_listbox)
        {
            world.boxes[control].style.clip_form_control_contents();
        }
        let owner_label = world.boxes[control].source_label.clone();
        let placeholder_style = if placeholder {
            styles.placeholder_style(owner)?.map(|style| {
                let mut style = style.into_style();
                style.force_layout_display(LayoutDisplay::Block);
                style
            })
        } else {
            None
        };
        let editor_style = styles.anonymous_style(
            LayoutAnonymousStyleContext::from_parent(&world.boxes[control])
                .expect("form control has a source owner"),
            LayoutDisplay::Block,
        )?;
        if texts.is_empty() && selection.is_some() {
            texts.push(Arc::from(""));
        }
        // Blink keeps the placeholder and inner editor as siblings. The
        // placeholder precedes the empty editor, which retains its own
        // font metrics and selection outside the placeholder's effect layers.
        // Both remain descendants of the control's effects and clipping.
        let contents = texts
            .into_iter()
            .map(|text| (text, placeholder))
            .chain(placeholder.then(|| (Arc::from(""), false)));
        for (row, (text, is_placeholder)) in contents.enumerate() {
            let wrapper_style = if is_placeholder {
                placeholder_style.as_ref().unwrap_or(&editor_style)
            } else {
                &editor_style
            };
            let text_style = ResolvedLayoutStyle::text_leaf_from(wrapper_style);
            let selection = if is_placeholder { None } else { selection };
            let mut wrapper = LayoutWorld::new_box(
                None,
                Some(owner),
                None,
                format!("form-content({owner_label})[{row}]"),
                Some(owner_label.clone()),
                None,
                Some(LayoutAnonymousReason::FormControlContent),
                LayoutBoxKind::AnonymousBlock,
                wrapper_style.clone(),
                None,
            );
            wrapper.inline_formatting_context = true;
            wrapper.text_selection = selection;
            let wrapper_id = world.allocate(wrapper);
            let text_id = world.allocate(LayoutWorld::new_box(
                None,
                Some(owner),
                None,
                format!("form-content({owner_label})[{row}]::text"),
                Some(owner_label.clone()),
                None,
                Some(LayoutAnonymousReason::FormControlContent),
                LayoutBoxKind::Text,
                text_style.clone(),
                Some(text),
            ));
            world.boxes[text_id].text_selection = selection;
            world.append_synthesized_child(wrapper_id, text_id)?;
            world.append_synthesized_child(control, wrapper_id)?;
        }
    }
    Ok(())
}

fn form_control_texts(
    semantics: &Option<LayoutElementSemantics>,
    text_transform: TextTransform,
    selection: Option<LayoutTextSelection>,
    password_offsets: &mut PasswordOffsetMap,
) -> Option<(Vec<Arc<str>>, bool, Option<LayoutTextSelection>)> {
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
    if kind == LayoutFormControlKind::Select {
        return Some((select_control_texts(&data), false, selection));
    }
    let (text, placeholder, selection) = form_control_text_with_selection(
        kind,
        &data.value,
        &data.placeholder,
        text_transform,
        selection,
        Some(password_offsets),
    );
    Some((
        if text.is_empty() {
            Vec::new()
        } else {
            vec![text.into()]
        },
        placeholder,
        selection,
    ))
}

/// Text rows actually displayed by a select, independently of option DOM text.
pub fn select_control_texts(data: &crate::LayoutFormControlData) -> Vec<Arc<str>> {
    if data.select_rows() > 1 {
        data.option_labels.clone()
    } else {
        let (text, _) = form_control_text(
            LayoutFormControlKind::Select,
            &data.value,
            &data.placeholder,
            TextTransform::None,
        );
        if text.is_empty() {
            Vec::new()
        } else {
            vec![text.into()]
        }
    }
}

/// Visible text and placeholder selection shared by resource discovery and
/// control rendering. Password values use `text_transform` before masking.
pub fn form_control_text(
    kind: LayoutFormControlKind,
    value: &str,
    placeholder_text: &str,
    text_transform: TextTransform,
) -> (String, bool) {
    let (text, placeholder, _) =
        form_control_text_with_selection(kind, value, placeholder_text, text_transform, None, None);
    (text, placeholder)
}

fn form_control_text_with_selection(
    kind: LayoutFormControlKind,
    value: &str,
    placeholder_text: &str,
    text_transform: TextTransform,
    selection: Option<LayoutTextSelection>,
    password_offsets: Option<&mut PasswordOffsetMap>,
) -> (String, bool, Option<LayoutTextSelection>) {
    if kind == LayoutFormControlKind::Input(LayoutInputControlKind::Password) && !value.is_empty() {
        let (text, offsets) = mask_password(
            value,
            text_transform,
            password_offsets.is_some() || selection.is_some(),
        );
        let selection = selection.and_then(|selection| offsets.project_selection(selection));
        if let Some(password_offsets) = password_offsets {
            *password_offsets = offsets;
        }
        return (text, false, selection);
    }
    let placeholder = value.is_empty()
        && !placeholder_text.is_empty()
        && matches!(
            kind,
            LayoutFormControlKind::TextArea
                | LayoutFormControlKind::Input(
                    LayoutInputControlKind::Text
                        | LayoutInputControlKind::Search
                        | LayoutInputControlKind::Email
                        | LayoutInputControlKind::Telephone
                        | LayoutInputControlKind::Password
                        | LayoutInputControlKind::Url
                        | LayoutInputControlKind::Number
                )
        );
    let text = match kind {
        LayoutFormControlKind::Input(input) => match input {
            LayoutInputControlKind::Button => value.to_string(),
            LayoutInputControlKind::Reset => {
                if value.is_empty() {
                    "Reset".to_owned()
                } else {
                    value.to_string()
                }
            }
            LayoutInputControlKind::Submit => {
                if value.is_empty() {
                    "Submit".to_owned()
                } else {
                    value.to_string()
                }
            }
            LayoutInputControlKind::File => {
                if value.is_empty() {
                    "Choose File".to_owned()
                } else {
                    value.to_string()
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
                if value.is_empty() {
                    placeholder_text.to_string()
                } else {
                    value.to_string()
                }
            }
        },
        LayoutFormControlKind::TextArea | LayoutFormControlKind::Select => {
            if value.is_empty() {
                placeholder_text.to_string()
            } else {
                value.to_string()
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
    (text, placeholder, selection)
}

/// The original UTF-16 range covered by each displayed bullet. Case
/// expansion can associate several bullets with one source character.
#[derive(Debug, Default)]
pub(crate) struct PasswordOffsetMap(Vec<Range<usize>>);

impl PasswordOffsetMap {
    pub(crate) fn source_offset(&self, display_offset: usize) -> usize {
        display_offset
            .min(self.0.len())
            .checked_sub(1)
            .map_or(0, |index| self.0[index].end)
    }

    fn project_selection(&self, selection: LayoutTextSelection) -> Option<LayoutTextSelection> {
        let display_offset = |offset| self.0.partition_point(|range| range.start < offset);
        let masked = LayoutTextSelection::new(
            display_offset(selection.start),
            display_offset(selection.end),
        );
        (selection.is_caret() || !masked.is_caret()).then_some(masked)
    }
}

fn mask_password(
    value: &str,
    text_transform: TextTransform,
    with_offsets: bool,
) -> (String, PasswordOffsetMap) {
    // Record where each source character ends during case conversion.
    let mut character_ends = Vec::new();
    let display = if text_transform == TextTransform::None {
        Cow::Borrowed(value)
    } else {
        let mut transform = TextTransformState::default();
        let mut transformed = String::with_capacity(value.len());
        let mut source_end = 0;
        for character in value.chars() {
            transformed.extend(transform.transform(character, text_transform));
            if with_offsets {
                source_end += character.len_utf16();
                character_ends.push((transformed.len(), source_end));
            }
        }
        Cow::Owned(transformed)
    };

    let mut text = String::with_capacity(display.len());
    let mut offsets = PasswordOffsetMap::default();
    let mut character = 0;
    let mut source_end = 0;
    for (start, grapheme) in display.grapheme_indices(true) {
        text.push('\u{2022}');
        if !with_offsets {
            continue;
        }
        let source_start = if character_ends.is_empty() {
            let start = source_end;
            source_end += grapheme.encode_utf16().count();
            start
        } else {
            while character_ends[character].0 <= start {
                character += 1;
            }
            let source_start = character
                .checked_sub(1)
                .map_or(0, |index| character_ends[index].1);
            let end = start + grapheme.len();
            while character_ends[character].0 < end {
                character += 1;
            }
            source_end = character_ends[character].1;
            source_start
        };
        offsets.0.push(source_start..source_end);
    }
    (text, offsets)
}

pub(crate) fn select_row_height(font_size: f32, line_height: f32) -> f32 {
    line_height.max(font_size) + 5.0
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
            let rows = data.select_rows();
            let listbox = rows > 1;
            let characters = data
                .maximum_option_characters
                .max(u16::try_from(data.value.chars().count()).unwrap_or(u16::MAX))
                .max(1);
            Size {
                width: text_width(usize::from(characters)) + if listbox { 4.0 } else { 18.0 },
                height: f32::from(rows)
                    * if listbox {
                        select_row_height(font_size, line_height)
                    } else {
                        single_line_height
                    },
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
