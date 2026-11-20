"""M0 spike: side-by-side translation quality comparison across models.

Usage: python -I eval_quality.py <llama_dir> <out.json> <label=model.gguf>...
Runs each model on GPU (-ngl 99) over CASES, writes results to JSON and a markdown table.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bench_llama import free_port, translate, wait_health  # noqa: E402

CASES = [
    # (id, target language, source)
    ("rust-error", "中文", "error[E0502]: cannot borrow `x` as mutable because it is also borrowed as immutable"),
    ("tech-doc", "中文", "The function returns a future that resolves once all pending writes have been flushed to disk."),
    ("idiom-en", "中文", "It's not rocket science, but it's easy to drop the ball if you cut corners."),
    ("ui-string", "中文", "Unsaved changes will be lost. Do you want to continue?"),
    ("word-en", "中文", "deprecated"),
    ("academic", "中文", "We propose a novel attention mechanism that scales linearly with sequence length while preserving the expressiveness of full attention."),
    ("npm-warn", "中文", "npm WARN deprecated inflight@1.0.6: This module is not supported, and leaks memory."),
    ("news", "中文", "Shares of the chipmaker jumped 12% in after-hours trading on Tuesday after it raised its full-year revenue forecast to $48 billion, citing stronger-than-expected demand from data centers."),
    ("colloquial", "英语", "这事儿说来话长，咱们边吃边聊吧。"),
    ("idiom-zh", "英语", "他做事总是虎头蛇尾，让人很头疼。"),
    ("tech-zh", "英语", "该接口在高并发场景下会出现偶发的超时，需要排查连接池配置。"),
    ("term-zh", "英语", "划词翻译"),
    ("copy-zh", "英语", "一键部署，零门槛上手，让每个人都能用上本地大模型。"),
    ("slang-zh", "英语", "这波操作直接给我整不会了。"),
    ("formal-zh", "英语", "根据相关规定，申请人应当在收到通知之日起十五日内提交补充材料。"),
]


def run_model(llama_dir, label, model):
    port = free_port()
    args = [f"{llama_dir}/llama-server.exe", "-m", model, "--host", "127.0.0.1",
            "--port", str(port), "-c", "4096", "--parallel", "1", "-ngl", "99"]
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    out = {}
    try:
        wait_health(port, timeout=300)
        translate(port, "英语", "你好")
        for cid, tgt, src in CASES:
            ttft, total, n, tps, text = translate(port, tgt, src)
            out[cid] = {"text": text.strip(), "ms": round(total * 1000)}
            print(f"[{label}] {cid}: {text.strip()}", flush=True)
    finally:
        proc.terminate()
        proc.wait(10)
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    llama_dir, out_json, *specs = sys.argv[1:]
    results = json.load(open(out_json, encoding="utf-8")) if os.path.exists(out_json) else {}
    for spec in specs:
        label, model = spec.split("=", 1)
        results[label] = run_model(llama_dir, label, model)
        json.dump(results, open(out_json, "w", encoding="utf-8"), ensure_ascii=False, indent=1)

    labels = list(results)
    md = ["# Translation quality comparison", ""]
    for cid, tgt, src in CASES:
        md.append(f"## {cid}  →{tgt}")
        md.append(f"> {src}")
        md.append("")
        for lb in labels:
            r = results[lb].get(cid)
            if r:
                md.append(f"- **{lb}** ({r['ms']} ms): {r['text']}")
        md.append("")
    open(os.path.splitext(out_json)[0] + ".md", "w", encoding="utf-8").write("\n".join(md))


if __name__ == "__main__":
    main()
