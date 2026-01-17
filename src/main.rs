mod ai;
mod config;
mod email;
mod gmail;
mod tasks;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::ai::AiClient;
use crate::config::{Config, GmailAccount};
use crate::gmail::GmailClient;
use crate::tasks::TaskStore;
use crate::tui::{Action, ReplyAction, Tui};

#[derive(Parser)]
#[command(name = "clinbox")]
#[command(about = "A terminal-first email client with AI-powered triage")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Maximum number of emails to fetch
    #[arg(short = 'n', long, default_value = "20")]
    max_emails: u32,

    /// Include all emails (not just unread)
    #[arg(short = 'a', long)]
    all: bool,

    /// Gmail account to use (by ID)
    #[arg(long, global = true)]
    account: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure Clinbox
    Config {
        /// Configuration key (ai.api_key, ai.model)
        key: String,
        /// Value to set
        value: String,
    },
    /// Manage Gmail accounts
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Show pending tasks
    Tasks,
    /// Show configuration status
    Status,
}

#[derive(Subcommand)]
enum AccountAction {
    /// Add a new Gmail account (starts OAuth flow)
    Add {
        /// Account identifier (e.g., "personal", "work")
        id: String,
        /// OAuth client ID (optional if credentials.json exists or another account is configured)
        #[arg(long)]
        client_id: Option<String>,
        /// OAuth client secret (optional if credentials.json exists or another account is configured)
        #[arg(long)]
        client_secret: Option<String>,
    },
    /// List configured accounts
    List,
    /// Remove an account
    Remove {
        /// Account identifier to remove
        id: String,
    },
    /// Set default account
    Default {
        /// Account identifier to set as default
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Config { key, value }) => {
            configure(&key, &value)?;
        }
        Some(Commands::Account { action }) => {
            handle_account_command(action).await?;
        }
        Some(Commands::Tasks) => {
            show_tasks()?;
        }
        Some(Commands::Status) => {
            show_status()?;
        }
        None => {
            run_interactive(cli.max_emails, cli.all, cli.account.as_deref()).await?;
        }
    }

    Ok(())
}

fn configure(key: &str, value: &str) -> Result<()> {
    let mut config = Config::load()?;

    match key {
        "ai.api_key" => config.ai.api_key = value.to_string(),
        "ai.model" => config.ai.model_analysis = value.to_string(),
        _ => anyhow::bail!(
            "Unknown config key: {}. Use 'clinbox account add' to configure Gmail accounts.",
            key
        ),
    }

    config.save()?;
    println!("Configuration updated: {} = {}", key, mask_secret(value));
    Ok(())
}

async fn handle_account_command(action: AccountAction) -> Result<()> {
    match action {
        AccountAction::Add {
            id,
            client_id,
            client_secret,
        } => {
            add_account(&id, client_id.as_deref(), client_secret.as_deref()).await?;
        }
        AccountAction::List => {
            list_accounts()?;
        }
        AccountAction::Remove { id } => {
            remove_account(&id)?;
        }
        AccountAction::Default { id } => {
            set_default_account(&id)?;
        }
    }
    Ok(())
}

async fn add_account(id: &str, client_id: Option<&str>, client_secret: Option<&str>) -> Result<()> {
    // Validate account ID to prevent path traversal
    if !id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("Account ID must only contain alphanumeric characters, '-', and '_'");
    }
    if id.is_empty() || id.len() > 50 {
        anyhow::bail!("Account ID must be 1-50 characters");
    }

    let mut config = Config::load()?;

    // Check if account already exists
    if config.get_account(id).is_some() {
        anyhow::bail!(
            "Account '{}' already exists. Use 'clinbox account remove {}' first.",
            id,
            id
        );
    }

    // Resolve credentials from various sources
    let (resolved_client_id, resolved_client_secret) =
        resolve_credentials(&config, client_id, client_secret)?;

    // Create the account
    let account = GmailAccount {
        id: id.to_string(),
        email: None,
        client_id: resolved_client_id.clone(),
        client_secret: resolved_client_secret.clone(),
    };

    // Run OAuth flow to get token
    println!("Starting OAuth flow for account '{}'...", id);
    GmailClient::oauth_flow(&account).await?;

    // Create client to fetch user email
    let client = GmailClient::new(&account).await?;
    let email = client.fetch_user_email().await?;

    // Add account with email to config
    let account_with_email = GmailAccount {
        id: id.to_string(),
        email: Some(email.clone()),
        client_id: resolved_client_id,
        client_secret: resolved_client_secret,
    };

    config.add_account(account_with_email)?;

    println!("Account '{}' added ({})", id, email);
    Ok(())
}

