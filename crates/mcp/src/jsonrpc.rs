use serde_json::{Value, json};

/// The protocol version we offer. Servers answer with the version they speak; we accept any.
pub const PROTOCOL: &str = "2025-03-26";

#[must_use]
pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

#[must_use]
pub fn response_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

#[must_use]
pub fn response_err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// What an incoming message is.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Invalid,
}

#[must_use]
pub fn classify(v: &Value) -> Incoming {
    let method = v.get("method").and_then(Value::as_str);
    let id = v.get("id").filter(|i| !i.is_null());
    match (method, id) {
        (Some(m), Some(id)) => Incoming::Request {
            id: id.clone(),
            method: m.to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        },
        (Some(m), None) => Incoming::Notification {
            method: m.to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        },
        (None, Some(id)) => {
            let Some(n) = id.as_u64() else {
                return Incoming::Invalid;
            };
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string();
                Incoming::Response {
                    id: n,
                    result: Err(msg),
                }
            } else {
                Incoming::Response {
                    id: n,
                    result: Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                }
            }
        }
        (None, None) => Incoming::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","id":1,"result":{}})),
            Incoming::Response {
                id: 1,
                result: Ok(_)
            }
        ));
        assert!(matches!(
            classify(&json!({"id":2,"error":{"message":"x"}})),
            Incoming::Response {
                id: 2,
                result: Err(_)
            }
        ));
        assert!(matches!(
            classify(&json!({"id":"a","method":"ping"})),
            Incoming::Request { .. }
        ));
        assert!(matches!(
            classify(&json!({"method":"notifications/initialized"})),
            Incoming::Notification { .. }
        ));
        assert_eq!(classify(&json!({"hello":1})), Incoming::Invalid);
        assert_eq!(classify(&json!({"id":"str"})), Incoming::Invalid);
    }
}
