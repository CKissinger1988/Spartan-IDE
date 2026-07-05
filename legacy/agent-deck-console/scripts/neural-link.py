#!/usr/bin/env python3
"""Safe local Neural Link for Jarvis/BrainBridge analysis packages.

This bridge intentionally avoids starting Jarvis, network reconnaissance,
credential collection, or autonomous assimilation loops. It creates local
analysis records that can be reviewed and, where dependencies allow, fed into
BrainBridge as ordinary local knowledge.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = ROOT / "config" / "neural-link.json"

TEXT_SUFFIXES = {
    ".py", ".ts", ".tsx", ".js", ".jsx", ".json", ".md", ".txt", ".toml",
    ".yaml", ".yml", ".css", ".html", ".sh", ".env.example", ".tsv"
}
SKIP_DIRS = {".git", "node_modules", "dist", "__pycache__", ".venv", "venv"}


def load_config() -> dict:
    return json.loads(CONFIG_PATH.read_text(encoding="utf-8"))


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def is_allowed(path: Path, config: dict) -> bool:
    resolved = path.resolve()
    for root in config["allowed_roots"]:
        try:
            resolved.relative_to(Path(root).resolve())
            return True
        except ValueError:
            continue
    return False


def iter_files(root: Path):
    for current, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            path = Path(current) / name
            if path.suffix.lower() in TEXT_SUFFIXES or name.endswith(".env.example"):
                yield path


def file_digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def analyze_path(target: Path, config: dict) -> dict:
    if not target.exists():
        raise SystemExit(f"Target does not exist: {target}")
    if not is_allowed(target, config):
        raise SystemExit(f"Target is outside allowed Neural Link roots: {target}")

    files = [target] if target.is_file() else list(iter_files(target))
    suffix_counts: dict[str, int] = {}
    keyword_counts = {
        "jarvis": 0,
        "brainbridge": 0,
        "brain_bridge": 0,
        "assimilation": 0,
        "analysis": 0,
        "mcp": 0,
        "agent-deck": 0,
        "skill": 0,
    }
    samples = []
    total_bytes = 0

    for path in files:
        try:
            size = path.stat().st_size
            total_bytes += size
            suffix = path.suffix.lower() or "<none>"
            suffix_counts[suffix] = suffix_counts.get(suffix, 0) + 1
            if len(samples) < 50:
                samples.append({
                    "path": str(path),
                    "bytes": size,
                    "sha256": file_digest(path),
                })
            text = path.read_text(encoding="utf-8", errors="ignore").lower()
            for key in keyword_counts:
                keyword_counts[key] += text.count(key)
        except OSError:
            continue

    return {
        "target": str(target),
        "analyzed_at": utc_now(),
        "file_count": len(files),
        "total_bytes": total_bytes,
        "suffix_counts": dict(sorted(suffix_counts.items())),
        "keyword_counts": keyword_counts,
        "sample_files": samples,
    }


def write_report(analysis: dict, config: dict) -> tuple[Path, Path]:
    reports_dir = ROOT / config["reports_dir"]
    reports_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    slug = hashlib.sha1(analysis["target"].encode("utf-8")).hexdigest()[:10]
    json_path = reports_dir / f"{stamp}-{slug}.json"
    md_path = reports_dir / f"{stamp}-{slug}.md"
    json_path.write_text(json.dumps(analysis, indent=2), encoding="utf-8")

    top_suffixes = ", ".join(f"{k}:{v}" for k, v in sorted(analysis["suffix_counts"].items(), key=lambda item: item[1], reverse=True)[:10])
    top_keywords = ", ".join(f"{k}:{v}" for k, v in analysis["keyword_counts"].items() if v)
    md_path.write_text(
        "\n".join([
            "# Neural Link Analysis",
            "",
            f"- Target: `{analysis['target']}`",
            f"- Analyzed: `{analysis['analyzed_at']}`",
            f"- Files: `{analysis['file_count']}`",
            f"- Bytes: `{analysis['total_bytes']}`",
            f"- Top suffixes: `{top_suffixes or 'none'}`",
            f"- Signal keywords: `{top_keywords or 'none'}`",
            "",
            "## Sample Files",
            *[f"- `{item['path']}` ({item['bytes']} bytes)" for item in analysis["sample_files"][:25]],
            "",
        ]),
        encoding="utf-8",
    )
    return json_path, md_path


def queue_for_brainbridge(analysis: dict, report_json: Path, report_md: Path, config: dict) -> Path:
    queue_path = ROOT / config["queue_path"]
    queue_path.parent.mkdir(parents=True, exist_ok=True)
    record = {
        "queued_at": utc_now(),
        "source": "Spartan_ide_neural_link",
        "type": "local_analysis",
        "target": analysis["target"],
        "report_json": str(report_json),
        "report_md": str(report_md),
        "metadata": {
            "file_count": analysis["file_count"],
            "total_bytes": analysis["total_bytes"],
            "disabled_capabilities": config["disabled_capabilities"],
        },
    }
    with queue_path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(record) + "\n")
    return queue_path


def status(config: dict) -> None:
    hub = Path(config["hub_root"])
    checks = {
        "hub_root": hub,
        "jarvis": hub / config["jarvis_module"],
        "brain_bridge": hub / config["brain_bridge_module"],
        "ai_assimilation": hub / config["ai_assimilation_module"],
        "queue": ROOT / config["queue_path"],
    }
    print("Spartan Neural Link")
    for name, path in checks.items():
        state = "ONLINE" if path.exists() else "MISSING"
        print(f"  {name:16} {state:8} {path}")
    print("  mode             LOCAL_DEFENSIVE_ANALYSIS")
    print("  disabled         " + ", ".join(config["disabled_capabilities"]))


def try_feed(config: dict, limit: int) -> None:
    queue_path = ROOT / config["queue_path"]
    if not queue_path.exists():
        print("No BrainBridge queue found.")
        return
    hub_root = Path(config["hub_root"])
    sys.path.insert(0, str(hub_root))
    try:
        from backend.core.brain_bridge import BrainBridge  # type: ignore
    except Exception as exc:
        print(f"BrainBridge import unavailable: {type(exc).__name__}: {exc}")
        print(f"Queued records remain at: {queue_path}")
        return

    brain = BrainBridge(db_path=str(hub_root / "vector_db"))
    processed = 0
    for line in queue_path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or processed >= limit:
            continue
        record = json.loads(line)
        report_md = Path(record["report_md"])
        if not report_md.exists():
            continue
        content = report_md.read_text(encoding="utf-8")
        ok = brain.feed_brain(content, record.get("metadata", {}))
        print(f"{'FED' if ok else 'FAILED'} {report_md}")
        processed += 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Safe local Neural Link for Jarvis/BrainBridge")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status")
    analyze = sub.add_parser("analyze")
    analyze.add_argument("target", nargs="?", default=str(ROOT))
    feed = sub.add_parser("feed")
    feed.add_argument("--limit", type=int, default=10)
    args = parser.parse_args()
    config = load_config()

    if args.cmd == "status":
        status(config)
    elif args.cmd == "analyze":
        analysis = analyze_path(Path(args.target), config)
        report_json, report_md = write_report(analysis, config)
        queue_path = queue_for_brainbridge(analysis, report_json, report_md, config)
        print(f"Analysis written: {report_md}")
        print(f"BrainBridge queue: {queue_path}")
    elif args.cmd == "feed":
        try_feed(config, args.limit)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
