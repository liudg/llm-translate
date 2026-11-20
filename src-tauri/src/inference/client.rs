//! Streaming client for llama-server's OpenAI-compatible chat endpoint.
//! Cancelling = dropping the connection (DESIGN 5.2).

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum ChatError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "已取消"),
            Self::Failed(m) => f.write_str(m),
        }
    }
}

pub fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        // CPU prompt processing of a long segment can take a while before the first token.
        .read_timeout(Duration::from_secs(180))
        .build()
        .expect("http client")
}

/// Pull the delta text out of one SSE `data:` payload.
fn delta_of(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    v["choices"][0]["delta"]["content"].as_str().map(str::to_owned)
}

/// POST `body` with `stream: true`, calling `on_delta` for every content token.
/// Returns the full text.
pub async fn stream_chat(
    client: &reqwest::Client,
    port: u16,
    mut body: Value,
    cancel: &CancellationToken,
    mut on_delta: impl FnMut(&str),
) -> Result<String, ChatError> {
    body["stream"] = Value::Bool(true);
    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let send = client.post(url).json(&body).send();
    let resp = tokio::select! {
        r = send => r.map_err(|e| ChatError::Failed(format!("推理服务无响应：{e}")))?,
        _ = cancel.cancelled() => return Err(ChatError::Cancelled),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or(text);
        return Err(ChatError::Failed(format!("推理服务返回 {status}：{msg}")));
    }

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut full = String::new();
    loop {
        let next = tokio::select! {
            n = stream.next() => n,
            _ = cancel.cancelled() => return Err(ChatError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| ChatError::Failed(format!("推理连接中断：{e}")))?;
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(full);
            }
            if let Some(d) = delta_of(data) {
                if !d.is_empty() {
                    on_delta(&d);
                    full.push_str(&d);
                }
            }
        }
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delta() {
        assert_eq!(
            delta_of(r#"{"choices":[{"index":0,"delta":{"content":"你好"}}]}"#).as_deref(),
            Some("你好")
        );
        assert_eq!(delta_of(r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#), None);
    }
}
