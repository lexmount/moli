/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The [`@import`][import] at-rule.
//!
//! [import]: https://drafts.csswg.org/css-cascade-3/#at-import

use crate::media_queries::MediaList;
use crate::parser::{Parse, ParserContext};
use crate::shared_lock::{DeepCloneWithLock, SharedRwLock, SharedRwLockReadGuard, ToCssWithGuard};
use crate::stylesheets::{
    layer_rule::LayerName,
    supports_rule::{parse_condition_or_declaration, SupportsCondition},
    CssRule, CssRuleType, StylesheetInDocument,
};
use crate::values::CssUrl;
use cssparser::{Parser, SourceLocation};
use std::fmt::{self, Write};
use style_traits::{CssStringWriter, CssWriter, ParseError, ToCss};
use to_shmem::{SharedMemoryBuilder, ToShmem};

#[cfg(feature = "gecko")]
type StyleSheet = crate::gecko::data::GeckoStyleSheet;
#[cfg(feature = "servo")]
type StyleSheet = ::servo_arc::Arc<crate::stylesheets::Stylesheet>;

/// A sheet that is held from an import rule.
#[derive(Debug)]
pub enum ImportSheet {
    /// A bonafide stylesheet.
    Sheet(StyleSheet),

    /// An @import created while parsing off-main-thread, whose Gecko sheet has
    /// yet to be created and attached.
    Pending,

    /// An @import created with a false <supports-condition>, so will never be fetched.
    Refused,
}

impl ImportSheet {
    /// Creates a new ImportSheet from a stylesheet.
    pub fn new(sheet: StyleSheet) -> Self {
        ImportSheet::Sheet(sheet)
    }

    /// Creates a pending ImportSheet for a load that has not started yet.
    pub fn new_pending() -> Self {
        ImportSheet::Pending
    }

    /// Creates a refused ImportSheet for a load that will not happen.
    pub fn new_refused() -> Self {
        ImportSheet::Refused
    }

    /// Returns a reference to the stylesheet in this ImportSheet, if it exists.
    pub fn as_sheet(&self) -> Option<&StyleSheet> {
        match *self {
            #[cfg(feature = "gecko")]
            ImportSheet::Sheet(ref s) => {
                debug_assert!(!s.hack_is_null());
                if s.hack_is_null() {
                    return None;
                }
                Some(s)
            },
            #[cfg(feature = "servo")]
            ImportSheet::Sheet(ref s) => Some(s),
            ImportSheet::Refused | ImportSheet::Pending => None,
        }
    }

    /// Returns the media list for this import rule.
    pub fn media<'a>(&'a self, guard: &'a SharedRwLockReadGuard) -> Option<&'a MediaList> {
        self.as_sheet().and_then(|s| s.media(guard))
    }

    /// Returns the rule list for this import rule.
    pub fn rules<'a>(&'a self, guard: &'a SharedRwLockReadGuard) -> &'a [CssRule] {
        match self.as_sheet() {
            Some(s) => s.contents(guard).rules(guard),
            None => &[],
        }
    }
}

impl DeepCloneWithLock for ImportSheet {
    fn deep_clone_with_lock(&self, _lock: &SharedRwLock, _guard: &SharedRwLockReadGuard) -> Self {
        match *self {
            #[cfg(feature = "gecko")]
            ImportSheet::Sheet(ref s) => {
                use crate::gecko_bindings::bindings;
                let clone = unsafe { bindings::Gecko_StyleSheet_Clone(s.raw() as *const _) };
                ImportSheet::Sheet(unsafe { StyleSheet::from_addrefed(clone) })
            },
            #[cfg(feature = "servo")]
            ImportSheet::Sheet(ref s) => {
                use servo_arc::Arc;
                ImportSheet::Sheet(Arc::new((&**s).clone()))
            },
            ImportSheet::Pending => ImportSheet::Pending,
            ImportSheet::Refused => ImportSheet::Refused,
        }
    }
}

/// The layer specified in an import rule (can be none, anonymous, or named).
#[derive(Debug, Clone)]
pub enum ImportLayer {
    /// No layer specified
    None,

    /// Anonymous layer (`layer`)
    Anonymous,

    /// Named layer (`layer(name)`)
    Named(LayerName),
}

/// The supports condition in an import rule.
#[derive(Debug, Clone)]
pub struct ImportSupportsCondition {
    /// The supports condition.
    pub condition: SupportsCondition,

