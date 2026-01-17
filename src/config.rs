use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub gmail: GmailConfig,
    pub ai: AiConfig,
    pub tasks: TasksConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: String,
    pub api_key: String,
    pub model_analysis: String,
    pub model_reply: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksConfig {
    pub provider: String,
    pub file_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gmail: GmailConfig {
                client_id: String::new(),
                client_secret: String::new(),
            },
            ai: AiConfig {
                provider: "openrouter".to_string(),
                api_key: String::new(),
                model_analysis: "google/gemini-2.0-flash-001".to_string(),
                model_reply: "anthropic/claude-sonnet-4".to_string(),
            },
            tasks: TasksConfig {
                provider: "local".to_string(),
                file_path: None,
            },
        }
    }
}

impl Config {
    /// Returns the config directory path (~/.clinbox)
    pub fn config_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not find home directory")?;
        Ok(home.join(".clinbox"))
    }

    /// Returns the config file path
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    /// Returns the token file path
    pub fn token_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("token.json"))
    }

    /// Returns the tasks file path
    pub fn tasks_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("tasks.json"))
    }

    /// Load config from file or create default
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .context("Failed to read config file")?;
            let config: Config = serde_json::from_str(&content)
                .context("Failed to parse config file")?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save config to file
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::config_dir()?;
        fs::create_dir_all(&config_dir)
            .context("Failed to create config directory")?;

        let config_path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize config")?;
        fs::write(&config_path, content)
            .context("Failed to write config file")?;

        Ok(())
    }

    /// Check if the config is valid for operation
    pub fn is_valid(&self) -> bool {
        !self.gmail.client_id.is_empty()
            && !self.gmail.client_secret.is_empty()
            && !self.ai.api_key.is_empty()
    }
}