/// Resolve OAuth credentials from various sources
fn resolve_credentials(
    config: &Config,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<(String, String)> {
    // 1. If both provided explicitly, use them
    if let (Some(id), Some(secret)) = (client_id, client_secret) {
        return Ok((id.to_string(), secret.to_string()));
    }

    // 2. Try to reuse from existing accounts
    if let Some(existing) = config.gmail.accounts.first() {
        println!("Using credentials from existing account '{}'", existing.id);
        return Ok((existing.client_id.clone(), existing.client_secret.clone()));
    }

    // 3. Try to read from credentials.json
    if let Some((id, secret)) = read_credentials_file()? {
        println!("Using credentials from credentials.json");
        return Ok((id, secret));
    }

    // 4. No credentials found
    anyhow::bail!(
        "No credentials found. Provide --client-id and --client-secret, \
        or place a credentials.json file in the current directory or ~/.clinbox/"
    );
}

/// Read credentials from credentials.json file
fn read_credentials_file() -> Result<Option<(String, String)>> {
    use std::fs;

    // Locations to search
    let locations = [
        std::env::current_dir()
            .ok()
            .map(|p| p.join("credentials.json")),
        Config::config_dir()
            .ok()
            .map(|p| p.join("credentials.json")),
    ];

    for location in locations.into_iter().flatten() {
        if location.exists() {
            let content = fs::read_to_string(&location)
                .with_context(|| format!("Failed to read {}", location.display()))?;

            let creds: CredentialsFile = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {}", location.display()))?;

            return Ok(Some((
                creds.installed.client_id,
                creds.installed.client_secret,
            )));
        }
    }

    Ok(None)
}

#[derive(serde::Deserialize)]
struct CredentialsFile {
    installed: InstalledCredentials,
}

#[derive(serde::Deserialize)]
struct InstalledCredentials {
    client_id: String,
    client_secret: String,
}

fn list_accounts() -> Result<()> {
    let config = Config::load()?;

    if config.gmail.accounts.is_empty() {
        println!("No accounts configured.");
        println!("\nAdd an account with:");
        println!(
            "  clinbox account add <id> --client-id <CLIENT_ID> --client-secret <CLIENT_SECRET>"
        );
        return Ok(());
    }

    println!("Accounts:\n");
    for account in &config.gmail.accounts {
        let is_default = config.gmail.default_account.as_deref() == Some(&account.id);
        let marker = if is_default { "* " } else { "  " };
        let default_label = if is_default { " [default]" } else { "" };
        let email = account.email.as_deref().unwrap_or("(email not set)");
        println!("{}{} ({}){}", marker, account.id, email, default_label);
    }

    Ok(())
}

fn remove_account(id: &str) -> Result<()> {
    let mut config = Config::load()?;
    config.remove_account(id)?;
    println!("Account '{}' removed.", id);
    Ok(())
}

fn set_default_account(id: &str) -> Result<()> {
    let mut config = Config::load()?;
    config.set_default_account(id)?;
    println!("Default account set to '{}'.", id);
    Ok(())
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}...{}", &s[..4], &s[s.len() - 4..])
    }
}

fn show_tasks() -> Result<()> {
    let store = TaskStore::load()?;
    let pending = store.pending();

    if pending.is_empty() {
        println!("📭 No pending tasks");
        return Ok(());
    }

    println!("📝 Pending Tasks ({}):\n", pending.len());
    for task in pending {
        let date = task.created_at.format("%Y-%m-%d").to_string();
        println!("  • {} ({})", task.title, date);
        if let Some(desc) = &task.description {
            println!("    {}", desc);
        }
        if let Some(subject) = &task.source_email_subject {
            println!("    📧 From: {}", subject);
        }
        println!();
    }

    Ok(())
}

