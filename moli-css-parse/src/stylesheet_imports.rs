use std::{collections::HashSet, sync::atomic::AtomicBool};

use cssparser::SourceLocation;
use style::{
    context::QuirksMode,
    media_queries::MediaList,
    servo_arc::Arc,
    shared_lock::{Locked, SharedRwLock},
    stylesheets::{
        AllowImportRules, CssRule, Origin, Stylesheet, StylesheetContents, StylesheetInDocument,
        StylesheetLoader, UrlExtraData,
        import_rule::{ImportLayer, ImportRule, ImportSheet, ImportSupportsCondition},
    },
    values::CssUrl,
};
use url::Url;

use crate::CssNativeStylesheet;

/// Accepts native import rules during parsing. The resource owner schedules
/// requests and replaces accepted placeholders with their response sheets.
pub struct StylesheetImportPlaceholderLoader;

impl StylesheetLoader for StylesheetImportPlaceholderLoader {
    fn request_stylesheet(
        &self,
        url: CssUrl,
        location: SourceLocation,
        lock: &SharedRwLock,
        media: Arc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer,
    ) -> Arc<Locked<ImportRule>> {
        let stylesheet = match url.url() {
            Some(base) if supports.as_ref().is_none_or(|condition| condition.enabled) => {
                let contents = StylesheetContents::from_str(
                    "",
                    UrlExtraData::from(base.as_ref().clone()),
                    Origin::Author,
                    lock,
                    None,
                    None,
                    QuirksMode::NoQuirks,
                    AllowImportRules::No,
                    None,
                );
                ImportSheet::new(Arc::new(Stylesheet {
                    contents: lock.wrap(contents),
                    shared_lock: lock.clone(),
                    media,
                    disabled: AtomicBool::new(false),
                }))
            }
            _ => ImportSheet::new_refused(),
        };
        Arc::new(lock.wrap(ImportRule {
            url,
            stylesheet,
            supports,
            layer,
            source_location: location,
        }))
    }
}

pub fn parse_detached_author_stylesheet_with_stylo(
    css_text: &str,
    base_url: &Url,
    quirks_mode: QuirksMode,
) -> CssNativeStylesheet {
    crate::stylo_stylesheet::ensure_stylo_cssom_parsing_prefs();
    let lock = SharedRwLock::new();
    let media = Arc::new(lock.wrap(MediaList::empty()));
    Arc::new(Stylesheet::from_str(
        css_text,
        UrlExtraData::from(base_url.clone()),
        Origin::Author,
        media,
        lock,
        Some(&StylesheetImportPlaceholderLoader),
        None,
        quirks_mode,
        AllowImportRules::Yes,
    ))
}

