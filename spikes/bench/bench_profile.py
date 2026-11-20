"""M1 check: load time, latency, speed, VRAM and thinking-mode leakage per promptProfile.

Usage: python -I bench_profile.py <llama_dir> <model.gguf> <profile> <label> [extra server args...]
profile: index-translate | hy-mt
"""
import json
import socket
import subprocess
import sys
import time
import urllib.request

PROFILES = {
    "index-translate": {
        "tmpl": "请将以下文本翻译为{tgt}，直接输出翻译结果，不要进行任何解释。\n\n{src}",
        "params": {"temperature": 0, "chat_template_kwargs": {"enable_thinking": False}},
    },
    "hy-mt": {
        "tmpl": "将以下文本翻译为{tgt}，注意只需要输出翻译后的结果，不要额外解释：\n\n{src}",
        "params": {"top_k": 20, "top_p": 0.6, "repeat_penalty": 1.05, "temperature": 0.7},
    },
}

CASES = [
    ("short-zh2en", "英语", "这个功能在终端里也能正常使用吗？"),
    ("short-en2zh", "中文", "Does this feature also work properly in the terminal?"),
    ("slang-zh2en", "英语", "这波操作太秀了，直接给我整不会了。"),
    ("para-en2zh", "中文",
     "Large language models have changed how software is built. Instead of writing explicit rules, "
     "developers now describe the desired behavior in natural language and let the model infer the details. "
     "This shift brings new challenges in testing, latency, and cost, especially when models run locally "
     "on consumer hardware with limited memory."),
    ("para-zh2en", "英语",
     "本地部署的翻译模型不依赖网络，响应速度稳定，也不会把用户的数据上传到云端。"
     "但它对硬件有一定要求，需要在模型大小、翻译质量和推理速度之间做出权衡。"
     "对于日常的划词和截图翻译场景，一个经过量化的小模型通常已经足够。"),
]


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def vram_used_mb():
    try:
        out = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], text=True)
        return int(out.strip().splitlines()[0])
    except Exception:
        return -1


def wait_health(port, proc, timeout=180):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"server exited with {proc.returncode}")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as r:
                if r.status == 200:
                    return time.time() - t0
        except Exception:
            pass
        time.sleep(0.05)
    raise RuntimeError("server not healthy")


def translate(port, profile, tgt, src):
    p = PROFILES[profile]
    body = {
        "messages": [{"role": "user", "content": p["tmpl"].format(tgt=tgt, src=src)}],
        "stream": True, "max_tokens": 1024, **p["params"],
    }
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    t0 = time.time()
    ttft = None
    out, reasoning = [], []
    n = 0
    with urllib.request.urlopen(req, timeout=300) as r:
        for raw in r:
            line = raw.decode("utf-8").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                break
            delta = json.loads(data)["choices"][0]["delta"]
            if delta.get("reasoning_content"):
                reasoning.append(delta["reasoning_content"])
            c = delta.get("content")
            if c:
                if ttft is None:
                    ttft = time.time() - t0
                out.append(c)
                n += 1
    total = time.time() - t0
    gen = total - (ttft or 0)
    tps = (n - 1) / gen if n > 1 and gen > 0 else 0
    return ttft or 0, total, n, tps, "".join(out), "".join(reasoning)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    llama_dir, model, profile, label, *extra = sys.argv[1:]
    port = free_port()
    base_vram = vram_used_mb()
    args = [f"{llama_dir}/llama-server.exe", "-m", model, "--host", "127.0.0.1",
            "--port", str(port), "-c", "4096", "--parallel", "1", *extra]
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=open(f"{label}.server.log", "w"))
    try:
        load = wait_health(port, proc)
        translate(port, profile, "英语", "你好")  # warm-up
        vram = vram_used_mb()
        print(f"[{label}] load={load:.2f}s vram_delta={vram - base_vram}MB args={' '.join(extra)}")
        for name, tgt, src in CASES:
            ttft, total, n, tps, text, reasoning = translate(port, profile, tgt, src)
            think = "think" in text.lower() or bool(reasoning)
            print(f"  {name:12s} ttft={ttft*1000:6.0f}ms total={total*1000:6.0f}ms tokens={n:4d} "
                  f"{tps:6.1f} tok/s thinking_leak={think}")
            print(f"    -> {text!r}")
            if reasoning:
                print(f"    reasoning: {reasoning[:200]!r}")
    finally:
        proc.terminate()
        proc.wait(10)


if __name__ == "__main__":
    main()
