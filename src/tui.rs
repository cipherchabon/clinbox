use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::io::{self, stdout};

use crate::ai::ArticleSummary;
use crate::email::{Email, EmailAnalysis};

pub enum Action {
    Archive,
    Delete,
    Task,
    Reply,
    Summary,
    Open,
    Skip,
    ViewFull,
    Quit,
}

pub enum ReplyAction {
    Send,
    Edit,
    Cancel,
}

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn restore(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    pub fn draw_email(
        &mut self,
        email: &Email,
        analysis: Option<&EmailAnalysis>,
        current: usize,
        total: usize,
    ) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            // Main layout: header, content, footer
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Header
                    Constraint::Length(5), // Email metadata
                    Constraint::Min(10),   // AI analysis + body
                    Constraint::Length(3), // Actions
                ])
                .split(area);

            // Header
            let header = Paragraph::new(format!(
                " 📧 Clinbox                                          [{}/{}]",
                current, total
            ))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            // Email metadata
            let date_str = email.date.format("%Y-%m-%d %H:%M").to_string();
            let metadata = format!(
                " From: {}\n Subject: {}\n Date: {}",
                email.sender_name(),
                truncate(&email.subject, 60),
                date_str
            );
            let metadata_widget = Paragraph::new(metadata)
                .style(Style::default().fg(Color::White))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
            frame.render_widget(metadata_widget, chunks[1]);

            // AI analysis + body preview
            let content_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // AI summary
                    Constraint::Min(4),    // Body preview
                ])
                .split(chunks[2]);

            if let Some(analysis) = analysis {
                let priority_style = match analysis.priority {
                    crate::email::Priority::Urgent => Style::default().fg(Color::Red),
                    crate::email::Priority::ActionRequired => Style::default().fg(Color::Yellow),
                    crate::email::Priority::Informative => Style::default().fg(Color::Blue),
                    crate::email::Priority::Low => Style::default().fg(Color::Gray),
                    crate::email::Priority::Spam => Style::default().fg(Color::DarkGray),
                };

                let ai_text = format!(
                    " 🤖 AI Analysis:\n {}\n\n {} {} | {} | ~{} min{}",
                    analysis.summary,
                    analysis.priority.emoji(),
                    analysis.priority.label(),
                    analysis.category.label(),
                    analysis.estimated_time_minutes,
                    analysis
                        .suggested_action
                        .as_ref()
                        .map(|a| format!("\n ➡️  {}", a))
                        .unwrap_or_default()
                );

                let ai_widget = Paragraph::new(ai_text).style(priority_style).block(
                    Block::default()
                        .borders(Borders::LEFT | Borders::RIGHT)
                        .border_style(Style::default().fg(Color::DarkGray)),
                );
                frame.render_widget(ai_widget, content_chunks[0]);
            } else {
                let loading = Paragraph::new(" 🔄 Analyzing email...")
                    .style(Style::default().fg(Color::Yellow))
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
                frame.render_widget(loading, content_chunks[0]);
            }

            // Body preview
            let body_preview = truncate(&email.body_text(), 500);
            let body_widget = Paragraph::new(format!(" {}", body_preview.replace('\n', "\n ")))
                .style(Style::default().fg(Color::Gray))
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .title(" Preview ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray)),
                );
            frame.render_widget(body_widget, content_chunks[1]);

            // Actions footer
            let actions = " [a]rchive [d]elete [t]ask [r]eply [n]ote [o]pen [v]iew [s]kip [q]uit ";
            let actions_widget = Paragraph::new(actions)
                .style(Style::default().fg(Color::Green))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(actions_widget, chunks[3]);
        })?;

        Ok(())
    }

    pub fn draw_message(&mut self, message: &str, is_error: bool) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();
            let style = if is_error {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            };

            let widget = Paragraph::new(message)
                .style(style)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));

            let centered = centered_rect(60, 20, area);
            frame.render_widget(widget, centered);
        })?;
        Ok(())
    }

    pub fn draw_task_input(&mut self, title: &str, email_subject: &str) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            let text = format!(
                "Creating task from email:\n\n\
                 Subject: {}\n\n\
                 Task title: {}\n\n\
                 Press [Enter] to confirm, [Esc] to cancel",
                email_subject, title
            );

            let widget = Paragraph::new(text)
                .style(Style::default().fg(Color::Cyan))
                .alignment(Alignment::Center)
                .block(Block::default().title(" New Task ").borders(Borders::ALL));

            let centered = centered_rect(70, 40, area);
            frame.render_widget(widget, centered);
        })?;
        Ok(())
    }

    pub fn draw_full_email(&mut self, email: &Email) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            let body = email.body_text();
            let content = format!(
                "From: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}",
                email.from,
                email.to,
                email.date.format("%Y-%m-%d %H:%M:%S"),
                email.subject,
                body
            );

            let widget = Paragraph::new(content)
                .style(Style::default().fg(Color::White))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(" Full Email - Press any key to go back ")
                        .borders(Borders::ALL),
                );

            frame.render_widget(widget, area);
        })?;
        Ok(())
    }

    pub fn draw_summary(
        &mut self,
        total: usize,
        archived: usize,
        deleted: usize,
        tasks_created: usize,
        skipped: usize,
        replied: usize,
        summaries_saved: usize,
    ) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            let mut text = format!(
                "📊 Session Summary\n\n\
                 Total emails processed: {}\n\
                 ✅ Archived: {}\n\
                 🗑️  Deleted: {}\n\
                 📝 Tasks created: {}\n\
                 💬 Replied: {}",
                total, archived, deleted, tasks_created, replied
            );

            if summaries_saved > 0 {
                text.push_str(&format!("\n 📓 Summaries saved: {}", summaries_saved));
            }

            text.push_str(&format!("\n ⏭️  Skipped: {}\n\n Press any key to exit", skipped));

            let widget = Paragraph::new(text)
                .style(Style::default().fg(Color::Cyan))
                .alignment(Alignment::Center)
                .block(Block::default().title(" Clinbox ").borders(Borders::ALL));

            let centered = centered_rect(50, 40, area);
            frame.render_widget(widget, centered);
        })?;
        Ok(())
    }

    pub fn wait_for_action(&self) -> Result<Action> {
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('a') => return Ok(Action::Archive),
                    KeyCode::Char('d') => return Ok(Action::Delete),
                    KeyCode::Char('t') => return Ok(Action::Task),
                    KeyCode::Char('r') => return Ok(Action::Reply),
                    KeyCode::Char('n') => return Ok(Action::Summary),
                    KeyCode::Char('o') => return Ok(Action::Open),
                    KeyCode::Char('v') => return Ok(Action::ViewFull),
                    KeyCode::Char('s') => return Ok(Action::Skip),
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(Action::Quit),
                    _ => {}
                }
            }
        }
    }

    pub fn wait_for_key(&self) -> Result<()> {
        loop {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                return Ok(());
            }
        }
    }

    pub fn wait_for_confirm(&self) -> Result<bool> {
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Enter => return Ok(true),
                    KeyCode::Esc => return Ok(false),
                    _ => {}
                }
            }
        }
    }

    pub fn draw_reply_draft(&mut self, email: &Email, draft: &str) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Header
                    Constraint::Length(4), // To/Subject
                    Constraint::Min(10),   // Draft content
                    Constraint::Length(3), // Actions
                ])
                .split(area);

            // Header
            let header = Paragraph::new(" 📝 Reply Draft (AI Generated)")
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            // To/Subject
            let subject = if email.subject.starts_with("Re:") || email.subject.starts_with("RE:") {
                email.subject.clone()
            } else {
                format!("Re: {}", email.subject)
            };
            let metadata = format!(" To: {}\n Subject: {}", email.from, subject);
            let metadata_widget = Paragraph::new(metadata)
                .style(Style::default().fg(Color::White))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
            frame.render_widget(metadata_widget, chunks[1]);

            // Draft content
            let draft_widget = Paragraph::new(format!(" {}", draft.replace('\n', "\n ")))
                .style(Style::default().fg(Color::Green))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(" Draft ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                );
            frame.render_widget(draft_widget, chunks[2]);

            // Actions
            let actions = " [s]end  [e]dit in browser  [c]ancel ";
            let actions_widget = Paragraph::new(actions)
                .style(Style::default().fg(Color::Yellow))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(actions_widget, chunks[3]);
        })?;
        Ok(())
    }

    pub fn wait_for_reply_action(&self) -> Result<ReplyAction> {
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('s') => return Ok(ReplyAction::Send),
                    KeyCode::Char('e') => return Ok(ReplyAction::Edit),
                    KeyCode::Char('c') | KeyCode::Esc => return Ok(ReplyAction::Cancel),
                    _ => {}
                }
            }
        }
    }

    pub fn draw_summary_preview(&mut self, email: &Email, summary: &ArticleSummary) -> Result<()> {
        self.terminal.draw(|frame| {
            let area = frame.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Header
                    Constraint::Length(4), // Email info
                    Constraint::Min(10),   // Summary content
                    Constraint::Length(3), // Actions
                ])
                .split(area);

            // Header
            let header = Paragraph::new(" 📝 Article Summary (AI Generated)")
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            // Email info
            let info = format!(
                " From: {}\n Subject: {}",
                email.sender_name(),
                truncate(&email.subject, 60)
            );
            let info_widget = Paragraph::new(info)
                .style(Style::default().fg(Color::White))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
            frame.render_widget(info_widget, chunks[1]);

            // Summary content with key takeaways
            let content_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(60), // Summary
                    Constraint::Percentage(40), // Key takeaways
                ])
                .split(chunks[2]);

            // Summary
            let summary_widget = Paragraph::new(format!(" {}", summary.summary.replace('\n', "\n ")))
                .style(Style::default().fg(Color::Green))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(" Resumen ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                );
            frame.render_widget(summary_widget, content_chunks[0]);

            // Key takeaways
            let takeaways_text = summary
                .key_takeaways
                .iter()
                .map(|t| format!(" • {}", t))
                .collect::<Vec<_>>()
                .join("\n");
            let takeaways_widget = Paragraph::new(takeaways_text)
                .style(Style::default().fg(Color::Yellow))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(" Puntos Clave ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow)),
                );
            frame.render_widget(takeaways_widget, content_chunks[1]);

            // Actions
            let actions = " [Enter] Save to Notion  [Esc] Cancel ";
            let actions_widget = Paragraph::new(actions)
                .style(Style::default().fg(Color::Magenta))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(actions_widget, chunks[3]);
        })?;
        Ok(())
    }

    pub fn wait_for_yes_no(&self) -> Result<bool> {
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                    _ => {}
                }
            }
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_len).collect::<String>())
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
