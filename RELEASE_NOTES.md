## v0.6.0

### New features
- Added `--sess-dir` to override the directory used to load and save sessions.
- Added initial Agent Client Protocol (ACP) support:
  - Use `gaius acp` to connect an ACP client over stdio.
  - Supports ACP protocol versions 1 and 2.
  - Creates sessions and resumes existing sessions, replaying their conversation history.
  - Streams assistant responses and reasoning, along with tool-call and tool-result updates.
  - Provides model and reasoning configuration options to ACP clients.
  - Handles prompt cancellation and session close/delete requests.

### Improvements
- Tool path arguments now expand `~` to the home directory.
- Skills can be queued while the agent is busy, like prompts.
- Cycling agents with Tab no longer clears the prompt.
- The session-continue message on exit is shown only when the session file still exists.
- Add -build to the version number with a number indicating commits since last tag.

## v0.5.3

### Improvements
- genai v0.7.4 fixes ModelName serialization; revert serialization workaround
- fix clippy warnings from new rust version

## v0.5.2

### Improvements
- Fixed issue where OpenRouter assistant messages have a custom message part with ModelIden that couldn't be deserialized.

## v0.5.1

### Improvements
- The installer now lets you choose an installation directory from writable locations on your `PATH`, with `~/.local/bin` as the default.
- The theme picker now highlights the current theme, restores the saved theme when cancelled, and supports exiting with Ctrl+C.
- Fixed the positioning of the info menu.
- Clarified the `write` and `bash` tool descriptions, including the requirement to set `overwrite=true` when replacing an existing file.

## v0.5.0

### New features
- Added an `upgrade` command to download and install the latest release for the current platform.
- Added date headers and text filtering to the session picker; sort by modified date.
- Added project-directory tracking to sessions. Display project in TUI header.
- Changed provider authentication to explicit `login` and `token-refresh` commands instead of command-line options.

### Improvements
- Model and reasoning-effort pickers now indicate the active selection.
- File and directory tools now respect `.gitignore` rules when traversing project files.
- Improved tool-call compatibility by clarifying required arguments in tool descriptions.
- Improved `read` tool line-limit wording.
- Renamed the `question` tool to `ask`.
- Status messages clear automatically after a short delay.
- Renaming a session returns to the full session list.
- Updated the GenAI client library to v0.7.

## v0.4.0

## New features
- Added an optional DuckDuckGo web search tool. It is disabled by default.
- Added an overwrite option to the write tool so existing files can be replaced when requested.
- Added replace all option to edit tool to allow multiple replacements.

## Improvements
- Renamed the file tools to read, write, and edit, and renamed create_file to write.
- Standardized tool argument names to better match other harnesses, and refined tool and argument descriptions.
- Render tool errors where parameters are missing or the wrong type in a striked dim style.
- Improved prompt-history navigation with the up and down keys.
- Reject empty shell commands and empty values for required tool arguments, with clearer validation errors.
- Updated the README and website documentation to reflect the current tools and commands.

## v0.3.0

### New features
- Added `--agent` and `--model` options for configuring the harness in non-TUI mode.
- Added a built-in `minimal` agent with only read_file, create_file, edit_file and bash.
- Agent definitions can now include descriptions and enable or disable individual tools.
- Added configuration for tool output limits, and truncate oversized tool output.
- The TUI remembers and restores the last-used agent.
- Session list now marks the active session.

### Improvements
- Improved prompt-history navigation at the beginning and end of the input.
- Only allow cancelling an agent while busy.
- Fixed reporting of "new lines" count.
- Made the grep tool's `path` argument optional.
- Standardized tool argument names and missing-argument errors.
- The question tool now consistently uses `prompt` rather than `title`.
- Only expose the skills tool when skills are available.
- Use platform-appropriate directories for configuration.

## v0.2.0

### New features
- Added /theme to change the display theme, and save display preferences across sessions
- Added OAuth sign-in for Grok and Codex, including browser-based login and token refresh
- Added --cli for a simple interactive mode
- Added macOS builds and update installer to download Linux and macOS releases

### Improvements
- Made rate-limit waits cancellable
- Don't clear prompt input when canceling file selection
- Made grep and glob results use relative paths
- Set width when rendering Markdown tables
- Don't ask for model during setup and instead print system message on load
