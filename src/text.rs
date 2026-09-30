//! Terminal-mode model of the M8 screen.

use crate::font::{self, Metrics};
use crate::proto::{Char, Command, Rect, Rgb, SystemInfo, Waveform};
use crate::screen::{MODEL_01_SIZE, MODEL_02_SIZE};

/// Plausible bounds for the grid pitch.
const MIN_PITCH_X: usize = 6;
const MIN_PITCH_Y: usize = 8;
const MAX_PITCH: usize = 32;

/// A rectangle this thin, or this small both ways, is a piece of an outline.
const FRAGMENT_THICKNESS: usize = 2;
const FRAGMENT_SIZE: usize = 3;
/// An outline no bigger than this across the grid is the cursor.
const MAX_CURSOR_COLS: usize = 4;
const MAX_CURSOR_ROWS: usize = 2;
/// More pieces than this in one run is not a cursor.
const MAX_CURSOR_FRAGMENTS: usize = 16;
/// Minimum luma gap before a highlight and the text on it are readable.
const MIN_CONTRAST: i32 = 64;

/// Sub-cell coverage bits, one per quadrant of a cell.
pub mod ink {
    pub const TOP_LEFT: u8 = 1 << 0;
    pub const TOP_RIGHT: u8 = 1 << 1;
    pub const BOTTOM_LEFT: u8 = 1 << 2;
    pub const BOTTOM_RIGHT: u8 = 1 << 3;
}

/// The 16 quadrant block characters, indexed by an ink mask.
const QUADRANTS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

/// A square at least this big, and no bigger than [`DOT_MAX`].
const DOT_MIN: usize = 2;
const DOT_MAX: usize = 3;
/// How much of its own bounds a gathered shape must cover to count as solid.
const FILL_NUMERATOR: usize = 3;
const FILL_DENOMINATOR: usize = 5;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    /// Character drawn by the device, or 0 for none.
    pub glyph: u8,
    pub fg: Rgb,
    pub bg: Rgb,
    /// Quadrant coverage from the oscilloscope trace.
    ink: u8,
    ink_color: Rgb,
    /// A solid shape or an indicator dot the device drew here.
    mark: Option<(Mark, Rgb)>,
    /// Background from a cursor or selection outline enclosing this cell.
    pub highlight: Option<Rgb>,
}

impl Cell {
    fn blank(bg: Rgb) -> Self {
        Self {
            glyph: 0,
            fg: Rgb::default(),
            bg,
            ink: 0,
            ink_color: Rgb::default(),
            mark: None,
            highlight: None,
        }
    }
}

/// What a rectangle left in a cell.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Mark {
    /// A solid shape: a volume bar, a keyboard key, a fader.
    Block,
    /// A small indicator, like the dot beside each track.
    Dot,
}

impl Mark {
    fn character(self) -> char {
        match self {
            Mark::Block => '█',
            Mark::Dot => '•',
        }
    }
}

fn luma(c: Rgb) -> i32 {
    (c.r as i32 * 30 + c.g as i32 * 59 + c.b as i32 * 11) / 100
}

/// Returns `fg` if it stands out against `bg`.
fn readable(fg: Rgb, bg: Rgb) -> Rgb {
    if (luma(fg) - luma(bg)).abs() >= MIN_CONTRAST {
        return fg;
    }
    if luma(bg) > 127 {
        Rgb { r: 0, g: 0, b: 0 }
    } else {
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        }
    }
}

/// An outline the device drew, and what became of it.
#[derive(Copy, Clone, Debug)]
pub struct Outline {
    /// How many pieces it was made of.
    pub lines: usize,
    /// Pixel bounds, as `(x0, y0, x1, y1)`.
    pub bounds: (i32, i32, i32, i32),
    /// The cells it was taken to enclose, as `(col0, row0, col1, row1)`.
    pub cells: Option<(usize, usize, usize, usize)>,
}

/// A rectangular span of cells.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Span {
    cols: std::ops::RangeInclusive<usize>,
    rows: std::ops::RangeInclusive<usize>,
}

/// Where the cursor is, and how far its brackets reach.
#[derive(Clone, Debug)]
struct Cursor {
    highlight: Span,
    touched: Span,
}

/// A small rectangle: a piece of an outline the device has drawn.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct Fragment {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb,
}

pub struct TextScreen {
    width_px: usize,
    height_px: usize,
    metrics: Metrics,
    font_mode: Option<usize>,
    is_model_02: bool,

    grid_x: Axis,
    grid_y: Axis,
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,

    background: Rgb,
    last_color: Rgb,
    prev_waveform_len: usize,
    /// Outline pieces seen since the last [`TextScreen::flush`].
    pending_fragments: Vec<Fragment>,
    /// Where the cursor is: the cells it highlights, and its wider span.
    cursor: Option<Cursor>,
    /// What the last flush decided about each outline it found.
    outlines: Vec<Outline>,

    /// Set when any cell changed since the last repaint.
    pub dirty: bool,
    /// Set when the grid was reshaped and the terminal needs a full repaint.
    pub reshaped: bool,
}

impl TextScreen {
    pub fn new() -> Result<Self, String> {
        let metrics = font::metrics(0).ok_or("no default font")?;
        let (w, h) = MODEL_01_SIZE;
        let mut screen = Self {
            width_px: w as usize,
            height_px: h as usize,
            metrics,
            font_mode: None,
            is_model_02: false,
            grid_x: Axis::new(guess_pitch_x(&metrics), MIN_PITCH_X),
            grid_y: Axis::new(guess_pitch_y(&metrics), MIN_PITCH_Y),
            cols: 0,
            rows: 0,
            cells: Vec::new(),
            background: Rgb::default(),
            last_color: Rgb::default(),
            prev_waveform_len: 0,
            pending_fragments: Vec::new(),
            cursor: None,
            outlines: Vec::new(),
            dirty: true,
            reshaped: true,
        };
        screen.reshape();
        Ok(screen)
    }

