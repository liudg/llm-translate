# 开源翻译模型调研（2026-11-20）

## 调研方法

| 来源 | 方式 | 原始结果 |
|---|---|---|
| Hugging Face | API：`pipeline_tag=translation` 按点赞/热度/新近排序；名称含 translat/MT/nmt 的模型（2025 年后、点赞 ≥15） | `hf_scan.txt`、`cards1.txt`、`index2b.md` |
| ModelScope | API 关键词"翻译/translate/translation/MT"，多种排序，筛 2025-06 后 | `ms_scan.txt` |
| WMT25 | 官方 Findings 的英→中人工评测排名 | 见下 |
| GitHub | 搜索 API，按星标排序 | 见下 |

## 与中英翻译相关的候选

| 模型 | 出品 / 时间 | 尺寸 | GGUF | 证据 |
|---|---|---|---|---|
| **Index-Translate** | B 站 IndexTeam，2026-09 | 2B / 9B / 35B-A3B（基于 Qwen3.5） | 官方 | 官方报告：2B 全面超过 Hy-MT2-1.8B（FLORES 0.866 vs 0.852，MEME 网络梗 0.64 vs 0.36，指令遵循 0.76 vs 0.49）；9B 基础质量≈Hy-MT2-7B，梗/指令明显更强；内置术语表参数；GitHub 2.1k 星 |
| **Hy-MT2** | 腾讯，2026-05 | 1.8B / 7B / 30B-A3B | 官方 | HF 翻译类点赞第一（1293）；ModelScope 下载量领先；前代 Hunyuan-MT-7B 在 WMT25 英→中人工评测第 2（88.2） |
| HY-MT1.5 | 腾讯，2025-12 | 1.8B / 7B | 官方 | M0 实测 1.8B 有语义反转错误 |
| Seed-X-PPO | 字节，2025-07 | 7B | 社区 | 主打中英；官方建议 beam search（llama.cpp 默认不支持） |
| TranslateGemma | Google，2026-01 | 4B / 12B / 27B | 社区 | Index 报告中 12B：FLORES 0.873，但指令遵循很差（0.31） |
| MiLMMT-46 | 小米，2026-08 v1.0 | 1B / 4B / 12B | 社区 | 作者称 12B 超过 Seed-X-7B、GemmaX2-9B、Tower-Plus-9B |
| LMT-60 | 东北大学小牛翻译，2025-11 | 0.6B / 1.7B / 4B / 8B（基于 Qwen3） | 社区 | 以中英为中心；部分方向接近 Hunyuan-MT-7B |
| Riva-Translate-4B v2 | NVIDIA，2026-04 | 4B | 无 | 支持简繁中英；FLORES en→zh XCOMET 0.91 |
| Marco-MT-Algharb | 阿里，2025-09 | 14B | — | WMT25 英→中人工评测第 1（88.4），但只做英→外，不做中→英 |

## 排除

- 网易有道 Confucius4-T3PO：14B 同声传译模型，不适合划词/截图。
- Cohere North-Small-Translate、Lara-4-Mini：需要申请访问（gated）。
- 讯飞 Spark-X2.5：通用模型，不是翻译专用。
- plamo-translate（日英）、Rosetta（韩语）、Sakura（日中）、sarvam（印度语种）：语种不符。
- NLLB / Opus-MT / MADLAD / MTranServer：传统 NMT，中英质量明显低于 LLM 翻译模型。

## 注意

- 各家分数大多是自家报告，测试集与设置不同，只能作参考；最终以我们自己的实测为准。
- Index-Translate 基于 Qwen3.5，需关闭思考模式（`enable_thinking: false`），官方推荐 greedy（temperature=0）。
