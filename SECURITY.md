# Security Policy

## Reporting Security Issues

We take the security of **Pico** seriously. If you believe you have discovered a vulnerability, security flaw, or safe execution breach in Pico, please do not open a public issue. Instead, report it responsibly:

- **Email**: `tirthsaraiya29@gmail.com`
- **Subject**: `[SECURITY VULNERABILITY] Pico - <Brief Description>`
- Please include:
  - Description of the vulnerability and its potential impact.
  - Minimal reproducible example or proof-of-concept steps.
  - Target system details (OS version, runtime configuration).

We will acknowledge receipt within 48 hours and work with you to analyze, mitigate, and deploy a fix before public disclosure.

---

## Pico Threat Model & Security Architecture

Pico is an AI assistant capable of executing actions on a user's local operating system. As such, the system is designed under a **Zero-Trust Principle for Model Outputs**:

1. **Untrusted LLM Output**:
   - Model outputs, function call requests, retrieved web documents, and external process stdout are treated as untrusted strings.
   - Model decisions cannot override security policies, bypass sandboxing, or escalate privileges.

2. **Multi-Tiered Tool Permissions**:
   - `ReadOnly`: Operations that inspect state without mutation (e.g. `get_system_info`, `read_file` within allowed boundaries).
   - `LowRisk`: Benign local mutations (e.g. `manage_timer`, `set_clipboard`).
   - `ConfirmationRequired`: System actions that alter files, launch shell commands, or terminate processes.
   - `Restricted`: Explicitly blacklisted operations (e.g. arbitrary elevated shell invocation, accessing sensitive system directories like `C:\Windows\System32`, `~/.ssh`).

3. **Path Validation & Sandbox Enclosure**:
   - File system tools canonicalize all paths and prevent path traversal attacks (`..`, symlink escapes, junction bypasses).
   - Sensitive user credential paths (`.ssh`, `.aws`, `.gnupg`, browser profile credentials) are barred by default.

4. **Command Execution Safety**:
   - Arbitrary shell strings are not evaluated via unrestricted shell interpreters without explicit user consent.
   - Command arguments are strictly validated against injection patterns.

5. **No Hardcoded Secrets**:
   - No API keys, credentials, or personal secrets are baked into binaries or repositories.
   - Audit trails are logged locally with secret masking enabled.

---

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