    pub fn cell(&self, col: usize, row: usize) -> Cell {
        self.cells[row * self.cols + col]
    }

    /// The character to print for a cell, and the colours to print it in.
    pub fn glyph_at(&self, col: usize, row: usize) -> (char, Rgb, Rgb) {
        let cell = self.cells[row * self.cols + col];
        let bg = cell.highlight.unwrap_or(cell.bg);

        if cell.glyph != 0 && cell.glyph != b' ' {
            let fg = match cell.highlight {
                Some(_) => readable(cell.fg, bg),
                None => cell.fg,
            };
            return (cell.glyph as char, fg, bg);
        }
        if let Some((mark, color)) = cell.mark {
            return (mark.character(), color, bg);
        }
        if cell.ink != 0 {
            return (QUADRANTS[cell.ink as usize], cell.ink_color, bg);
        }
        (' ', cell.fg, bg)
    }

    pub fn background(&self) -> Rgb {
        self.background
    }

    pub fn last_color(&self) -> Rgb {
        self.last_color
    }

    pub fn apply(&mut self, command: &Command) {
        match command {
            Command::Rect(r) => self.draw_rect(r),
            Command::Char(c) => self.draw_char(c),
            Command::Waveform(w) => self.draw_waveform(w),
            Command::Joypad => {}
            Command::System(info) => self.apply_system_info(info),
        }
    }

    fn apply_system_info(&mut self, info: &SystemInfo) {
        let model_02 = info.is_model_02();
        if model_02 != self.is_model_02 || self.font_mode.is_none() {
            self.is_model_02 = model_02;
            let (w, h) = if model_02 {
                MODEL_02_SIZE
            } else {
                MODEL_01_SIZE
            };
            self.width_px = w as usize;
            self.height_px = h as usize;
            self.reshape();
        }

        if let Some(index) = font::mode_index(model_02, info.font_mode as usize) {
            if self.font_mode != Some(index) {
                if let Some(metrics) = font::metrics(index) {
                    self.font_mode = Some(index);
                    self.metrics = metrics;
                    self.grid_x = Axis::new(guess_pitch_x(&metrics), MIN_PITCH_X);
                    self.grid_y = Axis::new(guess_pitch_y(&metrics), MIN_PITCH_Y);
                    self.reshape();
                }
            }
        }
    }

