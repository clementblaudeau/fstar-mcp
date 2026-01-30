//! MCP tool implementations for F* IDE.

use crate::fstar::{FStarConfig, IdeLookupResponse};
use crate::session::{
    AutocompleteResponse, CloseSessionResponse, CompletionInfo, CreateFStarResponse,
    DiagnosticInfo, FragmentInfo, LookupResponse, RangeInfo, RestartSolverResponse,
    SessionManager, TypecheckResponse, UpdateBufferResponse,
};
use async_trait::async_trait;
use pmcp::types::capabilities::ServerCapabilities;
use pmcp::types::ToolInfo;
use pmcp::{Server, ToolHandler};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

// Shared session manager for all tools
lazy_static::lazy_static! {
    static ref SESSION_MANAGER: Arc<SessionManager> = Arc::new(SessionManager::new());
}

// ============================================================================
// Tool: create_fstar
// ============================================================================

pub struct CreateFStarTool;

#[derive(Debug, Deserialize)]
struct CreateFStarArgs {
    file_path: String,
    config_path: String,
}

#[async_trait]
impl ToolHandler for CreateFStarTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: CreateFStarArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let file_path = PathBuf::from(&params.file_path);
        let config_path = PathBuf::from(&params.config_path);

        // Load configuration
        let config = FStarConfig::from_file_with_env(&config_path)
            .map_err(|e| pmcp::Error::validation(format!("Failed to load config: {}", e)))?;

        // Create session
        let session_id = SESSION_MANAGER
            .create_session(&file_path, config)
            .await
            .map_err(|e| pmcp::Error::Internal(format!("Failed to create session: {}", e)))?;

        // Read file contents
        let code = tokio::fs::read_to_string(&file_path)
            .await
            .map_err(|e| pmcp::Error::validation(format!("Failed to read file: {}", e)))?;

        // Run initial typecheck
        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.typecheck(&code).await
            } else {
                return Err(pmcp::Error::Internal("Session disappeared".to_string()));
            }
        };

        // Get created_at timestamp
        let created_at = {
            let sessions = SESSION_MANAGER.sessions.read().await;
            sessions.get(&session_id)
                .map(|s| s.created_at.to_rfc3339())
                .unwrap_or_default()
        };

        match result {
            Ok(fb_result) => {
                let has_errors = fb_result.diagnostics.iter().any(|d| d.level == "error");

                let response = CreateFStarResponse {
                    session_id: session_id.clone(),
                    status: if has_errors {
                        "error".to_string()
                    } else {
                        "ok".to_string()
                    },
                    diagnostics: fb_result.diagnostics.iter().map(DiagnosticInfo::from).collect(),
                    fragments: fb_result.fragments.iter().map(FragmentInfo::from).collect(),
                    created_at,
                };

                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "session_id": session_id,
                "status": "error",
                "error": format!("Typecheck failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "create_fstar",
            Some("Create a new F* session for a file and run initial typecheck. Returns session ID for subsequent operations.".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the F* file to typecheck"
                    },
                    "config_path": {
                        "type": "string",
                        "description": "Path to the .fst.config.json configuration file"
                    }
                },
                "required": ["file_path", "config_path"]
            }),
        ))
    }
}

// ============================================================================
// Tool: typecheck_buffer
// ============================================================================

pub struct TypecheckBufferTool;

#[derive(Debug, Deserialize)]
struct TypecheckBufferArgs {
    session_id: String,
    code: String,
    kind: Option<String>,
    lax: Option<bool>,
    to_line: Option<u32>,
    to_column: Option<u32>,
}

