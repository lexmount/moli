use moli_page_types::{ScriptKind, ScriptMode, ScriptSourceKind};

use super::scheduling::{
    ScriptElementClassification, ScriptElementClassificationInput, ScriptPreparationClassification,
    ScriptPreparationClassificationInput, ScriptPreparationDisposition, ScriptSchedulingInput,
};

pub fn classify_script_preparation(
    input: ScriptPreparationClassificationInput<'_>,
) -> ScriptPreparationClassification {
    let element = classify_script_element(input.element);
    let scheduling = ScriptSchedulingInput {
        parser_inserted: input.parser_inserted,
        allow_parser_blocking_modes: input.allow_parser_blocking_modes,
        force_async: input.force_async,
        async_attribute_present: input.async_attribute_present,
        defer_attribute_present: input.defer_attribute_present,
        kind: element.kind,
        source_kind: input.source_kind,
    };
    let disposition = match element.kind {
        ScriptKind::Classic => ScriptPreparationDisposition::Classic(
            classify_script_mode(scheduling).expect("classic scripts have an execution mode"),
        ),
        ScriptKind::Module => ScriptPreparationDisposition::Module(
            classify_script_mode(scheduling).expect("module scripts have an execution mode"),
        ),
        ScriptKind::ImportMap => ScriptPreparationDisposition::ImportMap,
        ScriptKind::DataBlock => ScriptPreparationDisposition::DataBlock,
    };
    ScriptPreparationClassification {
        disposition,
        legacy_event_for_mismatch: element.legacy_event_for_mismatch,
    }
}

pub fn classify_script_mode(input: ScriptSchedulingInput) -> Option<ScriptMode> {
    let force_async = input.force_async && !input.parser_inserted;

    let mode = match input.kind {
        ScriptKind::Module => {
            if input.async_attribute_present || force_async {
                ScriptMode::Async
            } else if input.parser_inserted && input.allow_parser_blocking_modes {
                ScriptMode::ModuleDefer
            } else {
                ScriptMode::ModuleInOrder
            }
        }
        ScriptKind::Classic => match input.source_kind {
            ScriptSourceKind::Inline => ScriptMode::Normal,
            ScriptSourceKind::External => {
                if input.async_attribute_present || force_async {
                    ScriptMode::Async
                } else if input.parser_inserted && input.allow_parser_blocking_modes {
                    if input.defer_attribute_present {
                        ScriptMode::Defer
                    } else {
                        ScriptMode::Normal
                    }
                } else {
                    ScriptMode::InOrder
                }
            }
        },
        ScriptKind::ImportMap | ScriptKind::DataBlock => return None,
    };
    Some(mode)
}

pub fn classify_script_element(
    input: ScriptElementClassificationInput<'_>,
) -> ScriptElementClassification {
    let kind = classify_script_kind_from_attributes(input.script_type, input.language);
    let legacy_event_for_mismatch = kind == ScriptKind::Classic
        && legacy_event_for_attributes_mismatch(input.for_attribute, input.event);

    ScriptElementClassification {
        kind,
        legacy_event_for_mismatch,
    }
}

pub fn classify_script_kind(script_type: Option<&str>) -> ScriptKind {
    classify_script_kind_from_type(script_type)
}

pub fn html_script_element_supports_type(script_type: &str) -> bool {
    matches!(script_type, "classic" | "module" | "importmap")
}

pub fn script_fetch_is_render_blocking(
    kind: ScriptKind,
    source_kind: ScriptSourceKind,
    parser_inserted: bool,
    has_async_or_defer: bool,
    blocking_attribute: Option<&str>,
    document_allows_render_blocking: bool,
) -> bool {
    let potentially_render_blocking =
        blocking_attribute.is_some_and(|value| {
            value
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("render"))
        }) || (kind == ScriptKind::Classic && parser_inserted && !has_async_or_defer);
    potentially_render_blocking
        && match source_kind {
            ScriptSourceKind::External => document_allows_render_blocking,
            // Inline modules can carry render-blocking options to their imports.
            ScriptSourceKind::Inline => kind == ScriptKind::Module,
        }
}

fn classify_script_kind_from_attributes(
    script_type: Option<&str>,
    language: Option<&str>,
) -> ScriptKind {
    if script_type.is_some() {
        return classify_script_kind_from_type(script_type);
    }

    classify_script_kind_from_language(language)
}

fn classify_script_kind_from_type(script_type: Option<&str>) -> ScriptKind {
    let Some(raw_script_type) = script_type else {
        return ScriptKind::Classic;
    };

    if raw_script_type.is_empty() {
        return ScriptKind::Classic;
    }

    let script_type = trim_ascii_whitespace(raw_script_type);
    if script_type.is_empty() {
        // Current WPT script-type-and-language-js.html expects a whitespace-only
        // type attribute not to execute, while a truly empty type remains classic.
        return ScriptKind::DataBlock;
    }

    if is_javascript_mime_essence_for_script_type(script_type) {
        return ScriptKind::Classic;
    }

    if script_type.eq_ignore_ascii_case("module") {
        return ScriptKind::Module;
    }

    if script_type.eq_ignore_ascii_case("importmap") {
        return ScriptKind::ImportMap;
    }

    ScriptKind::DataBlock
}

fn classify_script_kind_from_language(language: Option<&str>) -> ScriptKind {
    let Some(language) = language else {
        return ScriptKind::Classic;
    };
    if language.is_empty() {
        return ScriptKind::Classic;
    }
    let mime = format!("text/{language}");
    if is_javascript_mime_essence_exact(&mime) {
        ScriptKind::Classic
    } else {
        ScriptKind::DataBlock
    }
}

fn is_javascript_mime_essence_for_script_type(input: &str) -> bool {
    let essence = trim_ascii_whitespace(input).to_ascii_lowercase();
    moli_web_mime::is_javascript_mime_essence(&essence)
}

fn is_javascript_mime_essence_exact(input: &str) -> bool {
    moli_web_mime::is_javascript_mime_essence(&input.to_ascii_lowercase())
}

fn legacy_event_for_attributes_mismatch(for_attribute: Option<&str>, event: Option<&str>) -> bool {
    let (Some(for_attribute), Some(event)) = (for_attribute, event) else {
        return false;
    };

    let for_attribute = trim_ascii_whitespace(for_attribute);
    if !for_attribute.eq_ignore_ascii_case("window") {
        return true;
    }

    !matches!(
        trim_ascii_whitespace(event).to_ascii_lowercase().as_str(),
        "onload" | "onload()"
    )
}

fn trim_ascii_whitespace(value: &str) -> &str {
    value.trim_matches(|ch: char| matches!(ch, '\t' | '\n' | '\u{000C}' | '\r' | ' '))
}