    /// Rebuilds the grid for the current pixel size and learned pitches.
    fn reshape(&mut self) {
        let cols = self.width_px.div_ceil(self.grid_x.pitch);
        let rows = self.height_px.div_ceil(self.grid_y.pitch) + 1;

        if (cols, rows) == (self.cols, self.rows) && !self.cells.is_empty() {
            self.cells.fill(Cell::blank(self.background));
            self.dirty = true;
            self.reshaped = true;
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = vec![Cell::blank(self.background); cols * rows];
        self.dirty = true;
        self.reshaped = true;
    }

    /// Feeds one character position to the pitch detectors.
    fn learn_grid(&mut self, x: usize, cell_top: usize) {
        let changed = self.grid_x.observe(x) | self.grid_y.observe(cell_top);
        if changed {
            self.reshape();
        }
    }

    /// Screen y in pixels, with this font mode's vertical shift applied.
    fn shifted_y(&self, y: u16) -> i32 {
        y as i32 + self.metrics.screen_offset_y
    }

    fn draw_char(&mut self, c: &Char) {
        let y = self.shifted_y(c.y);
        if y < 0 {
            return;
        }
        let cell_top = (y + self.metrics.text_offset_y).max(0) as usize;
        self.learn_grid(c.x as usize, cell_top);

        let Some(col) = self.grid_x.index(c.x as usize) else {
            return;
        };
        let Some(row) = self.grid_y.index(cell_top) else {
            return;
        };
        if col >= self.cols || row >= self.rows {
            return;
        }

        let index = row * self.cols + col;
        let cell = &mut self.cells[index];
        let bg = if c.fg == c.bg { cell.bg } else { c.bg };
        let updated = Cell {
            glyph: c.c,
            fg: c.fg,
            bg,
            ink: 0,
            ink_color: Rgb::default(),
            mark: None,
            highlight: cell.highlight,
        };
        if *cell != updated {
            *cell = updated;
            self.dirty = true;
        }
    }

    fn draw_rect(&mut self, r: &Rect) {
        let y = self.shifted_y(r.y);
        self.last_color = r.color;

        if r.x == 0 && y <= 0 && r.w as usize >= self.width_px && r.h as usize >= self.height_px {
            self.background = r.color;
        }

        let x0 = r.x as i32;
        let x1 = x0 + r.w as i32;
        let y1 = y + r.h as i32;

        if is_fragment(r.w as usize, r.h as usize) {
            self.pending_fragments.push(Fragment {
                x0,
                y0: y,
                x1: x1 - 1,
                y1: y1 - 1,
                color: r.color,
            });
            return;
        }

        if (r.h as usize) * 2 >= self.grid_y.pitch {
            self.paint_solid(x0, y, x1, y1, r.color);
        }
    }

    /// Resolves the outline pieces collected during this batch of commands.
    pub fn flush(&mut self) {
        self.outlines.clear();
        if self.pending_fragments.is_empty() {
            return;
        }
        if !self.grid_x.is_known() || !self.grid_y.is_known() {
            return;
        }
        let fragments = std::mem::take(&mut self.pending_fragments);

        let mut leftovers: Vec<Fragment> = Vec::new();
        let mut candidates: Vec<(Cursor, Rgb)> = Vec::new();

        for run in fragments.chunk_by(|a, b| a.color == b.color) {
            let bounds = (
                run.iter().map(|f| f.x0).min().unwrap_or(0),
                run.iter().map(|f| f.y0).min().unwrap_or(0),
                run.iter().map(|f| f.x1).max().unwrap_or(0),
                run.iter().map(|f| f.y1).max().unwrap_or(0),
            );
            let enclosed = self.cursor_cells(run, bounds);
            self.outlines.push(Outline {
                lines: run.len(),
                bounds,
                cells: enclosed
                    .clone()
                    .map(|(cols, rows)| (*cols.start(), *rows.start(), *cols.end(), *rows.end())),
            });

            match enclosed {
                Some((cols, rows)) => {
                    let touched = Span {
                        cols: self.grid_x.index_clamped(bounds.0.max(0) as usize)
                            ..=self
                                .grid_x
                                .index_clamped(bounds.2.max(0) as usize)
                                .min(self.cols - 1),
                        rows: self.grid_y.index_clamped(bounds.1.max(0) as usize)
                            ..=self
                                .grid_y
                                .index_clamped(bounds.3.max(0) as usize)
                                .min(self.rows - 1),
                    };
                    candidates.push((
                        Cursor {
                            highlight: Span { cols, rows },
                            touched,
                        },
                        run[0].color,
                    ));
                }
                None => leftovers.extend_from_slice(run),
            }
        }

        for group in group_adjacent(leftovers) {
            let x0 = group.iter().map(|f| f.x0).min().unwrap_or(0);
            let y0 = group.iter().map(|f| f.y0).min().unwrap_or(0);
            let x1 = group.iter().map(|f| f.x1).max().unwrap_or(0);
            let y1 = group.iter().map(|f| f.y1).max().unwrap_or(0);
            let width = (x1 - x0 + 1).max(0) as usize;
            let height = (y1 - y0 + 1).max(0) as usize;

            let dot_sized = group.len() == 1
                && (DOT_MIN..=DOT_MAX).contains(&width)
                && (DOT_MIN..=DOT_MAX).contains(&height);

            let covered: usize = group
                .iter()
                .map(|f| (f.x1 - f.x0 + 1).max(0) as usize * (f.y1 - f.y0 + 1).max(0) as usize)
                .sum();
            let (pitch_x, pitch_y) = (self.grid_x.pitch, self.grid_y.pitch);
            let boxy = width * 2 >= pitch_x && height * 2 >= pitch_y;
            let bar = (width >= pitch_x * 2 && height * 3 >= pitch_y)
                || (height >= pitch_y * 2 && width * 3 >= pitch_x);
            let filled = covered * FILL_DENOMINATOR >= width * height * FILL_NUMERATOR;
            let solid = filled && (boxy || bar);

            let mark = match (dot_sized, solid) {
                (true, _) => Mark::Dot,
                (_, true) => Mark::Block,
                _ => continue,
            };
            for f in group {
                self.mark_cells(f.x0, f.y0, f.x1, f.y1, mark, f.color);
            }
        }

        if let Some((cursor, color)) = self.pick_cursor(candidates) {
            self.move_cursor(cursor, color);
        }
    }

    /// Which of a frame's cursor-shaped outlines is the cursor.
    ///
    /// More than one box in a frame can pass for a cursor: the device draws
    /// indicators the same shape, in the panels and on the meters. Whichever was
    /// the cursor last frame still is, so a box elsewhere cannot take the
    /// highlight off it and hand it back on the next frame, which read as the
    /// cursor flickering wherever the screen was busy.
    fn pick_cursor(&self, mut candidates: Vec<(Cursor, Rgb)>) -> Option<(Cursor, Rgb)> {
        let standing = self.cursor.as_ref().map(|cursor| &cursor.highlight);
        let at = candidates
            .iter()
            .position(|(cursor, _)| Some(&cursor.highlight) == standing);
        match at {
            Some(at) => Some(candidates.swap_remove(at)),
            None => candidates.pop(),
        }
    }

    /// What a cell holds besides text, as a short description.
    pub fn ink_debug(&self, col: usize, row: usize) -> Option<String> {
        let cell = self.cells[row * self.cols + col];
        match (cell.mark, cell.ink) {
            (Some((mark, c)), _) => Some(format!("{mark:?} rgb {},{},{}", c.r, c.g, c.b)),
            (None, 0) => None,
            (None, mask) => Some(format!("trace mask {mask}")),
        }
    }

    /// What the last [`TextScreen::flush`] made of the outlines it saw.
    pub fn outlines(&self) -> &[Outline] {
        &self.outlines
    }

    /// The cells an outline encloses.
    fn cursor_cells(
        &self,
        run: &[Fragment],
        (x0, y0, x1, y1): (i32, i32, i32, i32),
    ) -> Option<(
        std::ops::RangeInclusive<usize>,
        std::ops::RangeInclusive<usize>,
    )> {
        // Drawn in the ground colour it is an erase, not a highlight: taking it
        // for the cursor would paint a cell the colour it already is.
        if run.first().is_some_and(|f| f.color == self.background) {
            return None;
        }
        // The device redraws the cursor more than once in a frame, and identical
        // redraws chunk into one run, so it is the distinct edges that are the
        // outline's own.
        let mut distinct = 0;
        for (at, f) in run.iter().enumerate() {
            if !run[..at].iter().any(|seen| seen == f) {
                distinct += 1;
            }
        }
        if distinct > MAX_CURSOR_FRAGMENTS {
            return None;
        }
        if x1 - x0 < FRAGMENT_SIZE as i32 || y1 - y0 < FRAGMENT_SIZE as i32 {
            return None;
        }
        // An outline narrower or shorter than a cell is not enclosing one. The
        // device draws boxes that size as indicators in the side panels, and
        // they would otherwise pass for the cursor and take the highlight.
        if x1 - x0 + 1 < self.grid_x.pitch as i32 || y1 - y0 + 1 < self.grid_y.pitch as i32 {
            return None;
        }

        let cols = self.grid_x.centres_within(x0, x1, self.cols)?;
        let rows = self.grid_y.centres_within(y0, y1, self.rows)?;
        if cols.end() - cols.start() + 1 > MAX_CURSOR_COLS
            || rows.end() - rows.start() + 1 > MAX_CURSOR_ROWS
        {
            return None;
        }
        Some((cols, rows))
    }

    /// Highlights the cursor's cells, clearing wherever it was before.
    fn move_cursor(&mut self, cursor: Cursor, color: Rgb) {
        let moved = self.cursor.as_ref().map(|c| &c.highlight) != Some(&cursor.highlight);
        if !moved {
            return;
        }
        if let Some(previous) = self.cursor.take() {
            self.scrub(&previous.touched);
        }
        self.scrub(&cursor.touched);

        for row in cursor.highlight.rows.clone() {
            for col in cursor.highlight.cols.clone() {
                let Some(cell) = self.cells.get_mut(row * self.cols + col) else {
                    continue;
                };
                if cell.highlight != Some(color) {
                    cell.highlight = Some(color);
                    self.dirty = true;
                }
            }
        }
        self.cursor = Some(cursor);
    }

    /// Removes any highlight and trace ink from a span of cells.
    fn scrub(&mut self, span: &Span) {
        for row in span.rows.clone() {
            for col in span.cols.clone() {
                let Some(cell) = self.cells.get_mut(row * self.cols + col) else {
                    continue;
                };
                if cell.highlight.is_some() || cell.ink != 0 {
                    cell.highlight = None;
                    cell.ink = 0;
                    self.dirty = true;
                }
            }
        }
    }

    fn draw_waveform(&mut self, w: &Waveform) {
        let len = if w.samples.is_empty() {
            self.prev_waveform_len
        } else {
            w.samples.len()
        };
        self.prev_waveform_len = w.samples.len();
        if len == 0 {
            return;
        }

        let strip_x = self.width_px.saturating_sub(len) as i32;
        let strip_h = self.metrics.waveform_max_height as i32 + 1;
        self.clear_area(strip_x, 0, self.width_px as i32, strip_h);

        let max = self.metrics.waveform_max_height;
        for (i, &sample) in w.samples.iter().enumerate() {
            let x = strip_x + i as i32;
            let y = sample.min(max) as i32;
            self.trace_pixel(x, y, w.color);
        }
    }

    /// Fills pixel-space rectangle `[x0,x1) x [y0,y1)` with `color`.
    fn paint_solid(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgb) {
        let cols = self.grid_x.cells_for(x0, x1 - 1, self.cols);
        let rows = self.grid_y.cells_for(y0, y1 - 1, self.rows);
        let (Some(cols), Some(rows)) = (cols, rows) else {
            return;
        };

        for row in rows {
            for col in cols.clone() {
                let covers_cell = x0 <= self.grid_x.start_of(col) as i32
                    && y0 <= self.grid_y.start_of(row) as i32
                    && x1 >= (self.grid_x.start_of(col) + self.grid_x.pitch) as i32
                    && y1 >= (self.grid_y.start_of(row) + self.grid_y.pitch) as i32;
                let cell = &mut self.cells[row * self.cols + col];
                let updated = if covers_cell {
                    Cell::blank(color)
                } else {
                    Cell {
                        mark: Some((Mark::Block, color)),
                        ..*cell
                    }
                };
                if *cell != updated {
                    *cell = updated;
                    self.dirty = true;
                }
            }
        }
    }

    /// Marks the cells a shape covers, without touching their background.
    fn mark_cells(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, mark: Mark, color: Rgb) {
        let cols = self.grid_x.cells_for(x0, x1, self.cols);
        let rows = self.grid_y.cells_for(y0, y1, self.rows);
        let (Some(cols), Some(rows)) = (cols, rows) else {
            return;
        };

        for row in rows {
            for col in cols.clone() {
                let cell = &mut self.cells[row * self.cols + col];
                if cell.mark != Some((mark, color)) {
                    cell.mark = Some((mark, color));
                    self.dirty = true;
                }
            }
        }
    }

    /// Records one oscilloscope sample as sub-cell ink.
    fn trace_pixel(&mut self, x: i32, y: i32, color: Rgb) {
        if x < 0 || y < 0 {
            return;
        }
        let (Some(col), Some(row)) = (self.grid_x.index(x as usize), self.grid_y.index(y as usize))
        else {
            return;
        };
        if col >= self.cols || row >= self.rows {
            return;
        }
        let quadrant = {
            let in_right = x as usize - self.grid_x.start_of(col) >= self.grid_x.pitch / 2;
            let in_bottom = y as usize - self.grid_y.start_of(row) >= self.grid_y.pitch / 2;
            match (in_bottom, in_right) {
                (false, false) => ink::TOP_LEFT,
                (false, true) => ink::TOP_RIGHT,
                (true, false) => ink::BOTTOM_LEFT,
                (true, true) => ink::BOTTOM_RIGHT,
            }
        };
        let cell = &mut self.cells[row * self.cols + col];
        if cell.ink & quadrant == 0 || cell.ink_color != color {
            cell.ink |= quadrant;
            cell.ink_color = color;
            self.dirty = true;
        }
    }

    /// Resets pixel-space rectangle `[x0,x1) x [y0,y1)` to the background.
    fn clear_area(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let cols = self.grid_x.cells_for(x0, x1 - 1, self.cols);
        let rows = self.grid_y.cells_for(y0, y1 - 1, self.rows);
        let (Some(cols), Some(rows)) = (cols, rows) else {
            return;
        };
        let background = self.background;

        for row in rows {
            for col in cols.clone() {
                let cell = &mut self.cells[row * self.cols + col];
                let updated = Cell {
                    ink: 0,
                    ink_color: Rgb::default(),
                    mark: None,
                    bg: background,
                    highlight: None,
                    ..*cell
                };
                if *cell != updated {
                    *cell = updated;
                    self.dirty = true;
                }
            }
        }
    }

    /// Clears the whole grid, used on reconnect.
    pub fn clear(&mut self) {
        self.cells.fill(Cell::blank(self.background));
        self.dirty = true;
        self.reshaped = true;
    }
}

/// One axis of the character grid.
struct Axis {
    pitch: usize,
    min_pitch: usize,
    /// Distinct positions seen, capped.
    positions: std::collections::BTreeSet<usize>,
}

/// Distinct positions kept per axis. A Model:02 shows 40 columns.
const POSITION_SAMPLES: usize = 128;

impl Axis {
    fn new(guess: usize, min_pitch: usize) -> Self {
        Self {
            pitch: guess.clamp(min_pitch, MAX_PITCH),
            min_pitch,
            positions: std::collections::BTreeSet::new(),
        }
    }

