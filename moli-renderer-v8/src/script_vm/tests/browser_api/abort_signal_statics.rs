use super::*;

#[test]
fn abort_signal_static_factories_use_intrinsics_and_webidl_arguments() {
    let mut vm = new_parsed_test_vm(
        "https://abort-signal-statics.test/",
        "<!doctype html><body>",
    );
    let probe = include_str!("abort_signal_statics.js");
    let result = vm
        .eval(&format!(
            r#"{probe}
            JSON.stringify((() => {{
                const frame = document.body.appendChild(document.createElement('iframe'));
                try {{ return [abortSignalStaticsProbe(), abortSignalStaticsProbe(frame.contentWindow)]; }}
                finally {{ frame.remove(); }}
            }})())"#
        ))
        .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    for row in rows.as_array().unwrap() {
        assert_eq!(row["failures"], serde_json::json!([]), "{row}");
        assert_eq!(row["scenarios"].as_array().unwrap().len(), 4);
    }
}
