//! Terminal plumbing for the text frontend.

use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::{Duration, Instant};

use crate::config::{ColorDepth, Config, Theme};
use crate::keys::{FACE_BUTTONS, FACE_SLOT_COLS, FACE_SLOT_ROWS};
use crate::proto::Rgb;
use crate::text::TextScreen;

/// Kitty keyboard flags: disambiguate, report event types, report all keys.
const KEYBOARD_FLAGS: u8 = 1 | 2 | 8;

/// Everything needed to put the terminal back as it was found. Popping the
/// keyboard flags matters most: a terminal left reporting releases sends two
/// events per keypress, and a shell acting on both doubles every key typed.
const RESTORE: &str = "\x1b[<1u\x1b[=0;1u\x1b[0m\x1b[?25h\x1b[?1049l";

/// The terminal settings to restore, for the signal handler to reach.
static SAVED_TERMIOS: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());

/// Puts the terminal back and exits.
extern "C" fn restore_on_signal(signal: libc::c_int) {
    unsafe {
        let saved = SAVED_TERMIOS.load(Ordering::SeqCst);
        if !saved.is_null() {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved);
        }
        libc::write(
            libc::STDOUT_FILENO,
            RESTORE.as_ptr() as *const libc::c_void,
            RESTORE.len(),
        );
        libc::_exit(128 + signal);
    }
}

const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
const HIDE_CURSOR: &str = "\x1b[?25l";
const CLEAR_SCREEN: &str = "\x1b[2J";

/// How much colour the terminal can take.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Colors {
    TrueColor,
    Ansi256,
}

impl Colors {
    /// Resolves a configured depth.
    pub fn resolve(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => Colors::TrueColor,
            ColorDepth::Ansi256 => Colors::Ansi256,
            ColorDepth::Auto => Colors::detect(),
        }
    }

    /// Guesses from the environment, the way most terminal programs do.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Colors::TrueColor;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term.contains("direct") || term.contains("kitty") || term.contains("alacritty") {
            return Colors::TrueColor;
        }
        Colors::Ansi256
    }

    fn sgr(self, c: Rgb, foreground: bool) -> String {
        let layer = if foreground { 38 } else { 48 };
        match self {
            Colors::TrueColor => format!("\x1b[{layer};2;{};{};{}m", c.r, c.g, c.b),
            Colors::Ansi256 => format!("\x1b[{layer};5;{}m", ansi256(c)),
        }
    }
}

