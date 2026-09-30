//! Software framebuffer that the M8's draw commands are replayed into.

use crate::font::{self, Font};
use crate::proto::{Char, Command, Rect, Rgb, SystemInfo, Waveform};

/// Display geometry of the two M8 hardware generations.
pub const MODEL_01_SIZE: (u32, u32) = (320, 240);
pub const MODEL_02_SIZE: (u32, u32) = (480, 320);

pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    /// Set when the framebuffer has changed since the last upload.
    pub dirty: bool,
    /// Set when `width`/`height` changed and the texture must be recreated.
    pub resized: bool,

    font: Font,
    font_mode: Option<usize>,
    is_model_02: bool,
    background: Rgb,
    /// Colour carried forward for rectangle commands that omit it.
    last_color: Rgb,
    prev_waveform_len: usize,
    waveform_cleared: bool,
}

impl Screen {
    pub fn new() -> Result<Self, String> {
        let (width, height) = MODEL_01_SIZE;
        let mut screen = Self {
            width,
            height,
            pixels: vec![Rgb::default().as_u32(); (width * height) as usize],
            dirty: true,
            resized: true,
            font: font::load(0)?,
            font_mode: None,
            is_model_02: false,
            background: Rgb::default(),
            last_color: Rgb::default(),
            prev_waveform_len: 0,
            waveform_cleared: false,
        };
        screen.set_font_mode(0)?;
        Ok(screen)
    }

    pub fn apply(&mut self, command: &Command) -> Result<(), String> {
        match command {
            Command::Rect(r) => self.draw_rect(r),
            Command::Char(c) => self.draw_char(c),
            Command::Waveform(w) => self.draw_waveform(w),
            Command::Joypad => {}
            Command::System(info) => self.apply_system_info(info)?,
        }
        Ok(())
    }

    fn apply_system_info(&mut self, info: &SystemInfo) -> Result<(), String> {
        self.set_model_02(info.is_model_02());
        self.set_font_mode(info.font_mode as usize)?;
        Ok(())
    }

    fn set_model_02(&mut self, model_02: bool) {
        if self.is_model_02 == model_02 && self.font_mode.is_some() {
            return;
        }
        self.is_model_02 = model_02;
        let (w, h) = if model_02 {
            MODEL_02_SIZE
        } else {
            MODEL_01_SIZE
        };
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            self.pixels = vec![self.background.as_u32(); (w * h) as usize];
            self.resized = true;
            self.dirty = true;
        }
    }

    /// `mode` is the raw font mode the device reports.
    fn set_font_mode(&mut self, mode: usize) -> Result<(), String> {
        let Some(index) = font::mode_index(self.is_model_02, mode) else {
            return Ok(());
        };
        if self.font_mode == Some(index) {
            return Ok(());
        }
        self.font = font::load(index)?;
        self.font_mode = Some(index);
        Ok(())
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for row in y0..y1 {
            let start = (row as u32 * self.width) as usize;
            self.pixels[start + x0 as usize..start + x1 as usize].fill(color);
        }
        self.dirty = true;
    }

    fn plot(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.pixels[(y as u32 * self.width + x as u32) as usize] = color;
        self.dirty = true;
    }

    fn draw_rect(&mut self, r: &Rect) {
        let x = r.x as i32;
        let y = r.y as i32 + self.font.metrics.screen_offset_y;

        if x == 0 && y <= 0 && r.w as u32 == self.width && r.h as u32 >= self.height {
            self.background = r.color;
        }

        self.last_color = r.color;
        self.fill(x, y, r.w as i32, r.h as i32, r.color.as_u32());
    }

    fn draw_char(&mut self, c: &Char) {
        let x = c.x as i32;
        let y = c.y as i32 + self.font.metrics.text_offset_y + self.font.metrics.screen_offset_y;

        if c.fg != c.bg {
            self.fill(
                x,
                y,
                self.font.metrics.glyph_w as i32,
                self.font.metrics.glyph_h as i32,
                c.bg.as_u32(),
            );
        }

        let fg = c.fg.as_u32();
        let mut ink = Vec::new();
        self.font
            .glyph(c.c, |dx, dy| ink.push((dx as i32, dy as i32)));
        for (dx, dy) in ink {
            self.plot(x + dx, y + dy, fg);
        }
    }

    fn draw_waveform(&mut self, w: &Waveform) {
        if w.samples.is_empty() && self.waveform_cleared {
            return;
        }

        let strip_len = if w.samples.is_empty() {
            self.prev_waveform_len
        } else {
            w.samples.len()
        };
        let strip_x = self.width as i32 - strip_len as i32;
        let max_height = self.font.metrics.waveform_max_height;

        self.fill(
            strip_x,
            0,
            strip_len as i32,
            max_height as i32 + 1,
            self.background.as_u32(),
        );

        let color = w.color.as_u32();
        for (i, &sample) in w.samples.iter().enumerate() {
            self.plot(strip_x + i as i32, sample.min(max_height) as i32, color);
        }

        self.prev_waveform_len = strip_len;
        self.waveform_cleared = w.samples.is_empty();
        self.dirty = true;
    }

    pub fn last_color(&self) -> Rgb {
        self.last_color
    }

    /// The theme background colour the device last asked for.
    pub fn background(&self) -> Rgb {
        self.background
    }

    /// Clears to the current background colour, used when resetting the display.
    pub fn clear(&mut self) {
        let bg = self.background.as_u32();
        self.pixels.fill(bg);
        self.dirty = true;
    }
}
