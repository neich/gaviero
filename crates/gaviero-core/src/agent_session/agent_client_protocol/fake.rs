//! Scripted ACP agent used by unit/integration tests and the `fake-acp-agent` bin.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Run a scripted ACP agent on stdio.
pub async fn serve_stdio(scenario: &str) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    serve(BufReader::new(stdin), stdout, scenario).await
}

pub async fn serve<R, W>(mut reader: BufReader<R>, mut writer: W, scenario: &str) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut next_agent_id: u64 = 9000;
    let mut turn_count = 0;
    let mut session_params = Value::Null;
    let mut model_params = Value::Null;
    let mut reasoning_params = Value::Null;
    // agy scenarios: what the client sent, and a prompt held open while a
    // permission request waits for the client's answer.
    let mut resume_params = Value::Null;
    let mut mode_params = Value::Null;
    let mut agy_pending: Option<(Value, u64)> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        // agy: the client's answer to the permission request a held prompt is
        // waiting on. Echo the chosen option, settle the tool, finish the turn.
        if method.is_empty()
            && let Some((prompt_id, perm_id)) = agy_pending.take()
        {
            if msg.get("id").and_then(Value::as_u64) != Some(perm_id) {
                agy_pending = Some((prompt_id, perm_id));
                continue;
            }
            let option = msg["result"]["outcome"]["optionId"]
                .as_str()
                .unwrap_or("none")
                .to_string();
            emit_update(
                &mut writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": format!("OPTION:{option}") }
                }),
            )
            .await?;
            emit_update(
                &mut writer,
                json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed" }),
            )
            .await?;
            write_json(&mut writer, &agy_prompt_result(prompt_id)).await?;
            continue;
        }
        if is_agy_scenario(scenario) {
            match method {
                "initialize" => {
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": 1,
                                "agentCapabilities": {
                                    "loadSession": true,
                                    "promptCapabilities": { "image": true, "audio": false, "embeddedContext": true },
                                    "sessionCapabilities": { "resume": {}, "close": {}, "additionalDirectories": {} }
                                },
                                "agentInfo": { "name": "fake-agy", "version": "0.0.0" },
                                "authMethods": []
                            }
                        }),
                    )
                    .await?;
                    continue;
                }
                "session/new" => {
                    session_params = msg["params"].clone();
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "sessionId": "sess_agy",
                                "models": {
                                    "availableModels": [ { "modelId": "fake-low" }, { "modelId": "fake-high" } ],
                                    "currentModelId": "fake-low"
                                },
                                "modes": {
                                    "currentModeId": "default",
                                    "availableModes": [ { "id": "default" }, { "id": "accept-edits" } ]
                                }
                            }
                        }),
                    )
                    .await?;
                    continue;
                }
                "session/resume" => {
                    resume_params = msg["params"].clone();
                    let reply = if msg["params"]["sessionId"] == "sess_agy" {
                        json!({ "jsonrpc": "2.0", "id": id, "result": {} })
                    } else {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32603, "message": "Failed to read session; retry, or load it with session/load" }
                        })
                    };
                    write_json(&mut writer, &reply).await?;
                    continue;
                }
                "session/set_model" => {
                    let model = msg["params"]["modelId"].as_str().unwrap_or("");
                    let reply = if ["fake-low", "fake-high"].contains(&model) {
                        model_params = msg["params"].clone();
                        json!({ "jsonrpc": "2.0", "id": id, "result": {} })
                    } else {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32602, "message": format!("Unknown model ID: {model}") }
                        })
                    };
                    write_json(&mut writer, &reply).await?;
                    continue;
                }
                "session/set_mode" => {
                    mode_params = msg["params"].clone();
                    write_json(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": {} }))
                        .await?;
                    continue;
                }
                "session/prompt" => {
                    let prompt_id = id.clone().unwrap_or(Value::Null);
                    if scenario == "agy_inspect" {
                        emit_update(&mut writer, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": json!({ "new": session_params, "resume": resume_params, "model": model_params, "mode": mode_params, "prompt": msg["params"]["prompt"] }).to_string() } })).await?;
                        emit_update(&mut writer, json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "File hello.txt", "kind": "edit", "status": "in_progress", "rawInput": { "Path": "hello.txt", "Content": "hi" } })).await?;
                        emit_update(&mut writer, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "pending" })).await?;
                        emit_update(&mut writer, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed" })).await?;
                        write_json(&mut writer, &agy_prompt_result(prompt_id)).await?;
                    } else {
                        let (name, title, raw) = if scenario == "agy_mcp_permission" {
                            ("mcp_gaviero_memory_search", "Run mcp_gaviero_memory_search", json!({ "query": "x" }))
                        } else {
                            ("run_command", "npm install", json!({ "CommandLine": "npm install", "WaitMsBeforeAsync": 5000 }))
                        };
                        emit_update(&mut writer, json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": title, "kind": "execute", "status": "in_progress", "rawInput": raw })).await?;
                        emit_update(&mut writer, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "pending" })).await?;
                        next_agent_id += 1;
                        write_json(
                            &mut writer,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": next_agent_id,
                                "method": "session/request_permission",
                                "params": {
                                    "sessionId": "sess_agy",
                                    "toolCall": {
                                        "sessionUpdate": "tool_call",
                                        "toolCallId": "t1",
                                        "name": name,
                                        "title": title,
                                        "rawInput": raw,
                                        "status": "pending",
                                        "kind": "execute"
                                    },
                                    "options": [
                                        { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                                        { "optionId": "deny", "name": "Deny", "kind": "reject_once" }
                                    ]
                                }
                            }),
                        )
                        .await?;
                        agy_pending = Some((prompt_id, next_agent_id));
                    }
                    continue;
                }
                _ => {}
            }
        }
        match method {
            "initialize" => {
                write_json(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": {
                                "loadSession": false,
                                "mcpCapabilities": { "http": true, "sse": false },
                                "promptCapabilities": { "image": false, "audio": false, "embeddedContext": true }
                            },
                            "agentInfo": { "name": "fake-acp-agent", "version": "0.0.0" },
                            "authMethods": [],
                            "configOptions": config_options_for(scenario)
                        }
                    }),
                )
                .await?;
            }
            "session/new" => {
                session_params = msg["params"].clone();
                let extra_nonempty = session_params
                    .get("additionalDirectories")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty());
                if scenario == "reject_extra_dirs" && extra_nonempty {
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32602,
                                "message": "Invalid params: additionalDirectories is not supported"
                            }
                        }),
                    )
                    .await?;
                } else {
                    let config_options = config_options_for(scenario);
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "sessionId": "sess_test",
                                "configOptions": config_options
                            }
                        }),
                    )
                    .await?;
                }
            }
            "session/set_config_option" | "session/set_model" => {
                let config_id = msg["params"]["configId"].as_str().unwrap_or("");
                let is_model = method == "session/set_model" || config_id == "model";
                if is_model {
                    model_params = msg["params"].clone();
                    write_json(
                        &mut writer,
                        &json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                    )
                    .await?;
                } else if config_id == "reasoning_effort"
                    && !advertised_reasoning_values(scenario)
                        .iter()
                        .any(|value| Some(value.as_str()) == msg["params"]["value"].as_str())
                {
                    // Live dsh rejects a select value it did not advertise with
                    // `unknown reasoning effort for provider/model: …`; mirror
                    // that so a mis-mapped gaviero effort fails the test rather
                    // than passing silently.
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32602,
                                "message": format!(
                                    "unknown reasoning effort for provider/model: {}",
                                    msg["params"]["value"]
                                )
                            }
                        }),
                    )
                    .await?;
                } else {
                    reasoning_params = msg["params"].clone();
                    write_json(
                        &mut writer,
                        &json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                    )
                    .await?;
                }
            }
            "session/prompt" => {
                turn_count += 1;
                if scenario == "inspect"
                    || scenario == "model_catalog"
                    || scenario == "thinking_only"
                    || scenario == "reasoning_default_off"
                    || scenario == "thinking_disabled"
                {
                    emit_update(&mut writer, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": json!({ "turn": turn_count, "new": session_params, "model": model_params, "reasoning": reasoning_params, "prompt": msg["params"]["prompt"] }).to_string() } })).await?;
                }
                handle_prompt(&mut writer, id, scenario, &mut next_agent_id).await?;
                if scenario == "die_mid" {
                    return Ok(());
                }
            }
            "session/cancel" => {}
            _ => {
                if let Some(id) = id {
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("unknown method {method}") }
                        }),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

