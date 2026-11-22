#!/usr/bin/env python3
"""把 Google Play 官方设备目录裁成 data/device_catalog.json。

用法：
    curl -sO https://storage.googleapis.com/play_public/supported_devices.csv
    python3 tools/import_play_catalog.py supported_devices.csv

为什么要这一步：`data/devices.json` 里只放**有出处的 SoC 映射**（人工审核，
每条带 src）；这个脚本生成的是官方目录那一层（model code → codename / 营销名 /
品牌），用来校验手填的 model code 是不是真机存在、以及在 `devices` 里显示
「你这款机型官方叫什么」。它不含 SoC，也不允许被拿来推 SoC。

CSV 是 UTF-16LE 带 BOM，字段：Retail Branding, Marketing Name, Device, Model。
其中 `Device` 是 OEM 内部 codename，`Model` 才是真机 `ro.product.model`。
"""
import csv
import json
import sys

BRANDS = {
    "xiaomi", "redmi", "poco", "vivo", "iqoo", "oppo", "realme",
    "oneplus", "honor", "huawei", "meizu",
}


def main() -> int:
    src = sys.argv[1] if len(sys.argv) > 1 else "supported_devices.csv"
    out_path = sys.argv[2] if len(sys.argv) > 2 else "data/device_catalog.json"
    devices = {}
    with open(src, encoding="utf-16") as f:
        for row in csv.DictReader(f):
            brand = (row.get("Retail Branding") or "").strip()
            if brand.lower() not in BRANDS:
                continue
            model = (row.get("Model") or "").strip()
            device = (row.get("Device") or "").strip()
            name = (row.get("Marketing Name") or "").strip()
            if not model or not device:
                continue
            devices.setdefault(model, {"device": device, "name": name, "brand": brand})
    payload = {
        "note": "Google Play 设备目录的中国品牌子集：model code（ro.product.model）"
                " → {codename, 营销名, 品牌}。官方清单，不含 SoC，不能拿来推 SoC。",
        "source": "https://storage.googleapis.com/play_public/supported_devices.csv",
        "fetched": "2026-11-21",
        "devices": dict(sorted(devices.items())),
    }
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, separators=(",", ":"))
    print(f"{len(devices)} 条 → {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
