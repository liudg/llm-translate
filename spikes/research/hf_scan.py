"""Scan Hugging Face for translation models from several angles. Prints a de-duplicated table."""
import json
import sys
import urllib.parse
import urllib.request

PROXY = {"https": "http://127.0.0.1:7897"}
opener = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY))
API = "https://huggingface.co/api/models?"


def q(**params):
    url = API + urllib.parse.urlencode(params)
    with opener.open(url, timeout=60) as r:
        return json.load(r)


def row(m):
    return (f"{m.get('likes', 0):6d} likes {m.get('downloads', 0):9d} dl "
            f"{(m.get('createdAt') or '')[:10]} {(m.get('pipeline_tag') or ''):16s} {m['id']}")


def show(title, items, n=40):
    print(f"\n### {title}")
    for m in items[:n]:
        print(row(m))


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    show("pipeline_tag=translation, by likes", q(pipeline_tag="translation", sort="likes", limit=40))
    show("pipeline_tag=translation, by trending", q(pipeline_tag="translation", sort="trendingScore", limit=40))
    recent = [m for m in q(pipeline_tag="translation", sort="likes", limit=1000)
              if (m.get("createdAt") or "") >= "2025-06"]
    show("pipeline_tag=translation, created since 2025-06, by likes", recent)

    # LLM-based MT models are often tagged text-generation; search names instead
    seen = {}
    for kw in ["translat", "-MT", "MT-", "nmt", "seamless", "madlad", "nllb"]:
        for m in q(search=kw, sort="likes", limit=200):
            seen[m["id"]] = m
    llm = [m for m in seen.values() if (m.get("createdAt") or "") >= "2025-01"
           and "gguf" not in m["id"].lower() and m.get("likes", 0) >= 15]
    llm.sort(key=lambda m: -m.get("likes", 0))
    show("name contains translat/MT/nmt..., created since 2025, likes>=15, non-GGUF", llm, 80)


if __name__ == "__main__":
    main()
