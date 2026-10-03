use crate::manifest::{
    ExpectedStatus, WptExpectation, WptManifestAction, WptManifestDragDirectory, WptManifestTest,
    load_expected, load_manifest_case, load_selected_manifest,
};
use crate::report::{WptActualStatus, WptCaseRun, WptPageReport};
use crate::server::WptFixtureServer;
use anyhow::{Context, Result, anyhow};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WptWaitUntil {
    DomContentLoaded,
    Load,
    NetworkIdle,
    DomStable,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WptCaseEvaluation {
    pub actual: WptActualStatus,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WptCasePlan {
    pub test: WptManifestTest,
    pub expected: WptExpectation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WptCaseRequest {
    pub url: String,
    pub wait_until: WptWaitUntil,
    pub timeout: Duration,
    pub actions: Vec<WptManifestAction>,
}

#[derive(Debug, Clone)]
pub enum WptCasePlanAction {
    Skip(WptCaseRun),
    Execute(Box<WptCasePlan>),
}

pub const WPT_REPORT_COMPLETE_EXPRESSION: &str = r#"
window.__moliWptReport &&
window.__moliWptReport.complete === true &&
(
  (Array.isArray(window.__moliWptReport.tests) &&
    window.__moliWptReport.tests.length > 0) ||
  window.__moliWptReport.status?.status !== "OK"
)
"#;

const COLLECT_WPT_REPORT_SNAPSHOT_EXPRESSION: &str = r#"
JSON.stringify(window.__moliWptReport || {
  complete: false,
  status: {
    status: "ERROR",
    message: "window.__moliWptReport is unavailable"
  },
  tests: []
})
"#;

pub fn collect_wpt_report_snapshot_expression() -> &'static str {
    COLLECT_WPT_REPORT_SNAPSHOT_EXPRESSION
}

fn validate_supported_manifest_entry(test: &WptManifestTest) -> Result<()> {
    if !matches!(
        test.test_type.as_str(),
        "testharness" | "idlharness" | "crashtest"
    ) {
        return Err(anyhow!(
            "unsupported WPT test type '{}' for {}",
            test.test_type,
            test.id
        ));
    }
    if !matches!(test.global.as_str(), "window" | "worker" | "sharedworker") {
        return Err(anyhow!(
            "unsupported WPT global '{}' for {}",
            test.global,
            test.id
        ));
    }
    if test.notes.trim().is_empty() {
        return Err(anyhow!("WPT case {} is missing manifest notes", test.id));
    }
    if test.requires_source_metadata()
        && test
            .source_path
            .as_deref()
            .is_none_or(|path| path.trim().is_empty())
    {
        return Err(anyhow!(
            "WPT case {} uses an upstream source but is missing source_path",
            test.id
        ));
    }
    if test.requires_source_metadata()
        && test
            .source_commit
            .as_deref()
            .is_none_or(|commit| commit.trim().is_empty())
    {
        return Err(anyhow!(
            "WPT case {} uses an upstream source but is missing source_commit",
            test.id
        ));
    }
    for (index, action) in test.actions.iter().enumerate() {
        match action {
            WptManifestAction::Evaluate { expression } if expression.trim().is_empty() => {
                return Err(anyhow!(
                    "WPT case {} has empty evaluate action at index {}",
                    test.id,
                    index
                ));
            }
            WptManifestAction::InsertText { text } if text.is_empty() => {
                return Err(anyhow!(
                    "WPT case {} has empty insert-text action at index {}",
                    test.id,
                    index
                ));
            }
            WptManifestAction::Delay { ms } if *ms == 0 => {
                return Err(anyhow!(
                    "WPT case {} has zero-ms delay action at index {}",
                    test.id,
                    index
                ));
            }
            WptManifestAction::DispatchDrag {
                event,
                x,
                y,
                modifiers,
                items,
                files,
                directories,
                ..
            } => {
                if event.trim().is_empty() {
                    return Err(anyhow!(
                        "WPT case {} has empty dispatch-drag event at index {}",
                        test.id,
                        index
                    ));
                }
                if !x.is_finite() || !y.is_finite() {
                    return Err(anyhow!(
                        "WPT case {} has non-finite dispatch-drag coordinates at index {}",
                        test.id,
                        index
                    ));
                }
                if modifiers & !0b1111 != 0 {
                    return Err(anyhow!(
                        "WPT case {} has unsupported dispatch-drag modifiers at index {}",
                        test.id,
                        index
                    ));
                }
                for item in items {
                    if item.mime_type.trim().is_empty() {
                        return Err(anyhow!(
                            "WPT case {} has dispatch-drag item with empty MIME type at index {}",
                            test.id,
                            index
                        ));
                    }
                }
                for file in files {
                    if file.name.trim().is_empty() || file.mime_type.trim().is_empty() {
                        return Err(anyhow!(
                            "WPT case {} has dispatch-drag file with empty name or MIME type at index {}",
                            test.id,
                            index
                        ));
                    }
                }
                for directory in directories {
                    validate_drag_directory(test, index, directory)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_drag_directory(
    test: &WptManifestTest,
    action_index: usize,
    directory: &WptManifestDragDirectory,
) -> Result<()> {
    if directory.name.trim().is_empty() {
        return Err(anyhow!(
            "WPT case {} has dispatch-drag directory with empty name at index {}",
            test.id,
            action_index
        ));
    }
    for file in &directory.files {
        if file.name.trim().is_empty() || file.mime_type.trim().is_empty() {
            return Err(anyhow!(
                "WPT case {} has dispatch-drag directory file with empty name or MIME type at index {}",
                test.id,
                action_index
            ));
        }
    }
    if let Some(generated) = &directory.generated_files
        && (generated.count == 0
            || generated.name_prefix.trim().is_empty()
            || generated.mime_type.trim().is_empty())
    {
        return Err(anyhow!(
            "WPT case {} has invalid dispatch-drag generated directory files at index {}",
            test.id,
            action_index
        ));
    }
    for child in &directory.directories {
        validate_drag_directory(test, action_index, child)?;
    }
    Ok(())
}

fn wait_until_for(test: &WptManifestTest) -> Result<WptWaitUntil> {
    match test.wait_until.as_str() {
        "domcontentloaded" => Ok(WptWaitUntil::DomContentLoaded),
        "load" => Ok(WptWaitUntil::Load),
        "networkidle" => Ok(WptWaitUntil::NetworkIdle),
        "domstable" => Ok(WptWaitUntil::DomStable),
        "done" => Ok(WptWaitUntil::Done),
        other => Err(anyhow!(
            "unsupported wait_until '{other}' for WPT case {}",
            test.id
        )),
    }
}

fn timeout_for(test: &WptManifestTest) -> Duration {
    Duration::from_millis(test.timeout_ms)
}

fn resolve_expectation(
    expectations: &BTreeMap<String, WptExpectation>,
    test: &WptManifestTest,
) -> WptExpectation {
    expectations
        .get(&test.id)
        .cloned()
        .unwrap_or(WptExpectation {
            status: if test.status == "pass" {
                ExpectedStatus::Pass
            } else {
                ExpectedStatus::Fail
            },
            reason: String::new(),
        })
}

fn plan_case_runs(
    tests: Vec<WptManifestTest>,
    expectations: &BTreeMap<String, WptExpectation>,
) -> Vec<WptCasePlan> {
    tests
        .into_iter()
        .map(|test| WptCasePlan {
            expected: resolve_expectation(expectations, &test),
            test,
        })
        .collect()
}

pub fn load_selected_case_plans() -> Result<Vec<WptCasePlan>> {
    let manifest = load_selected_manifest()?;
    let expectations = load_expected()?;
    Ok(plan_case_runs(manifest, &expectations))
}

pub fn load_case_plan(case_id: &str) -> Result<Option<WptCasePlan>> {
    let Some(test) = load_manifest_case(case_id)? else {
        return Ok(None);
    };
    let expectations = load_expected()?;
    Ok(Some(WptCasePlan {
        expected: resolve_expectation(&expectations, &test),
        test,
    }))
}

pub fn prepare_case_plan(plan: WptCasePlan) -> Result<WptCasePlanAction> {
    if plan.expected.status == ExpectedStatus::Skip {
        return Ok(WptCasePlanAction::Skip(skipped_case_run(
            plan.test,
            plan.expected,
        )));
    }

    validate_supported_manifest_entry(&plan.test)?;
    Ok(WptCasePlanAction::Execute(Box::new(plan)))
}

pub fn case_request(server: &WptFixtureServer, test: &WptManifestTest) -> Result<WptCaseRequest> {
    Ok(WptCaseRequest {
        url: server.case_url(test),
        wait_until: wait_until_for(test)?,
        timeout: timeout_for(test),
        actions: test.actions.clone(),
    })
}

fn skipped_case_run(test: WptManifestTest, expected: WptExpectation) -> WptCaseRun {
    WptCaseRun {
        id: test.id,
        upstream: test.upstream,
        upstream_commit: test.upstream_commit,
        local_path: test.local_path,
        tags: test.tags,
        expected: expected.status,
        expected_reason: expected.reason,
        actual: WptActualStatus::Skipped,
        failures: Vec::new(),
    }
}

fn completed_case_run(
    test: WptManifestTest,
    expected: WptExpectation,
    actual: WptActualStatus,
    failures: Vec<String>,
) -> WptCaseRun {
    WptCaseRun {
        id: test.id,
        upstream: test.upstream,
        upstream_commit: test.upstream_commit,
        local_path: test.local_path,
        tags: test.tags,
        expected: expected.status,
        expected_reason: expected.reason,
        actual,
        failures,
    }
}

fn evaluate_case_report(
    page_report: &WptPageReport,
    script_failures: Vec<String>,
) -> WptCaseEvaluation {
    let mut failures = page_report.failure_messages();
    failures.extend(script_failures);
    let actual = if page_report.passed() && failures.is_empty() {
        WptActualStatus::Pass
    } else {
        WptActualStatus::Fail
    };

    WptCaseEvaluation { actual, failures }
}

fn completed_case_run_from_report(
    test: WptManifestTest,
    expected: WptExpectation,
    page_report: &WptPageReport,
    script_failures: Vec<String>,
) -> WptCaseRun {
    let evaluation = evaluate_case_report(page_report, script_failures);
    completed_case_run(test, expected, evaluation.actual, evaluation.failures)
}

pub fn completed_case_plan_from_report(
    plan: WptCasePlan,
    page_report: &WptPageReport,
    script_failures: Vec<String>,
) -> WptCaseRun {
    completed_case_run_from_report(plan.test, plan.expected, page_report, script_failures)
}

pub fn decode_page_report_value(value: &serde_json::Value) -> Result<WptPageReport> {
    let raw = value
        .get("value")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "WPT report expression did not return a string payload: {}",
                value
            )
        })?;
    serde_json::from_str(raw).context("failed to decode WPT page report")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_test() -> WptManifestTest {
        WptManifestTest {
            id: "dom/eventtarget-basic".to_string(),
            upstream: "dom/events/EventTarget.html".to_string(),
            upstream_commit: "fixture".to_string(),
            local_path: "ported/dom/eventtarget-basic.html".to_string(),
            query: String::new(),
            test_type: "testharness".to_string(),
            global: "window".to_string(),
            origin: crate::manifest::WptManifestOrigin::Trusted,
            status: "pass".to_string(),
            wait_until: "done".to_string(),
            timeout_ms: 1000,
            suite: crate::manifest::WptManifestSuite::Smoke,
            source: None,
            source_path: None,
            source_commit: None,
            tags: vec!["dom".to_string()],
            actions: Vec::new(),
            notes: "covers EventTarget basics".to_string(),
        }
    }

    #[test]
    fn resolve_expectation_defaults_from_manifest_status() {
        let mut test = manifest_test();
        let expectation = resolve_expectation(&BTreeMap::new(), &test);
        assert_eq!(expectation.status, ExpectedStatus::Pass);

        test.status = "fail".to_string();
        let expectation = resolve_expectation(&BTreeMap::new(), &test);
        assert_eq!(expectation.status, ExpectedStatus::Fail);
    }

    #[test]
    fn validate_supported_manifest_entry_rejects_unsupported_shape() {
        let mut test = manifest_test();
        test.global = "sharedworker".to_string();
        assert!(validate_supported_manifest_entry(&test).is_ok());

        let mut test = manifest_test();
        test.global = "serviceworker".to_string();
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.test_type = "reftest".to_string();
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.notes.clear();
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.source = Some(crate::manifest::WptManifestSource::UpstreamWpt);
        test.source_path = Some("dom/events/EventTarget.html".to_string());
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.source = Some(crate::manifest::WptManifestSource::UpstreamWpt);
        test.source_commit = Some("59c8d70847593453e2e611b184d6e9072a527ba0".to_string());
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.actions = vec![WptManifestAction::Evaluate {
            expression: " ".to_string(),
        }];
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.actions = vec![WptManifestAction::InsertText {
            text: String::new(),
        }];
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.actions = vec![WptManifestAction::Delay { ms: 0 }];
        assert!(validate_supported_manifest_entry(&test).is_err());

        let mut test = manifest_test();
        test.actions = vec![WptManifestAction::DispatchDrag {
            event: String::new(),
            x: 1.0,
            y: 1.0,
            modifiers: 0,
            items: Vec::new(),
            files: Vec::new(),
            directories: Vec::new(),
            drag_operations_mask: 1,
        }];
        assert!(validate_supported_manifest_entry(&test).is_err());
    }

    #[test]
    fn plan_case_runs_pairs_manifest_tests_with_expectations() {
        let mut expectations = BTreeMap::new();
        expectations.insert(
            "dom/eventtarget-basic".to_string(),
            WptExpectation {
                status: ExpectedStatus::Skip,
                reason: "blocked".to_string(),
            },
        );

        let plans = plan_case_runs(vec![manifest_test()], &expectations);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].test.id, "dom/eventtarget-basic");
        assert_eq!(plans[0].expected.status, ExpectedStatus::Skip);
        assert_eq!(plans[0].expected.reason, "blocked");
    }

    #[test]
    fn plan_case_runs_defaults_missing_expectations_from_manifest_status() {
        let mut test = manifest_test();
        test.status = "fail".to_string();

        let plans = plan_case_runs(vec![test], &BTreeMap::new());
        assert_eq!(plans[0].expected.status, ExpectedStatus::Fail);
        assert_eq!(plans[0].expected.reason, "");
    }

    #[test]
    fn prepare_case_plan_turns_skip_expectation_into_run() {
        let plan = WptCasePlan {
            test: manifest_test(),
            expected: WptExpectation {
                status: ExpectedStatus::Skip,
                reason: "blocked".to_string(),
            },
        };

        match prepare_case_plan(plan).unwrap() {
            WptCasePlanAction::Skip(run) => {
                assert_eq!(run.actual, WptActualStatus::Skipped);
                assert_eq!(run.expected_reason, "blocked");
            }
            WptCasePlanAction::Execute(_) => panic!("skip plan should not execute"),
        }
    }

    #[test]
    fn prepare_case_plan_validates_executable_plan() {
        let mut test = manifest_test();
        test.test_type = "reftest".to_string();
        let plan = WptCasePlan {
            test,
            expected: WptExpectation {
                status: ExpectedStatus::Pass,
                reason: String::new(),
            },
        };

        assert!(prepare_case_plan(plan).is_err());
    }

    #[test]
    fn case_request_uses_fixture_url_wait_condition_and_timeout() {
        let test = manifest_test();
        let server = crate::WptFixtureServer::for_test_addr(
            "127.0.0.1:12345".parse().expect("valid socket addr"),
        );

        let request = case_request(&server, &test).unwrap();
        assert_eq!(
            request.url,
            "http://localhost:12345/wpt/ported/dom/eventtarget-basic.html"
        );
        assert_eq!(request.wait_until, WptWaitUntil::Done);
        assert_eq!(request.timeout, Duration::from_millis(1000));
        assert!(request.actions.is_empty());
    }

    #[test]
    fn wait_until_for_parses_supported_manifest_values() {
        let mut test = manifest_test();

        test.wait_until = "domcontentloaded".to_string();
        assert_eq!(
            wait_until_for(&test).unwrap(),
            WptWaitUntil::DomContentLoaded
        );

        test.wait_until = "load".to_string();
        assert_eq!(wait_until_for(&test).unwrap(), WptWaitUntil::Load);

        test.wait_until = "networkidle".to_string();
        assert_eq!(wait_until_for(&test).unwrap(), WptWaitUntil::NetworkIdle);

        test.wait_until = "domstable".to_string();
        assert_eq!(wait_until_for(&test).unwrap(), WptWaitUntil::DomStable);

        test.wait_until = "done".to_string();
        assert_eq!(wait_until_for(&test).unwrap(), WptWaitUntil::Done);
    }

    #[test]
    fn wait_until_for_rejects_unknown_manifest_value() {
        let mut test = manifest_test();
        test.wait_until = "interactive".to_string();

        assert!(wait_until_for(&test).is_err());
    }

    #[test]
    fn timeout_for_uses_manifest_milliseconds() {
        let mut test = manifest_test();
        test.timeout_ms = 4321;

        assert_eq!(timeout_for(&test), Duration::from_millis(4321));
    }

    #[test]
    fn wpt_report_complete_expression_checks_completion_flag() {
        assert!(WPT_REPORT_COMPLETE_EXPRESSION.contains("__moliWptReport"));
        assert!(WPT_REPORT_COMPLETE_EXPRESSION.contains("complete === true"));
        assert!(WPT_REPORT_COMPLETE_EXPRESSION.contains("tests.length > 0"));
        assert!(WPT_REPORT_COMPLETE_EXPRESSION.contains("status?.status !== \"OK\""));
    }

    #[test]
    fn collect_wpt_report_snapshot_expression_reads_report_state() {
        let script = collect_wpt_report_snapshot_expression();

        assert!(script.contains("JSON.stringify(window.__moliWptReport"));
        assert!(script.contains("\"window.__moliWptReport is unavailable\""));
    }

    #[test]
    fn skipped_case_run_preserves_case_metadata() {
        let test = manifest_test();
        let run = skipped_case_run(
            test,
            WptExpectation {
                status: ExpectedStatus::Skip,
                reason: "not implemented".to_string(),
            },
        );

        assert_eq!(run.id, "dom/eventtarget-basic");
        assert_eq!(run.tags, ["dom"]);
        assert_eq!(run.expected, ExpectedStatus::Skip);
        assert_eq!(run.actual, WptActualStatus::Skipped);
        assert_eq!(run.expected_reason, "not implemented");
    }

    #[test]
    fn completed_case_run_preserves_actual_result_and_failures() {
        let test = manifest_test();
        let run = completed_case_run(
            test,
            WptExpectation {
                status: ExpectedStatus::Pass,
                reason: String::new(),
            },
            WptActualStatus::Fail,
            vec!["subtest failed".to_string()],
        );

        assert_eq!(run.id, "dom/eventtarget-basic");
        assert_eq!(run.expected, ExpectedStatus::Pass);
        assert_eq!(run.actual, WptActualStatus::Fail);
        assert_eq!(run.failures, ["subtest failed"]);
    }

    #[test]
    fn evaluate_case_report_passes_only_when_page_and_scripts_pass() {
        let report = WptPageReport {
            complete: true,
            status: crate::WptOverallStatus {
                status: "OK".to_string(),
                message: String::new(),
            },
            tests: vec![crate::WptSubtest {
                name: "subtest".to_string(),
                status: "PASS".to_string(),
                message: String::new(),
            }],
        };

        let evaluation = evaluate_case_report(&report, Vec::new());
        assert_eq!(evaluation.actual, WptActualStatus::Pass);
        assert!(evaluation.failures.is_empty());

        let evaluation = evaluate_case_report(&report, vec!["script run failed: boom".to_string()]);
        assert_eq!(evaluation.actual, WptActualStatus::Fail);
        assert_eq!(evaluation.failures, ["script run failed: boom"]);
    }

    #[test]
    fn evaluate_case_report_includes_page_report_failures() {
        let report = WptPageReport {
            complete: true,
            status: crate::WptOverallStatus {
                status: "OK".to_string(),
                message: String::new(),
            },
            tests: vec![crate::WptSubtest {
                name: "subtest".to_string(),
                status: "FAIL".to_string(),
                message: "expected true".to_string(),
            }],
        };

        let evaluation = evaluate_case_report(&report, Vec::new());
        assert_eq!(evaluation.actual, WptActualStatus::Fail);
        assert_eq!(evaluation.failures, ["subtest: FAIL: expected true"]);
    }

    #[test]
    fn completed_case_run_from_report_evaluates_and_builds_run() {
        let report = WptPageReport {
            complete: true,
            status: crate::WptOverallStatus {
                status: "OK".to_string(),
                message: String::new(),
            },
            tests: vec![crate::WptSubtest {
                name: "subtest".to_string(),
                status: "PASS".to_string(),
                message: String::new(),
            }],
        };
        let run = completed_case_run_from_report(
            manifest_test(),
            WptExpectation {
                status: ExpectedStatus::Pass,
                reason: String::new(),
            },
            &report,
            Vec::new(),
        );

        assert_eq!(run.id, "dom/eventtarget-basic");
        assert_eq!(run.actual, WptActualStatus::Pass);
        assert!(run.failures.is_empty());
    }

    #[test]
    fn completed_case_plan_from_report_uses_plan_metadata() {
        let report = WptPageReport {
            complete: true,
            status: crate::WptOverallStatus {
                status: "OK".to_string(),
                message: String::new(),
            },
            tests: vec![crate::WptSubtest {
                name: "subtest".to_string(),
                status: "PASS".to_string(),
                message: String::new(),
            }],
        };
        let plan = WptCasePlan {
            test: manifest_test(),
            expected: WptExpectation {
                status: ExpectedStatus::Pass,
                reason: String::new(),
            },
        };
        let run = completed_case_plan_from_report(plan, &report, Vec::new());

        assert_eq!(run.id, "dom/eventtarget-basic");
        assert_eq!(run.actual, WptActualStatus::Pass);
    }

    #[test]
    fn decode_page_report_value_reads_json_string_payload() {
        let value = serde_json::json!({
            "value": r#"{"complete":true,"status":{"status":"OK","message":""},"tests":[{"name":"subtest","status":"PASS","message":""}]}"#
        });

        let report = decode_page_report_value(&value).unwrap();
        assert!(report.passed());
    }

    #[test]
    fn decode_page_report_value_rejects_missing_string_payload() {
        let value = serde_json::json!({ "value": null });

        assert!(decode_page_report_value(&value).is_err());
    }
}
