//! Diagnostics for URL filters rejected by Stylo's Servo parser before paint.

use cssparser::{Parser, SourceLocation, Token};
use moli_layout::{PaintDiagnostic, PaintDiagnosticSeverity};
use style::{
    error_reporting::{ContextualParseError, ParseErrorReporter},
    stylesheets::UrlExtraData,
};

pub(crate) struct FilterParseDiagnostics<F> {
    emit: F,
}

pub(crate) const FILTER_PARSE_DIAGNOSTICS: FilterParseDiagnostics<fn(PaintDiagnostic)> =
    FilterParseDiagnostics {
        emit: log_diagnostic,
    };

fn log_diagnostic(diagnostic: PaintDiagnostic) {
    tracing::warn!(target: "moli_paint", code = %diagnostic.code, "{}", diagnostic.message);
}

impl<F: Fn(PaintDiagnostic)> ParseErrorReporter for FilterParseDiagnostics<F> {
    fn report_error(
        &self,
        _url: &UrlExtraData,
        _location: SourceLocation,
        error: ContextualParseError<'_>,
    ) {
        let ContextualParseError::UnsupportedPropertyDeclaration(declaration, ..) = error else {
            return;
        };
        let mut parser = Parser::new(declaration);
        let Ok(name) = parser.expect_ident_cloned() else {
            return;
        };
        let code = if name.eq_ignore_ascii_case("backdrop-filter")
            || name.eq_ignore_ascii_case("-webkit-backdrop-filter")
        {
            "backdrop-filter-url-reference-unsupported"
        } else if name.eq_ignore_ascii_case("filter") {
            "filter-url-reference-unsupported"
        } else {
            return;
        };
        if parser.expect_colon().is_err() {
            return;
        }
        while let Ok(token) = parser.next() {
            let url = match token {
                Token::UnquotedUrl(_) => true,
                Token::Function(name) => name.eq_ignore_ascii_case("url"),
                _ => false,
            };
            if url {
                (self.emit)(PaintDiagnostic::new(
                    code,
                    format!(
                        "Stylo's Servo parser rejects URL-referenced {name}; the declaration is ignored"
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use style::{context::QuirksMode, properties::parse_style_attribute, stylesheets::CssRuleType};

    #[test]
    fn rejected_url_filters_emit_diagnostics_at_the_real_parse_boundary() {
        crate::style_engine::ensure_stylo_browser_compat_prefs();
        let diagnostics = RefCell::new(Vec::new());
        let reporter = FilterParseDiagnostics {
            emit: |diagnostic| diagnostics.borrow_mut().push(diagnostic),
        };
        let url = UrlExtraData::from(url::Url::parse("https://example.com/").unwrap());
        for (declaration, expected) in [
            (
                "backdrop-filter: blur(3px) url(#f) saturate(2)",
                Some("backdrop-filter-url-reference-unsupported"),
            ),
            (
                "backdrop-filter: URL(\"#f\")",
                Some("backdrop-filter-url-reference-unsupported"),
            ),
            ("filter: url(#f)", Some("filter-url-reference-unsupported")),
            ("backdrop-filter: blur(3px) drop-shadow(1px 1px red)", None),
            ("backdrop-filter: unknown(\"url(#f)\")", None),
            ("background-image: url(test.png)", None),
        ] {
            diagnostics.borrow_mut().clear();
            parse_style_attribute(
                declaration,
                &url,
                Some(&reporter),
                QuirksMode::NoQuirks,
                CssRuleType::Style,
            );
            let diagnostics = diagnostics.borrow();
            assert_eq!(
                diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.code.as_str()),
                expected,
                "{declaration}"
            );
            assert_eq!(diagnostics.len(), usize::from(expected.is_some()));
        }
    }
}
