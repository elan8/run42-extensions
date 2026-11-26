"""Run the same real firmware and GPIO requests through both protocol hosts."""
import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CATEGORY = HERE.parent
ROOT = CATEGORY.parent
GPIOD_ODR = 0x40020C14


def run_host(backend: str, image: Path) -> None:
    labwired = backend == "labwired"
    name = "run42-elf-host" if labwired else "run42-renode-host"
    suffix = ".exe" if os.name == "nt" else ""
    host = CATEGORY / backend / "target" / "release" / (name + suffix)
    system = ROOT / ".cache/labwired/configs/systems/nucleo-f407.yaml" if labwired else HERE / "f407.repl"
    open_request = {"cmd": "open", "image": str(image), "system": str(system)}
    if labwired:
        open_request["tick_interval"] = 16
    levels = [False, True, True, False, True, False]
    steps = [
        {"cmd": "step", "ns": 1_000_000, "gpio": [["PA0", level]], "pads": [],
         "read": [GPIOD_ODR], "read_gpio": ["PD12"]}
        for level in levels
    ]
    unsupported_pin = {**steps[0], "gpio": [["PZ0", False]]}
    commands = [open_request, {"cmd": "symbol", "name": "Reset"}, *steps, unsupported_pin, {"cmd": "close"}]
    payload = "".join(json.dumps(command) + "\n" for command in commands)
    completed = subprocess.run(
        [host], input=payload, stdout=subprocess.PIPE, check=True, text=True, timeout=60
    )
    replies = [json.loads(line) for line in completed.stdout.splitlines()]
    assert len(replies) == len(commands), replies
    opened, symbol, *results, invalid, closed = replies
    assert opened["ok"] and opened["protocol"] == 2 and opened["version"], opened
    expected = {"digital-gpio", "read-word", "elf-symbols"}
    if labwired:
        expected.add("analog-pad-input")
    assert set(opened["capabilities"]) == expected, opened
    assert symbol["ok"] and 0x08000000 <= symbol["address"] < 0x08100000, symbol
    for tick, (result, level) in enumerate(zip(results, levels, strict=True), start=1):
        assert result["ok"] and result["time_ns"] == tick * 1_000_000, result
        assert result["levels"] == [level] and result["values"] == [int(level) << 12], result
    assert not invalid["ok"] and "PZ0" in invalid["error"], invalid
    assert closed["ok"], closed
    print(f"{backend}: shared ELF, six GPIO and clock checkpoints, invalid pin rejection")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=("labwired", "renode"))
    selected = parser.parse_args().backend
    with tempfile.TemporaryDirectory() as directory:
        elf = Path(directory) / "probe.elf"
        subprocess.run(
            ["rustc", "--target", "thumbv7em-none-eabihf", "--edition", "2021",
             "-C", "opt-level=2", "-C", "panic=abort", "-C", f"link-arg=-T{HERE / 'probe.ld'}",
             "-o", str(elf), str(HERE / "probe.rs")],
            check=True,
        )
        for backend in (selected,) if selected else ("labwired", "renode"):
            run_host(backend, elf)
