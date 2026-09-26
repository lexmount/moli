const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionNearestSelectStep {
    Continue,
    Select,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionDisabledAncestorStep {
    Continue,
    Disabled(bool),
}

pub fn option_disabled_ancestor_step(
    namespace: &str,
    local_name: &str,
    has_disabled_attribute: bool,
) -> OptionDisabledAncestorStep {
    if namespace != HTML_NAMESPACE {
        return OptionDisabledAncestorStep::Continue;
    }

    match local_name {
        "select" | "hr" | "datalist" | "option" => OptionDisabledAncestorStep::Disabled(false),
        "optgroup" => OptionDisabledAncestorStep::Disabled(has_disabled_attribute),
        _ => OptionDisabledAncestorStep::Continue,
    }
}

/// State for the HTML "option element nearest ancestor select" algorithm.
///
/// Tree owners perform the actual ancestor walk and feed element names into
/// this state machine. Keeping the HTML association rule here lets native DOM
/// trees and detached bridge trees share the same barriers.
#[derive(Debug, Default)]
pub struct OptionNearestSelectTraversal {
    saw_optgroup: bool,
}

impl OptionNearestSelectTraversal {
    pub fn starting_at_optgroup() -> Self {
        Self { saw_optgroup: true }
    }

    pub fn visit_ancestor(&mut self, namespace: &str, local_name: &str) -> OptionNearestSelectStep {
        if namespace != HTML_NAMESPACE {
            return OptionNearestSelectStep::Continue;
        }

        match local_name {
            "datalist" | "hr" | "option" => OptionNearestSelectStep::Blocked,
            "optgroup" if self.saw_optgroup => OptionNearestSelectStep::Blocked,
            "optgroup" => {
                self.saw_optgroup = true;
                OptionNearestSelectStep::Continue
            }
            "select" => OptionNearestSelectStep::Select,
            _ => OptionNearestSelectStep::Continue,
        }
    }
}
