//! Tolerant parsing of the JSON arguments a model streams for a tool call.

use serde_json::{Value, json};

/// Parse streamed tool-call arguments.
///
/// Models sometimes close one brace or bracket too many (seen with DeepSeek: a valid object followed by an
/// extra `}`). A strict parse would turn that into a failed call and a wasted round trip, so when the
/// text holds one complete JSON object followed only by stray closers or whitespace, that object is used.
/// Anything else that does not parse is kept as `{"_raw": text}` so the model can see what it sent.
#[must_use]
pub fn parse_tool_args(args: &str) -> Value {
    if args.trim().is_empty() {
        return json!({});
    }
    if let Ok(v) = serde_json::from_str::<Value>(args) {
        return v;
    }
    let mut stream = serde_json::Deserializer::from_str(args).into_iter::<Value>();
    if let Some(Ok(v)) = stream.next()
        && v.is_object()
    {
        let rest = &args[stream.byte_offset()..];
        if rest
            .chars()
            .all(|c| c.is_whitespace() || c == '}' || c == ']')
        {
            return v;
        }
    }
    json!({ "_raw": args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_empty() {
        assert_eq!(parse_tool_args(""), json!({}));
        assert_eq!(parse_tool_args(r#"{"a":1}"#), json!({"a": 1}));
    }

    #[test]
    fn one_stray_closing_brace_is_forgiven() {
        let s = r#"{"commands":[{"command":"x"}],"expect":{"bbox":{"min":[0,0,0]}}}}"#;
        assert_eq!(
            parse_tool_args(s),
            json!({"commands":[{"command":"x"}],"expect":{"bbox":{"min":[0,0,0]}}})
        );
    }

    #[test]
    fn real_garbage_stays_raw() {
        assert_eq!(
            parse_tool_args(r#"{"a":1} and more"#),
            json!({"_raw": r#"{"a":1} and more"#})
        );
        assert_eq!(parse_tool_args(r#"{"a":"#), json!({"_raw": r#"{"a":"#}));
    }
}
