#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BinaryOperator, CallExpression, Expression, ForOfStatement, FunctionBody,
    ImportExpression, MetaProperty, ObjectExpression, ObjectProperty, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, StaticMemberExpression, TemplateLiteral,
};
use oxc_ast_visit::{Visit, walk};
#[cfg(test)]
use oxc_parser::Parser as OxcParser;
#[cfg(test)]
use oxc_span::SourceType;
use oxc_span::Span;

use crate::types::{
    DynamicImportRewriteKind, DynamicImportRewriteSite, ModuleAstSpan,
    ModuleSpecialFormRewriteSites,
};

pub(crate) struct ModuleRewriteSites {
    pub special_forms: ModuleSpecialFormRewriteSites,
    pub top_level_awaits: Vec<ModuleAstSpan>,
}

#[cfg(test)]
const SPECIAL_FORM_EXPRESSION_PREFIX: &str = "const __lm_special_form_expr = ";
#[cfg(test)]
const SPECIAL_FORM_EXPRESSION_SUFFIX: &str = ";";
#[cfg(test)]
const SPECIAL_FORM_PARSE_ERROR: &str = "unsupported module special-form syntax";

#[cfg(test)]
pub(crate) fn collect_module_special_form_rewrite_sites(
    source: &str,
) -> std::result::Result<ModuleSpecialFormRewriteSites, String> {
    if !source.contains("import") || source_is_only_js_trivia_or_hashbang(source) {
        return Ok(ModuleSpecialFormRewriteSites::default());
    }
    match collect_module_special_form_rewrite_sites_with_offset(source, 0) {
        Ok(sites) => Ok(sites),
        Err(error) if error == SPECIAL_FORM_PARSE_ERROR => {
            let wrapped =
                format!("{SPECIAL_FORM_EXPRESSION_PREFIX}{source}{SPECIAL_FORM_EXPRESSION_SUFFIX}");
            collect_module_special_form_rewrite_sites_with_offset(
                &wrapped,
                SPECIAL_FORM_EXPRESSION_PREFIX.len() as isize,
            )
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn collect_module_special_form_rewrite_sites_with_offset(
    source: &str,
    offset: isize,
) -> std::result::Result<ModuleSpecialFormRewriteSites, String> {
    let allocator = Allocator::default();
    let parsed = OxcParser::new(&allocator, source, SourceType::mjs()).parse();
    if !parsed.errors.is_empty() {
        return Err(SPECIAL_FORM_PARSE_ERROR.to_owned());
    }

    let mut collector = ModuleRewriteSiteCollector::new(source, offset);
    collector.visit_program(&parsed.program);
    collector.finish().map(|sites| sites.special_forms)
}

pub(crate) fn collect_module_rewrite_sites_from_program(
    source: &str,
    program: &Program<'_>,
) -> std::result::Result<ModuleRewriteSites, String> {
    let mut collector = ModuleRewriteSiteCollector::new(source, 0);
    collector.visit_program(program);
    collector.finish()
}

#[cfg(test)]
fn source_is_only_js_trivia_or_hashbang(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0;

    if bytes.starts_with(b"#!") {
        index = skip_line(bytes, 2);
    }

    loop {
        index = skip_ascii_whitespace(bytes, index);
        if index >= bytes.len() {
            return true;
        }

        if bytes[index..].starts_with(b"//") {
            index = skip_line(bytes, index + 2);
            continue;
        }

        if bytes[index..].starts_with(b"/*") {
            let Some(end) = find_block_comment_end(bytes, index + 2) else {
                return false;
            };
            index = end;
            continue;
        }

        return false;
    }
}

#[cfg(test)]
fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
    {
        index += 1;
    }
    index
}

#[cfg(test)]
fn skip_line(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
        index += 1;
    }
    index
}

#[cfg(test)]
fn find_block_comment_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return Some(index + 2);
        }
        index += 1;
    }
    None
}

struct ModuleRewriteSiteCollector<'s> {
    source: &'s str,
    offset: isize,
    special_forms: ModuleSpecialFormRewriteSites,
    top_level_awaits: Vec<ModuleAstSpan>,
    function_depth: usize,
    errors: Vec<String>,
}

impl<'s> ModuleRewriteSiteCollector<'s> {
    fn new(source: &'s str, offset: isize) -> Self {
        Self {
            source,
            offset,
            special_forms: ModuleSpecialFormRewriteSites::default(),
            top_level_awaits: Vec::new(),
            function_depth: 0,
            errors: Vec::new(),
        }
    }

    fn finish(mut self) -> std::result::Result<ModuleRewriteSites, String> {
        if !self.errors.is_empty() {
            return Err(self.errors.remove(0));
        }
        self.special_forms
            .import_metas
            .sort_by_key(|span| (span.start, span.end));
        self.special_forms
            .import_meta_urls
            .sort_by_key(|span| (span.start, span.end));
        self.special_forms
            .import_meta_resolves
            .sort_by_key(|site| (site.replace_start, site.replace_end));
        self.special_forms
            .dynamic_imports
            .sort_by_key(|site| (site.replace_start, site.replace_end));
        self.top_level_awaits
            .sort_by_key(|span| (span.start, span.end));
        Ok(ModuleRewriteSites {
            special_forms: self.special_forms,
            top_level_awaits: self.top_level_awaits,
        })
    }

