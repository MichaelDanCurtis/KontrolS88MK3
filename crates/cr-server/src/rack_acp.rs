//! Minimal, capability-restricted Agent Client Protocol v1 client for Rack planning.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const ACP_PROTOCOL_VERSION: u64 = 1;
const MAX_ACP_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACP_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

fn default_command() -> String {
    "gemini".to_owned()
}

fn default_args() -> Vec<String> {
    vec!["--acp".to_owned()]
}

/// Process used as a stable ACP v1 agent. The default targets Gemini CLI, but
/// any line-delimited JSON-RPC ACP agent can be substituted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RackAcpConfig {
    #[serde(default = "default_command")]
    pub command: String,
    #[serde(default = "default_args")]
    pub args: Vec<String>,
    /// Optional isolated working directory. If omitted, Control Room uses a
    /// dedicated directory under the operating system's temporary directory.
    pub cwd: Option<PathBuf>,
}

impl Default for RackAcpConfig {
    fn default() -> Self {
        Self {
            command: default_command(),
            args: default_args(),
            cwd: None,
        }
    }
}

impl RackAcpConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.command.trim().is_empty(),
            "ai.acp.command must not be empty"
        );
        anyhow::ensure!(
            self.args.iter().all(|arg| !arg.contains('\0')),
            "ai.acp.args must not contain NUL bytes"
        );
        if let Some(cwd) = &self.cwd {
            anyhow::ensure!(cwd.is_absolute(), "ai.acp.cwd must be an absolute path");
            anyhow::ensure!(cwd.is_dir(), "ai.acp.cwd must name an existing directory");
        }
        Ok(())
    }

    pub fn label(&self) -> String {
        let command = Path::new(&self.command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.command);
        if self.args.is_empty() {
            command.to_owned()
        } else {
            format!("{command} {}", self.args.join(" "))
        }
    }
}

pub(crate) async fn complete(
    config: &RackAcpConfig,
    prompt: String,
    timeout: Duration,
) -> Result<String, String> {
    tokio::time::timeout(timeout, complete_inner(config, prompt))
        .await
        .map_err(|_| format!("ACP agent timed out after {} seconds", timeout.as_secs()))?
}

async fn complete_inner(config: &RackAcpConfig, prompt: String) -> Result<String, String> {
    let cwd = match &config.cwd {
        Some(path) => path.clone(),
        None => {
            let path = std::env::temp_dir().join("controlroom-rack-acp");
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("could not create isolated ACP directory: {error}"))?;
            path
        }
    };

    let mut command = Command::new(&config.command);
    command
        .args(&config.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start ACP agent {:?}: {error}", config.command))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ACP agent stdin was unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ACP agent stdout was unavailable".to_owned())?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            // Drain continuously so a verbose agent cannot block on its stderr
            // pipe. Provider diagnostics remain in the provider's own logs.
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0u8; 8192];
            while reader.read(&mut buffer).await.unwrap_or(0) != 0 {}
        });
    }

    let mut connection = AcpConnection::new(child, stdin, stdout);
    let mut output = String::new();
    let initialized = connection
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": ACP_PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                    "auth": {"terminal": false}
                },
                "clientInfo": {
                    "name": "control-room-rack-copilot",
                    "title": "Control Room Rack Copilot",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            &mut output,
        )
        .await?;
    let negotiated = initialized
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| "ACP initialize response omitted protocolVersion".to_owned())?;
    if negotiated != ACP_PROTOCOL_VERSION {
        return Err(format!(
            "ACP agent selected unsupported protocol version {negotiated}; Control Room supports v1"
        ));
    }

    let session = connection
        .request(
            2,
            "session/new",
            json!({"cwd": cwd, "mcpServers": []}),
            &mut output,
        )
        .await?;
    let session_id = session
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| "ACP session/new response omitted sessionId".to_owned())?
        .to_owned();
    let completed = connection
        .request(
            3,
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": prompt}]
            }),
            &mut output,
        )
        .await?;
    let stop_reason = completed
        .get("stopReason")
        .and_then(Value::as_str)
        .ok_or_else(|| "ACP session/prompt response omitted stopReason".to_owned())?;
    if stop_reason != "end_turn" {
        return Err(format!("ACP agent stopped with {stop_reason:?}"));
    }
    if output.trim().is_empty() {
        return Err("ACP agent returned no message text".to_owned());
    }
    Ok(output)
}