    /// Feeds in a position. Returns true if the grid changed.
    fn observe(&mut self, position: usize) -> bool {
        if self.positions.contains(&position) {
            return false;
        }
        if self.positions.len() >= POSITION_SAMPLES {
            return false;
        }
        self.positions.insert(position);

        let before = (self.pitch, self.origin());
        if let Some(pitch) = self.common_gap() {
            self.pitch = pitch;
        }
        before != (self.pitch, self.origin())
    }

    /// The most common gap between neighbouring positions, if there is one.
    fn common_gap(&self) -> Option<usize> {
        let mut counts: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();
        let mut previous = None;
        for &position in &self.positions {
            if let Some(previous) = previous {
                let gap = position - previous;
                if (self.min_pitch..=MAX_PITCH).contains(&gap) {
                    *counts.entry(gap).or_insert(0) += 1;
                }
            }
            previous = Some(position);
        }
        counts
            .into_iter()
            .max_by_key(|&(gap, count)| (count, std::cmp::Reverse(gap)))
            .map(|(gap, _)| gap)
    }

    /// True once a coordinate has been seen, so the phase is known.
    fn is_known(&self) -> bool {
        !self.positions.is_empty()
    }

    /// Offset of the first cell from zero, i.e. the grid's phase.
    fn origin(&self) -> usize {
        self.positions.first().map_or(0, |first| first % self.pitch)
    }

