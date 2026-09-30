//! Overlay drawing for the window: the keypad, and the settings panel.

use crate::font::Font;
use crate::keys::{FaceButton, FACE_BUTTONS, FACE_SLOT_COLS, FACE_SLOT_ROWS};
use crate::proto::Rgb;

/// Fully transparent, so the screen and the window's background show through.
const CLEAR: u32 = 0x0000_0000;

const KEY_UP_BG: Rgb = Rgb {
    r: 38,
    g: 40,
    b: 48,
};
const KEY_DOWN_BG: Rgb = Rgb {
    r: 120,
    g: 200,
    b: 240,
};
const MENU_BG: Rgb = Rgb {
    r: 24,
    g: 26,
    b: 32,
};
const MENU_BORDER: Rgb = Rgb {
    r: 120,
    g: 200,
    b: 240,
};
const MENU_FG: Rgb = Rgb {
    r: 220,
    g: 225,
    b: 235,
};
const MENU_DIM: Rgb = Rgb {
    r: 130,
    g: 136,
    b: 148,
};

/// A quarter of a key: the unit every keypad measurement is a multiple of.
const UNIT: usize = 8;
/// Key block size and spacing, in device pixels. Square, since the blocks carry
/// no label and only their position says which button they are.
const KEY_W: usize = UNIT * 4;
const KEY_H: usize = UNIT * 4;
const KEY_GAP: usize = UNIT;
/// How far a nudged key sits off its slot.
const KEY_NUDGE: usize = UNIT;
/// Blank space between the M8's screen and the keypad. At least a nudge, so the
/// pair that sits a step up stays out of the device's screen.
const PANEL_MARGIN: usize = UNIT;

/// Height of the keypad panel, the nudge below the bottom row included.
pub fn panel_height() -> usize {
    PANEL_MARGIN * 2 + FACE_SLOT_ROWS * KEY_H + (FACE_SLOT_ROWS - 1) * KEY_GAP + KEY_NUDGE
}

/// Width of the keypad, the nudge right of the last column included.
fn keypad_width() -> usize {
    FACE_SLOT_COLS * (KEY_W + KEY_GAP) - KEY_GAP + KEY_NUDGE
}

