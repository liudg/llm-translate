//! Request bodies per promptProfile (DESIGN 3.3). No system prompt.

use serde_json::{Value, json};

use super::detect::Direction;
use super::segment::estimate_tokens;
use crate::deploy::manifest::PromptProfile;

pub fn build(profile: PromptProfile, dir: Direction, text: &str) -> Value {
    let target = dir.target_name();
    // Output is rarely more than ~2x the source tokens; cap well inside a 4K context.
    let max_tokens = (estimate_tokens(text) * 3 + 64).min(2400);
    match profile {
        PromptProfile::IndexTranslate => json!({
            "messages": [{
                "role": "user",
                "content": format!("请将以下文本翻译为{target}，直接输出翻译结果，不要进行任何解释。\n\n{text}"),
            }],
            "temperature": 0,
            "chat_template_kwargs": { "enable_thinking": false },
            "max_tokens": max_tokens,
        }),
        PromptProfile::HyMt => json!({
            "messages": [{
                "role": "user",
                "content": format!("将以下文本翻译为{target}，注意只需要输出翻译后的结果，不要额外解释：\n\n{text}"),
            }],
            "top_k": 20,
            "top_p": 0.6,
            "repeat_penalty": 1.05,
            "temperature": 0.7,
            "max_tokens": max_tokens,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles() {
        let b = build(PromptProfile::IndexTranslate, Direction::Zh2En, "你好");
        assert_eq!(b["messages"][0]["content"], "请将以下文本翻译为英语，直接输出翻译结果，不要进行任何解释。\n\n你好");
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["temperature"], 0);
        let b = build(PromptProfile::HyMt, Direction::En2Zh, "hi");
        assert!(b["messages"][0]["content"].as_str().unwrap().starts_with("将以下文本翻译为中文"));
        assert_eq!(b["top_k"], 20);
        assert!(b.get("chat_template_kwargs").is_none());
    }
}
