# MCU host protocol 2

Communication is UTF-8 JSON Lines over the host's standard input and output.
Send one request and read its reply before issuing another. Successful replies
have `"ok": true`. Failures have `"ok": false` and an `"error"` string.
After a failed step the client should discard the session. Diagnostic output
outside the protocol belongs on standard error.

| Request | Fields | Successful reply |
| --- | --- | --- |
| `open` | `image`: ELF path, `system`: board configuration path, optional LabWired `tick_interval` | `protocol`: 2, `version`: emulator version, `resources`: additional input paths, optional `capabilities` |
| `symbol` | `name`: ELF symbol | `address`: unsigned integer |
| `step` | `ns`: unsigned duration in nanoseconds, `gpio`: array of [pad, boolean] pairs, `pads`: array of [pad, millivolts] pairs, `read`: array of unsigned word addresses, `read_gpio`: array of pads | `time_ns`: absolute virtual time since open, `values`: unsigned 32-bit words, `levels`: booleans |
| `close` | None | Host sends its reply and exits |

Each request has a `cmd` field containing the request name. Send empty arrays
when an input or observation list is unused. A step applies the provided inputs,
advances the virtual clock, and then returns observations in request order. A
zero-duration step can sample the initial state. LabWired quantizes elapsed time
to whole emulator cycles. Renode measures elapsed time through External Control.

When present, `capabilities` is an array of strings. Both hosts advertise
`digital-gpio`, `read-word`, and `elf-symbols`. LabWired also advertises
`analog-pad-input`. Pin names and valid addresses depend on the board. LabWired
takes a system YAML and Renode takes a platform description. Both reject
unsupported pins or bindings. The same request and reply shape works for clients
that do not read the optional capability field.