    /// Cell index containing `position`, or None if it is before the grid.
    fn index(&self, position: usize) -> Option<usize> {
        position
            .checked_sub(self.origin())
            .map(|offset| offset / self.pitch)
    }

    fn index_clamped(&self, position: usize) -> usize {
        self.index(position).unwrap_or(0)
    }

    /// First pixel of a cell.
    fn start_of(&self, index: usize) -> usize {
        self.origin() + index * self.pitch
    }

    /// The cells a pixel span `lo..=hi` occupies.
    fn cells_for(&self, lo: i32, hi: i32, count: usize) -> Option<std::ops::RangeInclusive<usize>> {
        if count == 0 || hi < 0 {
            return None;
        }
        let lo = lo.max(0) as usize;
        let hi = hi.max(0) as usize;
        if hi < lo {
            return None;
        }
        let first = self.index(lo).unwrap_or(0).min(count - 1);
        let last = self.index(hi).unwrap_or(0).min(count - 1);

        let overlap = |index: usize| {
            let start = self.start_of(index);
            let end = start + self.pitch - 1;
            (hi.min(end) + 1).saturating_sub(lo.max(start))
        };

        if hi - lo + 1 >= self.pitch {
            let mut covered = (first..=last).filter(|&index| overlap(index) * 2 >= self.pitch);
            let first_covered = covered.next();
            let last_covered = covered.last().or(first_covered);
            if let (Some(first), Some(last)) = (first_covered, last_covered) {
                return Some(first..=last);
            }
        }
        let best = (first..=last).max_by_key(|&index| overlap(index))?;
        Some(best..=best)
    }

