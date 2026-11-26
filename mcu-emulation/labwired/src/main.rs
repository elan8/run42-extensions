//! Firmware host for Run42's `elf` guest.
//!
//! Protocol: one JSON object per line on stdin, one reply per line on stdout. Every reply has
//! `"ok": true` or `"ok": false` with an `"error"` text. The commands:
//!
//! * `{"cmd":"open","image":PATH,"system":PATH,"tick_interval":N}` loads the firmware ELF onto the board
//!   a LabWired system manifest describes. Reply: `cpu_hz`.
//! * `{"cmd":"symbol","name":NAME}` resolves a firmware symbol. Reply: `address`.
//! * `{"cmd":"step","ns":N,"pads":[[PAD,MILLIVOLTS],...],"gpio":[[PAD,LEVEL],...],"read":[ADDRESS,...],"read_gpio":[PAD,...]}`
//!   applies the inputs, advances the firmware by N nanoseconds, and samples the words and GPIO levels.
//!   Reply: `values`, `levels`, `time_ns`, `cycles`.
//! * `{"cmd":"close"}` ends the session.
//!
//! Simulated time is exact: the cycle count after a step is the total nanoseconds times the clock rate,
//! rounded down, so many short steps do not drift from one long one.
use anyhow::{anyhow, bail, Context, Result};
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::machine::AdvanceRequest;
use labwired_core::{Bus, Machine};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

type Board = Machine<labwired_core::cpu::cortex_m::CortexM>;

struct Session {
    machine: Board,
    image: Vec<u8>,
    hz: u128,
    nanoseconds: u128,
    chip_path: PathBuf,
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    request[key]
        .as_str()
        .ok_or_else(|| anyhow!("`{key}` must be a string"))
}

fn open(request: &Value) -> Result<Session> {
    let image_path = PathBuf::from(text(request, "image")?);
    let system = PathBuf::from(text(request, "system")?);
    let mut manifest = SystemManifest::from_file(&system)
        .with_context(|| format!("system manifest {}", system.display()))?;
    let chip_path = system
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .with_context(|| format!("chip descriptor {}", chip_path.display()))?;
    manifest.chip = chip_path.to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&chip, &manifest)?;
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    if let Some(interval) = request["tick_interval"].as_u64() {
        if interval == 0 || interval > u32::MAX as u64 {
            bail!("`tick_interval` must be between 1 and {}", u32::MAX);
        }
        machine.config.peripheral_tick_interval = interval as u32;
    }
    let image =
        std::fs::read(&image_path).with_context(|| format!("firmware {}", image_path.display()))?;
    machine.load_firmware(&labwired_loader::load_elf_bytes(&image)?)?;
    let hz = machine.bus.cpu_hz as u128;
    Ok(Session {
        machine,
        image,
        hz,
        nanoseconds: 0,
        chip_path,
    })
}

fn step(session: &mut Session, request: &Value) -> Result<Value> {
    if let Some(gpio) = request.get("gpio") {
        for (pin, level) in serde_json::from_value::<Vec<(String, bool)>>(gpio.clone())? {
            if !SystemBus::drive_pin_input(&mut session.machine.bus, &pin, level) {
                bail!("the board cannot drive GPIO {pin}");
            }
        }
    }
    if let Some(pads) = request["pads"].as_array() {
        for pad in pads {
            let name = pad[0]
                .as_str()
                .ok_or_else(|| anyhow!("a pad is [name, millivolts]"))?;
            let millivolts = pad[1]
                .as_f64()
                .ok_or_else(|| anyhow!("pad {name} needs a number"))?;
            if !millivolts.is_finite() {
                bail!("pad {name} is not finite");
            }
            // The ADC is a 12-bit converter on a 3.3 V reference; a pad outside that range saturates.
            let clamped = millivolts.round().clamp(0.0, u16::MAX as f64) as u16;
            if !session.machine.bus.set_analog_pad_mv(name, clamped) {
                bail!("the board has no analog pad {name}");
            }
        }
    }
    session.nanoseconds += request["ns"]
        .as_u64()
        .ok_or_else(|| anyhow!("`ns` must be an integer"))? as u128;
    let target = u64::try_from(session.nanoseconds * session.hz / 1_000_000_000)?;
    while session.machine.total_cycles < target {
        let left = target - session.machine.total_cycles;
        session
            .machine
            .advance(AdvanceRequest::run(None).with_cycle_limit(left))?;
    }
    let mut values = vec![];
    for address in request["read"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
    {
        let address = address
            .as_u64()
            .ok_or_else(|| anyhow!("an address is an integer"))?;
        values.push(session.machine.bus.read_u32(address)?);
    }
    if session.machine.total_cycles != target {
        bail!(
            "LabWired clock mismatch: expected cycle {target}, got {}",
            session.machine.total_cycles
        );
    }
    let mut levels = vec![];
    if let Some(pins) = request.get("read_gpio") {
        for pin in serde_json::from_value::<Vec<String>>(pins.clone())? {
            let (address, bit) = SystemBus::resolve_pin_idr_pub(&session.machine.bus, &pin)
                .with_context(|| format!("the board cannot observe GPIO {pin}"))?;
            levels.push(session.machine.bus.read_u32(address)? & (1 << bit) != 0);
        }
    }
    Ok(json!({
        "ok": true, "values": values, "levels": levels, "time_ns": u64::try_from(session.nanoseconds)?,
        "cycles": session.machine.total_cycles
    }))
}

fn handle(session: &mut Option<Session>, request: &Value) -> Result<Value> {
    match text(request, "cmd")? {
        "open" => {
            let opened = open(request)?;
            let pin: Value = serde_json::from_str(include_str!("../upstream.json"))?;
            let reply = json!({
                "ok": true, "protocol": 2, "version": pin["revision"], "cpu_hz": opened.hz as u64,
                "resources": [opened.chip_path],
                "capabilities": ["digital-gpio", "read-word", "elf-symbols", "analog-pad-input"]
            });
            *session = Some(opened);
            Ok(reply)
        }
        "symbol" => {
            let session = session
                .as_ref()
                .ok_or_else(|| anyhow!("no firmware is open"))?;
            let name = text(request, "name")?;
            let address = labwired_loader::resolve_symbol_in_elf(&session.image, name)
                .ok_or_else(|| anyhow!("the firmware has no symbol {name}"))?;
            Ok(json!({"ok": true, "address": address}))
        }
        "step" => step(
            session
                .as_mut()
                .ok_or_else(|| anyhow!("no firmware is open"))?,
            request,
        ),
        other => bail!("unknown command {other}"),
    }
}

fn main() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut session: Option<Session> = None;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                writeln!(
                    stdout,
                    "{}",
                    json!({"ok": false, "error": format!("not JSON: {error}")})
                )?;
                stdout.flush()?;
                continue;
            }
        };
        if request["cmd"] == "close" {
            writeln!(stdout, "{}", json!({"ok": true}))?;
            stdout.flush()?;
            break;
        }
        let reply = handle(&mut session, &request)
            .unwrap_or_else(|error| json!({"ok": false, "error": format!("{error:#}")}));
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}
