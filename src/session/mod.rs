//! Session management for F* MCP server.

pub mod types;

use crate::fstar::{FStarConfig, FStarProcess, FullBufferResult, ProcessError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

pub use types::*;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    NotFound(String),
    #[error("Failed to create session: {0}")]
    CreateError(#[from] ProcessError),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::fstar::ConfigError),
}

/// A single F* session
pub struct Session {
    pub id: String,
    pub file_path: PathBuf,
    pub process: FStarProcess,
}

impl Session {
    /// Create a new session
    pub async fn new(
        file_path: &Path,
        config: FStarConfig,
        lax: bool,
    ) -> Result<Self, SessionError> {
        let id = Uuid::new_v4().to_string();
        let process = FStarProcess::spawn(config, file_path, lax).await?;

        Ok(Session {
            id,
            file_path: file_path.to_path_buf(),
            process,
        })
    }

    /// Run initial typecheck
    pub async fn typecheck(&mut self, code: &str) -> Result<FullBufferResult, ProcessError> {
        self.process.full_buffer_query(code, "full", None).await
    }

    /// Run typecheck with specified kind
    pub async fn typecheck_with_kind(
        &mut self,
        code: &str,
        kind: &str,
        to_position: Option<(u32, u32)>,
    ) -> Result<FullBufferResult, ProcessError> {
        self.process.full_buffer_query(code, kind, to_position).await
    }
}

/// Manages multiple F* sessions
pub struct SessionManager {
    pub sessions: Arc<RwLock<HashMap<String, Session>>>,
    /// Maps file paths to session IDs for auto-replacement
    file_to_session: Arc<RwLock<HashMap<PathBuf, String>>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            file_to_session: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new session, replacing any existing session for the same file
    pub async fn create_session(
        &self,
        file_path: &Path,
        config: FStarConfig,
    ) -> Result<String, SessionError> {
        // Check for existing session for this file
        let existing_session_id = {
            let file_map = self.file_to_session.read().await;
            file_map.get(file_path).cloned()
        };

        // Close existing session if any
        if let Some(old_id) = existing_session_id {
            self.close_session(&old_id).await.ok();
        }

        // Create new session
        let session = Session::new(file_path, config, false).await?;
        let session_id = session.id.clone();
        let file_path_owned = file_path.to_path_buf();

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), session);
        }

        // Update file mapping
        {
            let mut file_map = self.file_to_session.write().await;
            file_map.insert(file_path_owned, session_id.clone());
        }

        Ok(session_id)
    }

    /// Close a session
    pub async fn close_session(&self, session_id: &str) -> Result<(), SessionError> {
        let session = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(session_id)
        };

        if let Some(mut session) = session {
            // Remove from file mapping
            {
                let mut file_map = self.file_to_session.write().await;
                file_map.remove(&session.file_path);
            }

            // Kill the process
            session.process.kill().await.ok();
            Ok(())
        } else {
            Err(SessionError::NotFound(session_id.to_string()))
        }
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        // Sessions will be cleaned up when dropped
    }
}
