//! Capability-restricted structured-output adapters for local AI CLIs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const MAX_CLI_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLI_ERROR_BYTES: usize = 32 * 1024;
static WORKSPACE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn default_claude_command() -> String {
    "claude".to_owned()
}

fn default_grok_command() -> String {
    "grok".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RackClaudeConfig {
    #[serde(default = "default_claude_command")]
    pub command: String,
    pub model: Option<String>,
}

impl Default for RackClaudeConfig {
    fn default() -> Self {
        Self {
            command: default_claude_command(),
            model: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RackGrokConfig {
    #[serde(default = "default_grok_command")]
    pub command: String,
    pub model: Option<String>,
}

impl Default for RackGrokConfig {
    fn default() -> Self {
        Self {
            command: default_grok_command(),
            model: None,
        }
    }
}

impl RackClaudeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_command_and_model("claude", &self.command, self.model.as_deref())
    }

    pub fn model_label(&self) -> String {
        self.model.clone().unwrap_or_else(|| "CLI default".into())
    }
}

impl RackGrokConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_command_and_model("grok", &self.command, self.model.as_deref())
    }

    pub fn model_label(&self) -> String {
        self.model.clone().unwrap_or_else(|| "CLI default".into())
    }
}

fn validate_command_and_model(
    provider: &str,
    command: &str,
    model: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !command.trim().is_empty(),
        "ai.{provider}.command must not be empty"
    );
    if let Some(model) = model {
        anyhow::ensure!(
            !model.trim().is_empty(),
            "ai.{provider}.model must not be empty when specified"
        );
    }
    Ok(())
}

pub(crate) async fn complete_claude(
    config: &RackClaudeConfig,
    prompt: &str,
    schema: &Value,
    timeout: Duration,
) -> Result<String, String> {
    let workspace = TempWorkspace::new()?;
    let schema = serde_json::to_string(schema)
        .map_err(|error| format!("could not encode Rack plan schema: {error}"))?;
    let mut args = vec![
        "--print".to_owned(),
        "--output-format".to_owned(),
        "json".to_owned(),
        "--json-schema".to_owned(),
        schema,
        "--tools".to_owned(),
        String::new(),
        "--permission-mode".to_owned(),
        "dontAsk".to_owned(),
        "--max-turns".to_owned(),
        "1".to_owned(),
        "--no-session-persistence".to_owned(),
        "--disable-slash-commands".to_owned(),
        "--safe-mode".to_owned(),
    ];
    if let Some(model) = &config.model {
        args.extend(["--model".to_owned(), model.clone()]);
    }
    run_cli(
        "Claude",
        &config.command,
        &args,
        workspace.path(),
        Some(prompt.as_bytes()),
        &[],
        timeout,
    )
    .await
}

pub(crate) async fn complete_grok(
    config: &RackGrokConfig,
    prompt: &str,
    schema: &Value,
    timeout: Duration,
) -> Result<String, String> {
    let workspace = TempWorkspace::new()?;
    let prompt_path = workspace.path().join("rack-plan-prompt.txt");
    std::fs::write(&prompt_path, prompt)
        .map_err(|error| format!("could not write isolated Grok prompt: {error}"))?;
    let schema = serde_json::to_string(schema)
        .map_err(|error| format!("could not encode Rack plan schema: {error}"))?;
    let grok_home = prepare_grok_home(workspace.path())?;
    let mut args = vec![
        "--prompt-file".to_owned(),
        prompt_path.to_string_lossy().into_owned(),
        "--output-format".to_owned(),
        "json".to_owned(),
        "--json-schema".to_owned(),
        schema,
        "--tools".to_owned(),
        String::new(),
        "--permission-mode".to_owned(),
        "dontAsk".to_owned(),
        "--max-turns".to_owned(),
        "1".to_owned(),
        "--no-subagents".to_owned(),
        "--no-memory".to_owned(),
        "--disable-web-search".to_owned(),
        "--no-auto-update".to_owned(),
        "--verbatim".to_owned(),
        "--deny".to_owned(),
        "Bash".to_owned(),
        "--deny".to_owned(),
        "Edit".to_owned(),
        "--deny".to_owned(),
        "Write".to_owned(),
        "--deny".to_owned(),
        "Read".to_owned(),
        "--deny".to_owned(),
        "Grep".to_owned(),
        "--deny".to_owned(),
        "WebFetch".to_owned(),
        "--deny".to_owned(),
        "MCPTool".to_owned(),
    ];
    if let Some(model) = &config.model {
        args.extend(["--model".to_owned(), model.clone()]);
    }
    run_cli(
        "Grok",
        &config.command,
        &args,
        workspace.path(),
        None,
        &[(OsString::from("GROK_HOME"), grok_home.into_os_string())],
        timeout,
    )
    .await
}

