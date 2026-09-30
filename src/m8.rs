//! Serial transport for the M8: discovery, the read thread, and the few
//! commands the device takes from a host.

use std::io::{ErrorKind, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serialport::{SerialPort, SerialPortType};

use crate::slip::Slip;

/// USB identifiers of the Teensy the M8 is built on.
const M8_VID: u16 = 0x16C0;
const M8_PIDS: [u16; 2] = [0x048A, 0x048B];

const BAUD: u32 = 115_200;
const READ_BUF: usize = 4096;
/// Matches m8c's serial polling interval.
const READ_POLL: Duration = Duration::from_millis(4);

/// One decoded SLIP frame, still unparsed.
pub type Packet = Vec<u8>;

pub struct M8 {
    port: Box<dyn SerialPort>,
    packets: Receiver<Packet>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    pub path: String,
}

/// Lists candidate M8 serial devices, most-likely first.
pub fn find_devices() -> Vec<String> {
    let mut found = Vec::new();

    if let Ok(ports) = serialport::available_ports() {
        for port in ports {
            if let SerialPortType::UsbPort(info) = &port.port_type {
                if info.vid == M8_VID && M8_PIDS.contains(&info.pid) {
                    found.push(port.port_name);
                }
            }
        }
    }

    if found.is_empty() {
        found = fallback_devices();
    }

    found
}

/// The stable by-id symlinks the kernel creates, which name the device.
#[cfg(target_os = "linux")]
fn fallback_devices() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev/serial/by-id") else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("M8"))
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect()
}

/// macOS names these `cu.usbmodem` and a number, with no hint of what they are.
#[cfg(target_os = "macos")]
fn fallback_devices() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev") else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("cu.usbmodem"))
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect()
}

/// Windows names its ports `COM` and a number.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn fallback_devices() -> Vec<String> {
    Vec::new()
}

impl M8 {
    pub fn open(path: &str) -> Result<Self, String> {
        let port = serialport::new(path, BAUD)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(READ_POLL)
            .open()
            .map_err(|e| format!("cannot open {path}: {e}"))?;

        let mut read_handle = port
            .try_clone()
            .map_err(|e| format!("cannot clone serial handle: {e}"))?;

        let (tx, packets) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_reader = Arc::clone(&stop);

        let reader = thread::Builder::new()
            .name("m8-serial".into())
            .spawn(move || {
                let mut slip = Slip::new();
                let mut buf = [0u8; READ_BUF];
                while !stop_reader.load(Ordering::Relaxed) {
                    match read_handle.read(&mut buf) {
                        Ok(0) => thread::sleep(READ_POLL),
                        Ok(n) => {
                            let mut disconnected = false;
                            slip.feed(&buf[..n], |packet| {
                                if tx.send(packet.to_vec()).is_err() {
                                    disconnected = true;
                                }
                            });
                            if disconnected {
                                return;
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::TimedOut => {}
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        Err(_) => return,
                    }
                }
            })
            .map_err(|e| format!("cannot start serial thread: {e}"))?;

        Ok(Self { port, packets, stop, reader: Some(reader), path: path.to_string() })
    }

    /// Drains everything the read thread has decoded so far.
    pub fn poll(&self) -> Vec<Packet> {
        self.packets.try_iter().collect()
    }

    /// True once the read thread has exited, i.e. the device is gone.
    pub fn reader_finished(&self) -> bool {
        self.reader.as_ref().is_none_or(|r| r.is_finished())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.port
            .write_all(bytes)
            .and_then(|()| self.port.flush())
            .map_err(|e| format!("serial write failed: {e}"))
    }

    /// Tells the M8 to start streaming its display over serial.
    pub fn enable_display(&mut self, reset: bool) -> Result<(), String> {
        self.write(b"E")?;
        thread::sleep(Duration::from_millis(500));
        if reset {
            self.reset_display()?;
        }
        Ok(())
    }

    /// Asks for a full redraw of the display.
    pub fn reset_display(&mut self) -> Result<(), String> {
        self.write(b"R")
    }

    /// Keepalive, used to tell a quiet device from a disconnected one.
    pub fn ping(&mut self) -> Result<(), String> {
        self.write(b"X")
    }

    /// Sends the complete key state as a bitmask (see `input::Keys`).
    pub fn send_keys(&mut self, keys: u8) -> Result<(), String> {
        self.write(&[b'C', keys])
    }

    pub fn send_note_on(&mut self, note: u8, velocity: u8) -> Result<(), String> {
        self.write(&[b'K', note, velocity.min(0x7F)])
    }

    pub fn send_note_off(&mut self) -> Result<(), String> {
        self.write(&[b'K', 0xFF])
    }

    /// Stops the display stream and shuts the read thread down.
    pub fn disconnect(&mut self) {
        let _ = self.write(b"D");
        self.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for M8 {
    fn drop(&mut self) {
        self.disconnect();
    }
}
