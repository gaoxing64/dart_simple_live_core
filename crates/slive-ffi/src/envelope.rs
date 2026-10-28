//! JSON envelope：所有异步结果统一封装，`schema_version` 便于宿主做版本判断。

pub const SCHEMA_VERSION: u32 = 1;

pub fn ok(data: serde_json::Value) -> String {
    serde_json::json!({ "schema_version": SCHEMA_VERSION, "ok": true, "data": data }).to_string()
}

pub fn err(kind: &str, message: &str, status_code: i32) -> String {
    serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "ok": false,
        "error": { "kind": kind, "message": message, "status_code": status_code }
    })
    .to_string()
}

/// `SliveError` → (kind, message, status_code)
pub fn classify(e: &slive_core::error::SliveError) -> (&'static str, String, i32) {
    use slive_core::error::SliveError as E;
    match e {
        E::Network(m) => ("network", m.clone(), 0),
        E::Http { status, .. } => ("http", e.to_string(), *status as i32),
        E::Api { code, message } => ("api", message.clone(), *code),
        E::Decode(m) => ("decode", m.clone(), 0),
        E::Websocket(m) => ("websocket", m.clone(), 0),
        E::NotFound(m) => ("not_found", m.clone(), 0),
        E::InvalidInput(m) => ("invalid_input", m.clone(), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_envelope_shape() {
        let v: serde_json::Value = serde_json::from_str(&ok(serde_json::json!({"a":1}))).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["a"], 1);
    }

    #[test]
    fn err_envelope_shape() {
        let v: serde_json::Value = serde_json::from_str(&err("network", "x", 0)).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["kind"], "network");
    }

    #[test]
    fn classify_api_keeps_code() {
        let e = slive_core::error::SliveError::Api {
            code: -352,
            message: "风控".into(),
        };
        let (kind, msg, code) = classify(&e);
        assert_eq!((kind, msg.as_str(), code), ("api", "风控", -352));
    }
}
