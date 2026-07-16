//! Shared script classification policy for parser and renderer code.
//!
//! This crate owns the renderer-neutral rules for script kind/mode decisions
//! so parser planning and runtime scheduling agree without copying
//! classification logic.

mod classify;
mod nonce;
mod scheduling;

pub use classify::{
    classify_script_element, classify_script_kind, classify_script_mode,
    classify_script_preparation, html_script_element_supports_type,
};
pub use nonce::script_element_nonce_is_nonceable;
pub use scheduling::{
    ScriptElementClassification, ScriptElementClassificationInput, ScriptPreparationClassification,
    ScriptPreparationClassificationInput, ScriptPreparationDisposition, ScriptSchedulingInput,
};

#[cfg(test)]
mod tests {
    use moli_page_types::{ScriptKind, ScriptMode, ScriptSourceKind};

    use crate::{
        ScriptElementClassificationInput, ScriptPreparationClassificationInput,
        ScriptPreparationDisposition, ScriptSchedulingInput, classify_script_element,
        classify_script_kind, classify_script_mode, classify_script_preparation,
        html_script_element_supports_type,
    };

    #[test]
    fn classifies_module_and_dynamic_external_classic_modes() {
        assert_eq!(
            classify_script_mode(ScriptSchedulingInput {
                parser_inserted: true,
                allow_parser_blocking_modes: true,
                force_async: false,
                async_attribute_present: false,
                defer_attribute_present: false,
                kind: ScriptKind::Module,
                source_kind: ScriptSourceKind::External,
            }),
            Some(ScriptMode::ModuleDefer)
        );
        assert_eq!(
            classify_script_mode(ScriptSchedulingInput {
                parser_inserted: false,
                allow_parser_blocking_modes: false,
                force_async: false,
                async_attribute_present: false,
                defer_attribute_present: false,
                kind: ScriptKind::Classic,
                source_kind: ScriptSourceKind::External,
            }),
            Some(ScriptMode::InOrder)
        );
    }

    #[test]
    fn classifies_script_type_attribute_values() {
        assert_eq!(classify_script_kind(None), ScriptKind::Classic);
        assert_eq!(classify_script_kind(Some("")), ScriptKind::Classic);
        assert_eq!(classify_script_kind(Some(" ")), ScriptKind::DataBlock);
        assert_eq!(
            classify_script_kind(Some("text/javascript")),
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_kind(Some(" text/javascript\t")),
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_kind(Some("text/javascript; charset=utf-8")),
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_kind(Some("application/x-javascript")),
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_kind(Some("TEXT/JAVASCRIPT1.5")),
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_kind(Some("javascript")),
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_kind(Some("text/javascript\u{000B}")),
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_kind(Some("text/javascript\u{00A0}")),
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_kind(Some("text/javascript1.6")),
            ScriptKind::DataBlock
        );
        assert_eq!(classify_script_kind(Some("module")), ScriptKind::Module);
        assert_eq!(
            classify_script_kind(Some("importmap")),
            ScriptKind::ImportMap
        );
        assert_eq!(
            classify_script_kind(Some("application/json")),
            ScriptKind::DataBlock
        );
    }

    #[test]
    fn html_script_element_supports_uses_exact_supported_type_tokens() {
        assert!(html_script_element_supports_type("classic"));
        assert!(html_script_element_supports_type("module"));
        assert!(html_script_element_supports_type("importmap"));

        for unsupported in [
            "",
            " ",
            " classic ",
            "module ",
            "Classic",
            "Module",
            "text/javascript",
            "application/javascript",
            "speculationrules",
            "unsupported",
        ] {
            assert!(
                !html_script_element_supports_type(unsupported),
                "{unsupported:?} should not be supported"
            );
        }
    }

    #[test]
    fn classifies_language_only_script_attribute_values() {
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some("javascript"),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some("JAVASCRIPT1.5"),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::Classic
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some(" javascript "),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some("javascript "),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some("javascript\t"),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: Some("javascript1.6"),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::DataBlock
        );
        assert_eq!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: Some("application/javascript"),
                language: Some("unknown"),
                event: None,
                for_attribute: None,
            })
            .kind,
            ScriptKind::Classic
        );
    }

    #[test]
    fn classifies_legacy_for_event_script_execution_gate() {
        assert!(
            !classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: None,
                event: Some(" onload() "),
                for_attribute: Some(" window "),
            })
            .legacy_event_for_mismatch
        );
        assert!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: None,
                event: Some("onclick"),
                for_attribute: Some("window"),
            })
            .legacy_event_for_mismatch
        );
        assert!(
            classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: None,
                event: Some("\u{a0}onload"),
                for_attribute: Some("window"),
            })
            .legacy_event_for_mismatch
        );
        assert!(
            !classify_script_element(ScriptElementClassificationInput {
                script_type: None,
                language: None,
                event: Some("handler"),
                for_attribute: None,
            })
            .legacy_event_for_mismatch
        );
    }

    #[test]
    fn preparation_classification_combines_kind_and_scheduling_policy() {
        let classification = classify_script_preparation(ScriptPreparationClassificationInput {
            element: ScriptElementClassificationInput {
                script_type: Some("module"),
                language: None,
                event: None,
                for_attribute: None,
            },
            parser_inserted: true,
            allow_parser_blocking_modes: true,
            force_async: false,
            async_attribute_present: false,
            defer_attribute_present: false,
            source_kind: ScriptSourceKind::External,
        });

        assert_eq!(
            classification.disposition,
            ScriptPreparationDisposition::Module(ScriptMode::ModuleDefer)
        );
        assert!(!classification.legacy_event_for_mismatch);
    }

    #[test]
    fn non_executable_script_kinds_do_not_receive_scheduling_modes() {
        for (script_type, expected) in [
            ("importmap", ScriptPreparationDisposition::ImportMap),
            ("application/json", ScriptPreparationDisposition::DataBlock),
        ] {
            let classification =
                classify_script_preparation(ScriptPreparationClassificationInput {
                    element: ScriptElementClassificationInput {
                        script_type: Some(script_type),
                        language: None,
                        event: None,
                        for_attribute: None,
                    },
                    parser_inserted: true,
                    allow_parser_blocking_modes: true,
                    force_async: false,
                    async_attribute_present: true,
                    defer_attribute_present: true,
                    source_kind: ScriptSourceKind::External,
                });

            assert_eq!(classification.disposition, expected);
        }
    }
}
