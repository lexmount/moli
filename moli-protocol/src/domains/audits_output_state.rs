use std::collections::VecDeque;

use moli_core::page::{ContentSecurityPolicyViolationType, InspectorIssueSnapshot};
use serde_json::{Value, json};

const MAX_STORED_INSPECTOR_ISSUES: usize = 1_000;

pub(crate) fn inspector_issue_protocol_value(
    issue: &InspectorIssueSnapshot,
    frame_id: &str,
    loader_id: &str,
) -> Value {
    match issue {
        InspectorIssueSnapshot::QuirksMode(issue) => json!({
            "code": "QuirksModeIssue",
            "details": {
                "quirksModeIssueDetails": {
                    "isLimitedQuirksMode": issue.is_limited_quirks_mode(),
                    "documentNodeId": issue.document_node_id(),
                    "url": issue.url(),
                    "frameId": frame_id,
                    "loaderId": loader_id,
                }
            }
        }),
        InspectorIssueSnapshot::ContentSecurityPolicy(issue) => {
            let mut details = json!({
                "isReportOnly": issue.is_report_only(),
                "violatedDirective": issue.violated_directive(),
                "contentSecurityPolicyViolationType": csp_violation_type_name(
                    issue.violation_type()
                ),
            });
            if let Some(blocked_url) = issue.blocked_url() {
                details["blockedURL"] = json!(blocked_url);
            }
            if let Some(location) = issue.source_code_location() {
                details["sourceCodeLocation"] = json!({
                    "url": location.url(),
                    "lineNumber": location.line_number(),
                    "columnNumber": location.column_number(),
                });
            }
            if let Some(node_id) = issue.violating_node_id() {
                details["violatingNodeId"] = json!(node_id);
            }
            json!({
                "code": "ContentSecurityPolicyIssue",
                "details": {
                    "contentSecurityPolicyIssueDetails": details,
                }
            })
        }
    }
}