    fn push_unsupported_special_form(&mut self, span: Span, form: &str) {
        let snippet = self
            .source
            .get(span.start as usize..span.end as usize)
            .unwrap_or(form);
        self.errors
            .push(format!("unsupported {form} syntax `{snippet}`"));
    }

    fn shifted_span(&self, span: Span) -> Option<(usize, usize)> {
        let start = span.start as isize - self.offset;
        let end = span.end as isize - self.offset;
        if start < 0 || end < start {
            return None;
        }
        Some((start as usize, end as usize))
    }

    fn shifted_module_span(&self, span: Span) -> Option<ModuleAstSpan> {
        let (start, end) = self.shifted_span(span)?;
        Some(ModuleAstSpan {
            start: start.try_into().ok()?,
            end: end.try_into().ok()?,
        })
    }

    fn push_top_level_await(&mut self, span: Span) {
        if self.function_depth == 0
            && let Some(span) = self.shifted_module_span(span)
        {
            self.top_level_awaits.push(span);
        }
    }
}

impl<'a> Visit<'a> for ModuleRewriteSiteCollector<'_> {
    fn visit_static_member_expression(&mut self, expression: &StaticMemberExpression<'a>) {
        if static_member_is_import_meta_url(expression) {
            if let Some(span) = self.shifted_module_span(expression.span) {
                self.special_forms.import_meta_urls.push(span);
            }
            return;
        }

        walk::walk_static_member_expression(self, expression);
    }

    fn visit_call_expression(&mut self, expression: &CallExpression<'a>) {
        walk::walk_call_expression(self, expression);
    }

    fn visit_meta_property(&mut self, meta: &MetaProperty<'a>) {
        if meta_property_is_import_meta(meta)
            && let Some(span) = self.shifted_module_span(meta.span)
        {
            self.special_forms.import_metas.push(span);
        }
    }

    fn visit_await_expression(&mut self, expression: &oxc_ast::ast::AwaitExpression<'a>) {
        self.push_top_level_await(expression.span);
        if let Expression::ImportExpression(import) = &expression.argument {
            if let Some(site) = dynamic_import_site_from_oxc(
                self.source,
                self.offset,
                expression.span,
                import,
                DynamicImportRewriteKind::AwaitedNamespace,
            ) {
                self.special_forms.dynamic_imports.push(site);
            } else if dynamic_import_is_css_import(import) {
                return;
            } else {
                self.push_unsupported_special_form(expression.span, "dynamic import");
            }
            return;
        }

        walk::walk_await_expression(self, expression);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if let Some(site) = dynamic_import_site_from_oxc(
            self.source,
            self.offset,
            expression.span,
            expression,
            DynamicImportRewriteKind::Promise,
        ) {
            self.special_forms.dynamic_imports.push(site);
        } else if !dynamic_import_is_css_import(expression) {
            self.push_unsupported_special_form(expression.span, "dynamic import");
        }
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        if statement.r#await {
            self.push_top_level_await(statement.span);
        }
        walk::walk_for_of_statement(self, statement);
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.function_depth += 1;
        walk::walk_function_body(self, body);
        self.function_depth -= 1;
    }
}

fn static_member_is_import_meta_url(expression: &StaticMemberExpression<'_>) -> bool {
    expression.property.name == "url" && expression_is_import_meta(&expression.object)
}

fn expression_is_import_meta(expression: &Expression<'_>) -> bool {
    let Expression::MetaProperty(meta) = expression else {
        return false;
    };
    meta_property_is_import_meta(meta)
}

fn meta_property_is_import_meta(meta: &MetaProperty<'_>) -> bool {
    meta.meta.name == "import" && meta.property.name == "meta"
}

fn dynamic_import_site_from_oxc(
    source: &str,
    offset: isize,
    replace_span: Span,
    expression: &ImportExpression<'_>,
    kind: DynamicImportRewriteKind,
) -> Option<DynamicImportRewriteSite> {
    let (specifier, resolve_import_meta_first) =
        dynamic_import_specifier_from_expression(&expression.source)?;
    let import_type = match &expression.options {
        Some(options) => dynamic_import_options_type_from_expression(options)?,
        None => None,
    };
    let (replace_start, replace_end) = shifted_span(replace_span, offset)?;
    source.get(replace_span.start as usize..replace_span.end as usize)?;
    Some(DynamicImportRewriteSite {
        specifier,
        resolve_import_meta_first,
        import_type,
        replace_start,
        replace_end,
        kind,
    })
}

