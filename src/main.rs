mod ai;
mod config;
mod email;
mod gmail;
mod tasks;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::ai::AiClient;
use crate::config::Config;
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
}

#[derive(Subcommand)]
enum Commands {
    /// Configure Clinbox
    Config {
        /// Configuration key (gmail.client_id, gmail.client_secret, ai.api_key)
        key: String,
        /// Value to set
        value: String,
    },
    /// Show pending tasks
    Tasks,
    /// Show configuration status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Config { key, value }) => {
            configure(&key, &value)?;
        }
        Some(Commands::Tasks) => {
            show_tasks()?;
        }
        Some(Commands::Status) => {
            show_status()?;
        }
        None => {
            run_interactive(cli.max_emails, cli.all).await?;
        }
    }

    Ok(())
}

fn configure(key: &str, value: &str) -> Result<()> {
    let mut config = Config::load()?;

    match key {
        "gmail.client_id" => config.gmail.client_id = value.to_string(),
        "gmail.client_secret" => config.gmail.client_secret = value.to_string(),
        "ai.api_key" => config.ai.api_key = value.to_string(),
        "ai.model" => config.ai.model_analysis = value.to_string(),
        _ => anyhow::bail!("Unknown config key: {}", key),
    }

    config.save()?;
    println!("✅ Configuration updated: {} = {}", key, mask_secret(value));
    Ok(())
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}...{}", &s[..4], &s[s.len()-4..])
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

    println!("📁 Config directory: {}", config_dir.display());
    println!();
    println!("Configuration status:");
    println!("  Gmail Client ID: {}", if config.gmail.client_id.is_empty() { "❌ Not set" } else { "✅ Set" });
    println!("  Gmail Client Secret: {}", if config.gmail.client_secret.is_empty() { "❌ Not set" } else { "✅ Set" });
    println!("  AI API Key: {}", if config.ai.api_key.is_empty() { "❌ Not set" } else { "✅ Set" });
    println!("  AI Model: {}", config.ai.model_analysis);
    println!();

    if !config.is_valid() {
        println!("⚠️  Configuration incomplete. Run:");
        println!();
        if config.gmail.client_id.is_empty() {
            println!("  clinbox config gmail.client_id YOUR_CLIENT_ID");
        }
        if config.gmail.client_secret.is_empty() {
            println!("  clinbox config gmail.client_secret YOUR_CLIENT_SECRET");
        }
        if config.ai.api_key.is_empty() {
            println!("  clinbox config ai.api_key YOUR_OPENROUTER_KEY");
        }
    } else {
        println!("✅ Configuration complete. Run 'clinbox' to start.");
    }

    Ok(())
}

async fn run_interactive(max_emails: u32, include_all: bool) -> Result<()> {
    let config = Config::load()?;

    if !config.is_valid() {
        eprintln!("❌ Configuration incomplete. Run 'clinbox status' for details.");
        std::process::exit(1);
    }

    // Initialize clients
    println!("🔐 Connecting to Gmail...");
    let gmail = GmailClient::new(&config).await
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

    println!("📧 Found {} unread emails. Starting triage...\n", emails.len());

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
                    let title = analysis.as_ref()
                        .and_then(|a| a.suggested_action.clone())
                        .unwrap_or_else(|| email.subject.clone());

                    tui.draw_task_input(&title, &email.subject)?;

                    if tui.wait_for_confirm()? {
                        task_store.add(
                            title,
                            Some(analysis.as_ref().map(|a| a.summary.clone()).unwrap_or_default()),
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
                                            std::thread::sleep(std::time::Duration::from_millis(500));
                                            stats.replied += 1;
                                            break;
                                        }
                                        Err(e) => {
                                            tui.draw_message(&format!("❌ Failed to send: {}", e), true)?;
                                            std::thread::sleep(std::time::Duration::from_secs(2));
                                        }
                                    }
                                }
                                ReplyAction::Edit => {
                                    // Open in browser for editing
                                    let url = format!("https://mail.google.com/mail/u/0/#inbox/{}", email.id);
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