/// Maps a colour onto the 256-colour palette.
fn ansi256(c: Rgb) -> u8 {
    let level = |v: u8| -> u8 {
        match v {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    };
    const CUBE_VALUES: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let (r, g, b) = (level(c.r), level(c.g), level(c.b));
    let cube_error = (CUBE_VALUES[r as usize] - c.r as i32).pow(2)
        + (CUBE_VALUES[g as usize] - c.g as i32).pow(2)
        + (CUBE_VALUES[b as usize] - c.b as i32).pow(2);

    let luma = (c.r as i32 * 30 + c.g as i32 * 59 + c.b as i32 * 11) / 100;
    let step = ((luma - 8).clamp(0, 238) + 5) / 10;
    let gray_value = 8 + step * 10;
    let gray_error = 3 * (gray_value - luma).pow(2);

    if gray_error < cube_error {
        (232 + step.clamp(0, 23)) as u8
    } else {
        16 + 36 * r + 6 * g + b
    }
}

const KEY_WIDTH: usize = 7;
const KEY_GAP: usize = 1;
/// Blank row between the M8's screen and the keypad.
const KEYPAD_MARGIN: usize = 1;

const KEY_UP_BG: Rgb = Rgb { r: 38, g: 40, b: 48 };
const KEY_UP_FG: Rgb = Rgb { r: 150, g: 155, b: 165 };
const KEY_DOWN_BG: Rgb = Rgb { r: 120, g: 200, b: 240 };
const KEY_DOWN_FG: Rgb = Rgb { r: 10, g: 12, b: 16 };

const MENU_BG: Rgb = Rgb { r: 24, g: 26, b: 32 };
const MENU_FG: Rgb = Rgb { r: 220, g: 225, b: 235 };

fn keypad_width() -> usize {
    FACE_SLOT_COLS * (KEY_WIDTH + KEY_GAP) - KEY_GAP
}

/// A lone escape byte is ambiguous until more arrive, or enough time passes.
const ESCAPE_AMBIGUITY_WINDOW: Duration = Duration::from_millis(30);

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Escape,
    Tab,
    Enter,
    Backspace,
    Delete,
    /// A function key, `F1` upwards.
    Function(u8),
    /// A key we have no name for, identified by its protocol code.
    Other(u32),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum KeyKind {
    Press,
    Repeat,
    Release,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub kind: KeyKind,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Restores the terminal on the way out, including on error paths.
pub struct Terminal {
    original: libc::termios,
    out: io::BufWriter<io::Stdout>,
    /// Previously painted cells, for diffing.
    painted: Vec<PaintedCell>,
    painted_cols: usize,
    painted_rows: usize,
    origin: (usize, usize),
    pub size: (usize, usize),
    colors: Colors,
    theme: Theme,
    /// Button state the keypad was last drawn for.
    painted_keys: Option<u8>,
}

/// How to draw one cell.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct Style {
    fg: Rgb,
    bg: Rgb,
    reverse: bool,
    device_colors: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
struct PaintedCell {
    ch: char,
    style: Style,
}

impl Default for PaintedCell {
    fn default() -> Self {
        Self {
            ch: '\0',
            style: Style {
                fg: Rgb::default(),
                bg: Rgb::default(),
                reverse: false,
                device_colors: true,
            },
        }
    }
}

impl Terminal {
    pub fn new(config: &Config) -> Result<Self, String> {
        let stdin = io::stdin();
        let fd = stdin.as_raw_fd();
        if unsafe { libc::isatty(fd) } != 1 {
            return Err("stdin is not a terminal".into());
        }

        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(format!("cannot read terminal settings: {}", io::Error::last_os_error()));
        }

        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(format!("cannot set raw mode: {}", io::Error::last_os_error()));
        }

        SAVED_TERMIOS.store(Box::leak(Box::new(original)), Ordering::SeqCst);
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            let handler = restore_on_signal as *const () as libc::sighandler_t;
            unsafe { libc::signal(signal, handler) };
        }
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let saved = SAVED_TERMIOS.load(Ordering::SeqCst);
            if !saved.is_null() {
                unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved) };
            }
            let _ = io::stdout().write_all(RESTORE.as_bytes());
            let _ = io::stdout().flush();
            previous_hook(info);
        }));

        let mut terminal = Self {
            original,
            out: io::BufWriter::with_capacity(1 << 16, io::stdout()),
            painted: Vec::new(),
            painted_cols: 0,
            painted_rows: 0,
            origin: (0, 0),
            size: (80, 24),
            colors: Colors::resolve(config.color_depth),
            theme: config.theme,
            painted_keys: None,
        };

        write!(
            terminal.out,
            "{ENTER_ALT_SCREEN}{HIDE_CURSOR}\x1b[>{KEYBOARD_FLAGS}u{CLEAR_SCREEN}"
        )
        .map_err(|e| e.to_string())?;
        terminal.out.flush().map_err(|e| e.to_string())?;
        terminal.size = terminal_size();
        Ok(terminal)
    }

    /// Applies changed settings, forcing a full repaint.
    pub fn reconfigure(&mut self, config: &Config) {
        self.colors = Colors::resolve(config.color_depth);
        self.theme = config.theme;
        self.painted.clear();
        self.painted_keys = None;
    }

    /// The escape sequence that selects a style.
    fn sgr(&self, style: Style) -> String {
        if !style.device_colors || self.theme == Theme::Terminal {
            let reverse = if style.reverse { "\x1b[7m" } else { "\x1b[27m" };
            return format!("\x1b[39;49m{reverse}");
        }
        format!(
            "\x1b[27m{}{}",
            self.colors.sgr(style.fg, true),
            self.colors.sgr(style.bg, false)
        )
    }

    /// Re-reads the terminal size, returning true if it changed.
    pub fn poll_resize(&mut self) -> bool {
        let size = terminal_size();
        if size != self.size {
            self.size = size;
            self.painted.clear();
            self.painted_keys = None;
            true
        } else {
            false
        }
    }

    /// Paints the M8 grid and the keypad below it, centred.
    pub fn paint(
        &mut self,
        screen: &TextScreen,
        pressed: u8,
        show_controls: bool,
        full: bool,
    ) -> Result<(), String> {
        let (term_cols, term_rows) = self.size;
        let usable_rows = term_rows;
        let cols = screen.cols.min(term_cols);
        let rows = screen.rows.min(usable_rows);
        if cols == 0 || rows == 0 {
            return Ok(());
        }

        let spare = usable_rows - rows;
        let key_height = match spare {
            _ if !show_controls || keypad_width() > term_cols => 0,
            s if s >= FACE_SLOT_ROWS * 2 + KEYPAD_MARGIN => 2,
            s if s >= FACE_SLOT_ROWS + KEYPAD_MARGIN => 1,
            _ => 0,
        };
        let keypad_rows =
            if key_height == 0 { 0 } else { FACE_SLOT_ROWS * key_height + KEYPAD_MARGIN };

        let content_rows = rows + keypad_rows;
        let origin = ((term_cols - cols) / 2, (usable_rows - content_rows) / 2);
        let reshaped = full
            || self.painted.len() != cols * rows
            || (self.painted_cols, self.painted_rows) != (cols, rows)
            || self.origin != origin;

        if reshaped {
            self.painted = vec![PaintedCell::default(); cols * rows];
            self.painted_cols = cols;
            self.painted_rows = rows;
            self.origin = origin;
            self.painted_keys = None;
            let ground = self.sgr(Style {
                fg: Rgb { r: 255, g: 255, b: 255 },
                bg: screen.background(),
                reverse: false,
                device_colors: true,
            });
            write!(self.out, "{ground}{CLEAR_SCREEN}").map_err(|e| e.to_string())?;
        }

        let cell_style = |col: usize, row: usize| {
            let (ch, fg, bg) = screen.glyph_at(col, row);
            let style = Style {
                fg,
                bg,
                reverse: screen.cell(col, row).highlight.is_some(),
                device_colors: true,
            };
            (ch, style)
        };

        for row in 0..rows {
            let mut col = 0;
            while col < cols {
                let (ch, style) = cell_style(col, row);
                if self.painted[row * cols + col] == (PaintedCell { ch, style }) {
                    col += 1;
                    continue;
                }

                write!(
                    self.out,
                    "\x1b[{};{}H{}",
                    origin.1 + row + 1,
                    origin.0 + col + 1,
                    self.sgr(style)
                )
                .map_err(|e| e.to_string())?;

                while col < cols {
                    let (ch, cell_style) = cell_style(col, row);
                    if cell_style != style {
                        break;
                    }
                    self.painted[row * cols + col] = PaintedCell { ch, style };
                    self.out
                        .write_all(ch.encode_utf8(&mut [0u8; 4]).as_bytes())
                        .map_err(|e| e.to_string())?;
                    col += 1;
                }
            }
        }

        if key_height > 0 && self.painted_keys != Some(pressed) {
            self.paint_keypad(origin.1 + rows + KEYPAD_MARGIN, key_height, pressed)?;
            self.painted_keys = Some(pressed);
        }

        self.out.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl Terminal {
    /// Draws the settings menu as a panel over the middle of the screen.
    pub fn paint_menu(
        &mut self,
        title: &str,
        items: &[(String, String)],
        selected: usize,
        footer: &str,
        status: &str,
    ) -> Result<(), String> {
        let title = format!(" {title} ");
        let footer = format!(" {footer} ");
        let (term_cols, term_rows) = self.size;

        const CHROME: usize = 7;
        let (first, count) = visible_rows(items.len(), selected, term_rows.saturating_sub(CHROME));
        let rows: Vec<String> = items
            .iter()
            .enumerate()
            .skip(first)
            .take(count)
            .map(|(index, (name, value))| {
                let marker = if index == selected { '>' } else { ' ' };
                format!(" {marker} {name:<14}{value}")
            })
            .collect();

        let inner = footer
            .chars()
            .count()
            .max(title.chars().count())
            .max(rows.iter().map(|row| row.chars().count()).max().unwrap_or(0) + 1)
            .max(30)
            .min(term_cols.saturating_sub(2));

        let mut lines = vec![String::new()];
        lines.extend(rows);
        lines.push(String::new());
        lines.push(format!(" {}", trim_to(status, inner.saturating_sub(1))));
        lines.push(String::new());
        lines.push(footer);

        let height = lines.len() + 2;
        let left = term_cols.saturating_sub(inner + 2) / 2;
        let top = term_rows.saturating_sub(height) / 2;

        let frame = self.sgr(Style {
            fg: MENU_FG,
            bg: MENU_BG,
            reverse: false,
            device_colors: true,
        });
        let dashes = "─".repeat(inner);
        let title_pad = inner.saturating_sub(title.chars().count());

        let mut rows = Vec::new();
        rows.push(format!("┌{title}{}┐", "─".repeat(title_pad)));
        for line in &lines {
            let pad = inner.saturating_sub(line.chars().count());
            let line: String = line.chars().take(inner).collect();
            rows.push(format!("│{line}{}│", " ".repeat(pad)));
        }
        rows.push(format!("└{dashes}┘"));

        for (offset, row) in rows.iter().enumerate() {
            write!(self.out, "\x1b[{};{}H{frame}{row}", top + offset + 1, left + 1)
                .map_err(|e| e.to_string())?;
        }
        self.out.flush().map_err(|e| e.to_string())?;
        self.painted.clear();
        self.painted_keys = None;
        Ok(())
    }

    /// Draws the face buttons as blocks, arranged as they are on the device.
    fn paint_keypad(&mut self, top: usize, key_height: usize, pressed: u8) -> Result<(), String> {
        let left = self.size.0.saturating_sub(keypad_width()) / 2;

        for (label, button, slot_col, slot_row) in FACE_BUTTONS {
            let x = left + slot_col * (KEY_WIDTH + KEY_GAP);
            let y = top + slot_row * key_height;
            let down = pressed & button != 0;
            let (fg, bg) = if down {
                (KEY_DOWN_FG, KEY_DOWN_BG)
            } else {
                (KEY_UP_FG, KEY_UP_BG)
            };
            let colors = self.sgr(Style { fg, bg, reverse: down, device_colors: true });

            let label_row = y + key_height / 2;
            for row in y..y + key_height {
                let text = if row == label_row {
                    centre(label, KEY_WIDTH)
                } else {
                    " ".repeat(KEY_WIDTH)
                };
                write!(self.out, "\x1b[{};{}H{colors}{text}", row + 1, x + 1)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

/// The rows to draw when there are more than the terminal has room for.
fn visible_rows(total: usize, selected: usize, room: usize) -> (usize, usize) {
    if room == 0 || total <= room {
        return (0, total);
    }
    let first = selected.saturating_sub(room / 2).min(total - room);
    (first, room)
}

/// Shortens `text` to `width` columns, marking where it was cut.
fn trim_to(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars().take(width.saturating_sub(1)).chain(std::iter::once('~')).collect()
}

/// Pads `text` with spaces so it sits in the middle of `width` columns.
fn centre(text: &str, width: usize) -> String {
    let text: String = text.chars().take(width).collect();
    let padding = width - text.chars().count();
    let left = padding / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(padding - left))
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.out.write_all(RESTORE.as_bytes());
        let _ = self.out.flush();
        let fd = io::stdin().as_raw_fd();
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &self.original) };
    }
}

fn terminal_size() -> (usize, usize) {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    if result != 0 || size.ws_col == 0 || size.ws_row == 0 {
        return (80, 24);
    }
    (size.ws_col as usize, size.ws_row as usize)
}

/// Reads stdin and turns it into key events.
pub struct Keyboard {
    buffer: Vec<u8>,
    /// When the bytes currently in the buffer arrived.
    buffered_at: Instant,
    reports_releases: bool,
}

impl Keyboard {
    pub fn new() -> Self {
        Self { buffer: Vec::new(), buffered_at: Instant::now(), reports_releases: false }
    }

    /// True once the terminal has actually sent a key release.
    pub fn reports_releases(&self) -> bool {
        self.reports_releases
    }

    pub fn poll(&mut self) -> Vec<KeyEvent> {
        let mut chunk = [0u8; 1024];
        while stdin_ready() {
            let read = unsafe {
                libc::read(
                    io::stdin().as_raw_fd(),
                    chunk.as_mut_ptr() as *mut libc::c_void,
                    chunk.len(),
                )
            };
            if read <= 0 {
                break;
            }
            if self.buffer.is_empty() {
                self.buffered_at = Instant::now();
            }
            self.buffer.extend_from_slice(&chunk[..read as usize]);
        }

        let escape_settled = self.buffered_at.elapsed() >= ESCAPE_AMBIGUITY_WINDOW;
        let events = decode(&mut self.buffer, escape_settled);
        if events.iter().any(|e| e.kind == KeyKind::Release) {
            self.reports_releases = true;
        }
        events
    }
}

/// True when there is input waiting, without blocking on it. Not O_NONBLOCK:
/// on a tty stdin and stdout share one file description, so that flag makes
/// writes fail with EAGAIN as soon as a repaint outruns the terminal.
fn stdin_ready() -> bool {
    let mut poll_fd =
        libc::pollfd { fd: io::stdin().as_raw_fd(), events: libc::POLLIN, revents: 0 };
    let ready = unsafe { libc::poll(&mut poll_fd, 1, 0) };
    ready > 0 && poll_fd.revents & libc::POLLIN != 0
}

/// Pulls every complete key event out of `buffer`.
fn decode(buffer: &mut Vec<u8>, escape_settled: bool) -> Vec<KeyEvent> {
    let mut events = Vec::new();
    let mut at = 0;

    while at < buffer.len() {
        let byte = buffer[at];
        if byte != 0x1B {
            events.extend(legacy_key(byte));
            at += 1;
            continue;
        }

        match parse_escape(&buffer[at..]) {
            Escape::Event(event, length) => {
                events.push(event);
                at += length;
            }
            Escape::Ignored(length) => at += length,
            Escape::Incomplete => {
                if buffer.len() - at == 1 && escape_settled {
                    events.push(press(Key::Escape));
                    at += 1;
                    continue;
                }
                break;
            }
        }
    }

    buffer.drain(..at);
    events
}

enum Escape {
    Event(KeyEvent, usize),
    /// A sequence we understand but don't care about, e.g. a query reply.
    Ignored(usize),
    Incomplete,
}

/// Parses one escape sequence starting at the escape byte.
fn parse_escape(bytes: &[u8]) -> Escape {
    if bytes.len() < 2 {
        return Escape::Incomplete;
    }
    if bytes[1] == b'O' {
        let Some(&letter) = bytes.get(2) else { return Escape::Incomplete };
        return match ss3_function(letter) {
            Some(key) => Escape::Event(press(key), 3),
            None => Escape::Ignored(3),
        };
    }
    if bytes[1] != b'[' {
        return match legacy_key(bytes[1]) {
            Some(event) => Escape::Event(KeyEvent { alt: true, ..event }, 2),
            None => Escape::Ignored(2),
        };
    }

    let mut at = 2;
    let private = bytes.get(at).is_some_and(|b| matches!(b, b'?' | b'>' | b'<' | b'='));
    if private {
        at += 1;
    }
    let params_start = at;
    while at < bytes.len() && matches!(bytes[at], b'0'..=b'9' | b';' | b':') {
        at += 1;
    }
    let Some(&final_byte) = bytes.get(at) else {
        return Escape::Incomplete;
    };
    if !(0x40..=0x7E).contains(&final_byte) {
        return Escape::Ignored(at + 1);
    }
    let length = at + 1;
    if private {
        return Escape::Ignored(length);
    }

    let params = std::str::from_utf8(&bytes[params_start..at]).unwrap_or("");
    let mut fields = params.split(';');
    let first = fields.next().unwrap_or("");
    let modifiers = fields.next().unwrap_or("");

    let (shift, alt, ctrl, kind) = decode_modifiers(modifiers);
    let key = match final_byte {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'P' => Key::Function(1),
        b'Q' => Key::Function(2),
        b'S' => Key::Function(4),
        b'~' => match first.parse::<u32>() {
            Ok(3) => Key::Delete,
            Ok(code) => match tilde_function(code) {
                Some(key) => key,
                None => Key::Other(code),
            },
            Err(_) => return Escape::Ignored(length),
        },
        b'u' => {
            let code = first.split(':').next().unwrap_or("");
            match code.parse::<u32>() {
                Ok(code) => key_from_code(code),
                Err(_) => return Escape::Ignored(length),
            }
        }
        _ => return Escape::Ignored(length),
    };

    Escape::Event(KeyEvent { key, kind, shift, alt, ctrl }, length)
}

/// Decodes the `modifiers:event-type` field.
fn decode_modifiers(field: &str) -> (bool, bool, bool, KeyKind) {
    let mut parts = field.split(':');
    let bits = parts.next().unwrap_or("").parse::<u32>().unwrap_or(1).saturating_sub(1);
    let kind = match parts.next().unwrap_or("").parse::<u32>() {
        Ok(2) => KeyKind::Repeat,
        Ok(3) => KeyKind::Release,
        _ => KeyKind::Press,
    };
    (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, kind)
}

/// The function key an SS3 sequence stands for.
fn ss3_function(letter: u8) -> Option<Key> {
    match letter {
        b'P' => Some(Key::Function(1)),
        b'Q' => Some(Key::Function(2)),
        b'R' => Some(Key::Function(3)),
        b'S' => Some(Key::Function(4)),
        _ => None,
    }
}

/// The function key a numbered `CSI n ~` sequence stands for.
fn tilde_function(code: u32) -> Option<Key> {
    let number = match code {
        11..=15 => code - 10,
        17..=21 => code - 11,
        23..=24 => code - 12,
        _ => return None,
    };
    Some(Key::Function(number as u8))
}

fn key_from_code(code: u32) -> Key {
    match code {
        9 => Key::Tab,
        13 => Key::Enter,
        27 => Key::Escape,
        127 => Key::Backspace,
        _ => match char::from_u32(code) {
            Some(c) if !c.is_control() => Key::Char(c),
            _ => Key::Other(code),
        },
    }
}

/// Interprets a byte outside any escape sequence.
fn legacy_key(byte: u8) -> Option<KeyEvent> {
    let key = match byte {
        0x09 => Key::Tab,
        0x0A | 0x0D => Key::Enter,
        0x7F | 0x08 => Key::Backspace,
        0x01..=0x1A => {
            let letter = (b'a' + byte - 1) as char;
            return Some(KeyEvent {
                key: Key::Char(letter),
                kind: KeyKind::Press,
                shift: false,
                alt: false,
                ctrl: true,
            });
        }
        0x20..=0x7E => Key::Char(byte as char),
        _ => return None,
    };
    Some(press(key))
}

fn press(key: Key) -> KeyEvent {
    KeyEvent { key, kind: KeyKind::Press, shift: false, alt: false, ctrl: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(input: &str, escape_settled: bool) -> (Vec<KeyEvent>, Vec<u8>) {
        let mut buffer = input.as_bytes().to_vec();
        let events = decode(&mut buffer, escape_settled);
        (events, buffer)
    }

    #[test]
    fn legacy_arrows_are_presses() {
        let (events, rest) = feed("\x1b[A\x1b[D", true);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].key, Key::Up);
        assert_eq!(events[0].kind, KeyKind::Press);
        assert_eq!(events[1].key, Key::Left);
        assert!(rest.is_empty());
    }

    #[test]
    fn legacy_printable_and_control_bytes() {
        let (events, _) = feed("x\x03", true);
        assert_eq!(events[0].key, Key::Char('x'));
        assert!(!events[0].ctrl);
        assert_eq!(events[1].key, Key::Char('c'));
        assert!(events[1].ctrl);
    }

    #[test]
    fn kitty_release_and_repeat_are_distinguished() {
        let (events, _) = feed("\x1b[97;1:1u\x1b[97;1:2u\x1b[97;1:3u", true);
        let kinds: Vec<KeyKind> = events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![KeyKind::Press, KeyKind::Repeat, KeyKind::Release]);
        assert!(events.iter().all(|e| e.key == Key::Char('a')));
    }

    #[test]
    fn modifiers_decode_from_the_bitmask() {
        let (events, _) = feed("\x1b[1;2:1D\x1b[1;3:1D\x1b[1;5:1D", true);
        assert!(events[0].shift && !events[0].alt && !events[0].ctrl);
        assert!(events[1].alt && !events[1].shift);
        assert!(events[2].ctrl && !events[2].shift);
        assert!(events.iter().all(|e| e.key == Key::Left));
    }

    #[test]
    fn named_keys_decode_from_their_codes() {
        let (events, _) = feed("\x1b[9u\x1b[13u\x1b[27u\x1b[127u\x1b[3~", true);
        let decoded: Vec<Key> = events.iter().map(|e| e.key).collect();
        assert_eq!(
            decoded,
            vec![Key::Tab, Key::Enter, Key::Escape, Key::Backspace, Key::Delete]
        );
    }

    #[test]
    fn alternate_key_codes_use_the_base_layout_key() {
        let (events, _) = feed("\x1b[97:65;1:1u", true);
        assert_eq!(events[0].key, Key::Char('a'));
    }

    #[test]
    fn the_function_keys_are_decoded_from_both_encodings() {
        let (events, _) = feed("\x1b[P\x1b[Q\x1b[13~\x1b[S\x1b[15~\x1b[24~", true);
        assert_eq!(
            events.iter().map(|e| e.key).collect::<Vec<_>>(),
            vec![
                Key::Function(1),
                Key::Function(2),
                Key::Function(3),
                Key::Function(4),
                Key::Function(5),
                Key::Function(12),
            ]
        );

        let (events, _) = feed("\x1bOP\x1bOR", true);
        assert_eq!(
            events.iter().map(|e| e.key).collect::<Vec<_>>(),
            vec![Key::Function(1), Key::Function(3)]
        );
    }

    #[test]
    fn a_function_key_keeps_its_modifiers_and_its_release() {
        let (events, _) = feed("\x1b[1;5P\x1b[15;1:3~", true);
        assert_eq!(events[0].key, Key::Function(1));
        assert!(events[0].ctrl);
        assert_eq!(events[1].key, Key::Function(5));
        assert_eq!(events[1].kind, KeyKind::Release);
    }

    #[test]
    fn a_menu_taller_than_the_terminal_scrolls_to_keep_the_selection_in_view() {
        assert_eq!(visible_rows(5, 3, 10), (0, 5));
        assert_eq!(visible_rows(11, 5, 6), (2, 6));
        assert_eq!(visible_rows(11, 0, 6), (0, 6));
        assert_eq!(visible_rows(11, 10, 6), (5, 6));
        assert_eq!(visible_rows(11, 0, 0), (0, 11));
    }

    #[test]
    fn partial_sequences_stay_buffered() {
        let (events, rest) = feed("\x1b[1;2", true);
        assert!(events.is_empty());
        assert_eq!(rest, b"\x1b[1;2");
    }

    #[test]
    fn a_bare_escape_waits_for_the_ambiguity_window() {
        let (events, rest) = feed("\x1b", false);
        assert!(events.is_empty());
        assert_eq!(rest, b"\x1b");

        let (events, rest) = feed("\x1b", true);
        assert_eq!(events[0].key, Key::Escape);
        assert!(rest.is_empty());
    }

    #[test]
    fn protocol_replies_are_ignored() {
        let (events, rest) = feed("\x1b[?11u\x1b[A", true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key, Key::Up);
        assert!(rest.is_empty());
    }

    #[test]
    fn escape_prefixed_keys_are_alt_chords() {
        let (events, _) = feed("\x1bx", true);
        assert_eq!(events[0].key, Key::Char('x'));
        assert!(events[0].alt);
    }

    #[test]
    fn colour_approximation_lands_on_the_right_ranges() {
        assert_eq!(ansi256(Rgb { r: 0, g: 0, b: 0 }), 16);
        assert_eq!(ansi256(Rgb { r: 255, g: 255, b: 255 }), 231);
        let mid_gray = ansi256(Rgb { r: 128, g: 128, b: 128 });
        assert!((232..=255).contains(&mid_gray), "expected grayscale ramp, got {mid_gray}");
        let red = ansi256(Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(red, 16 + 36 * 5);
    }
}
