# MCU emulation

The LabWired and Renode hosts implement [protocol version 2](protocol/README.md).
Run42 selects a host and sends it the firmware and board configuration. The host
owns MCU execution. Run42 owns the co-simulation and verification verdicts.

## Build and test

Use Rust 1.100, Python 3.12 or newer, and Git. These commands are relative to the
repository root.

```sh
python3 mcu-emulation/labwired/fetch.py
cargo build --release --locked --manifest-path mcu-emulation/labwired/Cargo.toml
cargo build --release --locked --manifest-path mcu-emulation/renode/Cargo.toml
rustup target add thumbv7em-none-eabihf
python3 mcu-emulation/renode/install.py
```

Set `RUN42_RENODE` to the printed executable path, or to an existing Renode 1.17
installation. The optional installer verifies and caches the official portable
release on Linux x64 and Windows x64. Run the shared STM32F407 conformance probe
with `python3 mcu-emulation/conformance/test_hosts.py`.

LabWired Core is obtained at the revision in `labwired/upstream.json`. Its MIT
license is available in that checkout. Renode is separately distributed by its
upstream project and retains the notices in its portable archive.
