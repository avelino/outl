//! JSON-RPC 2.0 message shapes used by the MCP transport.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 error codes we surface.
pub const PARSE_ERROR: i64 = -32700;
/// Method not recognized.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Invalid params.
pub const INVALID_PARAMS: i64 = -32602;
/// Generic server-side error.
pub const INTERNAL_ERROR: i64 = -32603;

/// Inbound JSON-RPC request.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol marker. Always `"2.0"`.
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Request id. Notifications omit the field.
    pub id: Option<Value>,
    /// Method name.
    pub method: String,
    /// Parameters (object or array, depending on method).
    pub params: Option<Value>,
}

/// Outbound JSON-RPC response.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    /// Protocol marker. Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Echo of the request id.
    pub id: Value,
    /// Successful result payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error envelope, when the call failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcErrorBody>,
}

/// JSON-RPC error body.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcErrorBody {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional data payload (we don't use it today).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// Build a successful response.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcErrorBody {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Internal error helper for dispatch. Codes mirror JSON-RPC standard.
#[derive(Debug, Clone)]
pub struct JsonRpcError {
    /// Numeric error code (JSON-RPC standard or custom).
    pub code: i64,
    /// Human-readable message.
    pub message: String,
}

impl JsonRpcError {
    /// Build an invalid-params error.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
        }
    }

    /// Build a method-not-found error.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: format!("method `{method}` not implemented"),
        }
    }

    /// Build a server-side internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: message.into(),
        }
    }
}
