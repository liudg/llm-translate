"""M0 spike: measure llama-server load time, TTFT and tokens/s for HY-MT1.5.

Usage: python -I bench_llama.py <llama_dir> <model.gguf> <label> [extra server args...]
"""
import json
import socket
import subprocess
import sys
import time
import urllib.request

PROMPT_TMPL = "将以下文本翻译为{tgt}，注意只需要输出翻译后的结果，不要额外解释：\n\n{src}"

CASES = [
    ("short-zh2en", "英语", "这个功能在终端里也能正常使用吗？"),
    ("short-en2zh", "中文", "Does this feature also work properly in the terminal?"),
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


def wait_health(port, timeout=120):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as r:
                if r.status == 200:
                    return time.time() - t0
        except Exception:
            pass
        time.sleep(0.05)
    raise RuntimeError("server not healthy")


def translate(port, tgt, src):
    body = {
        "messages": [{"role": "user", "content": PROMPT_TMPL.format(tgt=tgt, src=src)}],
        "stream": True,
        "top_k": 20, "top_p": 0.6, "repeat_penalty": 1.05, "temperature": 0.7,
        "max_tokens": 1024,
    }
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    t0 = time.time()
    ttft = None
    out = []
    n = 0
    with urllib.request.urlopen(req, timeout=120) as r:
        for raw in r:
            line = raw.decode("utf-8").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                break
            chunk = json.loads(data)
            delta = chunk["choices"][0]["delta"].get("content")
            if delta:
                if ttft is None:
                    ttft = time.time() - t0
                out.append(delta)
                n += 1
    total = time.time() - t0
    gen = total - (ttft or 0)
    tps = (n - 1) / gen if n > 1 and gen > 0 else 0
    return ttft, total, n, tps, "".join(out)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    llama_dir, model, label, *extra = sys.argv[1:]
    port = free_port()
    args = [f"{llama_dir}/llama-server.exe", "-m", model, "--host", "127.0.0.1",
            "--port", str(port), "-c", "4096", "--parallel", "1", *extra]
    t0 = time.time()
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        load = wait_health(port)
        print(f"[{label}] load={load:.2f}s args={' '.join(extra)}")
        translate(port, "英语", "你好")  # warm-up
        for name, tgt, src in CASES:
            ttft, total, n, tps, text = translate(port, tgt, src)
            print(f"  {name:12s} ttft={ttft*1000:6.0f}ms total={total*1000:6.0f}ms tokens={n:4d} {tps:6.1f} tok/s")
            print(f"    -> {text}")
    finally:
        proc.terminate()
        proc.wait(10)
    _ = t0


if __name__ == "__main__":
    main()