struct AcpConnection {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl AcpConnection {
    fn new(child: Child, stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            _child: child,
            stdin,
            lines: BufReader::new(stdout).lines(),
        }
    }

    async fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        output: &mut String,
    ) -> Result<Value, String> {
        let collect_output = method == "session/prompt";
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .map_err(|error| format!("failed reading ACP response: {error}"))?
                .ok_or_else(|| format!("ACP agent exited while waiting for {method}"))?;
            if line.len() > MAX_ACP_MESSAGE_BYTES {
                return Err("ACP message exceeded 4 MiB".to_owned());
            }
            let message: Value = serde_json::from_str(&line)
                .map_err(|error| format!("ACP agent emitted invalid JSON-RPC: {error}"))?;

            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown ACP error");
                    return Err(format!("ACP {method} failed ({code}): {detail}"));
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("ACP {method} response omitted result"));
            }

            if message.get("method").and_then(Value::as_str) == Some("session/update") {
                if collect_output {
                    append_agent_text(&message, output)?;
                }
                continue;
            }

            if message.get("id").is_some() && message.get("method").is_some() {
                self.reject_agent_request(&message).await?;
            }
        }
    }

    async fn reject_agent_request(&mut self, request: &Value) -> Result<(), String> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if request.get("method").and_then(Value::as_str) == Some("session/request_permission") {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"outcome": {"outcome": "cancelled"}}
            }))
            .await
        } else {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "Control Room exposes no ACP client tools"}
            }))
            .await
        }
    }

    async fn send(&mut self, message: Value) -> Result<(), String> {
        let mut encoded = serde_json::to_vec(&message)
            .map_err(|error| format!("could not encode ACP request: {error}"))?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .await
            .map_err(|error| format!("failed writing ACP request: {error}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| format!("failed flushing ACP request: {error}"))
    }
}

fn append_agent_text(message: &Value, output: &mut String) -> Result<(), String> {
    let update = message.pointer("/params/update");
    if update
        .and_then(|value| value.get("sessionUpdate"))
        .and_then(Value::as_str)
        != Some("agent_message_chunk")
    {
        return Ok(());
    }
    let Some(text) = update
        .and_then(|value| value.get("content"))
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if output.len().saturating_add(text.len()) > MAX_ACP_OUTPUT_BYTES {
        return Err("ACP agent output exceeded 4 MiB".to_owned());
    }
    output.push_str(text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_only_agent_text_chunks() {
        let mut output = String::new();
        append_agent_text(
            &json!({
                "method": "session/update",
                "params": {"update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "{\"summary\":"}
                }}
            }),
            &mut output,
        )
        .unwrap();
        append_agent_text(
            &json!({
                "method": "session/update",
                "params": {"update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "secret"}
                }}
            }),
            &mut output,
        )
        .unwrap();
        assert_eq!(output, "{\"summary\":");
    }

    #[test]
    fn default_agent_is_generic_acp_over_gemini_cli() {
        let config = RackAcpConfig::default();
        assert_eq!(config.command, "gemini");
        assert_eq!(config.args, ["--acp"]);
        assert_eq!(config.label(), "gemini --acp");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn negotiates_v1_cancels_permissions_and_collects_streamed_output() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "controlroom-acp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let script = directory.join("mock-acp-agent");
        std::fs::write(
            &script,
            r##"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}'
IFS= read -r new_session
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"test-session"}}'
IFS= read -r prompt
printf '%s\n' '{"jsonrpc":"2.0","id":99,"method":"session/request_permission","params":{"sessionId":"test-session","toolCall":{"toolCallId":"nope"},"options":[]}}'
IFS= read -r permission_response
case "$permission_response" in
  *'"cancelled"'*) ;;
  *) exit 7 ;;
esac
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"test-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"{\"summary\":\"safe\""}}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"test-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"}"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
"##,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();

        let result = complete(
            &RackAcpConfig {
                command: script.to_string_lossy().into_owned(),
                args: vec![],
                cwd: Some(directory.clone()),
            },
            "make a patch".into(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(result, r#"{"summary":"safe"}"#);

        std::fs::remove_file(script).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
