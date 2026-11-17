//! The operations of the tasknotes-spec conformance suite that the task crate
//! implements, in the shapes the adapter contract asks (`ADAPTER_CONTRACT.md`).
//!
//! Portable: no I/O and no clock; the caller passes `today`.

use serde_json::{Map, Value, json};

/// The name the adapter reports for this implementation.
pub const IMPLEMENTATION: &str = "mdbase-tasks (MadameLabs)";
/// The `tasknotes-spec` version the claim targets (the clone's `package.json`).
pub const SPEC_VERSION: &str = "0.3.0";
/// The profiles claimed.
pub const PROFILES: &[&str] = &["core-lite"];
/// The capabilities claimed.
pub const CAPABILITIES: &[&str] = &["dependencies", "time-tracking", "archive"];
/// The validation modes claimed.
pub const VALIDATION_MODES: &[&str] = &["strict"];

/// A failed operation: a stable code, a human message, and the field when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpError {
    /// Stable code, e.g. `unsupported_operation`.
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
    /// The input field at fault, when there is one.
    pub field: Option<String>,
}

impl OpError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
        }
    }
}

/// The claim the adapter reports, as the `meta.claim` result.
pub fn claim() -> Value {
    json!({
        "implementation": IMPLEMENTATION,
        "version": env!("CARGO_PKG_VERSION"),
        "spec_version": SPEC_VERSION,
        "validation_modes": VALIDATION_MODES,
        "profiles": PROFILES,
        "capabilities": CAPABILITIES,
    })
}

/// Runs one operation over its input object. `Ok` is the result; `Err` is the
/// failure the envelope reports.
pub fn execute(operation: &str, input: &Value, _today: &str) -> Result<Value, OpError> {
    if !input.is_object() {
        return Err(OpError::new(
            "invalid_input",
            "the operation input must be a JSON object",
        ));
    }
    match operation {
        "meta.claim" => Ok(claim()),
        _ => Err(OpError::new(
            "unsupported_operation",
            format!("operation {operation} is not implemented"),
        )),
    }
}

/// The envelope for `execute`'s outcome, as the adapter contract shapes it.
pub fn envelope(operation: &str, outcome: Result<Value, OpError>) -> Value {
    match outcome {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(e) => {
            let mut details = Map::new();
            details.insert("operation".into(), json!(operation));
            details.insert("code".into(), json!(e.code));
            details.insert("message".into(), json!(e.message));
            if let Some(field) = e.field {
                details.insert("field".into(), json!(field));
            }
            json!({
                "ok": false,
                "error": e.message,
                "error_details": Value::Object(details),
            })
        }
    }
}
