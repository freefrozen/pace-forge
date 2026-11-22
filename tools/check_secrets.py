#!/usr/bin/env python3
"""扫描仓库里疑似凭证与个人信息，CI 与 pre-commit 共用。

为什么需要：这个仓被 issue 讨论过「把明文 key 贴进公开仓库」的事，
而且代码注释里曾经写着具体校区和真机坐标。机器扫一遍比靠人记着便宜。

用法：
    python tools/check_secrets.py           # 扫 git 跟踪的文件
    python tools/check_secrets.py --all      # 连未跟踪也扫（发布前用）

命中返回 1，干净返回 0。
"""

import re
import subprocess
import sys
from pathlib import Path

# 只扫可能是文本的文件；二进制直接跳过（证据包里的图片、zip 不算泄露面）
SKIP_SUFFIX = {
    ".png", ".jpg", ".jpeg", ".gif", ".ico", ".zip", ".gz", ".tar", ".exe",
    ".so", ".dylib", ".a", ".o", ".rlib", ".wasm", ".pdf", ".bin",
}

PATTERNS = [
    # GitHub 老式 PAT / 细粒度 token
    (re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}"), "GitHub token"),
    # 常见云厂商 key
    (re.compile(r"AKIA[0-9A-Z]{16}"), "AWS access key"),
    (re.compile(r"sk-[A-Za-z0-9]{20,}"), "OpenAI/DeepSeek 风格 key"),
    (re.compile(r"glpat-[A-Za-z0-9_-]{20,}"), "GitLab token"),
    # 私钥块
    (re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----"), "私钥"),
    # 11 位手机号：前后都不能是数字，避免把 13 位毫秒戳拦下来
    (re.compile(r"(?<!\d)1[3-9]\d{9}(?!\d)"), "手机号"),
]

# 允许出现的地方：文档里讲「口令怎么设」的示例、测试里的假 token
ALLOW_SUBSTR = (
    "你的口令",
    "PLACEHOLDER",
    "0000000000",
    # 测试里用的假号码，不是任何人的手机号
    "13812348888",
    "13800000000",
    "13900000000",
)

# 校验和、内容哈希这类长十六进制串里会出现「像手机号」的子串，先跳过
HEX_TOKEN = re.compile(r"[0-9a-f]{24,}")


# 全量扫描时跳过的目录：依赖树、构建产物、虚拟环境都不属于「本仓的代码」
SKIP_DIRS = {
    ".git", "target", "node_modules", "vendor", "dist", "build",
    ".venv", "venv", "__pycache__", ".cargo", ".idea", ".vscode",
}


def tracked_files() -> list[Path] | None:
    """git 跟踪的文件；不在 git 仓库或没装 git 时返回 None。

    release 的 zip/tar 包里没有 `.git`，而脚本是随包发出去的；pre-commit
    也直接调它。以前这里让 `FileNotFoundError` / `CalledProcessError` 一路
    冒到顶，用户看到的是 Python 堆栈，而不是「换个目录跑」。
    """
    try:
        out = subprocess.run(
            ["git", "ls-files"], capture_output=True, text=True, check=True
        ).stdout
    except (FileNotFoundError, subprocess.CalledProcessError, OSError):
        return None
    return [Path(p) for p in out.splitlines() if p]


def walk_all() -> list[Path]:
    return [
        p
        for p in Path(".").rglob("*")
        if not any(part in SKIP_DIRS for part in p.parts)
    ]


def scan(paths: list[Path]) -> list[str]:
    hits: list[str] = []
    for p in paths:
        if p.suffix.lower() in SKIP_SUFFIX or not p.is_file():
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            if any(a in line for a in ALLOW_SUBSTR):
                continue
            if HEX_TOKEN.search(line):
                continue
            for pat, label in PATTERNS:
                m = pat.search(line)
                if m:
                    # 只报位置和类型，不把命中的原文打出来（日志本身也会被贴进 issue）
                    hits.append(f"{p}:{lineno}: {label}（{len(m.group(0))} 字符）")
                    break
    return hits


def main() -> int:
    if "--all" in sys.argv:
        paths = walk_all()
    else:
        got = tracked_files()
        if got is None:
            print("不在 git 仓库里（或没装 git），退回全量扫描；跳过的目录见 SKIP_DIRS。")
            paths = walk_all()
        else:
            paths = got
    hits = scan(paths)
    if hits:
        print("发现疑似凭证/个人信息，先处理再提交：\n")
        for h in hits:
            print("  " + h)
        print("\n确认是误报的话，改写成示例值，别留原文。")
        return 1
    print(f"扫描 {len(paths)} 个路径，未发现疑似凭证/个人信息。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