fn show_status() -> Result<()> {
    let config = Config::load()?;
    let config_dir = Config::config_dir()?;

    println!("Config directory: {}", config_dir.display());
    println!();

    // Gmail accounts
    println!("Gmail Accounts:");
    if config.gmail.accounts.is_empty() {
        println!("  No accounts configured");
    } else {
        for account in &config.gmail.accounts {
            let is_default = config.gmail.default_account.as_deref() == Some(&account.id);
            let marker = if is_default { "* " } else { "  " };
            let default_label = if is_default { " [default]" } else { "" };
            let email = account.email.as_deref().unwrap_or("(not authenticated)");
            println!("{}{}: {}{}", marker, account.id, email, default_label);
        }
    }
    println!();

    // AI configuration
    println!("AI Configuration:");
    println!(
        "  API Key: {}",
        if config.ai.api_key.is_empty() {
            "Not set"
        } else {
            "Set"
        }
    );
    println!("  Model: {}", config.ai.model_analysis);
    println!();

    if !config.is_valid() {
        println!("Configuration incomplete. Run:");
        println!();
        if config.gmail.accounts.is_empty() {
            println!(
                "  clinbox account add <id> --client-id <CLIENT_ID> --client-secret <CLIENT_SECRET>"
            );
        }
        if config.ai.api_key.is_empty() {
            println!("  clinbox config ai.api_key YOUR_OPENROUTER_KEY");
        }
    } else {
        println!("Configuration complete. Run 'clinbox' to start.");
    }

    Ok(())
}

