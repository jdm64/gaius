![GAIUS](docs/logo.png)

A LLM agent harness build in Rust, powered by `genai` for provider abstraction.

## Features

- **Multi-provider support**: Works with OpenAI, Anthropic, OpenRouter, and other LLM providers through the `genai` abstraction layer
- **Terminal User Interface (TUI)**: Rich interactive interface built with `ratatui`
- **Session management**: Save, load, rename, and delete conversation sessions
- **Agent system**: Support for custom agent definitions with specific prompts
- **Model management**: Model discovery, caching, and selection with recent model tracking
- **Built-in tools**: Basic set of tools for the llm to call
- **Markdown rendering**: Markdown support in the terminal interface
- **Token tracking**: Monitor token usage across conversations

## Installation

### Prebuilt binaries

```bash
# Download most recent stable version
curl -fsSL https://raw.githubusercontent.com/jdm64/gaius/master/install.sh | sh

# Dwnload the latest continuous build (for example, from the master branch)
curl -fsSL https://raw.githubusercontent.com/jdm64/gaius/master/install.sh | sh -s -- --build master
```

### Build from source

- Rust 2024 edition or later
- Cargo package manager

```bash
git clone https://github.com/jdm64/gaius.git
cd gaius/cli
cargo build
cargo run
```

## Usage

### Interactive TUI mode

```bash
gaius
```

This launches the interactive terminal interface where you can:

- Type prompts and interact with LLMs
- Use slash commands (e.g., `/new`, `/sessions`, `/models`, `/agents`, `/skills`)
- Switch between different models
- Manage conversation sessions

### Command-line options

```bash
# Enter simple interactive mode
gaius --cli

# Run a single prompt and exit
gaius --prompt "Hello, how are you?"

# Run a prompt from a file
gaius --prompt-file prompt.txt

# Continue a saved session
gaius --session <session-id>

# OAuth login to Codex or Grok
gaius --login <provider>

# Show help
gaius --help

# Show version
gaius --version
```

## Configuration

### First-time setup

When you run `gaius` for the first time, it will guide you through an interactive setup wizard to configure:

- Provider URL
- API key

Then when the TUI loads use /models command to select a model to use.

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

# Only set if the defaults are not optimal for your use case
# Long tool responses are truncated in the middle. By default 30k chars are returned with:
# 60% (18k) from the start and 40% (12k) from the end of the original output.
[tools]
max_output = 30000
max_output_split = 60
```

### Agent definitions

Custom agents can be defined in `~/.config/gaius/agents/*.toml`:

```toml
name    = "my-agent"
desc    = "a simple coding agent" # optional
prompt  = "You are a helpful coding assistant..."

# Optional section to enable/disable tools
[tools]
include="bash,edit_file,read_file"
exclude="read_file"
```

## Runtime data

Gaius stores data in the following locations:

| Path | Purpose |
|------|---------|
| `~/.config/gaius/config.toml` | Global configuration file |
| `~/.config/gaius/prefs_display.json` | Display preferences |
| `~/.config/gaius/agents/*.toml` | Agent definitions |
| `~/.local/share/gaius/sessions/*.mpk` | Session history (MessagePack format) |
| `~/.cache/gaius/models_cache.json` | Cached model list per provider |
| `~/.cache/gaius/models_recent.json` | Recently used model IDs |
| `~/.cache/gaius/prompt_history.json` | History of recent used prompts |

*Note: See [dirs crate](https://crates.io/crates/dirs) for paths on MacOS and Windows*

## Slash commands

In the TUI, you can use these slash commands:

- `/new` - Clear history and create a new session
- `/sessions` - Load and delete sessions
- `/models` - List and select models
- `/reasoning` - Set reasoning effort level
- `/agents` - List and select agents
- `/skills` - List available skills
- `/plan` - Toggle plan mode on/off
- `/rebuild` - Reload agents, skills, and AGENTS.md
- `/compact` - Compact conversation history into summary
- `/fork` - Copy the current session with a new id
- `/info` - Show session info
- `/theme` - Select the color theme
- `/show-thinking` - Toggle rendering of thinking messages
- `/show-tokens` - Toggle rendering of token info messages
- `/show-diff` - Toggle rendering of diff messages
- `/streaming` - Toggle streaming mode

## Tools

Gaius provides built-in tools that the agent can use during conversations:

| Tool | Description |
|------|-------------|
| `read` | Read workspace file |
| `write` | Create or replace file |
| `edit` | Replace one unique exact text match in a file |
| `bash` | Run command with bash -c in the workspace |
| `glob` | Find files matching glob |
| `grep` | Search file contents with regular expression |
| `question` | Ask user a question, optionally with choices |
| `plan` | Create or update task plan |
| `skill` | Load skill instructions |
| `webfetch` | Fetch URL as Markdown |
| `websearch` | Search the web and return ranked results |
