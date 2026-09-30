//! Parsing of the display commands the M8 sends over serial.

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn as_u32(self) -> u32 {
        0xFF00_0000 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub color: Rgb,
}

#[derive(Copy, Clone, Debug)]
pub struct Char {
    pub c: u8,
    pub x: u16,
    pub y: u16,
    pub fg: Rgb,
    pub bg: Rgb,
}

#[derive(Clone, Debug)]
pub struct Waveform {
    pub color: Rgb,
    pub samples: Vec<u8>,
}

#[derive(Copy, Clone, Debug)]
pub struct SystemInfo {
    pub hardware: u8,
    pub version: (u8, u8, u8),
    pub font_mode: u8,
}

impl SystemInfo {
    pub fn hardware_name(&self) -> &'static str {
        match self.hardware {
            0 => "Headless",
            1 => "Beta M8",
            2 => "Production M8 (Model:01)",
            3 => "Production M8 (Model:02)",
            _ => "Unknown",
        }
    }

    /// The Model:02 has a 480x320 display; everything else is 320x240.
    pub fn is_model_02(&self) -> bool {
        self.hardware == 3
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Rect(Rect),
    Char(Char),
    Waveform(Waveform),
    /// Key state as reported by the device's own hardware keys.
    Joypad,
    System(SystemInfo),
}

fn u16le(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn rgb(data: &[u8], at: usize) -> Rgb {
    Rgb {
        r: data[at],
        g: data[at + 1],
        b: data[at + 2],
    }
}

/// Parses one SLIP frame.
pub fn parse(data: &[u8], last_color: Rgb) -> Result<Command, String> {
    let kind = *data.first().ok_or("empty packet")?;
    match kind {
        0xFE => {
            let (w, h, color) = match data.len() {
                5 => (1, 1, last_color),
                8 => (1, 1, rgb(data, 5)),
                9 => (u16le(data, 5), u16le(data, 7), last_color),
                12 => (u16le(data, 5), u16le(data, 7), rgb(data, 9)),
                n => return Err(format!("draw rectangle: bad length {n}")),
            };
            Ok(Command::Rect(Rect {
                x: u16le(data, 1),
                y: u16le(data, 3),
                w,
                h,
                color,
            }))
        }
        0xFD => {
            if data.len() != 12 {
                return Err(format!("draw character: bad length {}", data.len()));
            }
            Ok(Command::Char(Char {
                c: data[1],
                x: u16le(data, 2),
                y: u16le(data, 4),
                fg: rgb(data, 6),
                bg: rgb(data, 9),
            }))
        }
        0xFC => {
            if !(4..=484).contains(&data.len()) {
                return Err(format!("draw oscilloscope: bad length {}", data.len()));
            }
            Ok(Command::Waveform(Waveform {
                color: rgb(data, 1),
                samples: data[4..].to_vec(),
            }))
        }
        0xFB => {
            if data.len() != 3 {
                return Err(format!("joypad state: bad length {}", data.len()));
            }
            Ok(Command::Joypad)
        }
        0xFF => {
            if data.len() != 6 {
                return Err(format!("system info: bad length {}", data.len()));
            }
            Ok(Command::System(SystemInfo {
                hardware: data[1],
                version: (data[2], data[3], data[4]),
                font_mode: data[5],
            }))
        }
        _ => Err(format!("unknown command 0x{kind:02X}")),
    }
}