async fn run_interactive(
    max_emails: u32,
    include_all: bool,
    account_id: Option<&str>,
) -> Result<()> {
    let config = Config::load()?;

    if !config.is_valid() {
        eprintln!("Configuration incomplete. Run 'clinbox status' for details.");
        std::process::exit(1);
    }

    // Get the account to use
    let account = if let Some(id) = account_id {
        config.get_account(id).ok_or_else(|| {
            anyhow::anyhow!(
                "Account '{}' not found. Run 'clinbox account list' to see available accounts.",
                id
            )
        })?
    } else {
        config.get_default_account().ok_or_else(|| {
            anyhow::anyhow!("No default account set. Run 'clinbox account add' to add an account.")
        })?
    };

    let account_label = account.email.as_deref().unwrap_or(&account.id);

    // Initialize clients
    println!("Connecting to Gmail ({})...", account_label);
    let gmail = GmailClient::new(account)
        .await
        .context("Failed to connect to Gmail")?;

    let ai = AiClient::new(&config);
    let mut task_store = TaskStore::load()?;

    // Fetch emails
    let emails = if include_all {
        println!("📥 Fetching latest {} emails...", max_emails);
        gmail.fetch_latest(max_emails).await?
    } else {
        println!("📥 Fetching unread emails...");
        gmail.fetch_unread(max_emails).await?
    };

    if emails.is_empty() {
        println!("📭 No unread emails. Inbox zero! 🎉");
        return Ok(());
    }

    println!(
        "📧 Found {} unread emails. Starting triage...\n",
        emails.len()
    );

    // Initialize TUI
    let mut tui = Tui::new()?;
    let mut stats = Stats::default();

    for (idx, email) in emails.iter().enumerate() {
        let current = idx + 1;
        let total = emails.len();

        // Show email without analysis first
        tui.draw_email(email, None, current, total)?;

        // Get AI analysis
        let analysis = match ai.analyze_email(email).await {
            Ok(a) => Some(a),
            Err(e) => {
                // Show error briefly but continue
                tui.draw_message(&format!("AI analysis failed: {}", e), true)?;
                std::thread::sleep(std::time::Duration::from_secs(1));
                None
            }
        };

        // Show email with analysis
        tui.draw_email(email, analysis.as_ref(), current, total)?;

        // Wait for user action
        loop {
            let action = tui.wait_for_action()?;

            match action {
                Action::Archive => {
                    gmail.archive(&email.id).await?;
                    tui.draw_message("✅ Archived", false)?;
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    stats.archived += 1;
                    break;
                }
                Action::Delete => {
                    gmail.delete(&email.id).await?;
                    tui.draw_message("🗑️ Deleted", false)?;
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    stats.deleted += 1;
                    break;
                }
                Action::Task => {
                    let title = analysis
                        .as_ref()
                        .and_then(|a| a.suggested_action.clone())
                        .unwrap_or_else(|| email.subject.clone());

                    tui.draw_task_input(&title, &email.subject)?;

                    if tui.wait_for_confirm()? {
                        task_store.add(
                            title,
                            Some(
                                analysis
                                    .as_ref()
                                    .map(|a| a.summary.clone())
                                    .unwrap_or_default(),
                            ),
                            Some(email.id.clone()),
                            Some(email.subject.clone()),
                        )?;
                        gmail.archive(&email.id).await?;
                        tui.draw_message("📝 Task created & email archived", false)?;
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        stats.tasks_created += 1;
                    }
                    break;
                }
                Action::Reply => {
                    // Generate AI draft
                    tui.draw_message("🤖 Generating reply draft...", false)?;

                    match ai.generate_reply(email).await {
                        Ok(draft) => {
                            tui.draw_reply_draft(email, &draft)?;

                            match tui.wait_for_reply_action()? {
                                ReplyAction::Send => {
                                    tui.draw_message("📤 Sending...", false)?;
                                    match gmail.send_reply(email, &draft).await {
                                        Ok(()) => {
                                            gmail.archive(&email.id).await?;
                                            tui.draw_message("✅ Reply sent & archived", false)?;
                                            std::thread::sleep(std::time::Duration::from_millis(
                                                500,
                                            ));
                                            stats.replied += 1;
                                            break;
                                        }
                                        Err(e) => {
                                            tui.draw_message(
                                                &format!("❌ Failed to send: {}", e),
                                                true,
                                            )?;
                                            std::thread::sleep(std::time::Duration::from_secs(2));
                                        }
                                    }
                                }
                                ReplyAction::Edit => {
                                    // Open in browser for editing
                                    let url = format!(
                                        "https://mail.google.com/mail/u/0/#inbox/{}",
                                        email.id
                                    );
                                    let _ = open::that(&url);
                                    tui.draw_message("📧 Opened in browser for editing", false)?;
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    break;
                                }
                                ReplyAction::Cancel => {
                                    // Re-draw email and continue
                                    tui.draw_email(email, analysis.as_ref(), current, total)?;
                                }
                            }
                        }
                        Err(e) => {
                            tui.draw_message(&format!("❌ Failed to generate draft: {}", e), true)?;
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            tui.draw_email(email, analysis.as_ref(), current, total)?;
                        }
                    }
                }
                Action::Open => {
                    let url = format!("https://mail.google.com/mail/u/0/#inbox/{}", email.id);
                    let _ = open::that(&url);
                    tui.draw_message("🌐 Opened in browser", false)?;
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    // Don't break - let user continue with other actions
                }
                Action::ViewFull => {
                    tui.draw_full_email(email)?;
                    tui.wait_for_key()?;
                    tui.draw_email(email, analysis.as_ref(), current, total)?;
                    // Don't break - let user continue with other actions
                }
                Action::Skip => {
                    stats.skipped += 1;
                    break;
                }
                Action::Quit => {
                    tui.draw_summary(
                        stats.total(),
                        stats.archived,
                        stats.deleted,
                        stats.tasks_created,
                        stats.skipped,
                        stats.replied,
                    )?;
                    tui.wait_for_key()?;
                    return Ok(());
                }
            }
        }
    }

    // Show final summary
    tui.draw_summary(
        stats.total(),
        stats.archived,
        stats.deleted,
        stats.tasks_created,
        stats.skipped,
        stats.replied,
    )?;
    tui.wait_for_key()?;

    Ok(())
}

#[derive(Default)]
struct Stats {
    archived: usize,
    deleted: usize,
    tasks_created: usize,
    skipped: usize,
    replied: usize,
}

impl Stats {
    fn total(&self) -> usize {
        self.archived + self.deleted + self.tasks_created + self.skipped + self.replied
    }
}
