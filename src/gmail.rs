use anyhow::{Context, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use crate::config::{Config, GmailAccount};
use crate::email::{Attachment, Email};

const GMAIL_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Write token file with secure permissions (owner read/write only)
fn write_token_file(path: &std::path::Path, content: &str) -> Result<()> {
    fs::write(path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}
const GMAIL_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    refresh_token: String,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

pub struct GmailClient {
    http: Client,
    access_token: String,
}

impl GmailClient {
    pub async fn new(account: &GmailAccount) -> Result<Self> {
        let token = Self::get_valid_token(account).await?;

        Ok(Self {
            http: Client::new(),
            access_token: token,
        })
    }

    async fn get_valid_token(account: &GmailAccount) -> Result<String> {
        let token_path = Config::token_path_for_account(&account.id)?;

        if token_path.exists() {
            let content = fs::read_to_string(&token_path)?;
            let stored: StoredToken = serde_json::from_str(&content)?;

            let is_expired = stored
                .expires_at
                .map(|exp| exp < Utc::now())
                .unwrap_or(true);

            if !is_expired {
                return Ok(stored.access_token);
            }

            if let Ok(new_token) = Self::refresh_token(account, &stored.refresh_token).await {
                return Ok(new_token);
            }
        }

        Self::oauth_flow(account).await
    }

    async fn refresh_token(account: &GmailAccount, refresh_token: &str) -> Result<String> {
        let client = Client::new();

        let params = [
            ("client_id", account.client_id.as_str()),
            ("client_secret", account.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let response = client.post(GMAIL_TOKEN_URL).form(&params).send().await?;

        if !response.status().is_success() {
            bail!("Failed to refresh token: {}", response.status());
        }

        let token_response: TokenResponse = response.json().await?;

        let expires_at = token_response
            .expires_in
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs));

        let stored = StoredToken {
            access_token: token_response.access_token.clone(),
            refresh_token: refresh_token.to_string(),
            expires_at,
        };
        let token_path = Config::token_path_for_account(&account.id)?;
        write_token_file(&token_path, &serde_json::to_string_pretty(&stored)?)?;

        Ok(token_response.access_token)
    }

    pub async fn oauth_flow(account: &GmailAccount) -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://localhost:{}", port);

        let scopes = "https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/userinfo.email";

        let auth_url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
            GMAIL_AUTH_URL,
            urlencoding::encode(&account.client_id),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(scopes)
        );

        println!("\nOpening browser for Gmail authorization...");
        println!("If it doesn't open, visit: {}\n", auth_url);
        let _ = open::that(&auth_url);

        let (stream, _) = listener.accept()?;
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        let code = request_line
            .split_whitespace()
            .nth(1)
            .and_then(|path| {
                path.split('?')
                    .nth(1)?
                    .split('&')
                    .find(|p| p.starts_with("code="))?
                    .strip_prefix("code=")
                    .map(|s| s.to_string())
            })
            .context("Failed to extract authorization code")?;

        let response_html = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
            <html><body><h1>Authorization successful!</h1>\
            <p>You can close this tab and return to the terminal.</p></body></html>";
        let mut stream = stream;
        stream.write_all(response_html.as_bytes())?;

        let client = Client::new();
        let decoded_code = urlencoding::decode(&code)?.into_owned();

        let params = [
            ("client_id", account.client_id.as_str()),
            ("client_secret", account.client_secret.as_str()),
            ("code", decoded_code.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ];

        let response = client.post(GMAIL_TOKEN_URL).form(&params).send().await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            bail!("Failed to exchange code for token: {}", error);
        }

        let token_response: TokenResponse = response.json().await?;

        let expires_at = token_response
            .expires_in
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs));

        let stored = StoredToken {
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.unwrap_or_default(),
            expires_at,
        };
        let tokens_dir = Config::tokens_dir()?;
        fs::create_dir_all(&tokens_dir)?;
        let token_path = Config::token_path_for_account(&account.id)?;
        write_token_file(&token_path, &serde_json::to_string_pretty(&stored)?)?;

        println!("Authorization successful!\n");
        Ok(token_response.access_token)
    }

    /// Fetch the authenticated user's email address
    pub async fn fetch_user_email(&self) -> Result<String> {
        let url = format!("{}/users/me/profile", GMAIL_API_BASE);

        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            bail!("Failed to fetch user profile: {}", response.status());
        }

        let profile: UserProfile = response.json().await?;
        Ok(profile.email_address)
    }

    pub async fn fetch_unread(&self, max_results: u32) -> Result<Vec<Email>> {
        let url = format!(
            "{}/users/me/messages?maxResults={}&q=is:unread",
            GMAIL_API_BASE, max_results
        );

        let response: MessageListResponse = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut emails = Vec::new();
        for msg_ref in response.messages.unwrap_or_default() {
            if let Ok(email) = self.fetch_email(&msg_ref.id).await {
                emails.push(email);
            }
        }

        Ok(emails)
    }

    /// Fetch latest emails (read and unread) sorted by date descending
    pub async fn fetch_latest(&self, max_results: u32) -> Result<Vec<Email>> {
        let url = format!(
            "{}/users/me/messages?maxResults={}&labelIds=INBOX",
            GMAIL_API_BASE, max_results
        );

        let response: MessageListResponse = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut emails = Vec::new();
        for msg_ref in response.messages.unwrap_or_default() {
            if let Ok(email) = self.fetch_email(&msg_ref.id).await {
                emails.push(email);
            }
        }

        Ok(emails)
    }

    pub async fn fetch_email(&self, id: &str) -> Result<Email> {
        let url = format!("{}/users/me/messages/{}?format=full", GMAIL_API_BASE, id);

        let response: MessageResponse = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        self.parse_message(response)
    }

    fn parse_message(&self, msg: MessageResponse) -> Result<Email> {
        let headers = msg.payload.headers.clone().unwrap_or_default();

        let get_header = |name: &str| -> String {
            headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .map(|h| h.value.clone())
                .unwrap_or_default()
        };

        let date = get_header("Date");
        let parsed_date = dateparse::parse(&date)
            .map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_default())
            .unwrap_or_else(|_| Utc::now());

        let (body_plain, body_html) = self.extract_body(&msg.payload);
        let attachments = self.extract_attachments(&msg.payload);
        let is_unread = msg
            .label_ids
            .as_ref()
            .is_some_and(|l| l.contains(&"UNREAD".to_string()));

        Ok(Email {
            id: msg.id,
            thread_id: msg.thread_id,
            subject: get_header("Subject"),
            from: get_header("From"),
            to: get_header("To"),
            date: parsed_date,
            snippet: msg.snippet.unwrap_or_default(),
            body_plain,
            body_html,
            labels: msg.label_ids.unwrap_or_default(),
            attachments,
            is_unread,
        })
    }

    fn extract_body(&self, payload: &MessagePart) -> (Option<String>, Option<String>) {
        let mut plain = None;
        let mut html = None;

        fn process_part(part: &MessagePart, plain: &mut Option<String>, html: &mut Option<String>) {
            let mime = part.mime_type.as_deref().unwrap_or("");

            if mime == "text/plain" {
                if let Some(data) = part.body.as_ref().and_then(|b| b.data.as_ref())
                    && let Ok(decoded) = URL_SAFE.decode(data)
                {
                    *plain = String::from_utf8(decoded).ok();
                }
            } else if mime == "text/html"
                && let Some(data) = part.body.as_ref().and_then(|b| b.data.as_ref())
                && let Ok(decoded) = URL_SAFE.decode(data)
            {
                *html = String::from_utf8(decoded).ok();
            }

            if let Some(parts) = &part.parts {
                for p in parts {
                    process_part(p, plain, html);
                }
            }
        }

        process_part(payload, &mut plain, &mut html);
        (plain, html)
    }

    fn extract_attachments(&self, payload: &MessagePart) -> Vec<Attachment> {
        let mut attachments = Vec::new();

        fn process_part(part: &MessagePart, attachments: &mut Vec<Attachment>) {
            if let Some(filename) = &part.filename
                && !filename.is_empty()
            {
                attachments.push(Attachment {
                    filename: filename.clone(),
                    mime_type: part.mime_type.clone().unwrap_or_default(),
                    size: part.body.as_ref().and_then(|b| b.size).unwrap_or(0),
                    attachment_id: part
                        .body
                        .as_ref()
                        .and_then(|b| b.attachment_id.clone())
                        .unwrap_or_default(),
                });
            }

            if let Some(parts) = &part.parts {
                for p in parts {
                    process_part(p, attachments);
                }
            }
        }

        process_part(payload, &mut attachments);
        attachments
    }

    pub async fn archive(&self, id: &str) -> Result<()> {
        let url = format!("{}/users/me/messages/{}/modify", GMAIL_API_BASE, id);

        let body = serde_json::json!({
            "removeLabelIds": ["INBOX", "UNREAD"]
        });

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            bail!("Failed to archive email: {}", response.status());
        }

        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let url = format!("{}/users/me/messages/{}/trash", GMAIL_API_BASE, id);

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .header("Content-Length", "0")
            .send()
            .await?;

        if !response.status().is_success() {
            bail!("Failed to delete email: {}", response.status());
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn mark_read(&self, id: &str) -> Result<()> {
        let url = format!("{}/users/me/messages/{}/modify", GMAIL_API_BASE, id);

        let body = serde_json::json!({
            "removeLabelIds": ["UNREAD"]
        });

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            bail!("Failed to mark email as read: {}", response.status());
        }

        Ok(())
    }

    /// Send a reply to an email
    pub async fn send_reply(&self, original: &crate::email::Email, body_text: &str) -> Result<()> {
        let url = format!("{}/users/me/messages/send", GMAIL_API_BASE);

        // Extract reply-to address or use from address
        let to_address = &original.from;

        // Build subject with Re: prefix if not already present
        let subject = if original.subject.starts_with("Re:") || original.subject.starts_with("RE:")
        {
            original.subject.clone()
        } else {
            format!("Re: {}", original.subject)
        };

        // Build RFC 2822 message
        let message = format!(
            "To: {}\r\n\
             Subject: {}\r\n\
             In-Reply-To: {}\r\n\
             References: {}\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             \r\n\
             {}",
            to_address, subject, original.id, original.id, body_text
        );

        // Encode as base64url
        let encoded = URL_SAFE_NO_PAD.encode(message.as_bytes());

        let payload = serde_json::json!({
            "raw": encoded,
            "threadId": original.thread_id
        });

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            bail!("Failed to send reply: {}", error);
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserProfile {
    email_address: String,
}

#[derive(Debug, Deserialize)]
struct MessageListResponse {
    messages: Option<Vec<MessageRef>>,
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageResponse {
    id: String,
    thread_id: String,
    label_ids: Option<Vec<String>>,
    snippet: Option<String>,
    payload: MessagePart,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePart {
    mime_type: Option<String>,
    headers: Option<Vec<Header>>,
    body: Option<MessageBody>,
    parts: Option<Vec<MessagePart>>,
    filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageBody {
    data: Option<String>,
    size: Option<u64>,
    attachment_id: Option<String>,
}

mod dateparse {
    use chrono::DateTime;

    pub fn parse(s: &str) -> Result<i64, ()> {
        if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
            return Ok(dt.timestamp());
        }

        let formats = [
            "%a, %d %b %Y %H:%M:%S %z",
            "%d %b %Y %H:%M:%S %z",
            "%Y-%m-%d %H:%M:%S",
        ];

        for fmt in formats {
            if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
                return Ok(dt.timestamp());
            }
        }

        Err(())
    }
}
