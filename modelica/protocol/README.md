# Modelica execution protocol 1

Run42 and a Modelica execution host exchange one UTF-8 JSON object per line
over stdin and stdout. Requests carry a `command` field and replies an
`event` field. Send one request at a time. Diagnostics outside the protocol
belong on stderr.

The host owns the compiled plant, its internal solver steps, and its initial
conditions. Run42 owns the communication schedule, firmware coupling, and
verification verdicts. The runtime protocol exchanges named scalar values
and simulation time in seconds.

## Start a session

The host initializes a single model before sending `hello`. The hello contains
`protocol_version: 1`, the starting `time`, a list of `capabilities`, and an
optional `engine` object for backend provenance. A host that cannot compile
the model sends a `compilation_failure` event instead of `hello` and exits.

```json
{"event":"compilation_failure","failure":{"summary":"...","diagnostics":[{"code":"ER002","severity":"Error","model":"Broken","phase":"Resolve","message":"...","labels":[{"file":"Broken.mo","byte_start":61,"byte_end":68,"line":4,"column":12,"primary":true,"message":null}],"notes":[]}]}}
```

`compile --emit flat-json` prints the same event on stdout before exiting
unsuccessfully. A client treats a host that sends no such event as reporting
the failure only as text on stderr. A hello without
`communication-step` uses the separate commands below. Older Rumoca hosts
also omit the starting time and the capability list.

## Advance and sample

A host advertising `communication-step` accepts a complete input frame, an
absolute target time, and the names of the requested outputs:

```json
{"command":"communication_step","time":0.0001,"inputs":[["u",1.0]],"outputs":["y"]}
```

It applies the input frame, advances the plant, and samples the requested
variables at one boundary. Input and output lists can be empty. The reply
reports the actual time reached and values in request order. It uses
`null` for missing or non-finite values.

```json
{"event":"values","time":0.0001,"values":[0.0002]}
```

For example, a host that stops early reports the time it reached. Run42
currently rejects a boundary mismatch before advancing the firmware.
An unsuccessful combined step returns an `error` with a `code` and
`message`. The client must discard that session because the input frame
may already have taken effect.

## Session commands

| Command | Fields | Reply |
| --- | --- | --- |
| `set_inputs` | `inputs`: ordered array of name/value pairs | `ok` with current time |
| `advance_to` | `time`: absolute target in seconds | `ok` with time reached |
| `values` | `names`: ordered array of variable names | `values` with time and ordered samples |
| `state` | None | `state` with time and a name-to-value map |
| `close` | None | `closed` with time, then process exit |

The Rumoca provider also implements the existing Rumoca session commands
used by Run42. Model loading and compiler metadata are separate from the
runtime messages. The current Run42 provider uses Rumoca-compatible
`compile --emit flat-json` for class inspection and `sim --serve-stdio`
for startup. A second provider can reuse the runtime messages with its
own compiler and metadata adapter.

OpenModelica is a candidate provider. Its compiler can export a Modelica
plant as an FMI 2.0 Co-Simulation FMU. A host could use the FMU's variables
for the input/output exchange and its FMI step function for advancement.
[OpenModelica FMI export](https://openmodelica.org/doc/OpenModelicaUsersGuide/v1.27.1/fmitlm.html)