/// Copies a parsed response into the installing author's lock domain. Its
/// import placeholders have media lists of their own, so they must move with
/// the rules rather than retaining locks from the background parse.
pub fn clone_detached_stylesheet_contents_with_stylo(
    stylesheet: &Stylesheet,
    lock: &SharedRwLock,
) -> Arc<StylesheetContents> {
    let (contents, imports) = {
        let guard = stylesheet.shared_lock.read();
        let contents = stylesheet.contents(&guard);
        let imports = native_stylesheet_import_rules(contents.rules(&guard))
            .filter_map(|(index, rule)| {
                let rule = rule.read_with(&guard);
                let child = rule.stylesheet.as_sheet()?;
                let child_contents = child.contents(&guard);
                debug_assert!(child_contents.rules(&guard).is_empty());
                let child = Stylesheet {
                    contents: lock.wrap(child_contents.deep_clone(lock, None, &guard)),
                    shared_lock: lock.clone(),
                    media: Arc::new(lock.wrap(child.media.read_with(&guard).clone())),
                    disabled: AtomicBool::new(false),
                };
                Some((
                    index,
                    CssRule::Import(Arc::new(lock.wrap(ImportRule {
                        url: rule.url.clone(),
                        stylesheet: ImportSheet::new(Arc::new(child)),
                        supports: rule.supports.clone(),
                        layer: rule.layer.clone(),
                        source_location: rule.source_location,
                    }))),
                ))
            })
            .collect::<Vec<_>>();
        (contents.deep_clone(lock, None, &guard), imports)
    };
    if !imports.is_empty() {
        let mut guard = lock.write();
        let rules = &mut contents.rules.write_with(&mut guard).0;
        for (index, rule) in imports {
            rules[index] = rule;
        }
    }
    contents
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StylesheetImports {
    pub has_import_rules: bool,
    pub has_invalid_url: bool,
    pub urls: Vec<Url>,
}

/// Stylo keeps imports in the initial rule sequence, alongside layer statements.
pub fn native_stylesheet_import_rules(
    rules: &[CssRule],
) -> impl Iterator<Item = (usize, &Arc<Locked<ImportRule>>)> {
    rules
        .iter()
        .take_while(|rule| matches!(rule, CssRule::Import(_) | CssRule::LayerStatement(_)))
        .enumerate()
        .filter_map(|(index, rule)| match rule {
            CssRule::Import(rule) => Some((index, rule)),
            _ => None,
        })
}

/// Discover requests from Stylo's top-level import rules. Keep the first URL
/// for each fragment-independent fetch identity in source order.
pub fn stylesheet_imports_from_native_with_stylo(stylesheet: &Stylesheet) -> StylesheetImports {
    let guard = stylesheet.shared_lock.read();
    let mut imports = StylesheetImports::default();
    let mut identities = HashSet::new();
    for (_, rule) in native_stylesheet_import_rules(stylesheet.contents(&guard).rules(&guard)) {
        imports.has_import_rules = true;
        let rule = rule.read_with(&guard);
        if rule
            .supports
            .as_ref()
            .is_some_and(|condition| !condition.enabled)
        {
            continue;
        }
        let Some(url) = rule.url.url() else {
            imports.has_invalid_url = true;
            continue;
        };
        if matches!(rule.stylesheet, ImportSheet::Refused) {
            continue;
        }
        let mut identity = url.as_ref().clone();
        identity.set_fragment(None);
        if identities.insert(identity) {
            imports.urls.push(url.as_ref().clone());
        }
    }
    imports
}

pub fn stylesheet_import_urls_with_stylo(css_text: &str, base_url: &Url) -> Vec<Url> {
    let stylesheet =
        parse_detached_author_stylesheet_with_stylo(css_text, base_url, QuirksMode::NoQuirks);
    stylesheet_imports_from_native_with_stylo(&stylesheet).urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_native_imports_in_rule_order_and_resolves_the_parser_base() {
        let base = Url::parse("https://example.test/processing/").unwrap();
        let urls = stylesheet_import_urls_with_stylo(
            r#"@charset "utf-8";
               @layer reset, theme;
               @IM\50 ORT "root.css#a" layer(theme);
               @import url("root.css#b");
               @import url("../other.css") print;
               .target { content: '@import "literal.css";' }
               @import url("too-late.css");
               @media screen { @import url("nested.css"); }"#,
            &base,
        );
        assert_eq!(
            urls,
            [
                base.join("root.css#a").unwrap(),
                base.join("../other.css").unwrap(),
            ]
        );
    }

    #[test]
    fn native_imports_distinguish_refused_supports_and_invalid_urls() {
        let base = Url::parse("https://example.test/").unwrap();
        let parse_imports =
            |css| {
                stylesheet_imports_from_native_with_stylo(
                    &parse_detached_author_stylesheet_with_stylo(css, &base, QuirksMode::NoQuirks),
                )
            };
        assert_eq!(
            parse_imports(r#"@import "http://[" supports(unknown-property: impossible);"#),
            StylesheetImports {
                has_import_rules: true,
                ..StylesheetImports::default()
            }
        );
        assert_eq!(
            parse_imports(r#"@import "http://["; @import "valid.css" supports(display: block);"#),
            StylesheetImports {
                has_import_rules: true,
                has_invalid_url: true,
                urls: vec![base.join("valid.css").unwrap()],
            }
        );
    }
}
