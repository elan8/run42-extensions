# Rumoca process host

This standalone host links the pinned Rumoca 0.10.4 compiler and solver. It
builds with public dependencies and speaks the Rumoca protocol over stdio.

```sh
cargo +1.100.0 build --release --locked --manifest-path modelica/rumoca/Cargo.toml
python3 modelica/rumoca/benchmark.py modelica/rumoca/target/release/run42-rumoca-host
```

Use the `.exe` suffix on Windows. Point `RUMOCA` at the host executable to
select it in Run42. Compilation failures arrive as structured diagnostics with
codes and source labels.

The host implements the [Modelica execution protocol](../protocol/README.md),
including the optional `communication-step` capability.

The benchmark compares the same executable and model, using separate commands
and the combined command. It measures wall time after startup and checks all
sampled outputs for equality. It does not include firmware emulation.

For full Run42 comparisons with the same host, set
`RUN42_RUMOCA_SEPARATE_STEPS=1` to omit the capability and exercise the
three-command path.
