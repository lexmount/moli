use cssparser::Parser;
use style::{
    counter_style::{CounterStyleRule, DescriptorId, parse_counter_style_name_definition},
    parser::ParserContext,
    shared_lock::{SharedRwLockReadGuard, ToCssWithGuard},
    stylesheets::{CssRule, CssRuleType},
};
use style_traits::{CssStringWriter, ToCss};

#[derive(Clone, Copy, Debug)]
pub enum CssCounterStyleProperty {
    Name,
    Descriptor(DescriptorId),
}

impl CssCounterStyleProperty {
    pub fn read(self, rule: &CounterStyleRule) -> String {
        match self {
            Self::Name => rule.name().to_css_string(),
            Self::Descriptor(id) => {
                let mut value = CssStringWriter::new();
                rule.descriptors()
                    .get(id, &mut value)
                    .expect("serializing a counter style descriptor into a string must not fail");
                value
            }
        }
    }

    /// The native setters enforce the descriptor grammar and the constraints
    /// of the rule's existing system before mutating it.
    pub fn set(self, rule: &mut CounterStyleRule, context: &ParserContext, value: &str) -> bool {
        let mut input = Parser::new(value);
        match self {
            Self::Name => {
                let Ok(name) = input.parse_entirely(parse_counter_style_name_definition) else {
                    return false;
                };
                if rule.name() == &name {
                    return false;
                }
                rule.set_name(name);
                true
            }
            Self::Descriptor(id) => rule
                .set_descriptor(id, context, &mut input)
                .unwrap_or(false),
        }
    }

    pub fn read_detached(self, css_text: &str) -> Option<String> {
        with_detached_counter_style(css_text, |rule, _| self.read(rule))
    }

    pub fn set_detached(self, css_text: &str, value: &str) -> Option<String> {
        with_detached_counter_style(css_text, |rule, guard| {
            let mut rule = rule.clone();
            crate::stylo_stylesheet::with_descriptor_declaration_context(
                CssRuleType::CounterStyle,
                |context| {
                    self.set(&mut rule, context, value)
                        .then(|| rule.to_css_string(guard))
                },
            )
        })?
    }
}

fn with_detached_counter_style<T>(
    css_text: &str,
    read: impl FnOnce(&CounterStyleRule, &SharedRwLockReadGuard<'_>) -> T,
) -> Option<T> {
    let sheet = style::moli_rule_tree::parse_stylesheet_rule_tree(css_text);
    let guard = sheet.shared_lock.read();
    let contents = sheet.contents.read_with(&guard);
    let rules = contents.rules.read_with(&guard);
    let [CssRule::CounterStyle(rule)] = rules.0.as_slice() else {
        return None;
    };
    Some(read(rule.read_with(&guard), &guard))
}
