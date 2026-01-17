use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub source_email_id: Option<String>,
    pub source_email_subject: Option<String>,
    pub created_at: DateTime<Utc>,
    pub due_date: Option<DateTime<Utc>>,
    pub completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TaskStore {
    pub tasks: Vec<Task>,
}

impl TaskStore {
    /// Load tasks from file
    pub fn load() -> Result<Self> {
        let path = Config::tasks_path()?;

        if path.exists() {
            let content = fs::read_to_string(&path).context("Failed to read tasks file")?;
            let store: TaskStore =
                serde_json::from_str(&content).context("Failed to parse tasks file")?;
            Ok(store)
        } else {
            Ok(TaskStore::default())
        }
    }

    /// Save tasks to file
    pub fn save(&self) -> Result<()> {
        let path = Config::tasks_path()?;
        fs::create_dir_all(path.parent().unwrap())?;

        let content = serde_json::to_string_pretty(self).context("Failed to serialize tasks")?;
        fs::write(&path, content).context("Failed to write tasks file")?;

        Ok(())
    }

    /// Add a new task
    pub fn add(
        &mut self,
        title: String,
        description: Option<String>,
        email_id: Option<String>,
        email_subject: Option<String>,
    ) -> Result<Task> {
        let task = Task {
            id: generate_id(),
            title,
            description,
            source_email_id: email_id,
            source_email_subject: email_subject,
            created_at: Utc::now(),
            due_date: None,
            completed: false,
            completed_at: None,
        };

        self.tasks.push(task.clone());
        self.save()?;

        Ok(task)
    }

    /// List pending tasks
    pub fn pending(&self) -> Vec<&Task> {
        self.tasks.iter().filter(|t| !t.completed).collect()
    }

    /// Mark a task as completed
    #[allow(dead_code)]
    pub fn complete(&mut self, id: &str) -> Result<()> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.completed = true;
            task.completed_at = Some(Utc::now());
            self.save()?;
        }
        Ok(())
    }

    /// Delete a task
    #[allow(dead_code)]
    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.tasks.retain(|t| t.id != id);
        self.save()?;
        Ok(())
    }
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("task_{}", timestamp)
}
