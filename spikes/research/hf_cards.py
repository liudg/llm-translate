"""Fetch model card metadata + README excerpts for candidate translation models."""
import json
import re
import sys
import urllib.request

PROXY = {"https": "http://127.0.0.1:7897"}
opener = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY))

KEYWORDS = re.compile(
    r"(chinese|中文|zh|languages?|benchmark|flores|wmt|comet|bleu|gguf|llama\.cpp|prompt|"
    r"template|base model|finetuned|parameters|license|sota|outperform|超越|评测|翻译)", re.I)


def get(url):
    with opener.open(url, timeout=60) as r:
        return r.read().decode("utf-8", "replace")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    for repo in sys.argv[1:]:
        print(f"\n{'=' * 100}\n{repo}")
        try:
            meta = json.loads(get(f"https://huggingface.co/api/models/{repo}"))
            card = meta.get("cardData") or {}
            files = [s["rfilename"] for s in meta.get("siblings", [])]
            print(f"created={meta.get('createdAt', '')[:10]} likes={meta.get('likes')} "
                  f"base={card.get('base_model')} lang={card.get('language')} license={card.get('license')}")
            print("files:", [f for f in files if f.endswith((".gguf", ".safetensors", ".json"))][:12])
            readme = get(f"https://huggingface.co/{repo}/raw/main/README.md")
            lines = [l.strip() for l in readme.splitlines() if l.strip()]
            hits = [l for l in lines if KEYWORDS.search(l) and len(l) < 400]
            for l in hits[:45]:
                print("  |", l)
        except Exception as e:  # noqa: BLE001
            print("  ERROR", e)


if __name__ == "__main__":
    main()
