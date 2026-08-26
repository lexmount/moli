use anyhow::{Context, Result, bail};
use moli_core::page::Page;
use serde_json::Value;

pub async fn evaluate(page: &mut Page, expression: &str) -> Result<Vec<u8>> {
    let result = page
        .evaluate_runtime_expression_by_value_with_await_async(expression, true)
        .await
        .context("failed to evaluate JavaScript expression")?;
    render_result(&result)
}

fn render_result(result: &Value) -> Result<Vec<u8>> {
    if let Some(exception) = result.get("exception").and_then(Value::as_str) {
        bail!("JavaScript evaluation failed: {exception}");
    }

    if result.get("type").and_then(Value::as_str) == Some("undefined") {
        return Ok(b"undefined\n".to_vec());
    }

    if let Some(value) = result.get("unserializableValue").and_then(Value::as_str) {
        return Ok(line(value.as_bytes()));
    }

    let Some(value) = result.get("value") else {
        let result_type = result
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let description = result
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(result_type);
        bail!(
            "JavaScript evaluation result `{description}` cannot be serialized by value; return text or JSON-compatible data"
        );
    };

    if let Some(text) = value.as_str() {
        return Ok(line(text.as_bytes()));
    }

    let encoded = serde_json::to_vec(value).context("failed to encode JavaScript result")?;
    Ok(line(&encoded))
}

fn line(value: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.len() + 1);
    output.extend_from_slice(value);
    output.push(b'\n');
    output
}

#[cfg(test)]
mod tests {
    use super::render_result;
    use serde_json::json;

    #[test]
    fn renders_strings_without_json_quotes() {
        assert_eq!(
            render_result(&json!({ "type": "string", "value": "hello" })).unwrap(),
            b"hello\n"
        );
    }

    #[test]
    fn renders_structured_values_as_compact_json() {
        assert_eq!(
            render_result(&json!({
                "type": "object",
                "value": { "title": "Moli", "count": 2 }
            }))
            .unwrap(),
            br#"{"title":"Moli","count":2}
"#
        );
    }

    #[test]
    fn renders_undefined_and_unserializable_primitives_like_a_console() {
        assert_eq!(
            render_result(&json!({ "type": "undefined" })).unwrap(),
            b"undefined\n"
        );
        assert_eq!(
            render_result(&json!({ "type": "number", "unserializableValue": "NaN" })).unwrap(),
            b"NaN\n"
        );
    }

    #[test]
    fn turns_javascript_exceptions_into_command_errors() {
        let error = render_result(&json!({
            "exception": "Error: extraction failed"
        }))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "JavaScript evaluation failed: Error: extraction failed"
        );
    }
}