#[async_trait]
impl ToolHandler for TypecheckBufferTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: TypecheckBufferArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        // lax: true is a shortcut for kind: "lax"
        let kind = if params.lax.unwrap_or(false) {
            "lax".to_string()
        } else {
            params.kind.unwrap_or_else(|| "full".to_string())
        };
        
        let to_position = match (params.to_line, params.to_column) {
            (Some(l), Some(c)) => Some((l, c)),
            _ => None,
        };

        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => session.typecheck_with_kind(&params.code, &kind, to_position).await,
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(fb_result) => {
                let has_errors = fb_result.diagnostics.iter().any(|d| d.level == "error");

                let response = TypecheckResponse {
                    status: if has_errors {
                        "error".to_string()
                    } else {
                        "ok".to_string()
                    },
                    diagnostics: fb_result.diagnostics.iter().map(DiagnosticInfo::from).collect(),
                    fragments: fb_result.fragments.iter().map(FragmentInfo::from).collect(),
                };

                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "status": "error",
                "error": format!("Typecheck failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "typecheck_buffer",
            Some("Typecheck code in an existing F* session".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "code": {
                        "type": "string",
                        "description": "The F* code to typecheck"
                    },
                    "lax": {
                        "type": "boolean",
                        "description": "If true, use lax mode (admits all SMT queries). Shortcut for kind='lax'"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["full", "lax", "cache", "reload-deps", "verify-to-position", "lax-to-position"],
                        "description": "Typecheck kind (default: full). Overridden by lax=true"
                    },
                    "to_line": {
                        "type": "integer",
                        "description": "Line to typecheck to (for position-based kinds)"
                    },
                    "to_column": {
                        "type": "integer",
                        "description": "Column to typecheck to (for position-based kinds)"
                    }
                },
                "required": ["session_id", "code"]
            }),
        ))
    }
}

// ============================================================================
// Tool: update_buffer
// ============================================================================

pub struct UpdateBufferTool;

#[derive(Debug, Deserialize)]
struct UpdateBufferArgs {
    session_id: String,
    file_path: String,
    contents: String,
}

#[async_trait]
impl ToolHandler for UpdateBufferTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: UpdateBufferArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => session.process.vfs_add(Some(&params.file_path), &params.contents).await,
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(()) => {
                let response = UpdateBufferResponse {
                    status: "ok".to_string(),
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "status": "error",
                "error": format!("vfs-add failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "update_buffer",
            Some("Add or update a file in F*'s virtual file system (vfs-add)".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Path to the file in the virtual file system"
                    },
                    "contents": {
                        "type": "string",
                        "description": "Contents of the file"
                    }
                },
                "required": ["session_id", "file_path", "contents"]
            }),
        ))
    }
}

// ============================================================================
// Tool: lookup_symbol
// ============================================================================

pub struct LookupSymbolTool;

#[derive(Debug, Deserialize)]
struct LookupSymbolArgs {
    session_id: String,
    file_path: String,
    line: u32,
    column: u32,
    symbol: String,
}

#[async_trait]
impl ToolHandler for LookupSymbolTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: LookupSymbolArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => {
                    session
                        .process
                        .lookup(&params.file_path, params.line, params.column, &params.symbol)
                        .await
                }
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(Some(lookup)) => {
                let response = match lookup {
                    IdeLookupResponse::Symbol(s) => LookupResponse {
                        kind: "symbol".to_string(),
                        name: Some(s.name),
                        type_info: s.type_info,
                        documentation: s.documentation,
                        defined_at: s.defined_at.as_ref().map(RangeInfo::from),
                    },
                    IdeLookupResponse::Module(m) => LookupResponse {
                        kind: "module".to_string(),
                        name: Some(m.name),
                        type_info: None,
                        documentation: None,
                        defined_at: Some(RangeInfo {
                            file: m.path,
                            start_line: 1,
                            start_column: 0,
                            end_line: 1,
                            end_column: 0,
                        }),
                    },
                };
                Ok(serde_json::to_value(response)?)
            }
            Ok(None) => {
                let response = LookupResponse {
                    kind: "not_found".to_string(),
                    name: None,
                    type_info: None,
                    documentation: None,
                    defined_at: None,
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "kind": "error",
                "error": format!("Lookup failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "lookup_symbol",
            Some("Look up type information, documentation, and definition location for a symbol".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Path to the file containing the symbol"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (1-based)"
                    },
                    "column": {
                        "type": "integer",
                        "description": "Column number (0-based)"
                    },
                    "symbol": {
                        "type": "string",
                        "description": "The symbol to look up"
                    }
                },
                "required": ["session_id", "file_path", "line", "column", "symbol"]
            }),
        ))
    }
}