    /// The cells whose centres fall inside the pixel span `x0..=x1`.
    fn centres_within(
        &self,
        x0: i32,
        x1: i32,
        count: usize,
    ) -> Option<std::ops::RangeInclusive<usize>> {
        let centre = |index: usize| (self.start_of(index) + self.pitch / 2) as i32;
        let first = (0..count).find(|&index| centre(index) >= x0)?;
        let last = (first..count)
            .take_while(|&index| centre(index) <= x1)
            .last()?;
        Some(first..=last)
    }
}

/// Gathers pieces into the shapes they belong to. Touching is not enough to
/// join: the meters touch the panel frame, and grouping on touch alone chains
/// the whole panel into one shape that then looks like nothing at all.
fn group_adjacent(fragments: Vec<Fragment>) -> Vec<Vec<Fragment>> {
    let touches = |a: &Fragment, b: &Fragment| {
        let overlapping =
            a.x0 <= b.x1 + 1 && b.x0 <= a.x1 + 1 && a.y0 <= b.y1 + 1 && b.y0 <= a.y1 + 1;
        let same_rows = a.y0 == b.y0 && a.y1 == b.y1;
        let same_cols = a.x0 == b.x0 && a.x1 == b.x1;
        overlapping && (same_rows || same_cols)
    };

    let mut groups: Vec<Vec<Fragment>> = Vec::new();
    for fragment in fragments {
        let mut joined: Vec<usize> = (0..groups.len())
            .filter(|&index| groups[index].iter().any(|f| touches(f, &fragment)))
            .collect();

        match joined.pop() {
            None => groups.push(vec![fragment]),
            Some(target) => {
                groups[target].push(fragment);
                for index in joined.into_iter().rev() {
                    let merged = groups.remove(index);
                    let target = if index < target { target - 1 } else { target };
                    groups[target].extend(merged);
                }
            }
        }
    }
    groups
}

/// True for a rectangle small enough to be a piece of an outline.
fn is_fragment(w: usize, h: usize) -> bool {
    w.min(h) <= FRAGMENT_THICKNESS || (w <= FRAGMENT_SIZE && h <= FRAGMENT_SIZE)
}

/// Starting guesses for the grid pitch, refined from the stream at runtime.
fn guess_pitch_x(metrics: &Metrics) -> usize {
    metrics.glyph_w + 2
}

fn guess_pitch_y(metrics: &Metrics) -> usize {
    metrics.glyph_h + 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Char, Rect, Waveform};