fn dynamic_import_specifier_from_expression(expression: &Expression<'_>) -> Option<(String, bool)> {
    if let Some(specifier) = static_string_from_expression(expression) {
        return Some((specifier, false));
    }
    let Expression::CallExpression(call) = expression else {
        return None;
    };
    if !static_member_is_import_meta_resolve_call(call) {
        return None;
    }
    Some((static_import_meta_resolve_specifier(call)?, true))
}

fn static_member_is_import_meta_resolve_call(expression: &CallExpression<'_>) -> bool {
    let Expression::StaticMemberExpression(member) = &expression.callee else {
        return false;
    };
    member.property.name == "resolve" && expression_is_import_meta(&member.object)
}

fn static_import_meta_resolve_specifier(expression: &CallExpression<'_>) -> Option<String> {
    let Some(argument) = expression.arguments.first() else {
        return Some("undefined".to_owned());
    };
    static_string_from_argument(argument)
}

fn dynamic_import_is_css_import(expression: &ImportExpression<'_>) -> bool {
    expression
        .options
        .as_ref()
        .and_then(dynamic_import_options_type_from_expression)
        .flatten()
        .is_some_and(|import_type| import_type.eq_ignore_ascii_case("css"))
}

fn shifted_span(span: Span, offset: isize) -> Option<(usize, usize)> {
    let start = span.start as isize - offset;
    let end = span.end as isize - offset;
    if start < 0 || end < start {
        return None;
    }
    Some((start as usize, end as usize))
}

fn static_string_from_argument(argument: &Argument<'_>) -> Option<String> {
    match argument {
        Argument::StringLiteral(literal) => Some(literal.value.as_str().to_owned()),
        Argument::TemplateLiteral(template) => static_string_from_template_literal(template),
        Argument::ParenthesizedExpression(parenthesized) => {
            static_string_from_expression(&parenthesized.expression)
        }
        Argument::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            let mut value = static_string_from_expression(&binary.left)?;
            value.push_str(&static_string_from_expression(&binary.right)?);
            Some(value)
        }
        _ => None,
    }
}

fn static_string_from_expression(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.as_str().to_owned()),
        Expression::TemplateLiteral(template) => static_string_from_template_literal(template),
        Expression::ParenthesizedExpression(parenthesized) => {
            static_string_from_expression(&parenthesized.expression)
        }
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            let mut value = static_string_from_expression(&binary.left)?;
            value.push_str(&static_string_from_expression(&binary.right)?);
            Some(value)
        }
        _ => None,
    }
}

fn static_string_from_template_literal(template: &TemplateLiteral<'_>) -> Option<String> {
    if !template.expressions.is_empty() {
        return None;
    }
    let mut value = String::new();
    for quasi in &template.quasis {
        value.push_str(quasi.value.cooked.as_ref()?.as_str());
    }
    Some(value)
}

fn dynamic_import_options_type_from_expression(
    expression: &Expression<'_>,
) -> Option<Option<String>> {
    let object = static_object_from_expression(expression)?;
    let mut found_attribute_bag = false;
    let mut import_type = None;

    for property in &object.properties {
        let property = strict_static_object_property(property)?;
        let key = property_key_static_name(property)?;
        match key.as_str() {
            "with" | "assert" => {
                if found_attribute_bag {
                    return None;
                }
                found_attribute_bag = true;
                import_type = import_attribute_type_from_expression(&property.value)?;
            }
            _ => return None,
        }
    }
    Some(import_type)
}

fn import_attribute_type_from_expression(expression: &Expression<'_>) -> Option<Option<String>> {
    let object = static_object_from_expression(expression)?;
    import_attribute_type_from_object(object)
}

fn import_attribute_type_from_object(object: &ObjectExpression<'_>) -> Option<Option<String>> {
    let mut import_type = None;
    for property in &object.properties {
        let property = strict_static_object_property(property)?;
        if property_key_static_name(property)? != "type" {
            return None;
        }
        if import_type.is_some() {
            return None;
        }
        import_type = Some(static_string_from_expression(&property.value)?);
    }
    Some(import_type)
}

fn static_object_from_expression<'a>(
    expression: &'a Expression<'a>,
) -> Option<&'a ObjectExpression<'a>> {
    match expression {
        Expression::ObjectExpression(object) => Some(object),
        Expression::ParenthesizedExpression(parenthesized) => {
            static_object_from_expression(&parenthesized.expression)
        }
        _ => None,
    }
}

fn strict_static_object_property<'a>(
    property: &'a ObjectPropertyKind<'a>,
) -> Option<&'a ObjectProperty<'a>> {
    let ObjectPropertyKind::ObjectProperty(property) = property else {
        return None;
    };
    if property.kind != PropertyKind::Init
        || property.method
        || property.shorthand
        || property.computed
    {
        return None;
    }
    Some(property)
}

fn property_key_static_name(property: &ObjectProperty<'_>) -> Option<String> {
    match &property.key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.as_str().to_owned()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.as_str().to_owned()),
        _ => None,
    }
}
