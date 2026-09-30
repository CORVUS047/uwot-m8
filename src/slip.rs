//! SLIP (RFC 1055) frame decoder for the M8's serial stream.

const END: u8 = 0xC0;
const ESC: u8 = 0xDB;
const ESC_END: u8 = 0xDC;
const ESC_ESC: u8 = 0xDD;

/// Maximum packet the M8 can send (oscilloscope waveform for a 480px display).
const MAX_PACKET: usize = 1024;

#[derive(Default)]
pub struct Slip {
    buf: Vec<u8>,
    escaped: bool,
}

impl Slip {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(MAX_PACKET), escaped: false }
    }

    /// Feeds bytes from the serial port, calling `on_packet` for every complete frame.
    pub fn feed(&mut self, bytes: &[u8], mut on_packet: impl FnMut(&[u8])) {
        for &b in bytes {
            if self.escaped {
                self.escaped = false;
                match b {
                    ESC_END => self.push(END),
                    ESC_ESC => self.push(ESC),
                    _ => self.reset(),
                }
                continue;
            }
            match b {
                END => {
                    if !self.buf.is_empty() {
                        on_packet(&self.buf);
                    }
                    self.reset();
                }
                ESC => self.escaped = true,
                _ => self.push(b),
            }
        }
    }

    fn push(&mut self, b: u8) {
        if self.buf.len() >= MAX_PACKET {
            self.reset();
        } else {
            self.buf.push(b);
        }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.escaped = false;
    }
}