fn csp_violation_type_name(violation_type: ContentSecurityPolicyViolationType) -> &'static str {
    match violation_type {
        ContentSecurityPolicyViolationType::Eval => "kEvalViolation",
        ContentSecurityPolicyViolationType::WasmEval => "kWasmEvalViolation",
        ContentSecurityPolicyViolationType::Inline => "kInlineViolation",
        ContentSecurityPolicyViolationType::TrustedTypesPolicy => "kTrustedTypesPolicyViolation",
        ContentSecurityPolicyViolationType::TrustedTypesSink => "kTrustedTypesSinkViolation",
        ContentSecurityPolicyViolationType::Url => "kURLViolation",
        ContentSecurityPolicyViolationType::SubresourceIntegrity => "kSRIViolation",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TargetAuditsOutputCursor {
    generation: u64,
    issue_start: usize,
    issue_end: usize,
}

impl TargetAuditsOutputCursor {
    pub(crate) fn generation(self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) fn issue_start(self) -> usize {
        self.issue_start
    }

    pub(crate) fn issue_end(self) -> usize {
        self.issue_end
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetAuditsStorageState {
    generation: u64,
    issue_end: usize,
    issues: VecDeque<InspectorIssueSnapshot>,
}

impl TargetAuditsStorageState {
    pub(crate) fn reset_for_new_document(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.issue_end = 0;
        self.issues.clear();
    }

    pub(crate) fn append_concrete_issue(&mut self, issue: InspectorIssueSnapshot) {
        self.issue_end = self
            .issue_end
            .checked_add(1)
            .expect("Audits issue index exhausted");
        self.issues.push_back(issue);
        if self.issues.len() > MAX_STORED_INSPECTOR_ISSUES {
            self.issues.pop_front();
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn first_issue_index(&self) -> usize {
        self.issue_end - self.issues.len()
    }

    #[cfg(test)]
    pub(crate) fn issue_end(&self) -> usize {
        self.issue_end
    }

    pub(crate) fn pending_cursor_from(
        &self,
        generation: u64,
        issue_index: usize,
    ) -> Option<TargetAuditsOutputCursor> {
        let issue_start = if generation == self.generation {
            issue_index.max(self.first_issue_index())
        } else {
            self.first_issue_index()
        };
        (issue_start < self.issue_end).then_some(TargetAuditsOutputCursor {
            generation: self.generation,
            issue_start,
            issue_end: self.issue_end,
        })
    }

    pub(crate) fn issues_for_cursor(
        &self,
        cursor: TargetAuditsOutputCursor,
    ) -> Option<Vec<InspectorIssueSnapshot>> {
        if cursor.generation != self.generation
            || cursor.issue_start < self.first_issue_index()
            || cursor.issue_start > cursor.issue_end
            || cursor.issue_end > self.issue_end
        {
            return None;
        }
        let start = cursor.issue_start - self.first_issue_index();
        let len = cursor.issue_end - cursor.issue_start;
        Some(self.issues.iter().skip(start).take(len).cloned().collect())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetAuditsSessionState {
    enabled: bool,
    output_generation: u64,
    next_issue_index: usize,
}

impl TargetAuditsSessionState {
    pub(crate) fn enable(
        &mut self,
        storage: &TargetAuditsStorageState,
    ) -> Option<TargetAuditsOutputCursor> {
        if self.enabled {
            return None;
        }
        self.enabled = true;
        self.output_generation = storage.generation();
        self.next_issue_index = storage.first_issue_index();
        self.pending_cursor(storage)
    }

    pub(crate) fn disable(&mut self) {
        self.enabled = false;
    }

    pub(crate) fn pending_cursor(
        &self,
        storage: &TargetAuditsStorageState,
    ) -> Option<TargetAuditsOutputCursor> {
        self.enabled.then_some(())?;
        storage.pending_cursor_from(self.output_generation, self.next_issue_index)
    }

    pub(crate) fn mark_emitted(&mut self, cursor: TargetAuditsOutputCursor) {
        self.output_generation = cursor.generation();
        self.next_issue_index = cursor.issue_end();
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType,
        InspectorIssueSnapshot, InspectorSourceCodeLocationSnapshot, QuirksModeIssueSnapshot,
    };
    use serde_json::json;

    use super::{
        MAX_STORED_INSPECTOR_ISSUES, TargetAuditsSessionState, TargetAuditsStorageState,
        inspector_issue_protocol_value,
    };

    fn issue(index: usize) -> InspectorIssueSnapshot {
        InspectorIssueSnapshot::QuirksMode(QuirksModeIssueSnapshot::new(
            false,
            u32::try_from(index + 1).unwrap(),
            format!("https://example.test/{index}"),
        ))
    }

    #[test]
    fn enable_replays_storage_once_and_reenable_replays_current_storage() {
        let mut storage = TargetAuditsStorageState::default();
        storage.append_concrete_issue(issue(0));
        storage.append_concrete_issue(issue(1));
        let mut session = TargetAuditsSessionState::default();

        let replay = session
            .enable(&storage)
            .expect("first enable should replay");
        assert_eq!(storage.issues_for_cursor(replay).unwrap().len(), 2);
        session.mark_emitted(replay);
        assert!(session.enable(&storage).is_none());
        assert!(session.pending_cursor(&storage).is_none());

        session.disable();
        let replay = session
            .enable(&storage)
            .expect("re-enable should replay current storage");
        assert_eq!(replay.issue_start(), 0);
        assert_eq!(replay.issue_end(), 2);
    }

    #[test]
    fn storage_matches_chromium_issue_limit_and_advances_slow_sessions() {
        let mut storage = TargetAuditsStorageState::default();
        for index in 0..=MAX_STORED_INSPECTOR_ISSUES {
            storage.append_concrete_issue(issue(index));
        }

        assert_eq!(storage.first_issue_index(), 1);
        assert_eq!(storage.issue_end(), MAX_STORED_INSPECTOR_ISSUES + 1);
        let cursor = storage.pending_cursor_from(0, 0).unwrap();
        let stored = storage.issues_for_cursor(cursor).unwrap();
        assert_eq!(stored.len(), MAX_STORED_INSPECTOR_ISSUES);
        assert_eq!(stored.first(), Some(&issue(1)));
    }

    #[test]
    fn new_document_invalidates_session_cursor_without_disabling_domain() {
        let mut storage = TargetAuditsStorageState::default();
        storage.append_concrete_issue(issue(0));
        let mut session = TargetAuditsSessionState::default();
        let cursor = session.enable(&storage).unwrap();
        session.mark_emitted(cursor);

        storage.reset_for_new_document();
        storage.append_concrete_issue(issue(2));
        assert!(storage.issues_for_cursor(cursor).is_none());
        let cursor = session
            .pending_cursor(&storage)
            .expect("enabled session should observe the replacement document");
        assert_eq!(storage.issues_for_cursor(cursor).unwrap(), vec![issue(2)]);
    }

    #[test]
    fn serializes_chromium_quirks_and_csp_issue_shapes() {
        let quirks = InspectorIssueSnapshot::QuirksMode(QuirksModeIssueSnapshot::new(
            true,
            17,
            "https://example.test/quirks".to_owned(),
        ));
        assert_eq!(
            inspector_issue_protocol_value(&quirks, "FRAME-1", "LOADER-1"),
            json!({
                "code": "QuirksModeIssue",
                "details": {
                    "quirksModeIssueDetails": {
                        "isLimitedQuirksMode": true,
                        "documentNodeId": 17,
                        "url": "https://example.test/quirks",
                        "frameId": "FRAME-1",
                        "loaderId": "LOADER-1",
                    }
                }
            })
        );

        let csp = InspectorIssueSnapshot::ContentSecurityPolicy(
            ContentSecurityPolicyIssueSnapshot::new(
                false,
                "script-src-elem".to_owned(),
                ContentSecurityPolicyViolationType::Inline,
            )
            .with_blocked_url(Some("https://blocked.test/script.js".to_owned()))
            .with_source_code_location(Some(InspectorSourceCodeLocationSnapshot::new(
                "https://example.test/page".to_owned(),
                4,
                9,
            )))
            .with_violating_node_id(Some(23)),
        );
        assert_eq!(
            inspector_issue_protocol_value(&csp, "FRAME-ignored", "LOADER-ignored"),
            json!({
                "code": "ContentSecurityPolicyIssue",
                "details": {
                    "contentSecurityPolicyIssueDetails": {
                        "isReportOnly": false,
                        "violatedDirective": "script-src-elem",
                        "contentSecurityPolicyViolationType": "kInlineViolation",
                        "blockedURL": "https://blocked.test/script.js",
                        "sourceCodeLocation": {
                            "url": "https://example.test/page",
                            "lineNumber": 4,
                            "columnNumber": 9,
                        },
                        "violatingNodeId": 23,
                    }
                }
            })
        );
    }
}