    const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };
    const CURSOR: Rgb = Rgb {
        r: 88,
        g: 140,
        b: 168,
    };

    /// A screen set up the way a Model:02 in its large font reports itself.
    fn model_02() -> TextScreen {
        let mut screen = TextScreen::new().unwrap();
        screen.apply(&Command::System(SystemInfo {
            hardware: 3,
            version: (6, 0, 0),
            font_mode: 1,
        }));
        screen
    }

    fn text(screen: &mut TextScreen, c: u8, x: u16, y: u16) {
        screen.apply(&Command::Char(Char {
            c,
            x,
            y,
            fg: WHITE,
            bg: BLACK,
        }));
    }

    fn rect(screen: &mut TextScreen, x: u16, y: u16, w: u16, h: u16, color: Rgb) {
        screen.apply(&Command::Rect(Rect { x, y, w, h, color }));
    }

    /// The twelve rectangles a Model:02 draws its cursor from.
    fn cursor_brackets(screen: &mut TextScreen, color: Rgb) {
        for (x, y, w, h) in [
            (79, 84, 3, 1),
            (108, 84, 3, 1),
            (79, 101, 3, 1),
            (108, 101, 3, 1),
            (79, 85, 1, 2),
            (79, 99, 1, 2),
            (80, 85, 1, 1),
            (80, 100, 1, 1),
            (110, 85, 1, 2),
            (110, 99, 1, 2),
            (109, 85, 1, 1),
            (109, 100, 1, 1),
        ] {
            rect(screen, x, y, w, h, color);
        }
    }

    /// A second cursor-shaped box, a few cells along from the real one: the shape
    /// the device draws for indicators and selections elsewhere on the screen.
    fn rival_brackets(screen: &mut TextScreen, color: Rgb) {
        for (x, y, w, h) in [
            (199, 84, 3, 1),
            (228, 84, 3, 1),
            (199, 101, 3, 1),
            (228, 101, 3, 1),
            (199, 85, 1, 2),
            (199, 99, 1, 2),
            (230, 85, 1, 2),
            (230, 99, 1, 2),
        ] {
            rect(screen, x, y, w, h, color);
        }
    }

    /// A box over the centre of cell (7, 6) but smaller than the 12x14 cell it
    /// sits in: the size the device draws indicators in its side panels.
    fn subcell_box(screen: &mut TextScreen, color: Rgb) {
        for (x, y, w, h) in [
            (86, 88, 10, 1),
            (86, 99, 10, 1),
            (86, 89, 1, 10),
            (95, 89, 1, 10),
        ] {
            rect(screen, x, y, w, h, color);
        }
    }

    #[test]
    fn the_device_grid_is_recovered_from_character_positions() {
        let mut screen = model_02();
        for (index, x) in [0, 12, 24, 84, 96].into_iter().enumerate() {
            text(&mut screen, b'0' + index as u8, x, 84);
        }
        assert_eq!(screen.grid_x.pitch, 12);
        assert_eq!(screen.cols, 40);
        assert_eq!(screen.glyph_at(0, 6).0, '0');
        assert_eq!(screen.glyph_at(7, 6).0, '3');
        assert_eq!(screen.glyph_at(8, 6).0, '4');
    }

    #[test]
    fn a_cursor_highlights_its_characters_and_nothing_else() {
        let mut screen = model_02();
        text(&mut screen, b'2', 84, 84);
        text(&mut screen, b'1', 96, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();

        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));
        assert_eq!(screen.cell(8, 6).highlight, Some(CURSOR));
        assert_eq!(screen.cell(6, 6).highlight, None);
        assert_eq!(screen.cell(9, 6).highlight, None);
        assert_eq!(screen.cell(7, 7).highlight, None);
    }

    #[test]
    fn a_cursor_leaves_no_ink_in_the_cells_it_reaches_into() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        for col in 6..=9 {
            assert_eq!(
                screen.glyph_at(col, 6).0,
                ' ',
                "column {col} kept bracket ink"
            );
            assert_eq!(
                screen.glyph_at(col, 7).0,
                ' ',
                "column {col} inked the row below"
            );
        }
    }

    #[test]
    fn text_under_a_cursor_keeps_its_highlight_and_stays_readable() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        screen.apply(&Command::Char(Char {
            c: b'7',
            x: 84,
            y: 84,
            fg: CURSOR,
            bg: BLACK,
        }));

        let (ch, fg, bg) = screen.glyph_at(7, 6);
        assert_eq!(ch, '7');
        assert_eq!(bg, CURSOR);
        assert_ne!(fg, CURSOR);
    }

    /// A box in a side panel, drawn small and in the ground colour, used to pass
    /// for the cursor: it took the highlight, the real cursor took it back on the
    /// next frame, and the cell flickered for as long as the panel kept redrawing.
    #[test]
    fn an_indicator_elsewhere_cannot_take_the_highlight_off_the_cursor() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));

        screen.dirty = false;
        for _ in 0..4 {
            // The real cursor, and a rival box drawn after it.
            cursor_brackets(&mut screen, CURSOR);
            rival_brackets(&mut screen, WHITE);
            screen.flush();
            assert_eq!(
                screen.cell(7, 6).highlight,
                Some(CURSOR),
                "the cursor lost its highlight to the rival box"
            );
        }
        assert!(!screen.dirty, "the cursor was repainted every frame");
    }

    /// Too small to be enclosing a cell, so not the cursor.
    #[test]
    fn a_box_smaller_than_a_cell_is_not_a_cursor() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        subcell_box(&mut screen, WHITE);
        screen.flush();
        assert!(
            screen.cells.iter().all(|cell| cell.highlight.is_none()),
            "a sub-cell box was taken for the cursor"
        );
    }

    /// Nothing enclosing, so nothing to highlight.
    #[test]
    fn an_outline_drawn_in_the_ground_colour_is_an_erase_not_a_cursor() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        let ground = screen.background();
        cursor_brackets(&mut screen, ground);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, None);
    }

    /// The device draws the cursor more than once per frame; identical redraws
    /// chunk into one run, which used to push it past the fragment limit and
    /// lose it for that frame.
    #[test]
    fn a_cursor_drawn_twice_in_one_frame_is_still_the_cursor() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));
    }

    #[test]
    fn a_breathing_cursor_holds_one_colour_and_stops_repainting() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));

        screen.dirty = false;
        for shade in [96, 104, 112, 120] {
            cursor_brackets(
                &mut screen,
                Rgb {
                    r: shade,
                    g: 200,
                    b: 240,
                },
            );
            screen.flush();
        }
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));
        assert!(
            !screen.dirty,
            "a breathing cursor kept marking the screen dirty"
        );
    }

    #[test]
    fn a_moving_cursor_takes_its_highlight_with_it() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));

        for (x, y, w, h) in [
            (79, 98, 3, 1),
            (108, 98, 3, 1),
            (79, 115, 3, 1),
            (108, 115, 3, 1),
            (79, 99, 1, 2),
            (110, 99, 1, 2),
        ] {
            rect(&mut screen, x, y, w, h, CURSOR);
        }
        screen.flush();

        assert_eq!(
            screen.cell(7, 6).highlight,
            None,
            "old position stayed highlighted"
        );
        assert_eq!(screen.cell(7, 7).highlight, Some(CURSOR));
    }

    #[test]
    fn a_panel_frame_is_dropped_rather_than_highlighted() {
        let mut screen = model_02();
        text(&mut screen, b'x', 0, 84);
        for (x, y, w, h) in [
            (408, 224, 57, 1),
            (408, 225, 1, 19),
            (464, 225, 1, 19),
            (409, 244, 55, 1),
        ] {
            rect(&mut screen, x, y, w, h, CURSOR);
        }
        screen.flush();
        assert!(screen.cells.iter().all(|cell| cell.highlight.is_none()));
        assert!(screen
            .cells
            .iter()
            .all(|cell| cell.mark.is_none() && cell.ink == 0));
    }

    #[test]
    fn outlines_wait_for_the_grid_phase() {
        let mut screen = model_02();
        cursor_brackets(&mut screen, CURSOR);
        screen.flush();
        assert!(screen
            .cells
            .iter()
            .all(|cell| cell.highlight.is_none() && cell.ink == 0));

        text(&mut screen, b'2', 84, 84);
        screen.flush();
        assert_eq!(screen.cell(7, 6).highlight, Some(CURSOR));
    }

    #[test]
    fn an_axis_learns_its_pitch_from_the_gaps_between_positions() {
        let mut axis = Axis::new(99, MIN_PITCH_Y);
        for position in [86, 100, 114] {
            axis.observe(position);
        }
        assert_eq!(axis.pitch, 14);
        assert_eq!(axis.origin(), 2);
        assert_eq!(axis.index(86), Some(6));
        assert_eq!(axis.index(100), Some(7));
    }

    #[test]
    fn an_axis_ignores_positions_that_would_shrink_the_pitch_absurdly() {
        let mut axis = Axis::new(12, MIN_PITCH_X);
        for position in [0, 12, 24, 36, 48] {
            axis.observe(position);
        }
        assert_eq!(axis.pitch, 12);
        axis.observe(25);
        assert_eq!(axis.pitch, 12);
        for position in [60, 72, 84] {
            axis.observe(position);
        }
        assert_eq!(axis.pitch, 12);
    }

    #[test]
    fn an_axis_recovers_from_a_coarse_early_guess() {
        let mut axis = Axis::new(12, MIN_PITCH_X);
        axis.observe(0);
        axis.observe(18);
        for position in [12, 24, 36, 48, 60, 72] {
            axis.observe(position);
        }
        assert_eq!(axis.pitch, 12);
    }

    #[test]
    fn cells_whose_centres_fall_inside_a_span_are_the_enclosed_ones() {
        let mut axis = Axis::new(12, MIN_PITCH_X);
        axis.observe(0);
        assert_eq!(axis.centres_within(79, 110, 40), Some(7..=8));
    }

    #[test]
    fn a_rectangle_is_a_fragment_when_it_is_small_enough_to_be_an_outline() {
        assert!(is_fragment(3, 1)); // cursor bracket, horizontal
        assert!(is_fragment(1, 2)); // cursor bracket, vertical
        assert!(is_fragment(1, 19)); // panel frame edge
        assert!(is_fragment(57, 1)); // panel frame edge
        assert!(!is_fragment(5, 10)); // a fader: drawn for its own sake
        assert!(!is_fragment(49, 7));
    }

    #[test]
    fn the_volume_bars_become_a_row_of_blocks_each() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        let left = Rgb {
            r: 10,
            g: 200,
            b: 10,
        };
        let right = Rgb {
            r: 200,
            g: 10,
            b: 10,
        };
        rect(&mut screen, 408, 42, 49, 7, left);
        rect(&mut screen, 408, 49, 49, 7, right);
        screen.flush();

        assert_eq!(screen.cell(38, 2).mark, None);
        for col in 34..=37 {
            assert_eq!(screen.cell(col, 2).mark, Some((Mark::Block, left)));
            assert_eq!(screen.cell(col, 3).mark, Some((Mark::Block, right)));
            assert_eq!(screen.glyph_at(col, 2).0, '█');
        }
    }

    #[test]
    fn a_volume_meter_drawn_as_a_colour_gradient_becomes_blocks() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);

        rect(&mut screen, 408, 42, 49, 7, BLACK);
        rect(&mut screen, 408, 49, 49, 7, BLACK);
        for (step, x) in (409..=450).enumerate() {
            let shade = Rgb {
                r: 248 - step as u8 * 3,
                g: 4 + step as u8 * 5,
                b: 216,
            };
            rect(&mut screen, x, 42, 1, 6, shade);
            rect(&mut screen, x, 49, 1, 6, shade);
        }
        rect(
            &mut screen,
            456,
            42,
            1,
            6,
            Rgb {
                r: 192,
                g: 208,
                b: 224,
            },
        );
        screen.flush();

        for row in [2, 3] {
            for col in 34..=37 {
                match screen.cell(col, row).mark {
                    Some((Mark::Block, color)) => assert_ne!(
                        color, BLACK,
                        "meter cell {col},{row} kept the colour of the clear"
                    ),
                    other => panic!("meter cell {col},{row} was {other:?}"),
                }
            }
        }
    }

    #[test]
    fn the_peak_hold_line_on_a_volume_bar_is_dropped() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        for x in [409, 421, 433] {
            rect(
                &mut screen,
                x,
                49,
                1,
                6,
                Rgb {
                    r: 248,
                    g: 4,
                    b: 216,
                },
            );
        }
        screen.flush();
        assert!(screen
            .cells
            .iter()
            .all(|cell| cell.mark.is_none() && cell.ink == 0));
    }

    #[test]
    fn the_battery_gauge_is_dropped() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        rect(
            &mut screen,
            408,
            63,
            15,
            5,
            Rgb {
                r: 40,
                g: 40,
                b: 40,
            },
        );
        for (x, y, w, h) in [
            (409, 63, 13, 1),
            (409, 64, 12, 2),
            (409, 66, 13, 1),
            (422, 64, 1, 2),
        ] {
            rect(&mut screen, x, y, w, h, CURSOR);
        }
        screen.flush();
        assert!(screen
            .cells
            .iter()
            .all(|cell| cell.mark.is_none() && cell.ink == 0));
    }

    #[test]
    fn the_midi_keyboard_keys_become_one_block_each() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        for x in [414, 422, 438, 446, 454] {
            rect(&mut screen, x, 224, 5, 10, CURSOR);
        }
        for (x, y, w, h) in [(408, 224, 57, 1), (416, 235, 1, 9), (424, 235, 1, 9)] {
            rect(&mut screen, x, y, w, h, CURSOR);
        }
        screen.flush();

        let marked: Vec<(usize, usize)> = (0..screen.rows)
            .flat_map(|row| (0..screen.cols).map(move |col| (col, row)))
            .filter(|&(col, row)| screen.cell(col, row).mark.is_some())
            .collect();
        let rows: Vec<usize> = marked.iter().map(|&(_, row)| row).collect();
        assert!(
            rows.windows(2).all(|w| w[0] == w[1]),
            "keys spread over rows: {marked:?}"
        );
        let cols: Vec<usize> = marked.iter().map(|&(col, _)| col).collect();
        assert_eq!(cols, vec![34, 35, 36, 37, 38]);
    }

    #[test]
    fn track_indicator_dots_survive_as_dots() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        rect(&mut screen, 422, 106, 3, 3, CURSOR);
        screen.flush();
        assert_eq!(screen.glyph_at(35, 7).0, '•');
    }

    #[test]
    fn the_oscilloscope_still_draws_as_a_trace() {
        let mut screen = model_02();
        text(&mut screen, b'x', 84, 84);
        screen.apply(&Command::Waveform(Waveform {
            color: Rgb { r: 0, g: 255, b: 0 },
            samples: vec![20, 21, 22, 23],
        }));
        assert!(QUADRANTS.contains(&screen.glyph_at(39, 1).0));
        assert_ne!(screen.glyph_at(39, 1).0, ' ');
    }

    #[test]
    fn highlighted_text_is_inverted_only_when_it_would_be_unreadable() {
        assert_eq!(readable(WHITE, BLACK), WHITE);
        assert_eq!(readable(CURSOR, CURSOR), WHITE);
        assert_eq!(readable(WHITE, WHITE), BLACK);
    }
}