/// Config options this scenario advertises, mirroring live dsh: a
/// `reasoning_effort` select (`category: "thought_level"`) whose values are the
/// DeepSeek adapter's `off | low | high | max`, the legacy `thinking` boolean
/// for the fallback path, or the opaque model catalog.
fn config_options_for(scenario: &str) -> Value {
    match scenario {
        "model_catalog" => json!([{
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": "[\"deepseek-official\",\"deepseek-v4-pro\"]",
            "options": [{
                "group": "deepseek-official",
                "name": "DeepSeek",
                "options": [
                    {
                        "value": "[\"deepseek-official\",\"deepseek-v4-flash\"]",
                        "name": "DeepSeek-V4-Flash"
                    },
                    {
                        "value": "[\"deepseek-official\",\"deepseek-v4-pro\"]",
                        "name": "DeepSeek-V4-Pro"
                    }
                ]
            }]
        }]),
        // Legacy shape: a bare boolean with no select values.
        "thinking_only" => json!([{ "id": "thinking", "type": "boolean" }]),
        // A deployment whose profile selected DeepSeek's `off`: every level is
        // still offered, but inheriting the default would run without thinking.
        "reasoning_default_off" => json!([{
            "id": "reasoning_effort",
            "name": "Reasoning effort",
            "category": "thought_level",
            "type": "select",
            "currentValue": "off",
            "options": [
                { "value": "off", "name": "Off" },
                { "value": "low", "name": "Low" },
                { "value": "high", "name": "High" },
                { "value": "max", "name": "Max" }
            ]
        }]),
        // `thinking: disabled` in the dsh deployment: only `off` is advertised,
        // so no ACP value can turn thinking back on.
        "thinking_disabled" => json!([{
            "id": "reasoning_effort",
            "name": "Reasoning effort",
            "category": "thought_level",
            "type": "select",
            "currentValue": "off",
            "options": [
                { "value": "off", "name": "Off" }
            ]
        }]),
        _ => json!([{
            "id": "reasoning_effort",
            "name": "Reasoning effort",
            "category": "thought_level",
            "type": "select",
            "currentValue": "high",
            "options": [
                { "value": "off", "name": "Off" },
                { "value": "low", "name": "Low" },
                { "value": "high", "name": "High" },
                { "value": "max", "name": "Max" }
            ]
        }]),
    }
}

/// The `reasoning_effort` select values `scenario` advertises, for validating
/// `session/set_config_option` the way live dsh does.
fn advertised_reasoning_values(scenario: &str) -> Vec<String> {
    config_options_for(scenario)
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| option.get("id").and_then(Value::as_str) == Some("reasoning_effort"))
        .filter_map(|option| option.get("options"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|entry| {
            entry
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

async fn handle_prompt<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    id: Option<Value>,
    scenario: &str,
    next_agent_id: &mut u64,
) -> Result<()> {
    match scenario {
        "die_mid" => {
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "partial" }
                }),
            )
            .await?;
            std::process::exit(1);
        }
        "write" => {
            *next_agent_id += 1;
            write_json(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": *next_agent_id,
                    "method": "fs/write_text_file",
                    "params": {
                        "sessionId": "sess_test",
                        "path": "src/from_acp.rs",
                        "content": "fn ok() {}\n"
                    }
                }),
            )
            .await?;
            // The client answers; we do not wait — tests that need the
            // reply still complete the prompt.
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "wrote file" }
                }),
            )
            .await?;
        }
        "write_outside" => {
            *next_agent_id += 1;
            write_json(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": *next_agent_id,
                    "method": "fs/write_text_file",
                    "params": {
                        "sessionId": "sess_test",
                        "path": "secret/out.rs",
                        "content": "nope\n"
                    }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "tried outside write" }
                }),
            )
            .await?;
        }
        "permission" => {
            *next_agent_id += 1;
            write_json(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": *next_agent_id,
                    "method": "session/request_permission",
                    "params": {
                        "sessionId": "sess_test",
                        "toolCall": { "toolCallId": "call_perm" },
                        "options": [
                            { "optionId": "allow-once", "name": "Allow", "kind": "allow_once" },
                            { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" }
                        ]
                    }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "asked" }
                }),
            )
            .await?;
        }
        "direct_write" => {
            let path = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("leaked.txt");
            let _ = std::fs::write(&path, "bypassed fs channel\n");
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "done" }
                }),
            )
            .await?;
        }
        // Like `direct_write`, but against paths a reconciler can classify:
        // a tracked file, a file created this turn, and a deletion. Every write
        // bypasses `fs/write_text_file` on purpose — this is the channel real
        // `dsh` uses for its own edits.
        "direct_write_multi" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let _ = std::fs::write(
                cwd.join("Cargo.toml"),
                "[package]\nname=\"t\"\nversion=\"9.9.9\"\n",
            );
            let _ = std::fs::write(cwd.join("new_file.rs"), "fn fresh() {}\n");
            let _ = std::fs::remove_file(cwd.join("src").join("obsolete.rs"));
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "wrote directly" }
                }),
            )
            .await?;
        }
        "read" => {
            *next_agent_id += 1;
            write_json(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": *next_agent_id,
                    "method": "fs/read_text_file",
                    "params": {
                        "sessionId": "sess_test",
                        "path": "Cargo.toml"
                    }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "read cargo" }
                }),
            )
            .await?;
        }
        _ => {
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "thinking" }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello from fake acp" }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "c1",
                    "title": "Read",
                    "kind": "read",
                    "rawInput": { "file_path": "README.md" }
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "c1",
                    "status": "completed"
                }),
            )
            .await?;
            emit_update(
                writer,
                json!({
                    "sessionUpdate": "usage_update",
                    "used": 11, "size": 1000000
                }),
            )
            .await?;
        }
    }
    write_json(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "stopReason": "end_turn" }
        }),
    )
    .await?;
    Ok(())
}

/// agy-shaped scenarios (`agy_inspect`, `agy_permission`, `agy_mcp_permission`).
fn is_agy_scenario(scenario: &str) -> bool {
    scenario.starts_with("agy_")
}

/// agy's `session/prompt` result: stop reason plus the token usage live agy
/// reports there (agy 1.5.18).
fn agy_prompt_result(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "stopReason": "end_turn",
            "usage": { "inputTokens": 11, "outputTokens": 7, "cachedReadTokens": 0, "thoughtTokens": 0, "totalTokens": 18 }
        }
    })
}

async fn emit_update<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    update: Value,
) -> Result<()> {
    write_json(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": "sess_test", "update": update }
        }),
    )
    .await
}

async fn write_json<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
