//! Run42's JSON-lines firmware host backed by a private Renode process.
mod control;

use anyhow::{anyhow, bail, ensure, Context, Result};
use control::Control;
use object::{Object, ObjectSymbol};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Runtime(Child);
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Session {
    control: Control,
    image: Vec<u8>,
    ports: BTreeMap<String, i32>,
    bus: Option<i32>,
    _runtime: Runtime,
    _directory: tempfile::TempDir,
}

impl Session {
    fn open(runtime: &Path, system: &Path, image: &Path) -> Result<Self> {
        let firmware = std::fs::read(image)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let directory = tempfile::tempdir()?;
        let script = directory.path().join("boot.resc");
        let startup_log = directory.path().join("renode.log");
        std::fs::write(&script, format!(
            "mach create \"run42\"\nmachine LoadPlatformDescription {}\nsysbus LoadELF {}\nemulation CreateExternalControlClient \"run42-link\" {}\n",
            monitor_path(system)?, monitor_path(image)?, listener.local_addr()?.port()
        ))?;
        // Keep Renode's stdin alive without sharing the helper's JSON stream. Its
        // console exits at EOF even if the helper is killed by the parent.
        let mut child = Runtime(
            Command::new(runtime)
                .args(["--disable-gui", "--console", "--plain", "--hide-log"])
                .arg(&script)
                .stdin(Stdio::piped())
                .stdout(Stdio::from(std::fs::File::create(&startup_log)?))
                .stderr(Stdio::inherit())
                .spawn()
                .with_context(|| format!("launching Renode at {}", runtime.display()))?,
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
            if let Some(status) = child.0.try_wait()? {
                bail!(
                    "Renode exited before connecting ({status}): {}",
                    startup_error(&startup_log)
                );
            }
            ensure!(
                Instant::now() < deadline,
                "Renode did not connect: {}",
                startup_error(&startup_log)
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(Self {
            control: Control::new(stream)?,
            image: firmware,
            ports: BTreeMap::new(),
            bus: None,
            _runtime: child,
            _directory: directory,
        })
    }

    fn gpio(&mut self, pad: &str, level: Option<bool>) -> Result<bool> {
        let upper = pad.to_ascii_uppercase();
        let bytes = upper.as_bytes();
        ensure!(
            bytes.len() >= 3 && bytes[0] == b'P' && bytes[1].is_ascii_uppercase(),
            "invalid STM32 GPIO pad {pad}"
        );
        let pin = upper[2..].parse::<u8>()?;
        ensure!(pin < 16, "invalid STM32 GPIO pad {pad}");
        let port = format!("sysbus.gpioPort{}", char::from(bytes[1]));
        let interface = match self.ports.get(&port) {
            Some(id) => *id,
            None => {
                let id = self
                    .control
                    .gpio_port(&port)
                    .with_context(|| format!("resolving GPIO {pad}"))?;
                self.ports.insert(port, id);
                id
            }
        };
        self.control
            .gpio(interface, pin, level)
            .with_context(|| format!("accessing GPIO {pad}"))
    }

    fn step(&mut self, request: &Value) -> Result<Value> {
        ensure!(
            request["pads"].as_array().is_some_and(Vec::is_empty),
            "Renode host currently supports digital GPIO inputs; analog pads are unsupported"
        );
        for (pad, level) in serde_json::from_value::<Vec<(String, bool)>>(request["gpio"].clone())?
        {
            self.gpio(&pad, Some(level))?;
        }
        self.control
            .advance(request["ns"].as_u64().context("missing step duration")?)?;
        let mut values = vec![];
        for address in serde_json::from_value::<Vec<u64>>(request["read"].clone())? {
            let bus = match self.bus {
                Some(id) => id,
                None => {
                    let id = self.control.system_bus()?;
                    self.bus = Some(id);
                    id
                }
            };
            values.push(self.control.read_word(bus, address)?);
        }
        let mut levels = vec![];
        for pad in serde_json::from_value::<Vec<String>>(request["read_gpio"].clone())? {
            levels.push(self.gpio(&pad, None)?);
        }
        Ok(json!({"ok": true, "values": values, "levels": levels, "time_ns": self.control.time()?}))
    }

    fn symbol(&self, name: &str) -> Result<Value> {
        let image = object::File::parse(self.image.as_slice())?;
        let symbol = image
            .symbol_by_name(name)
            .with_context(|| format!("no firmware symbol {name}"))?;
        Ok(json!({"ok": true, "address": symbol.address()}))
    }
}

fn startup_error(path: &Path) -> String {
    let log = std::fs::read_to_string(path).unwrap_or_default();
    let mut last: Vec<_> = log.lines().rev().take(12).collect();
    last.reverse();
    last.join("\n")
}

fn monitor_path(path: &Path) -> Result<String> {
    let absolute = path.canonicalize()?;
    #[cfg(windows)]
    let absolute = absolute
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    #[cfg(not(windows))]
    let absolute = absolute.to_str().context("Renode needs UTF-8 paths")?;
    ensure!(
        !absolute.contains(['"', '\n', '\r']),
        "invalid Renode platform or ELF path"
    );
    Ok(format!("\"{absolute}\""))
}

fn field<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    request[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing {key}"))
}

fn handle(session: &mut Option<Session>, request: &Value) -> Result<Value> {
    match field(request, "cmd")? {
        "open" => {
            let configured = PathBuf::from(
                std::env::var_os("RUN42_RENODE")
                    .context("set RUN42_RENODE to the Renode executable")?,
            );
            let runtime = configured.canonicalize().with_context(|| {
                format!("Renode runtime at RUN42_RENODE={}", configured.display())
            })?;
            let output = Command::new(&runtime)
                .arg("--version")
                .output()
                .with_context(|| format!("starting Renode at {}", runtime.display()))?;
            let version = String::from_utf8(output.stdout)?;
            ensure!(
                output.status.success() && version.contains("1.17."),
                "expected Renode 1.17, got {}",
                version.trim()
            );
            *session = Some(Session::open(
                &runtime,
                Path::new(field(request, "system")?),
                Path::new(field(request, "image")?),
            )?);
            Ok(json!({
                "ok": true, "protocol": 2, "version": version.trim(), "resources": [runtime],
                "capabilities": ["digital-gpio", "read-word", "elf-symbols"]
            }))
        }
        "step" => session
            .as_mut()
            .context("no firmware is open")?
            .step(request),
        "symbol" => session
            .as_ref()
            .context("no firmware is open")?
            .symbol(field(request, "name")?),
        other => bail!("unknown command {other}"),
    }
}

fn main() -> Result<()> {
    let mut session = None;
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let request: Value = serde_json::from_str(&line)?;
        if request["cmd"] == "close" {
            drop(session.take());
            writeln!(stdout, "{}", json!({"ok": true}))?;
            stdout.flush()?;
            break;
        }
        let reply = handle(&mut session, &request).unwrap_or_else(|e| {
            drop(session.take());
            json!({"ok": false, "error": format!("{e:#}")})
        });
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}
