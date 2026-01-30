//! Mock F* IDE process for testing.
//!
//! This module provides a mock implementation of the F* IDE protocol
//! that can be used for testing the MCP server without a real F* installation.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Represents a canned response for the mock F* process.
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Protocol info sent on startup
    ProtocolInfo { version: i32, features: Vec<String> },
    /// Success response
    Success { response: serde_json::Value },
    /// Failure response
    Failure { message: String },
    /// Full buffer progress messages (simulates streaming)
    FullBufferStream { messages: Vec<serde_json::Value> },
}

/// Configuration for mock F* behavior.
#[derive(Debug, Clone, Default)]
pub struct MockFStarConfig {
    /// Responses keyed by query type (e.g., "full-buffer", "lookup", "vfs-add")
    pub responses: HashMap<String, MockResponse>,
    /// Whether to simulate errors
    pub simulate_crash: bool,
    /// Delay in milliseconds before responding
    pub response_delay_ms: u64,
}

impl MockFStarConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a successful full-buffer response with no errors.
    pub fn with_successful_typecheck(mut self) -> Self {
        self.responses.insert(
            "full-buffer".to_string(),
            MockResponse::FullBufferStream {
                messages: vec![
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": { "stage": "full-buffer-started" }
                    }),
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": {
                            "stage": "full-buffer-fragment-ok",
                            "ranges": {
                                "fname": "Test.fst",
                                "beg": [1, 0],
                                "end": [10, 0]
                            },
                            "code-fragment": {
                                "code-digest": "abc123",
                                "range": { "fname": "Test.fst", "beg": [1, 0], "end": [10, 0] }
                            }
                        }
                    }),
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": { "stage": "full-buffer-finished" }
                    }),
                ],
            },
        );
        self
    }

    /// Add a full-buffer response with a type error.
    pub fn with_typecheck_error(mut self, line: u32, message: &str) -> Self {
        self.responses.insert(
            "full-buffer".to_string(),
            MockResponse::FullBufferStream {
                messages: vec![
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": { "stage": "full-buffer-started" }
                    }),
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": {
                            "stage": "full-buffer-fragment-started",
                            "ranges": { "fname": "Test.fst", "beg": [1, 0], "end": [line, 0] }
                        }
                    }),
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": {
                            "stage": "full-buffer-fragment-failed",
                            "ranges": { "fname": "Test.fst", "beg": [1, 0], "end": [line, 0] }
                        }
                    }),
                    serde_json::json!({
                        "kind": "response",
                        "status": "success",
                        "response": [{
                            "level": "error",
                            "number": 19,
                            "message": message,
                            "ranges": [{ "fname": "Test.fst", "beg": [line, 0], "end": [line, 10] }]
                        }]
                    }),
                    serde_json::json!({
                        "kind": "message",
                        "level": "progress",
                        "contents": { "stage": "full-buffer-finished" }
                    }),
                ],
            },
        );
        self
    }

    /// Add a successful lookup response for a symbol.
    pub fn with_symbol_lookup(mut self, name: &str, type_str: &str) -> Self {
        self.responses.insert(
            "lookup".to_string(),
            MockResponse::Success {
                response: serde_json::json!({
                    "kind": "symbol",
                    "name": name,
                    "type": type_str,
                    "documentation": "",
                    "definition": "",
                    "defined-at": {
                        "fname": "Prims.fst",
                        "beg": [1, 0],
                        "end": [1, 10]
                    }
                }),
            },
        );
        self
    }

    /// Add autocomplete suggestions.
    pub fn with_autocomplete(mut self, suggestions: Vec<(&str, &str)>) -> Self {
        let completions: Vec<serde_json::Value> = suggestions
            .into_iter()
            .map(|(annotation, candidate)| {
                serde_json::json!([candidate.len(), annotation, candidate])
            })
            .collect();

        self.responses.insert(
            "autocomplete".to_string(),
            MockResponse::Success {
                response: serde_json::Value::Array(completions),
            },
        );
        self
    }

    /// Add successful vfs-add response.
    pub fn with_vfs_add_success(mut self) -> Self {
        self.responses.insert(
            "vfs-add".to_string(),
            MockResponse::Success {
                response: serde_json::Value::Null,
            },
        );
        self
    }
}

/// A mock F* IDE process that responds to queries according to configuration.
pub struct MockFStarProcess {
    config: MockFStarConfig,
    query_id_counter: u64,
}

impl MockFStarProcess {
    pub fn new(config: MockFStarConfig) -> Self {
        Self {
            config,
            query_id_counter: 0,
        }
    }

