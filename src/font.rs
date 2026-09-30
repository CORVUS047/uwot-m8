//! The M8's bitmap fonts and the character grid they imply.

/// ASCII code of the first glyph in the atlas.
const FIRST_GLYPH: u8 = 33;
const GLYPH_COUNT: usize = 94;

/// Layout numbers for one font mode.
#[derive(Copy, Clone, Debug)]
pub struct Metrics {
    /// Glyph cell width in pixels.
    pub glyph_w: usize,
    /// Glyph cell height in pixels.
    pub glyph_h: usize,
    /// Vertical shift applied to every rectangle and glyph in this font mode.
    pub screen_offset_y: i32,
    /// Extra vertical shift applied to glyphs only, within their cell.
    pub text_offset_y: i32,
    /// Tallest oscilloscope sample value this font mode produces.
    pub waveform_max_height: u8,
}

struct FontSpec {
    bmp: &'static [u8],
    metrics: Metrics,
}

/// Indexed the way the M8 numbers its font modes.
const SPECS: [FontSpec; 5] = [
    FontSpec {
        bmp: include_bytes!("../assets/font_v1_small.bmp"),
        metrics: Metrics {
            glyph_w: 5,
            glyph_h: 7,
            screen_offset_y: 0,
            text_offset_y: 3,
            waveform_max_height: 24,
        },
    },
    FontSpec {
        bmp: include_bytes!("../assets/font_v1_large.bmp"),
        metrics: Metrics {
            glyph_w: 8,
            glyph_h: 9,
            screen_offset_y: -40,
            text_offset_y: 4,
            waveform_max_height: 22,
        },
    },
    FontSpec {
        bmp: include_bytes!("../assets/font_v2_small.bmp"),
        metrics: Metrics {
            glyph_w: 9,
            glyph_h: 9,
            screen_offset_y: -2,
            text_offset_y: 5,
            waveform_max_height: 38,
        },
    },
    FontSpec {
        bmp: include_bytes!("../assets/font_v2_large.bmp"),
        metrics: Metrics {
            glyph_w: 10,
            glyph_h: 10,
            screen_offset_y: -2,
            text_offset_y: 4,
            waveform_max_height: 38,
        },
    },
    FontSpec {
        bmp: include_bytes!("../assets/font_v2_huge.bmp"),
        metrics: Metrics {
            glyph_w: 12,
            glyph_h: 12,
            screen_offset_y: -54,
            text_offset_y: 4,
            waveform_max_height: 24,
        },
    },
];

/// Maps a font mode as reported by the device onto an index into the font.
pub fn mode_index(is_model_02: bool, device_mode: usize) -> Option<usize> {
    if device_mode > 2 {
        return None;
    }
    let index = if is_model_02 { device_mode + 2 } else { device_mode };
    (index < SPECS.len()).then_some(index)
}

/// Layout numbers for a font mode, without decoding its bitmap.
pub fn metrics(mode: usize) -> Option<Metrics> {
    SPECS.get(mode).map(|spec| spec.metrics)
}

pub struct Font {
    pub metrics: Metrics,
    /// One entry per pixel of the atlas, row-major, `true` where ink is.
    mask: Vec<bool>,
    atlas_w: usize,
}

/// Decodes the atlas for one font mode.
pub fn load(mode: usize) -> Result<Font, String> {
    let spec = SPECS.get(mode).ok_or_else(|| format!("no such font mode {mode}"))?;
    let (mask, atlas_w, atlas_h) = decode_mono_bmp(spec.bmp)?;

    let expected_w = spec.metrics.glyph_w * GLYPH_COUNT;
    if atlas_w != expected_w || atlas_h != spec.metrics.glyph_h {
        return Err(format!(
            "font {mode}: atlas is {atlas_w}x{atlas_h}, expected {expected_w}x{}",
            spec.metrics.glyph_h
        ));
    }

    Ok(Font { metrics: spec.metrics, mask, atlas_w })
}

impl Font {
    /// Calls `plot(dx, dy)` for every inked pixel of `c`.
    pub fn glyph(&self, c: u8, mut plot: impl FnMut(usize, usize)) {
        if c == b' ' || c < FIRST_GLYPH {
            return;
        }
        let index = (c - FIRST_GLYPH) as usize;
        if index >= GLYPH_COUNT {
            return;
        }
        let x0 = index * self.metrics.glyph_w;
        for y in 0..self.metrics.glyph_h {
            let row = y * self.atlas_w;
            for x in 0..self.metrics.glyph_w {
                if self.mask[row + x0 + x] {
                    plot(x, y);
                }
            }
        }
    }
}

/// Decodes an uncompressed 1-bit BMP into a top-down ink mask.
fn decode_mono_bmp(data: &[u8]) -> Result<(Vec<bool>, usize, usize), String> {
    let u16at = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    let u32at = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);

    if data.len() < 54 || &data[0..2] != b"BM" {
        return Err("not a BMP".into());
    }
    let pixel_offset = u32at(10) as usize;
    let dib_size = u32at(14) as usize;
    let width = u32at(18) as i32;
    let raw_height = u32at(22) as i32;
    let bpp = u16at(28);
    let compression = u32at(30);

    if bpp != 1 || compression != 0 {
        return Err(format!(
            "expected uncompressed 1bpp BMP, got {bpp}bpp compression {compression}"
        ));
    }
    if width <= 0 {
        return Err("bad BMP width".into());
    }

    let bottom_up = raw_height > 0;
    let height = raw_height.unsigned_abs() as usize;
    let width = width as usize;

    let palette = 14 + dib_size;
    if palette + 8 > data.len() {
        return Err("BMP truncated before palette".into());
    }
    let luma = |i: usize| {
        let e = palette + i * 4;
        data[e] as u32 + data[e + 1] as u32 + data[e + 2] as u32
    };
    let ink_bit = if luma(1) >= luma(0) { 1 } else { 0 };

    let row_bytes = ((width + 31) / 32) * 4;
    if pixel_offset + row_bytes * height > data.len() {
        return Err("BMP truncated before pixel data".into());
    }

    let mut mask = vec![false; width * height];
    for y in 0..height {
        let src_row = if bottom_up { height - 1 - y } else { y };
        let src = pixel_offset + src_row * row_bytes;
        for x in 0..width {
            let bit = (data[src + x / 8] >> (7 - (x % 8))) & 1;
            mask[y * width + x] = bit == ink_bit;
        }
    }
    Ok((mask, width, height))
}
