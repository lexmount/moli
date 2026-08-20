//! `robots.txt` document parsing and rule evaluation.

use crate::pattern::{agent_specificity, normalize_percent_encoding, pattern_matches};

/// A parsed `robots.txt` document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RobotsTxt {
    groups: Vec<Group>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Group {
    /// Lowercased `User-agent` values introducing this group.
    agents: Vec<String>,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    allow: bool,
    pattern: String,
}

impl RobotsTxt {
    /// Parses a `robots.txt` document.
    ///
    /// Parsing never fails. RFC 9309 §2.1 requires unparseable lines to be
    /// ignored rather than to invalidate the file, because a strict reader
    /// would let one typo silently drop a site's rules.
    pub fn parse(text: &str) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        // A `User-agent` line after a rule line starts a new group; consecutive
        // `User-agent` lines share one.
        let mut start_new_group = true;

        for line in text.trim_start_matches('\u{feff}').lines() {
            let Some((field, value)) = split_field(line) else {
                continue;
            };

            match field.as_str() {
                "user-agent" | "useragent" => {
                    if start_new_group {
                        groups.push(Group::default());
                        start_new_group = false;
                    }
                    if let Some(group) = groups.last_mut() {
                        group.agents.push(value.to_lowercase());
                    }
                }
                "allow" | "disallow" => {
                    let Some(group) = groups.last_mut() else {
                        // Rules that precede every `User-agent` line belong to
                        // no group and bind nobody.
                        continue;
                    };
                    start_new_group = true;
                    if value.is_empty() {
                        // RFC 9309 §2.2.2: an empty value places no
                        // restriction, so it contributes no rule at all rather
                        // than an empty pattern that would match everything.
                        continue;
                    }
                    group.rules.push(Rule {
                        allow: field == "allow",
                        pattern: normalize_percent_encoding(value),
                    });
                }
                // `Sitemap`, `Crawl-delay`, `Host`, and anything else are not
                // access rules.
                _ => {}
            }
        }

        Self { groups }
    }

    /// Whether `user_agent` may fetch `request_target`.
    pub fn allows(&self, user_agent: &str, request_target: &str) -> bool {
        let request_target = normalize_percent_encoding(request_target);
        let lowercase_user_agent = user_agent.to_lowercase();

        let Some(specificity) = self.best_specificity(&lowercase_user_agent) else {
            // No group speaks to this user agent, not even `*`.
            return true;
        };

        let mut decision: Option<(usize, bool)> = None;
        for rule in self.matching_rules(&lowercase_user_agent, specificity) {
            if !pattern_matches(&rule.pattern, &request_target) {
                continue;
            }
            let length = rule.pattern.chars().count();
            decision = match decision {
                // RFC 9309 §2.2.2: the longest match wins, and `Allow` wins a
                // tie so a narrow exception can reopen a broad `Disallow`.
                Some((best, allow)) if best > length || (best == length && allow) => {
                    Some((best, allow))
                }
                _ => Some((length, rule.allow)),
            };
        }

        decision.is_none_or(|(_, allow)| allow)
    }

    fn best_specificity(&self, lowercase_user_agent: &str) -> Option<usize> {
        self.groups
            .iter()
            .filter_map(|group| group.specificity(lowercase_user_agent))
            .max()
    }

    /// Rules from every group that matches at `specificity`.
    ///
    /// RFC 9309 §2.2.1 treats repeated groups for the same user agent as one
    /// merged group, so a file that names an agent twice keeps both rule sets.
    fn matching_rules<'a>(
        &'a self,
        lowercase_user_agent: &'a str,
        specificity: usize,
    ) -> impl Iterator<Item = &'a Rule> {
        self.groups
            .iter()
            .filter(move |group| group.specificity(lowercase_user_agent) == Some(specificity))
            .flat_map(|group| group.rules.iter())
    }
}

impl Group {
    fn specificity(&self, lowercase_user_agent: &str) -> Option<usize> {
        self.agents
            .iter()
            .filter_map(|agent| agent_specificity(agent, lowercase_user_agent))
            .max()
    }
}

/// Splits one line into a lowercased field name and its value.
///
/// Comments run from an unquoted `#` to the end of the line. A line without a
/// `:` separator carries no field and is dropped.
fn split_field(line: &str) -> Option<(String, &str)> {
    let line = match line.split_once('#') {
        Some((before, _)) => before,
        None => line,
    };
    let (field, value) = line.split_once(':')?;
    let field = field.trim().to_lowercase();
    if field.is_empty() {
        return None;
    }
    Some((field, value.trim()))
}