    /// Generate the protocol-info message sent on startup.
    pub fn protocol_info(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "protocol-info",
            "version": 3,
            "features": ["full-buffer", "vfs-add", "lookup", "autocomplete"]
        })
    }

    /// Process a query and return response(s).
    pub fn process_query(&mut self, query: &serde_json::Value) -> Vec<serde_json::Value> {
        let query_id = query
            .get("query-id")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();

        let query_type = query
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if self.config.simulate_crash {
            return vec![];
        }

        match self.config.responses.get(query_type) {
            Some(MockResponse::Success { response }) => {
                vec![serde_json::json!({
                    "query-id": query_id,
                    "kind": "response",
                    "status": "success",
                    "response": response
                })]
            }
            Some(MockResponse::Failure { message }) => {
                vec![serde_json::json!({
                    "query-id": query_id,
                    "kind": "response",
                    "status": "failure",
                    "response": message
                })]
            }
            Some(MockResponse::FullBufferStream { messages }) => {
                let mut responses = Vec::new();
                for (i, msg) in messages.iter().enumerate() {
                    let mut response = msg.clone();
                    // Add query-id with fractional component for streaming
                    let qid = if i == 0 {
                        query_id.clone()
                    } else {
                        format!("{}.{}", query_id, i)
                    };
                    if let Some(obj) = response.as_object_mut() {
                        obj.insert("query-id".to_string(), serde_json::Value::String(qid));
                    }
                    responses.push(response);
                }
                responses
            }
            Some(MockResponse::ProtocolInfo { version, features }) => {
                vec![serde_json::json!({
                    "kind": "protocol-info",
                    "version": version,
                    "features": features
                })]
            }
            None => {
                // Default: return success with null response
                vec![serde_json::json!({
                    "query-id": query_id,
                    "kind": "response",
                    "status": "success",
                    "response": null
                })]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_protocol_info() {
        let mock = MockFStarProcess::new(MockFStarConfig::new());
        let info = mock.protocol_info();

        assert_eq!(info["kind"], "protocol-info");
        assert_eq!(info["version"], 3);
        assert!(info["features"].as_array().unwrap().contains(&serde_json::json!("full-buffer")));
    }

    #[test]
    fn test_mock_successful_typecheck() {
        let config = MockFStarConfig::new().with_successful_typecheck();
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "1",
            "query": "full-buffer",
            "args": { "kind": "full", "code": "let x = 1", "line": 0, "column": 0 }
        });

        let responses = mock.process_query(&query);
        assert_eq!(responses.len(), 3); // started, fragment-ok, finished

        // Check first response is full-buffer-started
        assert_eq!(
            responses[0]["contents"]["stage"],
            "full-buffer-started"
        );

        // Check last response is full-buffer-finished
        assert_eq!(
            responses[2]["contents"]["stage"],
            "full-buffer-finished"
        );
    }

    #[test]
    fn test_mock_typecheck_error() {
        let config = MockFStarConfig::new().with_typecheck_error(5, "Type mismatch");
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "1",
            "query": "full-buffer",
            "args": { "kind": "full", "code": "let x : int = true", "line": 0, "column": 0 }
        });

        let responses = mock.process_query(&query);
        assert!(responses.len() >= 3);

        // Find the error response
        let error_response = responses.iter().find(|r| {
            r.get("kind").and_then(|k| k.as_str()) == Some("response")
        });
        assert!(error_response.is_some());

        let diags = error_response.unwrap()["response"].as_array().unwrap();
        assert_eq!(diags[0]["level"], "error");
        assert!(diags[0]["message"].as_str().unwrap().contains("Type mismatch"));
    }

    #[test]
    fn test_mock_lookup_symbol() {
        let config = MockFStarConfig::new().with_symbol_lookup("FStar.List.map", "('a -> 'b) -> list 'a -> list 'b");
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "2",
            "query": "lookup",
            "args": { "symbol": "map", "context": "code" }
        });

        let responses = mock.process_query(&query);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["status"], "success");
        assert_eq!(responses[0]["response"]["kind"], "symbol");
        assert_eq!(responses[0]["response"]["name"], "FStar.List.map");
    }

    #[test]
    fn test_mock_autocomplete() {
        let config = MockFStarConfig::new().with_autocomplete(vec![
            ("val", "FStar.List.map"),
            ("val", "FStar.List.mapT"),
            ("val", "FStar.List.mapi"),
        ]);
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "3",
            "query": "autocomplete",
            "args": { "partial-symbol": "map", "context": "code" }
        });

        let responses = mock.process_query(&query);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["status"], "success");

        let completions = responses[0]["response"].as_array().unwrap();
        assert_eq!(completions.len(), 3);
    }

    #[test]
    fn test_mock_vfs_add() {
        let config = MockFStarConfig::new().with_vfs_add_success();
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "4",
            "query": "vfs-add",
            "args": { "filename": "Test.fst", "contents": "module Test" }
        });

        let responses = mock.process_query(&query);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["status"], "success");
    }

    #[test]
    fn test_mock_crash_simulation() {
        let mut config = MockFStarConfig::new();
        config.simulate_crash = true;
        let mut mock = MockFStarProcess::new(config);

        let query = serde_json::json!({
            "query-id": "1",
            "query": "full-buffer",
            "args": {}
        });

        let responses = mock.process_query(&query);
        assert!(responses.is_empty());
    }
}
