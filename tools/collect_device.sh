#!/usr/bin/env bash
# 从一台真机采集 model code + 平台代号，输出可直接粘进 data/devices.json 的条目。
#
# 用法：adb 连上手机后跑
#     bash tools/collect_device.sh
#
# 只读两个属性，不改手机任何东西。把输出的那一行贴进 data/devices.json 的
# "models" 里，并把 src 换成你的出处（截图/日期/getprop 输出），因为
# models 层只收有出处的条目（见 api/device_table.rs 顶部说明）。
set -euo pipefail

model=$(adb shell getprop ro.product.model | tr -d '\r')
platform=$(adb shell getprop ro.board.platform | tr -d '\r')
brand=$(adb shell getprop ro.product.brand | tr -d '\r')
soc=$(adb shell getprop ro.soc.model 2>/dev/null | tr -d '\r' || true)

if [ -z "$model" ] || [ -z "$platform" ]; then
  echo "读不到 ro.product.model / ro.board.platform：先确认 adb devices 里能看到这台机" >&2
  exit 1
fi

cat <<JSON
  "$model": {"platform": "$platform", "brand": "$brand", "src": "getprop $(date +%Y-%m-%d)"}
JSON

echo >&2
echo "自检：平台代号 \"$platform\" 要在 data/devices.json 的 platforms 里；" >&2
echo "      soc 属性（ro.soc.model）读出的是：${soc:-（这台机没有这个属性）}" >&2
echo "      核对：pace-forge devices" >&2