    /// If the import is enabled, from the result of the import condition.
    pub enabled: bool,
}

impl ToCss for ImportLayer {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match *self {
            ImportLayer::None => Ok(()),
            ImportLayer::Anonymous => dest.write_str("layer"),
            ImportLayer::Named(ref name) => {
                dest.write_str("layer(")?;
                name.to_css(dest)?;
                dest.write_char(')')
            },
        }
    }
}

/// The [`@import`][import] at-rule.
///
/// [import]: https://drafts.csswg.org/css-cascade-3/#at-import
#[derive(Debug)]
pub struct ImportRule {
    /// The `<url>` this `@import` rule is loading.
    pub url: CssUrl,

    /// The stylesheet is always present. However, in the case of gecko async
    /// parsing, we don't actually have a Gecko sheet at first, and so the
    /// ImportSheet just has stub behavior until it appears.
    pub stylesheet: ImportSheet,

    /// A <supports-condition> for the rule.
    pub supports: Option<ImportSupportsCondition>,

    /// A `layer()` function name.
    pub layer: ImportLayer,

    /// The line and column of the rule's source code.
    pub source_location: SourceLocation,
}

impl ImportRule {
    /// Parses the layer() / layer / supports() part of the import header, as per
    /// https://drafts.csswg.org/css-cascade-5/#at-import:
    ///
    ///     [ layer | layer(<layer-name>) ]?
    ///     [ supports([ <supports-condition> | <declaration> ]) ]?
    ///
    /// We do this here so that the import preloader can look at this without having to parse the
    /// whole import rule or parse the media query list or what not.
    pub fn parse_layer_and_supports<'i, 't>(
        input: &mut Parser<'i, 't>,
        context: &mut ParserContext,
    ) -> Result<(ImportLayer, Option<ImportSupportsCondition>), ParseError<'i>> {
        let layer = if input
            .try_parse(|input| input.expect_ident_matching("layer"))
            .is_ok()
        {
            ImportLayer::Anonymous
        } else {
            input
                .try_parse(|input| {
                    input.expect_function_matching("layer")?;
                    input
                        .parse_nested_block(|input| LayerName::parse(context, input))
                        .map(|name| ImportLayer::Named(name))
                })
                .ok()
                .unwrap_or(ImportLayer::None)
        };

        let supports = if input
            .try_parse(|input| input.expect_function_matching("supports"))
            .is_ok()
        {
            let condition = input.parse_nested_block(parse_condition_or_declaration)?;
            let enabled =
                context.nest_for_rule(CssRuleType::Style, |context| condition.eval(context));
            Some(ImportSupportsCondition { condition, enabled })
        } else {
            None
        };

        Ok((layer, supports))
    }
}

impl ToShmem for ImportRule {
    fn to_shmem(&self, _builder: &mut SharedMemoryBuilder) -> to_shmem::Result<Self> {
        Err(String::from(
            "ToShmem failed for ImportRule: cannot handle imported style sheets",
        ))
    }
}

impl DeepCloneWithLock for ImportRule {
    fn deep_clone_with_lock(&self, lock: &SharedRwLock, guard: &SharedRwLockReadGuard) -> Self {
        ImportRule {
            url: self.url.clone(),
            stylesheet: self.stylesheet.deep_clone_with_lock(lock, guard),
            supports: self.supports.clone(),
            layer: self.layer.clone(),
            source_location: self.source_location.clone(),
        }
    }
}

impl ToCssWithGuard for ImportRule {
    fn to_css(&self, guard: &SharedRwLockReadGuard, dest: &mut CssStringWriter) -> fmt::Result {
        dest.write_str("@import ")?;
        self.url.to_css(&mut CssWriter::new(dest))?;

        if !matches!(self.layer, ImportLayer::None) {
            dest.write_char(' ')?;
            self.layer.to_css(&mut CssWriter::new(dest))?;
        }

        if let Some(ref supports) = self.supports {
            dest.write_str(" supports(")?;
            supports.condition.to_css(&mut CssWriter::new(dest))?;
            dest.write_char(')')?;
        }

        if let Some(media) = self.stylesheet.media(guard) {
            if !media.is_empty() {
                dest.write_char(' ')?;
                media.to_css(&mut CssWriter::new(dest))?;
            }
        }

        dest.write_char(';')
    }
}
