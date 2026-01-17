use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::email::{Category, Email, EmailAnalysis, Priority};

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

const ANALYSIS_PROMPT: &str = r#"You are an email assistant for a software developer.

Analyze this email and provide a JSON response with:
- priority: "urgent" | "action_required" | "informative" | "low" | "spam"
- category: "billing" | "security" | "infrastructure" | "seo" | "newsletter" | "personal" | "github" | "other"
- summary: 1-2 sentence summary in Spanish
- suggested_action: what to do (or null if no action needed), in Spanish
- estimated_time_minutes: how long the action would take (1, 2, 5, 10, 15, 30)

Priority guidelines:
- urgent: Production errors, security alerts, billing limits exceeded
- action_required: Needs response or action but not time-critical
- informative: Useful info to read later
- low: Can be archived (marketing, generic newsletters)
- spam: Irrelevant, delete

Respond ONLY with valid JSON, no markdown or explanation."#;

const REPLY_PROMPT: &str = r#"You are an email assistant helping a software developer write email replies.

Write a professional, concise reply to the email. Guidelines:
- Match the tone of the original email (formal/informal)
- Be helpful and direct
- Keep it brief (2-4 sentences typically)
- Write in the same language as the original email
- Don't use overly formal closings unless the original was formal
- If it's a notification/no-reply email, write a brief acknowledgment or suggest not replying

Respond with ONLY the reply text, no subject line, no greeting like "Here's a draft", just the email body ready to send."#;

pub struct AiClient {
    http: Client,
    api_key: String,
    model: String,
}

impl AiClient {
    pub fn new(config: &Config) -> Self {
        Self {
            http: Client::new(),
            api_key: config.ai.api_key.clone(),
            model: config.ai.model_analysis.clone(),
        }
    }

    pub async fn analyze_email(&self, email: &Email) -> Result<EmailAnalysis> {
        let email_content = format!(
            "From: {}\nSubject: {}\nDate: {}\nLabels: {}\n\nBody:\n{}",
            email.from,
            email.subject,
            email.date.format("%Y-%m-%d %H:%M"),
            email.labels.join(", "),
            truncate(&email.body_text(), 1500)
        );

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: ANALYSIS_PROMPT.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: email_content,
                },
            ],
            temperature: Some(0.3),
            max_tokens: Some(500),
        };

        let response = self.http
            .post(OPENROUTER_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/clinbox")
            .header("X-Title", "Clinbox")
            .json(&request)
            .send()
            .await
            .context("Failed to call AI API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("AI API error {}: {}", status, body);
        }

        let chat_response: ChatResponse = response.json().await
            .context("Failed to parse AI response")?;

        let content = chat_response.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Clean up JSON if wrapped in markdown
        let json_str = content.trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: AnalysisResponse = serde_json::from_str(json_str)
            .context("Failed to parse AI analysis JSON")?;

        Ok(EmailAnalysis {
            email_id: email.id.clone(),
            priority: parsed.priority,
            category: parsed.category,
            summary: parsed.summary,
            suggested_action: parsed.suggested_action,
            estimated_time_minutes: parsed.estimated_time_minutes.unwrap_or(1),
        })
    }

    pub async fn generate_reply(&self, email: &Email) -> Result<String> {
        let email_content = format!(
            "From: {}\nSubject: {}\nDate: {}\n\nBody:\n{}",
            email.from,
            email.subject,
            email.date.format("%Y-%m-%d %H:%M"),
            truncate(&email.body_text(), 2000)
        );

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: REPLY_PROMPT.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: email_content,
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(500),
        };

        let response = self.http
            .post(OPENROUTER_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/clinbox")
            .header("X-Title", "Clinbox")
            .json(&request)
            .send()
            .await
            .context("Failed to call AI API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("AI API error {}: {}", status, body);
        }

        let chat_response: ChatResponse = response.json().await
            .context("Failed to parse AI response")?;

        let content = chat_response.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(content.trim().to_string())
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnalysisResponse {
    priority: Priority,
    category: Category,
    summary: String,
    suggested_action: Option<String>,
    estimated_time_minutes: Option<u32>,
}
