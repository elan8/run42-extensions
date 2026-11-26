"""Compare separate commands with a combined step on the same Rumoca host."""
import argparse
import json
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

OUTPUTS = [f"y{i}" for i in range(1, 7)]
STEP_SECONDS = 0.0001
MODEL = (
    "model Bench\n  input Real u;\n"
    + "".join(f"  Real {name}(start=0, fixed=true);\n" for name in OUTPUTS)
    + "equation\n"
    + "".join(f"  der({name}) = -{i}*{name} + u;\n" for i, name in enumerate(OUTPUTS, 1))
    + "end Bench;\n"
)


def measure(exe: Path, model: Path, steps: int, combined: bool):
    with subprocess.Popen(
        [exe, "sim", model, "--model", "Bench", "--serve-stdio", "--input", "u=0"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    ) as process:
        def reply():
            result = json.loads(process.stdout.readline())
            if result["event"] == "error":
                raise RuntimeError(result)
            return result

        def send(command, **fields):
            process.stdin.write(json.dumps({"command": command, **fields}) + "\n")
            process.stdin.flush()
            return reply()

        hello = reply()
        if hello["protocol_version"] != 1 or "communication-step" not in hello["capabilities"]:
            raise ValueError("Host does not advertise the communication-step capability")
        samples = []
        start = time.perf_counter()
        for tick in range(1, steps + 1):
            inputs = [["u", float(tick % 2)]]
            target = tick * STEP_SECONDS
            if combined:
                sample = send("communication_step", time=target, inputs=inputs, outputs=OUTPUTS)
            else:
                send("set_inputs", inputs=inputs)
                send("advance_to", time=target)
                sample = send("values", names=OUTPUTS)
            if sample["event"] != "values" or abs(sample["time"] - target) > 1e-12:
                raise ValueError(f"Invalid sample for {target}: {sample}")
            samples.append(sample["values"])
        elapsed = time.perf_counter() - start
        if send("close")["event"] != "closed" or process.wait(timeout=10):
            raise RuntimeError("Rumoca host failed to close")
    return elapsed, samples


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", type=Path)
    parser.add_argument("--steps", type=int, default=3000)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    if args.steps < 1 or args.repeats < 1:
        parser.error("steps and repeats must be positive")
    timings = {False: [], True: []}
    with tempfile.TemporaryDirectory() as directory:
        model = Path(directory) / "Bench.mo"
        model.write_text(MODEL, encoding="utf-8")
        for trial in range(args.repeats):
            # Alternate order to reduce the influence of warm caches.
            results = {
                mode: measure(args.host.resolve(), model, args.steps, mode)
                for mode in (bool(trial % 2), not bool(trial % 2))
            }
            if results[False][1] != results[True][1]:
                raise ValueError("The separate and combined commands produced different samples")
            for mode in results:
                timings[mode].append(results[mode][0])
    separate, combined = (statistics.median(timings[mode]) for mode in (False, True))
    print(f"{args.steps} steps, {args.repeats} trials, six outputs, all samples identical")
    print(f"Separate: {separate / args.steps * 1e6:.1f} us per step")
    print(f"Combined: {combined / args.steps * 1e6:.1f} us per step ({separate / combined:.2f}x faster)")
