"""Fetch the public dependency pinned for the LabWired host."""
import json
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
TARGET = HERE.parents[1] / ".cache" / "labwired"


def git(*arguments: str) -> str:
    return subprocess.check_output(["git", "-C", str(TARGET), *arguments], text=True).strip()


if __name__ == "__main__":
    pin = json.loads((HERE / "upstream.json").read_text())
    TARGET.mkdir(parents=True, exist_ok=True)
    if not (TARGET / ".git").is_dir():
        git("init", "--quiet")
    if not (TARGET / "Cargo.toml").is_file():
        git("fetch", "--quiet", "--depth", "1", pin["repository"], pin["revision"])
        git("checkout", "--quiet", "--detach", "FETCH_HEAD")
    if git("rev-parse", "HEAD") != pin["revision"] or git("status", "--porcelain", "-uno"):
        raise SystemExit(f"Expected a clean LabWired checkout at {pin['revision']}: {TARGET}")
    print(TARGET)
