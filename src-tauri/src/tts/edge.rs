//! Direct WebSocket client for Microsoft Edge Read Aloud (MP3 synthesis).

use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_util::sync::CancellationToken;

use super::drm::{
    self, CHROMIUM_MAJOR_VERSION, SEC_MS_GEC_VERSION, TRUSTED_CLIENT_TOKEN,
};
use super::voices::{format_ssml_rate, ssml_voice_name};

const WSS_BASE: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum EdgeError {
    Cancelled,
    Network(String),
    Rejected(String),
    Protocol(String),
    NoAudio,
}

impl std::fmt::Display for EdgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "已停止朗读"),
            Self::Network(m) => write!(f, "朗读连接失败：{m}"),
            Self::Rejected(m) => write!(f, "微软语音服务拒绝连接：{m}"),
            Self::Protocol(m) => write!(f, "语音服务响应异常：{m}"),
            Self::NoAudio => write!(f, "未收到音频数据，该文本可能无法朗读"),
        }
    }
}

/// Remove control characters rejected by Edge SSML and XML-escape the text.
pub fn sanitize_and_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for c in input.chars() {
        let code = c as u32;
        if (code <= 8) || (11..=12).contains(&code) || (14..=31).contains(&code) {
            out.push(' ');
            continue;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn js_timestamp() -> String {
    Utc::now().format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
}

fn build_ssml(voice_short: &str, rate_pct: i32, text: &str) -> String {
    let voice = ssml_voice_name(voice_short);
    let rate = format_ssml_rate(rate_pct);
    let escaped = sanitize_and_escape(text);
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'>\
         <prosody pitch='+0Hz' rate='{rate}' volume='+0%'>{escaped}</prosody>\
         </voice>\
         </speak>"
    )
}

fn build_request() -> Result<tokio_tungstenite::tungstenite::http::Request<()>, EdgeError> {
    let conn_id = drm::connect_id();
    let gec = drm::generate_sec_ms_gec_v2();
    let url = format!(
        "{WSS_BASE}?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&ConnectionId={conn_id}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}"
    );
    let mut req = url
        .into_client_request()
        .map_err(|e| EdgeError::Protocol(e.to_string()))?;

    let ua = format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{CHROMIUM_MAJOR_VERSION}.0.0.0 Safari/537.36 Edg/{CHROMIUM_MAJOR_VERSION}.0.0.0"
    );
    let cookie = format!("muid={};", drm::generate_muid());
    let headers = req.headers_mut();
    headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));
    headers.insert(
        "Origin",
        HeaderValue::from_static("chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"),
    );
    headers.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(
        "User-Agent",
        HeaderValue::from_str(&ua).map_err(|e| EdgeError::Protocol(e.to_string()))?,
    );
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&cookie).map_err(|e| EdgeError::Protocol(e.to_string()))?,
    );
    Ok(req)
}

fn header_value<'a>(header_block: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    for line in header_block.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(pos) = line.iter().position(|&b| b == b':') {
            if &line[..pos] == key {
                return Some(&line[pos + 1..]);
            }
        }
    }
    None
}

