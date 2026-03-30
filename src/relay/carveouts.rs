use serde_json::Value;

/// Strip `params.capabilities.sampling` from an MCP `initialize` request.
///
/// Returns the (possibly modified) message bytes. If parsing fails or the
/// message is not an initialize request, the original bytes are returned
/// unmodified — the relay forwards unknown/malformed messages as-is per D-11.
pub fn apply_sampling_carveout(message: &[u8]) -> Vec<u8> {
    let Ok(mut json) = serde_json::from_slice::<Value>(message) else {
        return message.to_vec();
    };

    // Only apply to initialize requests
    let is_initialize = json
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|m| m == "initialize");

    if !is_initialize {
        return message.to_vec();
    }

    // Strip params.capabilities.sampling if present
    let modified = json
        .get_mut("params")
        .and_then(|p| p.get_mut("capabilities"))
        .and_then(|c| c.as_object_mut())
        .map(|caps| caps.remove("sampling").is_some())
        .unwrap_or(false);

    if modified {
        // Re-serialize — serde_json produces compact JSON
        serde_json::to_vec(&json).unwrap_or_else(|_| message.to_vec())
    } else {
        message.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sampling_from_initialize() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "sampling": {},
                    "roots": { "listChanged": true }
                },
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        });
        let input = serde_json::to_vec(&msg).unwrap();
        let output = apply_sampling_carveout(&input);
        let result: Value = serde_json::from_slice(&output).unwrap();

        // sampling should be gone
        assert!(result["params"]["capabilities"]["sampling"].is_null());
        // roots should remain
        assert!(result["params"]["capabilities"]["roots"].is_object());
        // other fields intact
        assert_eq!(result["method"], "initialize");
        assert_eq!(result["id"], 1);
    }

    #[test]
    fn no_op_when_no_sampling() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": { "listChanged": true }
                }
            }
        });
        let input = serde_json::to_vec(&msg).unwrap();
        let output = apply_sampling_carveout(&input);
        // Should be unchanged (returned as-is since no modification needed)
        assert_eq!(input, output);
    }

    #[test]
    fn no_op_for_non_initialize() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        let input = serde_json::to_vec(&msg).unwrap();
        let output = apply_sampling_carveout(&input);
        assert_eq!(input, output);
    }

    #[test]
    fn no_op_for_malformed_json() {
        let input = b"not json at all";
        let output = apply_sampling_carveout(input);
        assert_eq!(output, input);
    }

    #[test]
    fn no_op_when_no_params() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize"
        });
        let input = serde_json::to_vec(&msg).unwrap();
        let output = apply_sampling_carveout(&input);
        assert_eq!(input, output);
    }

    #[test]
    fn no_op_when_no_capabilities() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        });
        let input = serde_json::to_vec(&msg).unwrap();
        let output = apply_sampling_carveout(&input);
        assert_eq!(input, output);
    }
}