/// An ARGB pixel buffer with an alpha channel, drawn over the M8's screen.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![CLEAR; width * height],
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels = vec![CLEAR; width * height];
    }

    pub fn clear(&mut self) {
        self.pixels.fill(CLEAR);
    }

    /// Fills a rectangle, clipped to the canvas on every side.
    pub fn fill(&mut self, x: i32, y: i32, w: usize, h: usize, color: Rgb) {
        let color = 0xFF00_0000 | (color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32;
        let x0 = (x.max(0) as usize).min(self.width);
        let y0 = (y.max(0) as usize).min(self.height);
        let x1 = ((x + w as i32).max(0) as usize).min(self.width);
        let y1 = ((y + h as i32).max(0) as usize).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for row in y0..y1 {
            self.pixels[row * self.width + x0..row * self.width + x1].fill(color);
        }
    }

    /// Draws `text` with its top left at `(x, y)`.
    pub fn text(
        &mut self,
        font: &Font,
        x: i32,
        y: i32,
        text: &str,
        color: Rgb,
        scale: usize,
    ) -> usize {
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

    /// How many characters fit across `width` pixels.
    fn columns(font: &Font, width: usize, scale: usize) -> usize {
        width / ((font.metrics.glyph_w + 1) * scale).max(1)
    }
}

/// Shortens `text` to `width` characters, marking where it was cut.
fn trim_to(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    text.chars()
        .take(width - 1)
        .chain(std::iter::once('~'))
        .collect()
}

/// Draws the face buttons into the strip below the device's screen.
pub fn draw_keypad(canvas: &mut Canvas, top: usize, pressed: u8) {
    let left = canvas.width.saturating_sub(keypad_width()) / 2;
    for key in &FACE_BUTTONS {
        let (x, y) = key_at(left, top + PANEL_MARGIN, key);
        let bg = if pressed & key.mask != 0 {
            KEY_DOWN_BG
        } else {
            KEY_UP_BG
        };
        canvas.fill(x, y, KEY_W, KEY_H, bg);
    }
}

/// A key's top left corner, its nudge off the slot grid applied.
fn key_at(left: usize, top: usize, key: &FaceButton) -> (i32, i32) {
    let nudge = KEY_NUDGE as i32;
    (
        (left + key.col * (KEY_W + KEY_GAP)) as i32 + key.nudge.0 * nudge,
        (top + key.row * (KEY_H + KEY_GAP)) as i32 + key.nudge.1 * nudge,
    )
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

    // The overlay is only as wide as the M8's own screen, and a line like the
    // audio device's name runs well past that, so everything is cut to fit.
    let columns = Canvas::columns(font, canvas.width.saturating_sub(padding * 2), scale);
    let title = trim_to(title, columns);
    let footer = trim_to(footer, columns);
    let status = trim_to(status, columns);
    let items: Vec<(String, String)> = items
        .iter()
        .map(|(name, value)| {
            let value = trim_to(value, columns / 2);
            let room = columns.saturating_sub(value.chars().count() + "> ".len() + 4);
            (trim_to(name, room), value)
        })
        .collect();
    let (title, footer, status) = (title.as_str(), footer.as_str(), status.as_str());

    let widest = items
        .iter()
        .map(|(name, value)| Canvas::text_width(font, &format!("> {name}    {value}"), scale))
        .chain(std::iter::once(Canvas::text_width(font, footer, scale)))
        .chain(std::iter::once(Canvas::text_width(font, status, scale)))
        .chain(std::iter::once(Canvas::text_width(font, title, scale)))
        .max()
        .unwrap_or(0);

    let box_w = (widest + padding * 2).min(canvas.width);
    let status_lines = usize::from(!status.is_empty());
    let box_h = (line_h * (items.len() + 3 + status_lines) + padding * 2).min(canvas.height);
    let x = canvas.width.saturating_sub(box_w) / 2;
    let y = canvas.height.saturating_sub(box_h) / 2;

    canvas.fill(
        x as i32 - 1,
        y as i32 - 1,
        box_w + 2,
        box_h + 2,
        MENU_BORDER,
    );
    canvas.fill(x as i32, y as i32, box_w, box_h, MENU_BG);

    let mut line_y = y + padding;
    canvas.text(
        font,
        (x + padding) as i32,
        line_y as i32,
        title,
        MENU_BORDER,
        scale,
    );
    line_y += line_h * 2;

    for (index, (name, value)) in items.iter().enumerate() {
        let selected = index == selected;
        let text = format!("{} {name}", if selected { '>' } else { ' ' });
        let color = if selected { MENU_FG } else { MENU_DIM };
        canvas.text(
            font,
            (x + padding) as i32,
            line_y as i32,
            &text,
            color,
            scale,
        );
        let value_x = x + box_w - padding - Canvas::text_width(font, value, scale);
        canvas.text(font, value_x as i32, line_y as i32, value, color, scale);
        line_y += line_h;
    }

    line_y += line_h - line_h / 2;
    if !status.is_empty() {
        canvas.text(
            font,
            (x + padding) as i32,
            line_y as i32,
            status,
            MENU_FG,
            scale,
        );
        line_y += line_h;
    }
    canvas.text(
        font,
        (x + padding) as i32,
        line_y as i32,
        footer,
        MENU_DIM,
        scale,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        crate::font::load(2).expect("the bundled font")
    }

    /// The overlay is the width of the M8's screen, and the audio device's name
    /// is longer than that, so the panel has to be cut down to fit.
    #[test]
    fn a_status_line_wider_than_the_overlay_does_not_run_off_the_canvas() {
        let font = font();
        let mut canvas = Canvas::new(320, 300);
        let items = [("Audio".to_string(), "on".to_string())];
        draw_menu(
            &mut canvas,
            &font,
            "Settings",
            &items,
            0,
            "up/down select   left/right change   Esc back",
            "audio: M8 Analog Stereo to Scarlett Solo (3rd Gen.) Headphones / Line 1-2 \
             at 44100 Hz via PulseAudio   pads: none",
        );
        assert!(canvas.pixels.iter().any(|pixel| *pixel != CLEAR));
    }

    #[test]
    fn a_rectangle_off_any_edge_is_clipped_rather_than_indexed() {
        let mut canvas = Canvas::new(4, 4);
        let white = Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
        canvas.fill(100, 100, 8, 8, white);
        canvas.fill(-20, -20, 8, 8, white);
        canvas.fill(2, 2, 0, 0, white);
        assert!(canvas.pixels.iter().all(|pixel| *pixel == CLEAR));

        canvas.fill(-2, -2, 4, 4, white);
        assert_ne!(canvas.pixels[0], CLEAR);
        assert_eq!(canvas.pixels[2], CLEAR);
    }

    fn keypad() -> Canvas {
        let mut canvas = Canvas::new(keypad_width(), panel_height());
        draw_keypad(&mut canvas, 0, 0);
        canvas
    }

    fn at(canvas: &Canvas, x: i32, y: i32) -> u32 {
        canvas.pixels[y as usize * canvas.width + x as usize]
    }

    /// Renders the keypad as the sketch it was specified by: one character per
    /// key-sized cell, `1` where a block is and `0` where the gaps are. Sampled
    /// at each key's own centre, so a nudge moves the sample with it.
    #[test]
    fn the_keypad_sits_in_a_cross_with_the_face_buttons_alongside() {
        let canvas = keypad();
        let mut sketch = vec![vec!['0'; FACE_SLOT_COLS]; FACE_SLOT_ROWS];
        for key in &FACE_BUTTONS {
            let (x, y) = key_at(0, PANEL_MARGIN, key);
            let centre = at(&canvas, x + (KEY_W / 2) as i32, y + (KEY_H / 2) as i32);
            sketch[key.row][key.col] = if centre == CLEAR { '0' } else { '1' };
        }
        let sketch: Vec<String> = sketch.into_iter().map(String::from_iter).collect();
        assert_eq!(sketch, ["0111", "1110", "1100"]);
    }

    /// Every nudge is one step, so all four moved keys move by the same amount.
    #[test]
    fn the_nudged_keys_sit_one_step_off_their_slots_and_no_further() {
        let slot = |key: &FaceButton| {
            (
                (key.col * (KEY_W + KEY_GAP)) as i32,
                (PANEL_MARGIN + key.row * (KEY_H + KEY_GAP)) as i32,
            )
        };
        let step = KEY_NUDGE as i32;
        for key in &FACE_BUTTONS {
            let (x, y) = key_at(0, PANEL_MARGIN, key);
            let (slot_x, slot_y) = slot(key);
            assert!(matches!(key.nudge, (-1..=1, -1..=1)));
            assert_eq!(
                (x - slot_x, y - slot_y),
                (key.nudge.0 * step, key.nudge.1 * step)
            );
        }

        let find = |mask: u8| FACE_BUTTONS.iter().find(|key| key.mask == mask).unwrap();
        let up = key_at(0, PANEL_MARGIN, find(crate::keys::bits::UP));
        let option = key_at(0, PANEL_MARGIN, find(crate::keys::bits::OPT));
        let left = key_at(0, PANEL_MARGIN, find(crate::keys::bits::LEFT));
        let shift = key_at(0, PANEL_MARGIN, find(crate::keys::bits::SELECT));
        // OPTION: a step up from UP's row, and a step right of its own column.
        assert_eq!(option.1, up.1 - step);
        assert_eq!(option.0, up.0 + (KEY_W + KEY_GAP) as i32 + step);
        // SHIFT: a step lower than the row below LEFT.
        assert_eq!(shift.1, left.1 + (KEY_H + KEY_GAP) as i32 + step);
        // Nothing is pushed off the top of the panel.
        assert!(option.1 >= 0);
    }

    /// Nothing but the block's own colour: the blocks carry no label.
    #[test]
    fn a_pressed_key_is_a_plain_block_of_one_colour() {
        let mut canvas = Canvas::new(keypad_width(), panel_height());
        draw_keypad(&mut canvas, 0, crate::keys::bits::UP);

        let up = FACE_BUTTONS
            .iter()
            .find(|key| key.mask == crate::keys::bits::UP)
            .unwrap();
        let (up_x, up_y) = key_at(0, PANEL_MARGIN, up);
        let colors: std::collections::BTreeSet<u32> = (up_y..up_y + KEY_H as i32)
            .flat_map(|y| (up_x..up_x + KEY_W as i32).map(move |x| (x, y)))
            .map(|(x, y)| at(&canvas, x, y))
            .collect();
        assert_eq!(colors.len(), 1);
        assert_ne!(colors.first().copied(), Some(CLEAR));
    }

    /// The bottom row's step down has to be inside the panel, or the keypad is
    /// drawn over whatever sits under the window's strip.
    #[test]
    fn the_panel_is_tall_enough_for_the_row_nudged_below_it() {
        let lowest = FACE_BUTTONS
            .iter()
            .map(|key| key_at(0, PANEL_MARGIN, key).1 + KEY_H as i32)
            .max()
            .unwrap();
        assert!(
            lowest <= panel_height() as i32,
            "{lowest} past {}",
            panel_height()
        );

        let widest = FACE_BUTTONS
            .iter()
            .map(|key| key_at(0, PANEL_MARGIN, key).0 + KEY_W as i32)
            .max()
            .unwrap();
        assert_eq!(widest, keypad_width() as i32);
    }

    #[test]
    fn a_trimmed_line_says_where_it_was_cut() {
        assert_eq!(trim_to("abcdef", 6), "abcdef");
        assert_eq!(trim_to("abcdef", 4), "abc~");
        assert_eq!(trim_to("abcdef", 0), "");
    }
}