async fn run_cli(
    provider: &str,
    command: &str,
    args: &[String],
    cwd: &Path,
    stdin_bytes: Option<&[u8]>,
    envs: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<String, String> {
    let mut process = Command::new(command);
    process
        .args(args)
        .current_dir(cwd)
        .stdin(if stdin_bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    process.envs(envs.iter().cloned());
    let mut child = process
        .spawn()
        .map_err(|error| format!("could not start {provider} command {command:?}: {error}"))?;
    if let Some(bytes) = stdin_bytes {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("{provider} stdin was unavailable"))?;
        stdin.write_all(bytes).await.map_err(|error| {
            format!("could not send the Rack plan request to {provider}: {error}")
        })?;
        stdin
            .shutdown()
            .await
            .map_err(|error| format!("could not close {provider} stdin: {error}"))?;
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{provider} timed out after {} seconds", timeout.as_secs()))?
        .map_err(|error| format!("{provider} process failed: {error}"))?;
    if output.stdout.len() > MAX_CLI_OUTPUT_BYTES {
        return Err(format!("{provider} output exceeded 4 MiB"));
    }
    if !output.status.success() {
        let stderr =
            String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(MAX_CLI_ERROR_BYTES)]);
        return Err(format!(
            "{provider} exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    let body: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("{provider} returned invalid JSON: {error}"))?;
    if body.get("is_error").and_then(Value::as_bool) == Some(true) {
        let detail = body
            .get("result")
            .or_else(|| body.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("unknown provider error");
        return Err(format!("{provider} failed: {detail}"));
    }
    extract_structured_output(&body)
        .ok_or_else(|| format!("{provider} response omitted structured output"))
}

fn extract_structured_output(body: &Value) -> Option<String> {
    let output = body
        .get("structured_output")
        .or_else(|| body.get("structuredOutput"));
    if let Some(output) = output {
        return match output {
            Value::String(text) => Some(text.clone()),
            Value::Object(_) | Value::Array(_) => serde_json::to_string(output).ok(),
            _ => None,
        };
    }
    body.get("result")
        .or_else(|| body.get("text"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn prepare_grok_home(workspace: &Path) -> Result<PathBuf, String> {
    let grok_home = workspace.join("grok-home");
    std::fs::create_dir(&grok_home)
        .map_err(|error| format!("could not create isolated Grok home: {error}"))?;
    std::fs::write(
        grok_home.join("config.toml"),
        "[compat.claude]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n\n[compat.cursor]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n",
    )
    .map_err(|error| format!("could not write isolated Grok config: {error}"))?;

    if std::env::var_os("XAI_API_KEY").is_none() {
        let source_home = std::env::var_os("GROK_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".grok")));
        if let Some(source) = source_home.map(|home| home.join("auth.json"))
            && source.is_file()
        {
            let destination = grok_home.join("auth.json");
            std::fs::copy(&source, &destination)
                .map_err(|error| format!("could not stage Grok authentication: {error}"))?;
            set_owner_only(&destination)?;
        }
    }
    Ok(grok_home)
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
        .map_err(|error| format!("could not inspect staged Grok authentication: {error}"))?
        .permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("could not protect staged Grok authentication: {error}"))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new() -> Result<Self, String> {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = WORKSPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "controlroom-rack-provider-{}-{epoch:x}-{sequence:x}",
            std::process::id()
        ));
        std::fs::create_dir(&path)
            .map_err(|error| format!("could not create isolated provider directory: {error}"))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mock_script(name: &str, body: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "controlroom-cli-test-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let script = directory.join(name);
        std::fs::write(&script, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();
        (script, directory)
    }

    #[test]
    fn extracts_object_or_string_structured_output() {
        assert_eq!(
            extract_structured_output(&serde_json::json!({"structured_output": {"ok": true}})),
            Some("{\"ok\":true}".into())
        );
        assert_eq!(
            extract_structured_output(&serde_json::json!({"structured_output": "{\"ok\":true}"})),
            Some("{\"ok\":true}".into())
        );
        assert_eq!(
            extract_structured_output(&serde_json::json!({"structuredOutput": {"ok": true}})),
            Some("{\"ok\":true}".into())
        );
    }

    #[test]
    fn provider_configs_reject_empty_commands_and_models() {
        let claude = RackClaudeConfig {
            command: String::new(),
            ..RackClaudeConfig::default()
        };
        assert!(claude.validate().is_err());
        let grok = RackGrokConfig {
            model: Some(String::new()),
            ..RackGrokConfig::default()
        };
        assert!(grok.validate().is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_adapter_sends_stdin_and_disables_tools() {
        let (script, directory) = mock_script(
            "mock-claude",
            r#"
tools=missing
safe=no
schema=no
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tools) shift; [ -z "$1" ]; tools=empty ;;
    --safe-mode) safe=yes ;;
    --json-schema) shift; [ -n "$1" ]; schema=yes ;;
  esac
  shift
done
[ "$tools" = empty ]
[ "$safe" = yes ]
[ "$schema" = yes ]
prompt=$(cat)
case "$prompt" in *rack*) ;; *) exit 8 ;; esac
printf '%s\n' '{"type":"result","is_error":false,"structured_output":{"summary":"claude"}}'
"#,
        );
        let result = complete_claude(
            &RackClaudeConfig {
                command: script.to_string_lossy().into_owned(),
                model: None,
            },
            "rack request",
            &serde_json::json!({"type": "object"}),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(result, r#"{"summary":"claude"}"#);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grok_adapter_uses_prompt_file_and_denies_tools() {
        let (script, directory) = mock_script(
            "mock-grok",
            r#"
tools=missing
prompt_file=
deny_mcp=no
no_memory=no
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tools) shift; [ -z "$1" ]; tools=empty ;;
    --prompt-file) shift; prompt_file=$1 ;;
    --deny) shift; [ "$1" = MCPTool ] && deny_mcp=yes ;;
    --no-memory) no_memory=yes ;;
  esac
  shift
done
[ "$tools" = empty ]
[ "$deny_mcp" = yes ]
[ "$no_memory" = yes ]
[ -f "$prompt_file" ]
grep -q rack "$prompt_file"
printf '%s\n' '{"stopReason":"end_turn","structured_output":"{\"summary\":\"grok\"}"}'
"#,
        );
        let result = complete_grok(
            &RackGrokConfig {
                command: script.to_string_lossy().into_owned(),
                model: None,
            },
            "rack request",
            &serde_json::json!({"type": "object"}),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(result, r#"{"summary":"grok"}"#);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[ignore = "requires authenticated Claude CLI and makes a live model call"]
    #[tokio::test]
    async fn live_claude_structured_output() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"provider": {"type": "string"}, "ok": {"type": "boolean"}},
            "required": ["provider", "ok"]
        });
        let output = complete_claude(
            &RackClaudeConfig::default(),
            "Return provider claude and ok true. Do not use tools.",
            &schema,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            serde_json::json!({"provider": "claude", "ok": true})
        );
    }

    #[ignore = "requires authenticated Grok CLI and makes a live model call"]
    #[tokio::test]
    async fn live_grok_structured_output() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"provider": {"type": "string"}, "ok": {"type": "boolean"}},
            "required": ["provider", "ok"]
        });
        let output = complete_grok(
            &RackGrokConfig::default(),
            "Return provider grok and ok true. Do not use tools.",
            &schema,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            serde_json::json!({"provider": "grok", "ok": true})
        );
    }
}
