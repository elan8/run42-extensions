"""Install the pinned Renode runtime in this repository's ignored cache."""
import hashlib
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

VERSION = "1.17.0"
ASSETS = {
    "Linux": ("linux-portable.tar.gz", "4ba7c68b59e2447f188ef4b4b112fcccd2802582c460beedceb63b84e5605a5f", "renode"),
    "Windows": ("windows-portable.zip", "18bcf145422039e8702c3e5f9864c7787561bfd5deb7a3ee6e168ecf19e2b9db", "renode.exe"),
}
CACHE = Path(__file__).resolve().parents[2] / ".cache"


def executable(root: Path, name: str) -> Path:
    candidates = [path for path in root.rglob(name) if path.is_file() and len(path.relative_to(root).parts) <= 2]
    if len(candidates) != 1:
        raise ValueError(f"Expected one Renode executable under {root}, found {len(candidates)}")
    return candidates[0]


def install() -> Path:
    system = platform.system()
    if system not in ASSETS or platform.machine().lower() not in {"amd64", "x86_64"}:
        raise SystemExit("Install Renode 1.17 separately and set RUN42_RENODE for this platform")
    suffix, expected, program = ASSETS[system]
    name = f"renode-{VERSION}.{suffix}"
    destination = CACHE / f"renode-{VERSION}-{system.lower()}-x64"
    CACHE.mkdir(parents=True, exist_ok=True)
    if not destination.is_dir():
        with tempfile.TemporaryDirectory(dir=CACHE) as tempdir:
            archive = Path(tempdir) / name
            url = f"https://github.com/renode/renode/releases/download/v{VERSION}/{name}"
            with urllib.request.urlopen(url, timeout=60) as download, archive.open("wb") as output:
                shutil.copyfileobj(download, output)
            with archive.open("rb") as source:
                if hashlib.file_digest(source, "sha256").hexdigest() != expected:
                    raise ValueError(f"Renode archive failed SHA-256 verification: {name}")
            unpacked = Path(tempdir) / "unpacked"
            if suffix.endswith(".zip"):
                with zipfile.ZipFile(archive) as package:
                    package.extractall(unpacked)
            else:
                with tarfile.open(archive) as package:
                    package.extractall(unpacked, filter="data")
            check_version(executable(unpacked, program))
            try:
                unpacked.rename(destination)
            except OSError:
                if not destination.is_dir():
                    raise
    runtime = executable(destination, program)
    check_version(runtime)
    return runtime


def check_version(runtime: Path) -> None:
    version = subprocess.check_output([runtime, "--version"], timeout=30, text=True)
    if f"Renode v{VERSION}" not in version:
        raise ValueError(f"Unexpected Renode version at {runtime}: {version.strip()}")


if __name__ == "__main__":
    print(install())
