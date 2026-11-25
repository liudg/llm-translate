//! Curated Chinese and English Microsoft Edge Neural voices.

use serde::{Deserialize, Serialize};

pub const DEFAULT_ZH_VOICE: &str = "zh-CN-XiaoxiaoNeural";
pub const DEFAULT_EN_VOICE: &str = "en-US-AriaNeural";
pub const DEFAULT_RATE: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsLang {
    Zh,
    En,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub lang: TtsLang,
    pub badge: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCatalog {
    pub zh: &'static [VoiceInfo],
    pub en: &'static [VoiceInfo],
}

pub const ZH_VOICES: &[VoiceInfo] = &[
    VoiceInfo {
        id: "zh-CN-XiaoxiaoNeural",
        label: "晓晓",
        desc: "女声 · 温暖自然",
        lang: TtsLang::Zh,
        badge: Some("默认"),
    },
    VoiceInfo {
        id: "zh-CN-XiaoxiaoMultilingualNeural",
        label: "晓晓（多语）",
        desc: "女声 · 中英混排自然",
        lang: TtsLang::Zh,
        badge: Some("多语"),
    },
    VoiceInfo {
        id: "zh-CN-YunxiNeural",
        label: "云希",
        desc: "男声 · 阳光沉稳",
        lang: TtsLang::Zh,
        badge: None,
    },
    VoiceInfo {
        id: "zh-CN-YunyangNeural",
        label: "云扬",
        desc: "男声 · 清晰专业",
        lang: TtsLang::Zh,
        badge: None,
    },
    VoiceInfo {
        id: "zh-CN-XiaoyiNeural",
        label: "晓伊",
        desc: "女声 · 活泼轻快",
        lang: TtsLang::Zh,
        badge: None,
    },
];

pub const EN_VOICES: &[VoiceInfo] = &[
    VoiceInfo {
        id: "en-US-AriaNeural",
        label: "Aria",
        desc: "美式女声 · 自然流畅",
        lang: TtsLang::En,
        badge: Some("默认"),
    },
    VoiceInfo {
        id: "en-US-JennyNeural",
        label: "Jenny",
        desc: "美式女声 · 清晰亲切",
        lang: TtsLang::En,
        badge: None,
    },
    VoiceInfo {
        id: "en-US-AndrewMultilingualNeural",
        label: "Andrew（多语）",
        desc: "美式男声 · 中英混排自然",
        lang: TtsLang::En,
        badge: Some("多语"),
    },
    VoiceInfo {
        id: "en-US-GuyNeural",
        label: "Guy",
        desc: "美式男声 · 沉稳",
        lang: TtsLang::En,
        badge: None,
    },
    VoiceInfo {
        id: "en-GB-SoniaNeural",
        label: "Sonia",
        desc: "英式女声 · 标准英音",
        lang: TtsLang::En,
        badge: Some("英音"),
    },
    VoiceInfo {
        id: "en-GB-RyanNeural",
        label: "Ryan",
        desc: "英式男声 · 标准英音",
        lang: TtsLang::En,
        badge: Some("英音"),
    },
];

pub fn catalog() -> VoiceCatalog {
    VoiceCatalog { zh: ZH_VOICES, en: EN_VOICES }
}

pub fn is_valid_voice(id: &str, lang: TtsLang) -> bool {
    let list = match lang {
        TtsLang::Zh => ZH_VOICES,
        TtsLang::En => EN_VOICES,
    };
    list.iter().any(|v| v.id == id)
}

pub fn is_any_valid_voice(id: &str) -> bool {
    is_valid_voice(id, TtsLang::Zh) || is_valid_voice(id, TtsLang::En)
}

pub fn sanitize_voice(id: &str, lang: TtsLang) -> &'static str {
    let list = match lang {
        TtsLang::Zh => ZH_VOICES,
        TtsLang::En => EN_VOICES,
    };
    list.iter()
        .find(|v| v.id == id)
        .map(|v| v.id)
        .unwrap_or(match lang {
            TtsLang::Zh => DEFAULT_ZH_VOICE,
            TtsLang::En => DEFAULT_EN_VOICE,
        })
}

/// Convert a short Edge voice name (e.g. `zh-CN-XiaoxiaoNeural`) into the full
/// SSML voice identifier expected by Edge Read Aloud.
pub fn ssml_voice_name(short_name: &str) -> String {
    if short_name.starts_with("Microsoft Server Speech Text to Speech Voice (") {
        return short_name.to_string();
    }
    let mut parts = short_name.splitn(3, '-');
    if let (Some(lang), Some(region), Some(rest)) = (parts.next(), parts.next(), parts.next()) {
        if let Some((sub, name)) = rest.split_once('-') {
            format!("Microsoft Server Speech Text to Speech Voice ({lang}-{region}-{sub}, {name})")
        } else {
            format!("Microsoft Server Speech Text to Speech Voice ({lang}-{region}, {rest})")
        }
    } else {
        short_name.to_string()
    }
}

/// Clamp playback rate to `[0.5, 2.0]` and convert to Edge SSML percentage (`+0%`, `-25%`, `+50%`).
pub fn rate_to_percent(rate: f32) -> i32 {
    let r = if rate.is_finite() { rate.clamp(0.5, 2.0) } else { DEFAULT_RATE };
    ((r - 1.0) * 100.0).round() as i32
}

pub fn format_ssml_rate(rate_pct: i32) -> String {
    if rate_pct >= 0 {
        format!("+{rate_pct}%")
    } else {
        format!("{rate_pct}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ssml_voice_and_rate() {
        assert_eq!(
            ssml_voice_name("zh-CN-XiaoxiaoNeural"),
            "Microsoft Server Speech Text to Speech Voice (zh-CN, XiaoxiaoNeural)"
        );
        assert_eq!(
            ssml_voice_name("en-US-AndrewMultilingualNeural"),
            "Microsoft Server Speech Text to Speech Voice (en-US, AndrewMultilingualNeural)"
        );
        assert_eq!(
            ssml_voice_name("zh-CN-liaoning-XiaobeiNeural"),
            "Microsoft Server Speech Text to Speech Voice (zh-CN-liaoning, XiaobeiNeural)"
        );
        assert_eq!(format_ssml_rate(rate_to_percent(1.0)), "+0%");
        assert_eq!(format_ssml_rate(rate_to_percent(0.75)), "-25%");
        assert_eq!(format_ssml_rate(rate_to_percent(1.25)), "+25%");
        assert_eq!(format_ssml_rate(rate_to_percent(1.5)), "+50%");
        assert_eq!(format_ssml_rate(rate_to_percent(f32::NAN)), "+0%");
    }
}