// ============================================================================
// Tool: autocomplete
// ============================================================================

pub struct AutocompleteTool;

#[derive(Debug, Deserialize)]
struct AutocompleteArgs {
    session_id: String,
    partial_symbol: String,
}

#[async_trait]
impl ToolHandler for AutocompleteTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: AutocompleteArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => session.process.autocomplete(&params.partial_symbol).await,
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(completions) => {
                let response = AutocompleteResponse {
                    completions: completions
                        .into_iter()
                        .map(|(match_len, annotation, candidate)| CompletionInfo {
                            match_length: match_len,
                            annotation,
                            candidate,
                        })
                        .collect(),
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "error": format!("Autocomplete failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "autocomplete",
            Some("Get autocomplete suggestions for a partial symbol".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "partial_symbol": {
                        "type": "string",
                        "description": "Partial symbol to complete"
                    }
                },
                "required": ["session_id", "partial_symbol"]
            }),
        ))
    }
}

// ============================================================================
// Tool: restart_solver
// ============================================================================

pub struct RestartSolverTool;

#[derive(Debug, Deserialize)]
struct RestartSolverArgs {
    session_id: String,
}

#[async_trait]
impl ToolHandler for RestartSolverTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: RestartSolverArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => session.process.restart_solver().await,
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(()) => {
                let response = RestartSolverResponse {
                    status: "ok".to_string(),
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "status": "error",
                "error": format!("Restart solver failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "restart_solver",
            Some("Restart the Z3 SMT solver for a session".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    }
                },
                "required": ["session_id"]
            }),
        ))
    }
}

// ============================================================================
// Tool: close_session
// ============================================================================

pub struct CloseSessionTool;

#[derive(Debug, Deserialize)]
struct CloseSessionArgs {
    session_id: String,
}

#[async_trait]
impl ToolHandler for CloseSessionTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: CloseSessionArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        match SESSION_MANAGER.close_session(&params.session_id).await {
            Ok(()) => {
                let response = CloseSessionResponse {
                    status: "ok".to_string(),
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "status": "error",
                "error": format!("Close session failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "close_session",
            Some("Close an F* session and clean up resources".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    }
                },
                "required": ["session_id"]
            }),
        ))
    }
}

// ============================================================================
// Tool: list_sessions
// ============================================================================

pub struct ListSessionsTool;

#[async_trait]
impl ToolHandler for ListSessionsTool {
    async fn handle(&self, _args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let sessions = SESSION_MANAGER.list_sessions().await;
        
        Ok(json!({
            "sessions": sessions,
            "count": sessions.len()
        }))
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "list_sessions",
            Some("List all active F* sessions with status information".to_string()),
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        ))
    }
}

// ============================================================================
// Tool: lookup_by_name  
// ============================================================================

pub struct LookupByNameTool;

#[derive(Debug, Deserialize)]
struct LookupByNameArgs {
    session_id: String,
    name: String,
}