async fn synthesize_once(
    text: &str,
    voice: &str,
    rate_pct: i32,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, (EdgeError, Option<String>)> {
    if cancel.is_cancelled() {
        return Err((EdgeError::Cancelled, None));
    }

    let req = build_request().map_err(|e| (e, None))?;
    let connect_fut = timeout(CONNECT_TIMEOUT, connect_async(req));
    let (mut ws, _) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err((EdgeError::Cancelled, None)),
        res = connect_fut => match res {
            Ok(Ok(pair)) => pair,
            Ok(Err(WsError::Http(resp))) => {
                let status = resp.status();
                let date = resp.headers().get("date").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                let err = if status == StatusCode::FORBIDDEN {
                    EdgeError::Rejected(format!("HTTP {status}"))
                } else {
                    EdgeError::Network(format!("HTTP {status}"))
                };
                return Err((err, if status == StatusCode::FORBIDDEN { date } else { None }));
            }
            Ok(Err(e)) => return Err((EdgeError::Network(e.to_string()), None)),
            Err(_) => return Err((EdgeError::Network("连接超时，请检查网络".into()), None)),
        }
    };

    let ts = js_timestamp();
    let config_msg = format!(
        "X-Timestamp:{ts}\r\n\
         Content-Type:application/json; charset=utf-8\r\n\
         Path:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n"
    );
    ws.send(Message::Text(config_msg.into()))
        .await
        .map_err(|e| (EdgeError::Network(e.to_string()), None))?;

    let req_id = drm::connect_id();
    let ssml = build_ssml(voice, rate_pct, text);
    let ssml_msg = format!(
        "X-RequestId:{req_id}\r\n\
         Content-Type:application/ssml+xml\r\n\
         X-Timestamp:{ts}Z\r\n\
         Path:ssml\r\n\r\n\
         {ssml}"
    );
    ws.send(Message::Text(ssml_msg.into()))
        .await
        .map_err(|e| (EdgeError::Network(e.to_string()), None))?;

    let mut audio = Vec::with_capacity(32 * 1024);

    loop {
        let next_frame = timeout(FRAME_TIMEOUT, ws.next());
        let msg = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = ws.close(None).await;
                return Err((EdgeError::Cancelled, None));
            }
            res = next_frame => match res {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => return Err((EdgeError::Network(e.to_string()), None)),
                Ok(None) => break,
                Err(_) => return Err((EdgeError::Network("接收音频超时".into()), None)),
            }
        };

        match msg {
            Message::Text(t) => {
                let bytes = t.as_bytes();
                let header_end = bytes
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .unwrap_or(bytes.len());
                if header_value(&bytes[..header_end], b"Path") == Some(b"turn.end") {
                    break;
                }
            }
            Message::Binary(b) => {
                if b.len() < 2 {
                    continue;
                }
                let header_len = u16::from_be_bytes([b[0], b[1]]) as usize;
                let data_start = 2 + header_len;
                if data_start > b.len() {
                    continue;
                }
                let headers = &b[2..data_start];
                if header_value(headers, b"Path") != Some(b"audio") {
                    continue;
                }
                let payload = &b[data_start..];
                if !payload.is_empty() {
                    audio.extend_from_slice(payload);
                }
            }
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = ws.close(None).await;
    if audio.is_empty() {
        return Err((EdgeError::NoAudio, None));
    }
    Ok(audio)
}

/// Synthesize `text` into MP3 bytes using Microsoft Edge Read Aloud (direct connection).
/// On HTTP 403 with a `Date` header, automatically corrects clock skew and retries once.
pub async fn synthesize(
    text: &str,
    voice: &str,
    rate_pct: i32,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, EdgeError> {
    match synthesize_once(text, voice, rate_pct, cancel).await {
        Ok(bytes) => Ok(bytes),
        Err((EdgeError::Rejected(msg), Some(date))) => {
            if drm::adjust_clock_skew_from_date(&date) {
                synthesize_once(text, voice, rate_pct, cancel).await.map_err(|(e, _)| e)
            } else {
                Err(EdgeError::Rejected(msg))
            }
        }
        Err((e, _)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_control_chars_and_escapes_xml() {
        let raw = "a\x00b\x0bc&d<e>f\"g'h\ni\tj";
        assert_eq!(sanitize_and_escape(raw), "a b c&amp;d&lt;e&gt;f&quot;g&apos;h\ni\tj");
    }

    #[test]
    fn extracts_binary_headers() {
        let hdr = b"X-RequestId:abc\r\nContent-Type:audio/mpeg\r\nPath:audio\r\n";
        assert_eq!(header_value(hdr, b"Path"), Some(b"audio".as_slice()));
        assert_eq!(header_value(hdr, b"Content-Type"), Some(b"audio/mpeg".as_slice()));
        assert_eq!(header_value(hdr, b"Missing"), None);
    }

    /// Live integration smoke test against Microsoft Edge Read Aloud v2.
    /// Run with: `cargo test --lib tts::edge::tests::live_smoke -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_smoke() {
        let cancel = CancellationToken::new();
        let zh = synthesize("你好，世界！测试 & <XML> 特殊字符。", "zh-CN-XiaoxiaoNeural", 0, &cancel)
            .await
            .expect("zh synthesis failed");
        assert!(zh.len() > 1000, "zh audio too small: {} bytes", zh.len());

        let en = synthesize("Hello world, testing Edge Read Aloud v2.", "en-US-AriaNeural", 25, &cancel)
            .await
            .expect("en synthesis failed");
        assert!(en.len() > 1000, "en audio too small: {} bytes", en.len());
        println!("live_smoke OK: zh={} bytes, en={} bytes", zh.len(), en.len());
    }
}
