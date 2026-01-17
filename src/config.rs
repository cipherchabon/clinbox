use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Individual Gmail account configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailAccount {
    pub id: String,
    pub email: Option<String>,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub gmail: GmailConfig,
    pub ai: AiConfig,
    pub tasks: TasksConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    pub accounts: Vec<GmailAccount>,
    pub default_account: Option<String>,
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
                accounts: Vec::new(),
                default_account: None,
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

    /// Returns the tokens directory path (~/.clinbox/tokens)
    pub fn tokens_dir() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("tokens"))
    }

    /// Returns the token file path for a specific account
    pub fn token_path_for_account(account_id: &str) -> Result<PathBuf> {
        Ok(Self::tokens_dir()?.join(format!("{}.json", account_id)))
    }

    /// Returns the legacy token file path (for migration)
    fn legacy_token_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("token.json"))
    }

    /// Returns the tasks file path
    pub fn tasks_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("tasks.json"))
    }

    /// Load config from file or create default, with automatic migration
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let content = fs::read_to_string(&config_path).context("Failed to read config file")?;

            // Try to parse as new format first
            if let Ok(config) = serde_json::from_str::<Config>(&content) {
                return Ok(config);
            }

            // Try to parse as legacy format and migrate
            if let Ok(legacy) = serde_json::from_str::<LegacyConfig>(&content) {
                return Self::migrate_legacy(legacy);
            }

            // Failed to parse either format
            anyhow::bail!("Failed to parse config file. Please check the format.");
        }

        Ok(Config::default())
    }

    /// Migrate from legacy single-account config to new multi-account format
    fn migrate_legacy(legacy: LegacyConfig) -> Result<Self> {
        let mut config = Config {
            gmail: GmailConfig {
                accounts: Vec::new(),
                default_account: None,
            },
            ai: legacy.ai,
            tasks: legacy.tasks,
        };

        // If legacy had credentials, create a "default" account
        if !legacy.gmail.client_id.is_empty() && !legacy.gmail.client_secret.is_empty() {
            let account = GmailAccount {
                id: "default".to_string(),
                email: None,
                client_id: legacy.gmail.client_id,
                client_secret: legacy.gmail.client_secret,
            };
            config.gmail.accounts.push(account);
            config.gmail.default_account = Some("default".to_string());

            // Migrate token file
            let legacy_token = Self::legacy_token_path()?;
            if legacy_token.exists() {
                let tokens_dir = Self::tokens_dir()?;
                fs::create_dir_all(&tokens_dir)?;
                let new_token_path = Self::token_path_for_account("default")?;
                fs::rename(&legacy_token, &new_token_path)
                    .context("Failed to migrate token file")?;
            }
        }

        // Save the migrated config
        config.save()?;

        Ok(config)
    }

    /// Save config to file
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::config_dir()?;
        fs::create_dir_all(&config_dir).context("Failed to create config directory")?;

        let config_path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(&config_path, content).context("Failed to write config file")?;

        Ok(())
    }

    /// Check if the config is valid for operation (has at least one account and AI key)
    pub fn is_valid(&self) -> bool {
        !self.gmail.accounts.is_empty() && !self.ai.api_key.is_empty()
    }

    /// Get account by ID
    pub fn get_account(&self, id: &str) -> Option<&GmailAccount> {
        self.gmail.accounts.iter().find(|a| a.id == id)
    }

    /// Get the default account
    pub fn get_default_account(&self) -> Option<&GmailAccount> {
        if let Some(default_id) = &self.gmail.default_account {
            self.get_account(default_id)
        } else {
            self.gmail.accounts.first()
        }
    }

    /// Add a new account
    pub fn add_account(&mut self, account: GmailAccount) -> Result<()> {
        if self.gmail.accounts.iter().any(|a| a.id == account.id) {
            anyhow::bail!("Account '{}' already exists", account.id);
        }

        // Set as default if it's the first account
        if self.gmail.accounts.is_empty() {
            self.gmail.default_account = Some(account.id.clone());
        }

        self.gmail.accounts.push(account);
        self.save()
    }

    /// Remove an account
    pub fn remove_account(&mut self, id: &str) -> Result<()> {
        let initial_len = self.gmail.accounts.len();
        self.gmail.accounts.retain(|a| a.id != id);

        if self.gmail.accounts.len() == initial_len {
            anyhow::bail!("Account '{}' not found", id);
        }

        // Remove token file
        let token_path = Self::token_path_for_account(id)?;
        if token_path.exists() {
            fs::remove_file(&token_path)?;
        }

        // Update default if needed
        if self.gmail.default_account.as_deref() == Some(id) {
            self.gmail.default_account = self.gmail.accounts.first().map(|a| a.id.clone());
        }

        self.save()
    }

    /// Set the default account
    pub fn set_default_account(&mut self, id: &str) -> Result<()> {
        if !self.gmail.accounts.iter().any(|a| a.id == id) {
            anyhow::bail!("Account '{}' not found", id);
        }

        self.gmail.default_account = Some(id.to_string());
        self.save()
    }

    /// Update account email after OAuth
    #[allow(dead_code)]
    pub fn update_account_email(&mut self, id: &str, email: String) -> Result<()> {
        if let Some(account) = self.gmail.accounts.iter_mut().find(|a| a.id == id) {
            account.email = Some(email);
            self.save()?;
        }
        Ok(())
    }
}

/// Legacy config format for migration
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    gmail: LegacyGmailConfig,
    ai: AiConfig,
    tasks: TasksConfig,
}

#[derive(Debug, Deserialize)]
struct LegacyGmailConfig {
    client_id: String,
    client_secret: String,
}
