//! F* configuration handling.
//!
//! Mirrors the .fst.config.json format used by fstar-vscode-assistant.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),
}

/// F* configuration from .fst.config.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FStarConfig {
    /// Include directories (--include paths)
    #[serde(default)]
    pub include_dirs: Vec<String>,

    /// Other options to pass to fstar.exe
    #[serde(default)]
    pub options: Vec<String>,

    /// Path to fstar.exe (defaults to "fstar.exe")
    #[serde(default)]
    pub fstar_exe: Option<String>,

    /// Working directory for fstar.exe
    #[serde(default)]
    pub cwd: Option<String>,
}

impl FStarConfig {
    /// Load and resolve environment variables in the config
    pub fn from_file_with_env(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let resolved = substitute_env_vars(&contents)?;
        let mut config: FStarConfig = serde_json::from_str(&resolved)?;

        // If cwd isn't specified, use the config file's directory
        if config.cwd.is_none() {
            if let Some(parent) = path.parent() {
                config.cwd = Some(parent.to_string_lossy().to_string());
            }
        }

        Ok(config)
    }

    /// Get the F* executable path (with default)
    pub fn fstar_exe(&self) -> &str {
        self.fstar_exe.as_deref().unwrap_or("fstar.exe")
    }

    /// Get the working directory (with default)
    pub fn cwd_or(&self, default: &Path) -> PathBuf {
        self.cwd
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default.to_path_buf())
    }

    /// Build command-line arguments for F* IDE mode
    pub fn build_args(&self, file_path: &str, lax: bool) -> Vec<String> {
        let mut args = vec!["--ide".to_string(), file_path.to_string()];

        if lax {
            args.push("--admit_smt_queries".to_string());
            args.push("true".to_string());
        }

        // Add custom options
        args.extend(self.options.clone());

        // Add include directories
        for dir in &self.include_dirs {
            args.push("--include".to_string());
            args.push(dir.clone());
        }

        args
    }
}

/// Substitute environment variables in a string ($VAR or ${VAR})
fn substitute_env_vars(input: &str) -> Result<String, ConfigError> {
    let mut result = input.to_string();

    // Match ${VAR} style
    let re_braced = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    for cap in re_braced.captures_iter(input) {
        let var_name = &cap[1];
        let value = std::env::var(var_name)
            .map_err(|_| ConfigError::EnvVarNotFound(var_name.to_string()))?;
        result = result.replace(&cap[0], &value);
    }

    // Match $VAR style (not followed by {)
    let re_plain = regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let input_after_braced = result.clone();
    for cap in re_plain.captures_iter(&input_after_braced) {
        let var_name = &cap[1];
        // Skip if this was already handled as ${VAR}
        if input_after_braced.contains(&format!("${{{}}}", var_name)) {
            continue;
        }
        let value = std::env::var(var_name)
            .map_err(|_| ConfigError::EnvVarNotFound(var_name.to_string()))?;
        result = result.replace(&cap[0], &value);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = FStarConfig::default();
        assert_eq!(config.fstar_exe(), "fstar.exe");
        assert!(config.include_dirs.is_empty());
        assert!(config.options.is_empty());
    }

    #[test]
    fn test_build_args() {
        let config = FStarConfig {
            include_dirs: vec!["/path/to/lib".to_string()],
            options: vec!["--cache_dir".to_string(), ".cache".to_string()],
            fstar_exe: Some("fstar".to_string()),
            cwd: Some("/project".to_string()),
        };

        let args = config.build_args("/path/to/Test.fst", false);
        assert_eq!(args[0], "--ide");
        assert_eq!(args[1], "/path/to/Test.fst");
        assert!(args.contains(&"--include".to_string()));
        assert!(args.contains(&"/path/to/lib".to_string()));
        assert!(args.contains(&"--cache_dir".to_string()));
    }

    #[test]
    fn test_build_args_lax() {
        let config = FStarConfig::default();
        let args = config.build_args("Test.fst", true);
        assert!(args.contains(&"--admit_smt_queries".to_string()));
        assert!(args.contains(&"true".to_string()));
    }

    #[test]
    fn test_env_substitution() {
        std::env::set_var("TEST_FSTAR_VAR", "test_value");
        let result = substitute_env_vars("path/$TEST_FSTAR_VAR/file").unwrap();
        assert_eq!(result, "path/test_value/file");

        let result2 = substitute_env_vars("path/${TEST_FSTAR_VAR}/file").unwrap();
        assert_eq!(result2, "path/test_value/file");
        std::env::remove_var("TEST_FSTAR_VAR");
    }
}
