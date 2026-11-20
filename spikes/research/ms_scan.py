"""Scan ModelScope for translation models (keyword search, multiple sort orders)."""
import json
import sys
import urllib.request

API = "https://www.modelscope.cn/api/v1/dolphin/models"


def search(name, sort="Default", size=50):
    body = {"PageSize": size, "PageNumber": 1, "SortBy": sort, "Target": "",
            "SingleCriterion": [], "Name": name}
    req = urllib.request.Request(API, data=json.dumps(body).encode(), method="PUT",
                                 headers={"Content-Type": "application/json",
                                          "User-Agent": "Mozilla/5.0"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # direct: CN site
    with opener.open(req, timeout=60) as r:
        d = json.load(r)
    return (d.get("Data") or {}).get("Model", {}).get("Models") or []


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    seen = {}
    for kw in ["翻译", "translate", "translation", "-MT", "MT-"]:
        for sort in ["Default", "DownloadsCount", "StarsCount", "GmtModified"]:
            try:
                for m in search(kw, sort):
                    mid = f"{m.get('Path')}/{m.get('Name')}"
                    seen[mid] = m
            except Exception as e:  # noqa: BLE001
                print("ERR", kw, sort, e)
    import datetime
    def created(m):
        ts = m.get("CreatedTime") or 0
        return datetime.date.fromtimestamp(int(ts)).isoformat() if ts else ""
    rows = [m for m in seen.values() if created(m) >= "2025-06"]
    rows.sort(key=lambda m: -(m.get("Downloads") or 0))
    print(f"{len(seen)} models total, {len(rows)} created since 2025-06")
    for m in rows[:100]:
        print(f"{m.get('Downloads') or 0:9d} dl {m.get('Stars') or 0:5d} star {created(m)} "
              f"{m.get('Path')}/{m.get('Name')}  | {(m.get('ChineseName') or '')[:40]}")


if __name__ == "__main__":
    main()
