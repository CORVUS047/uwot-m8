//! Overlay drawing for the window: the keypad, and the settings panel.

use crate::font::Font;
use crate::keys::{FACE_BUTTONS, FACE_SLOT_COLS, FACE_SLOT_ROWS};
use crate::proto::Rgb;

/// Fully transparent, so the screen and the window's background show through.
const CLEAR: u32 = 0x0000_0000;

const KEY_UP_BG: Rgb = Rgb { r: 38, g: 40, b: 48 };
const KEY_UP_FG: Rgb = Rgb { r: 150, g: 155, b: 165 };
const KEY_DOWN_BG: Rgb = Rgb { r: 120, g: 200, b: 240 };
const KEY_DOWN_FG: Rgb = Rgb { r: 10, g: 12, b: 16 };
const MENU_BG: Rgb = Rgb { r: 24, g: 26, b: 32 };
const MENU_BORDER: Rgb = Rgb { r: 120, g: 200, b: 240 };
const MENU_FG: Rgb = Rgb { r: 220, g: 225, b: 235 };
const MENU_DIM: Rgb = Rgb { r: 130, g: 136, b: 148 };

/// Key block size and spacing, in device pixels.
const KEY_W: usize = 66;
const KEY_H: usize = 20;
const KEY_GAP: usize = 6;
/// Blank space between the M8's screen and the keypad.
const PANEL_MARGIN: usize = 8;

/// Height of the keypad panel for a given label scale.
pub fn panel_height() -> usize {
    PANEL_MARGIN * 2 + FACE_SLOT_ROWS * KEY_H + (FACE_SLOT_ROWS - 1) * KEY_GAP
}

fn keypad_width() -> usize {
    FACE_SLOT_COLS * (KEY_W + KEY_GAP) - KEY_GAP
}

/// An ARGB pixel buffer with an alpha channel, drawn over the M8's screen.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![CLEAR; width * height] }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels = vec![CLEAR; width * height];
    }

    pub fn clear(&mut self) {
        self.pixels.fill(CLEAR);
    }

    pub fn fill(&mut self, x: i32, y: i32, w: usize, h: usize, color: Rgb) {
        let color = 0xFF00_0000 | (color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32;
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w as i32).max(0) as usize).min(self.width);
        let y1 = ((y + h as i32).max(0) as usize).min(self.height);
        for row in y0..y1 {
            self.pixels[row * self.width + x0..row * self.width + x1].fill(color);
        }
    }

    /// Draws `text` with its top left at `(x, y)`.
    pub fn text(&mut self, font: &Font, x: i32, y: i32, text: &str, color: Rgb, scale: usize) -> usize {
        let advance = (font.metrics.glyph_w + 1) * scale;
        for (index, c) in text.bytes().enumerate() {
            let left = x + (index * advance) as i32;
            let mut dots = Vec::new();
            font.glyph(c, |dx, dy| dots.push((dx, dy)));
            for (dx, dy) in dots {
                self.fill(
                    left + (dx * scale) as i32,
                    y + (dy * scale) as i32,
                    scale,
                    scale,
                    color,
                );
            }
        }
        text.len() * advance
    }

    pub fn text_width(font: &Font, text: &str, scale: usize) -> usize {
        text.len() * (font.metrics.glyph_w + 1) * scale
    }
}

/// Draws the face buttons into the strip below the device's screen.
pub fn draw_keypad(canvas: &mut Canvas, font: &Font, top: usize, pressed: u8) {
    let scale = 1;
    let left = canvas.width.saturating_sub(keypad_width()) / 2;
    for (label, button, slot_col, slot_row) in FACE_BUTTONS {
        let x = left + slot_col * (KEY_W + KEY_GAP);
        let y = top + PANEL_MARGIN + slot_row * (KEY_H + KEY_GAP);
        let down = pressed & button != 0;
        let (bg, fg) = if down { (KEY_DOWN_BG, KEY_DOWN_FG) } else { (KEY_UP_BG, KEY_UP_FG) };

        canvas.fill(x as i32, y as i32, KEY_W, KEY_H, bg);
        let text_w = Canvas::text_width(font, label, scale);
        let text_x = x + KEY_W.saturating_sub(text_w) / 2;
        let text_y = y + KEY_H.saturating_sub(font.metrics.glyph_h * scale) / 2;
        canvas.text(font, text_x as i32, text_y as i32, label, fg, scale);
    }
}

/// Draws the settings panel over the middle of the screen.
pub fn draw_menu(
    canvas: &mut Canvas,
    font: &Font,
    title: &str,
    items: &[(String, String)],
    selected: usize,
    footer: &str,
    status: &str,
) {
    let scale = 1;
    let line_h = font.metrics.glyph_h * scale + 6;
    let padding = 10;

    let widest = items
        .iter()
        .map(|(name, value)| Canvas::text_width(font, &format!("> {name}    {value}"), scale))
        .chain(std::iter::once(Canvas::text_width(font, footer, scale)))
        .chain(std::iter::once(Canvas::text_width(font, status, scale)))
        .chain(std::iter::once(Canvas::text_width(font, title, scale)))
        .max()
        .unwrap_or(0);

    let box_w = widest + padding * 2;
    let status_lines = usize::from(!status.is_empty());
    let box_h = line_h * (items.len() + 3 + status_lines) + padding * 2;
    let x = canvas.width.saturating_sub(box_w) / 2;
    let y = canvas.height.saturating_sub(box_h) / 2;

    canvas.fill(x as i32 - 1, y as i32 - 1, box_w + 2, box_h + 2, MENU_BORDER);
    canvas.fill(x as i32, y as i32, box_w, box_h, MENU_BG);

    let mut line_y = y + padding;
    canvas.text(font, (x + padding) as i32, line_y as i32, title, MENU_BORDER, scale);
    line_y += line_h * 2;

    for (index, (name, value)) in items.iter().enumerate() {
        let selected = index == selected;
        let text = format!("{} {name}", if selected { '>' } else { ' ' });
        let color = if selected { MENU_FG } else { MENU_DIM };
        canvas.text(font, (x + padding) as i32, line_y as i32, &text, color, scale);
        let value_x = x + box_w - padding - Canvas::text_width(font, value, scale);
        canvas.text(font, value_x as i32, line_y as i32, value, color, scale);
        line_y += line_h;
    }

    line_y += line_h - line_h / 2;
    if !status.is_empty() {
        canvas.text(font, (x + padding) as i32, line_y as i32, status, MENU_FG, scale);
        line_y += line_h;
    }
    canvas.text(font, (x + padding) as i32, line_y as i32, footer, MENU_DIM, scale);
}
