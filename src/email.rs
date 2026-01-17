use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: DateTime<Utc>,
    pub snippet: String,
    pub body_plain: Option<String>,
    pub body_html: Option<String>,
    pub labels: Vec<String>,
    pub attachments: Vec<Attachment>,
    pub is_unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub attachment_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAnalysis {
    pub email_id: String,
    pub priority: Priority,
    pub category: Category,
    pub summary: String,
    pub suggested_action: Option<String>,
    pub estimated_time_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Urgent,
    ActionRequired,
    Informative,
    Low,
    Spam,
}

impl Priority {
    pub fn emoji(&self) -> &'static str {
        match self {
            Priority::Urgent => "🔴",
            Priority::ActionRequired => "🟡",
            Priority::Informative => "🔵",
            Priority::Low => "⚪",
            Priority::Spam => "🗑️",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Priority::Urgent => "URGENT",
            Priority::ActionRequired => "ACTION",
            Priority::Informative => "INFO",
            Priority::Low => "LOW",
            Priority::Spam => "SPAM",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Billing,
    Security,
    Infrastructure,
    Seo,
    Newsletter,
    Personal,
    Github,
    Other,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::Billing => "Billing",
            Category::Security => "Security",
            Category::Infrastructure => "Infra",
            Category::Seo => "SEO",
            Category::Newsletter => "Newsletter",
            Category::Personal => "Personal",
            Category::Github => "GitHub",
            Category::Other => "Other",
        }
    }
}

impl Email {
    /// Get the body as plain text
    pub fn body_text(&self) -> String {
        if let Some(plain) = &self.body_plain
            && !plain.is_empty() {
                return plain.clone();
            }

        if let Some(html) = &self.body_html
            && !html.is_empty()
                && let Ok(text) = html2text::from_read(html.as_bytes(), 80) {
                    return text;
                }

        self.snippet.clone()
    }

    /// Get a short sender name
    pub fn sender_name(&self) -> String {
        // Extract name from "Name <email@domain.com>" format
        if let Some(idx) = self.from.find('<') {
            let name = self.from[..idx].trim();
            if !name.is_empty() {
                return name.trim_matches('"').to_string();
            }
        }
        self.from.clone()
    }
}
