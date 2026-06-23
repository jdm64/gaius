![GAIUS](logo.png)

A LLM agent harness build in Rust, powered by `genai` for provider abstraction.
The TUI uses `ratatui` the UI

## Features

- **Multi-provider support**: Works with OpenAI, Anthropic, OpenRouter, and other LLM providers through the `genai` abstraction layer
- **Terminal User Interface (TUI)**: Rich interactive interface built with `ratatui`
- **Session management**: Save, load, rename, and delete conversation sessions
- **Agent system**: Support for custom agent definitions with specific prompts
- **Model management**: Model discovery, caching, and selection with recent model tracking
- **Built-in tools**: HTTP requests and shell command execution capabilities
- **Markdown rendering**: Full markdown support in the terminal interface
- **Token tracking**: Monitor token usage across conversations

## Installation

### Prerequisites

- Rust 2024 edition or later
- Cargo package manager

### Key Dependencies

- `ratatui` - TUI framework
- `crossterm` - Terminal backend
- `genai` - LLM provider abstraction
- `tokio` - Async runtime
- `serde` / `serde_json` - Serialization
- `rmp-serde` - MessagePack serialization

## Usage

### Interactive TUI mode

```bash
./target/release/gaius
```

This launches the interactive terminal interface where you can:

- Type prompts and interact with LLMs
- Use slash commands (e.g., `/new`, `/sessions`, `/models`, `/agents`)
- Switch between different models
- Manage conversation sessions

### Command-line mode

```bash
# Run a single prompt and exit
gaius --prompt "Hello, how are you?"

# Run a prompt from a file
gaius --prompt-file prompt.txt

# Continue a saved session
gaius --session <session-id>

# Show help
gaius --help
```

## Configuration

### First-time setup

When you run `gaius` for the first time, it will guide you through an interactive setup wizard to configure:

- API providers (URLs, API keys)
- Default model

### Configuration file

The main configuration file is located at `~/.config/gaius/config.toml`:

```toml
[[provider]]
name     = "example-provider"
kind     = "openai"           # Provider type (openai, anthropic, etc.)
url      = "https://api.example.com"
key      = "sk-..."

[[model]]
name     = "GPT 5.5"
provider = "example-provider"
id       = "openai/gpt-5.5"
```

### Agent definitions

Custom agents can be defined in `~/.config/gaius/agents/*.toml`:

```toml
name    = "my-agent"
prompt  = "You are a helpful coding assistant..."
```

## Runtime data

Gaius stores data in the following locations:

| Path | Purpose |
|------|---------|
| `~/.config/gaius/config.toml` | Global configuration file |
| `~/.config/gaius/agents/*.toml` | Agent definitions |
| `~/.local/share/gaius/sessions/*.mpk` | Session history (MessagePack format) |
| `~/.cache/gaius/models_cache.json` | Cached model list per provider |
| `~/.cache/gaius/prompt_history.json` | History of recent used prompts |
| `~/.cache/gaius/recent_models.json` | Recently used model IDs |

## Slash commands

In the TUI, you can use these slash commands:

- `/new` - Clear history and create a new session
- `/sessions` - Load and delete sessions
- `/models` - List and select models
- `/agents` - List and select agents
- `/streaming` - Toggle streaming mode on/off
- `/thinking` - Toggle rendering of thinking messages on/off
- `/show-tokens` - Toggle rendering of token info messages on/off
- `/show-diff` - Toggle rendering of diff messages on/off
- `/plan` - Toggle plan mode on/off

## Tools

Gaius provides built-in tools that the agent can use during conversations:

| Tool | Description |
|------|-------------|
| `read_file` | Read the contents of a file |
| `create_file` | Create a new file with the provided contents |
| `edit_file` | Modify an existing file by replacing exactly one string match |
| `bash` | Execute a bash command |
| `glob` | Find files matching a glob pattern |
| `grep` | Search file contents using regex pattern |
| `question` | Ask the user a question with optional choices |
| `plan` | Create a structured markdown formatted plan |
