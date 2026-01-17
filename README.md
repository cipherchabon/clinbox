# Clinbox

A terminal-first email client with AI-powered triage for developers.

## Features

- **One email at a time**: Focus on the current email without inbox anxiety
- **AI-powered analysis**: Automatic priority, category, and summary for each email
- **Quick actions**: Archive, delete, create task, reply, or skip with a single keystroke
- **AI-generated replies**: Get draft replies that match the tone of the original email
- **Local task storage**: Create tasks from emails without external dependencies
- **Privacy-focused**: Emails processed locally, only necessary content sent to LLM

## Installation

```bash
# Clone the repository
git clone https://github.com/cipherchabon/clinbox.git
cd clinbox

# Build
cargo build --release

# Install (optional)
cargo install --path .
```

## Configuration

### 1. Google Cloud Setup

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select an existing one
3. Enable the Gmail API
4. Go to "APIs & Services" > "Credentials"
5. Create OAuth 2.0 Client ID (Desktop application)
6. Add yourself as a test user in "OAuth consent screen"

### 2. OpenRouter Setup

1. Get an API key from [OpenRouter](https://openrouter.ai/)

### 3. Configure Clinbox

```bash
clinbox config gmail.client_id YOUR_CLIENT_ID
clinbox config gmail.client_secret YOUR_CLIENT_SECRET
clinbox config ai.api_key YOUR_OPENROUTER_API_KEY
```

Verify configuration:

```bash
clinbox status
```

## Usage

```bash
# Process unread emails (default)
clinbox

# Process last N emails (read and unread)
clinbox -a -n 50

# Show pending tasks
clinbox tasks

# Show configuration status
clinbox status
```

### Keyboard Shortcuts

| Key | Action | Description |
|-----|--------|-------------|
| `a` | Archive | Remove from inbox, mark as read |
| `d` | Delete | Move to trash |
| `t` | Task | Create task from email |
| `r` | Reply | Generate AI draft and send/edit |
| `o` | Open | Open in browser |
| `v` | View | Show full email body |
| `s` | Skip | Next email without action |
| `q` | Quit | Exit application |

## AI Models

By default, Clinbox uses:
- **Analysis**: `google/gemini-2.0-flash-001` (fast, economical)
- **Replies**: `anthropic/claude-sonnet-4` (high quality)

You can change the model:

```bash
clinbox config ai.model google/gemini-2.0-flash-001
```

## Configuration Files

All configuration is stored in `~/.clinbox/`:

- `config.json` - API keys and settings
- `token.json` - Gmail OAuth tokens
- `tasks.json` - Local task storage

## License

MIT