#[async_trait]
impl ToolHandler for LookupByNameTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: LookupByNameArgs = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        // Get the file path from the session
        let file_path = {
            let sessions = SESSION_MANAGER.sessions.read().await;
            match sessions.get(&params.session_id) {
                Some(session) => session.file_path.to_string_lossy().to_string(),
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        // Lookup at line 1, column 0 with the given symbol name
        // This is a simplified lookup that doesn't require position
        let result = {
            let mut sessions = SESSION_MANAGER.sessions.write().await;
            match sessions.get_mut(&params.session_id) {
                Some(session) => {
                    session.process
                        .lookup(&file_path, 1, 0, &params.name)
                        .await
                }
                None => {
                    return Err(pmcp::Error::validation(format!(
                        "Session not found: {}",
                        params.session_id
                    )));
                }
            }
        };

        match result {
            Ok(Some(lookup)) => {
                let response = match lookup {
                    IdeLookupResponse::Symbol(s) => LookupResponse {
                        kind: "symbol".to_string(),
                        name: Some(s.name),
                        type_info: s.type_info,
                        documentation: s.documentation,
                        defined_at: s.defined_at.as_ref().map(RangeInfo::from),
                    },
                    IdeLookupResponse::Module(m) => LookupResponse {
                        kind: "module".to_string(),
                        name: Some(m.name),
                        type_info: None,
                        documentation: None,
                        defined_at: Some(RangeInfo {
                            file: m.path,
                            start_line: 1,
                            start_column: 0,
                            end_line: 1,
                            end_column: 0,
                        }),
                    },
                };
                Ok(serde_json::to_value(response)?)
            }
            Ok(None) => {
                let response = LookupResponse {
                    kind: "not_found".to_string(),
                    name: None,
                    type_info: None,
                    documentation: None,
                    defined_at: None,
                };
                Ok(serde_json::to_value(response)?)
            }
            Err(e) => Ok(json!({
                "kind": "error",
                "error": format!("Lookup failed: {}", e)
            })),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "lookup_by_name",
            Some("Look up a symbol by name in the current scope (simpler than lookup_symbol, doesn't require position)".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "name": {
                        "type": "string",
                        "description": "The fully qualified name to look up (e.g., 'FStar.List.map')"
                    }
                },
                "required": ["session_id", "name"]
            }),
        ))
    }
}

// ============================================================================
// Get Proof Context Tool
// ============================================================================

#[derive(Debug, Deserialize)]
struct GetProofContextInput {
    session_id: String,
    line: Option<u32>,
}

pub struct GetProofContextTool;

#[async_trait]
impl ToolHandler for GetProofContextTool {
    async fn handle(&self, args: Value, _extra: pmcp::RequestHandlerExtra) -> pmcp::Result<Value> {
        let params: GetProofContextInput = serde_json::from_value(args)
            .map_err(|e| pmcp::Error::validation(format!("Invalid arguments: {}", e)))?;

        let sessions = SESSION_MANAGER.sessions.read().await;
        match sessions.get(&params.session_id) {
            Some(session) => {
                if let Some(line) = params.line {
                    // Find proof state at specific line
                    if let Some(proof_state) = session.find_proof_state_at_line(line) {
                        Ok(json!({
                            "found": true,
                            "line": line,
                            "proof_state": proof_state
                        }))
                    } else {
                        Ok(json!({
                            "found": false,
                            "line": line,
                            "message": "No proof state at this line"
                        }))
                    }
                } else {
                    // Return all proof states
                    let proof_states = session.get_proof_states();
                    Ok(json!({
                        "count": proof_states.len(),
                        "proof_states": proof_states
                    }))
                }
            }
            None => Err(pmcp::Error::validation(format!(
                "Session not found: {}",
                params.session_id
            ))),
        }
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            "get_proof_context",
            Some("Get proof obligations and goals at a position. Returns proof states collected during last typecheck. If line is provided, returns proof state at that line; otherwise returns all proof states.".to_string()),
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID from create_fstar"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Optional line number to get proof state at"
                    }
                },
                "required": ["session_id"]
            }),
        ))
    }
}

// ============================================================================
// Server Builder
// ============================================================================

pub fn create_fstar_server() -> Result<Server, Box<dyn std::error::Error>> {
    let server = Server::builder()
        .name("fstar-mcp")
        .version("0.1.0")
        .capabilities(ServerCapabilities::tools_only())
        .tool("create_fstar", CreateFStarTool)
        .tool("list_sessions", ListSessionsTool)
        .tool("typecheck_buffer", TypecheckBufferTool)
        .tool("update_buffer", UpdateBufferTool)
        .tool("lookup_symbol", LookupSymbolTool)
        .tool("lookup_by_name", LookupByNameTool)
        .tool("autocomplete", AutocompleteTool)
        .tool("restart_solver", RestartSolverTool)
        .tool("close_session", CloseSessionTool)
        .tool("get_proof_context", GetProofContextTool)
        .build()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    Ok(server)
}
