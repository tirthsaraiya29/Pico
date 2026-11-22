# Pico Extensible Tool Framework Guide

Every tool in Pico is registered within `pico-tools::ToolRegistry`, exposing a strict JSON Schema, execution timeout, cancellation handle, and permission requirement.

---

## Registered Tool Catalog

### 1. `get_system_info`
- **Permission Tier**: `ReadOnly`
- **Description**: Gathers operating system name, CPU core count, total and available memory, battery percentage, and charging state.
- **Parameters**: None.

### 2. `list_processes`
- **Permission Tier**: `ReadOnly`
- **Description**: Enumerates system processes sorted by working memory usage.
- **Parameters**:
  - `limit` (*integer, optional*): Maximum number of entries (1-50, default 10).

### 3. `kill_process`
- **Permission Tier**: `ConfirmationRequired`
- **Description**: Terminates a process by its PID. Requires explicit confirmation.
- **Parameters**:
  - `pid` (*integer, required*): The process ID to terminate.

### 4. `open_application`
- **Permission Tier**: `LowRisk`
- **Description**: Launches an application executable or system file (e.g. `notepad`, `calc`, `msedge`).
- **Parameters**:
  - `application` (*string, required*): Name or path of application.

### 5. `read_file`
- **Permission Tier**: `ReadOnly`
- **Description**: Reads file contents within permitted sandbox boundaries.
- **Parameters**:
  - `path` (*string, required*): Relative or absolute path.
  - `max_lines` (*integer, optional*): Maximum lines to read (default 500).

### 6. `write_file`
- **Permission Tier**: `ConfirmationRequired`
- **Description**: Writes text content to disk within sandbox boundaries.
- **Parameters**:
  - `path` (*string, required*): Target file path.
  - `content` (*string, required*): Content to write.

### 7. `search_files`
- **Permission Tier**: `ReadOnly`
- **Description**: Finds files matching pattern or extension.
- **Parameters**:
  - `pattern` (*string, required*): Substring to match.
  - `directory` (*string, optional*): Starting directory.

### 8. `calculator`
- **Permission Tier**: `ReadOnly`
- **Description**: Evaluates mathematical arithmetic expressions.
- **Parameters**:
  - `expression` (*string, required*): Arithmetic string (e.g. `(45 * 2) + 10`).

### 9. `manage_timer`
- **Permission Tier**: `LowRisk`
- **Description**: Sets an asynchronous countdown timer or voice reminder.
- **Parameters**:
  - `seconds` (*integer, required*): Countdown duration in seconds.
  - `label` (*string, optional*): Label for timer.

### 10. `clipboard`
- **Permission Tier**: `LowRisk`
- **Description**: Reads or updates the operating system clipboard text.
- **Parameters**:
  - `action` (*string, enum: "get" | "set", required*): Action to perform.
  - `text` (*string, required if action is "set"*): Text to place on clipboard.

### 11. `fetch_web_content`
- **Permission Tier**: `ReadOnly`
- **Description**: Fetches textual content from HTTP/HTTPS URL with SSRF protection.
- **Parameters**:
  - `url` (*string, required*): HTTP/HTTPS URL.

### 12. `execute_shell_command`
- **Permission Tier**: `ConfirmationRequired`
- **Description**: Executes validated shell command. Disallows command chaining and unvalidated metacharacters.
- **Parameters**:
  - `command` (*string, required*): Executable command name.
  - `arguments` (*array of strings, optional*): Command arguments.
