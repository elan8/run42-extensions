//! Synchronous subset of Renode 1.17's External Control socket protocol.
//! See Renode's tools/external_control_client and Network/ExternalControl sources.
use anyhow::{bail, ensure, Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const CHECK_VERSION: u16 = 0;
const RUN_FOR: u16 = 1;
const GET_TIME: u16 = 2;
const GET_MACHINE: u16 = 3;
const GPIO: u16 = 5;
const SYSTEM_BUS: u16 = 6;

pub struct Control {
    stream: TcpStream,
    request_id: u16,
    machine: i32,
}

impl Control {
    pub fn new(stream: TcpStream) -> Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_write_timeout(Some(Duration::from_secs(15)))?;
        stream.set_nodelay(true)?;
        let mut control = Self {
            stream,
            request_id: 0,
            machine: -1,
        };
        // Renode connects as the protocol client and negotiates first.
        let (id, version) = control.receive()?;
        ensure!(
            id & 0x8000 != 0 && version == [0, 0, 0, 0, 0, 0, 0],
            "unsupported Renode External Control protocol"
        );
        control.send(id, CHECK_VERSION, 2, &[])?;
        let reply = control.request(GET_MACHINE, &name_payload("run42")?)?;
        control.machine = descriptor(&reply)?;
        Ok(control)
    }

    fn send(&mut self, id: u16, command: u16, kind: u8, payload: &[u8]) -> Result<()> {
        let size = u32::try_from(payload.len() + 3)?;
        let mut frame = Vec::with_capacity(6 + size as usize);
        frame.extend(id.to_le_bytes());
        frame.extend(size.to_le_bytes());
        frame.extend(command.to_le_bytes());
        frame.push(kind);
        frame.extend(payload);
        self.stream
            .write_all(&frame)
            .context("sending Renode request")?;
        Ok(())
    }

    fn receive(&mut self) -> Result<(u16, Vec<u8>)> {
        let mut header = [0; 6];
        self.stream
            .read_exact(&mut header)
            .context("receiving Renode response")?;
        let reply_id = u16::from_le_bytes(header[..2].try_into()?);
        let len = u32::from_le_bytes(header[2..].try_into()?);
        ensure!(
            (3..=65536).contains(&len),
            "invalid Renode response envelope"
        );
        let mut reply = vec![0; len as usize];
        self.stream.read_exact(&mut reply)?;
        Ok((reply_id, reply))
    }

    fn request(&mut self, command: u16, payload: &[u8]) -> Result<Vec<u8>> {
        // The listener is the protocol server. Its request IDs leave bit 15 clear.
        let id = self.request_id;
        self.request_id = (id + 1) & 0x7fff;
        self.send(id, command, 0, payload)?;
        let (reply_id, mut reply) = self.receive()?;
        ensure!(reply_id == id, "Renode response ID mismatch");
        ensure!(
            u16::from_le_bytes(reply[..2].try_into()?) == command,
            "Renode response command mismatch"
        );
        if reply[2] != 2 {
            bail!(
                "Renode command {command} failed: {}",
                String::from_utf8_lossy(&reply[3..])
            );
        }
        Ok(reply.split_off(3))
    }

    pub fn interface(&mut self, command: u16, name: &str) -> Result<i32> {
        let mut payload = (-1_i32).to_le_bytes().to_vec();
        payload.extend(self.machine.to_le_bytes());
        payload.extend(name_payload(name)?);
        descriptor(&self.request(command, &payload)?)
    }

    pub fn gpio_port(&mut self, name: &str) -> Result<i32> {
        self.interface(GPIO, name)
    }

    pub fn gpio(&mut self, port: i32, pin: u8, level: Option<bool>) -> Result<bool> {
        let mut payload = port.to_le_bytes().to_vec();
        payload.push(u8::from(level.is_some()));
        payload.extend(i32::from(pin).to_le_bytes());
        if let Some(level) = level {
            payload.push(u8::from(level));
            self.request(GPIO, &payload)?;
            return Ok(level);
        }
        match self.request(GPIO, &payload)?.as_slice() {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => bail!("invalid Renode GPIO observation"),
        }
    }

    pub fn advance(&mut self, ns: u64) -> Result<()> {
        if ns != 0 {
            self.request(RUN_FOR, &ns.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn time(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.request(GET_TIME, &[0; 8])?.as_slice().try_into()?,
        ))
    }

    pub fn system_bus(&mut self) -> Result<i32> {
        self.interface(SYSTEM_BUS, "sysbus")
    }

    pub fn read_word(&mut self, bus: i32, address: u64) -> Result<u32> {
        let mut payload = bus.to_le_bytes().to_vec();
        payload.extend([0, 4]); // read, double word
        payload.extend(address.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        Ok(u32::from_le_bytes(
            self.request(SYSTEM_BUS, &payload)?.as_slice().try_into()?,
        ))
    }
}

fn name_payload(name: &str) -> Result<Vec<u8>> {
    let mut payload = u32::try_from(name.len())?.to_le_bytes().to_vec();
    payload.extend(name.as_bytes());
    Ok(payload)
}

fn descriptor(reply: &[u8]) -> Result<i32> {
    let index = i32::from_le_bytes(reply.try_into()?);
    ensure!(index >= 0, "Renode did not resolve the requested interface");
    Ok(index)
}
