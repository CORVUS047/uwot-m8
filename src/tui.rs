//! Terminal frontend: shows the M8's screen as text and sends keys back.

use std::time::{Duration, Instant};

use crate::config::{Bound, Config, MenuEnv, Setting};
use crate::keys::{self, bits, Keyjazz};
use crate::m8::{self, M8};
use crate::menu::{self, Menu};
use crate::midi;
use crate::outputs::Outputs;
use crate::pad::{PadEvent, Pads};
use crate::proto;
use crate::term::{Key, KeyEvent, KeyKind, Keyboard, Terminal};
use crate::text::TextScreen;

const FRAME: Duration = Duration::from_millis(8);
/// Empty polls before pinging the device to see whether it is still there.
const PING_AFTER_IDLE_FRAMES: u32 = 240;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
/// How long a button stays pressed on terminals that never report releases.
const PULSE: Duration = Duration::from_millis(60);
/// How far a number in the settings moves with Shift held.
const COARSE_STEP: i32 = 10;

const HELP: &str = "\
uwot-tui - show a Dirtywave M8's display in the terminal

Usage: uwot-tui [options]

Options:
  -d, --device <path>   Serial device to use (default: autodetect)
  -c, --colors <mode>   auto, truecolor or 256 (overrides the config file)
  -l, --list            List detected M8 devices and exit
  -h, --help            Show this help

Settings are kept in ~/.config/uwot-m8/config.conf and can be changed from
inside the app with Escape: whether the face buttons are drawn below the
screen, whether to use the M8's own colours or the terminal's theme, whether
to play the M8's own audio through this computer, and what the keys do.

In the settings, up and down move, left and right change a value (hold Shift
for larger steps), Enter opens a page or starts binding a key, and Escape goes
back. The MIDI page holds eight control-change bindings: each sends either one
value or a ramp between two, over a time and a curve of its own, when the key
bound to it is pressed.

A game controller works too, with no setting-up: the four face buttons press
the M8's own four in the positions they sit in, the d-pad and left stick move,
and the right shoulder is delete. Any button can be rebound, and the ones left
over take control-change bindings.

Controls:
  Arrow keys            Directions
  Z                     SELECT (shift)
  X or Space            START (play)
  A                     OPTION
  S                     EDIT
  Backspace             OPTION+EDIT (delete)
  Shift/Alt/Ctrl + key  Adds SELECT / OPTION / EDIT to that key
  Tab                   Toggle keyjazz (play notes with the keyboard)
  Escape                Open the settings menu
  Ctrl+Q                Quit

Keyjazz (while enabled):
  Z S X D C V G B H N J M   Lower octave, chromatic from C
  Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P   Octave above
  [ and ]               Octave down / up
  - and =               Velocity down / up
  _ and +               Velocity down / up in steps of one
";

struct Options {
    device: Option<String>,
    /// Overrides the colour depth from the config file, when given.
    colors: Option<crate::config::ColorDepth>,
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut options = Options {
        device: None,
        colors: None,
    };
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "-l" | "--list" => {
                let devices = m8::find_devices();
                if devices.is_empty() {
                    println!("No M8 devices found.");
                }
                for device in devices {
                    println!("{device}");
                }
                return Ok(None);
            }
            "-d" | "--device" => {
                options.device = Some(args.next().ok_or("--device needs a path")?);
            }
            "-c" | "--colors" => {
                let value = args.next().unwrap_or_default();
                let depth = crate::config::ColorDepth::parse(&value)
                    .ok_or_else(|| format!("bad colour mode: {value}"))?;
                options.colors = Some(depth);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(options))
}

/// Tracks which M8 buttons are pressed.
struct Buttons {
    /// One entry per key holding buttons down, with an expiry in pulse mode.
    down: Vec<(Key, u8, Option<Instant>)>,
    mask: u8,
}

impl Buttons {
    fn new() -> Self {
        Self {
            down: Vec::new(),
            mask: 0,
        }
    }

    fn press(&mut self, key: Key, buttons: u8, exact: bool) {
        let expiry = (!exact).then(|| Instant::now() + PULSE);
        match self
            .down
            .iter_mut()
            .find(|(existing, _, _)| *existing == key)
        {
            Some(entry) => *entry = (key, buttons, expiry),
            None => self.down.push((key, buttons, expiry)),
        }
        self.recompute();
    }

    fn release(&mut self, key: Key) {
        self.down.retain(|(existing, _, _)| *existing != key);
        self.recompute();
    }

    /// Drops pulses that have run their course. Returns true if anything went.
    fn expire(&mut self) -> bool {
        let now = Instant::now();
        let before = self.down.len();
        self.down
            .retain(|(_, _, expiry)| expiry.is_none_or(|at| at > now));
        if self.down.len() != before {
            self.recompute();
            return true;
        }
        false
    }

    fn clear(&mut self) {
        self.down.clear();
        self.mask = 0;
    }

    fn recompute(&mut self) {
        self.mask = self
            .down
            .iter()
            .fold(0, |mask, (_, buttons, _)| mask | buttons);
    }
}

/// Which keys are currently holding a control-change slot out.
struct CcKeys {
    down: Vec<(Key, usize, Option<Instant>)>,
}

impl CcKeys {
    fn new() -> Self {
        Self { down: Vec::new() }
    }

    /// Notes the key as down; false if it already was, so repeats don't retrigger.
    fn press(&mut self, key: Key, slot: usize, exact: bool) -> bool {
        let expiry = (!exact).then(|| Instant::now() + PULSE);
        match self
            .down
            .iter_mut()
            .find(|(existing, _, _)| *existing == key)
        {
            Some(entry) => {
                *entry = (key, slot, expiry);
                false
            }
            None => {
                self.down.push((key, slot, expiry));
                true
            }
        }
    }

    /// The slot the key was holding, now that it is up.
    fn release(&mut self, key: Key) -> Option<usize> {
        let at = self
            .down
            .iter()
            .position(|(existing, _, _)| *existing == key)?;
        Some(self.down.remove(at).1)
    }

    /// The slots whose pulses have run their course.
    fn expire(&mut self) -> Vec<usize> {
        let now = Instant::now();
        let mut done = Vec::new();
        self.down.retain(|(_, slot, expiry)| match expiry {
            Some(at) if *at <= now => {
                done.push(*slot);
                false
            }
            _ => true,
        });
        done
    }

    fn clear(&mut self) -> Vec<usize> {
        self.down.drain(..).map(|(_, slot, _)| slot).collect()
    }
}

/// The name a key is known by in the config file.
fn key_name(key: Key) -> Option<String> {
    Some(match key {
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Escape => "Escape".into(),
        Key::Tab => "Tab".into(),
        Key::Enter => "Enter".into(),
        Key::Backspace => "Backspace".into(),
        Key::Delete => "Delete".into(),
        Key::Char(' ') => "Space".into(),
        Key::Char(c) => c.to_ascii_lowercase().to_string(),
        Key::Function(number) => format!("F{number}"),
        Key::Other(_) => return None,
    })
}

/// Modifiers stand in for the M8's own chord buttons.
fn modifier_buttons(event: &KeyEvent) -> u8 {
    let mut buttons = 0;
    if event.shift {
        buttons |= bits::SELECT;
    }
    if event.alt {
        buttons |= bits::OPT;
    }
    if event.ctrl {
        buttons |= bits::EDIT;
    }
    buttons
}

enum Outcome {
    Continue,
    Quit,
}

struct App {
    config: Config,
    menu: Option<Menu>,
    /// What the menu's port row cycles through, read when the menu opens.
    env: MenuEnv,
    device: Option<M8>,
    /// The M8's audio, and the MIDI output the control changes go to.
    outputs: Outputs,
    screen: TextScreen,
    buttons: Buttons,
    /// Controllers, and the M8 buttons they are holding down.
    pads: Pads,
    pad_mask: u8,
    pad_cc: Vec<(String, usize)>,
    cc_keys: CcKeys,
    jazz: Keyjazz,
    sounding: Option<Key>,
    sent_mask: u8,
    idle_frames: u32,
    last_reconnect: Instant,
    message: String,
    repaint: bool,
    /// Set when the terminal has to be set up again for new settings.
    reconfigure: bool,
}

impl App {
    /// Every M8 button held, from the keyboard and the controllers together.
    fn mask(&self) -> u8 {
        self.buttons.mask | self.pad_mask
    }

    /// A controller button, which is bound by the same names as a key.
    fn handle_pad(&mut self, event: PadEvent) {
        if self.menu.is_some() {
            if !event.down {
                return;
            }
            let capturing = self.menu.as_ref().is_some_and(Menu::is_capturing);
            let Some(action) = menu::pad_action(&self.config, &event.name, capturing) else {
                return;
            };
            let Some(mut menu) = self.menu.take() else {
                return;
            };
            let outcome = menu.handle(action, &mut self.config, &self.env);
            self.menu = Some(menu);
            self.apply_menu(outcome);
            self.repaint = true;
            return;
        }

        match self.config.bound(&event.name) {
            Some(Bound::Cc(slot)) => {
                if event.down {
                    self.pad_cc.push((event.name.clone(), slot));
                    self.outputs.press(slot, &self.config);
                } else if let Some(at) =
                    self.pad_cc.iter().position(|(held, _)| *held == event.name)
                {
                    let (_, slot) = self.pad_cc.remove(at);
                    self.outputs.release(slot, &self.config);
                }
            }
            Some(Bound::Button(button)) => {
                if event.down {
                    self.pad_mask |= button;
                } else {
                    self.pad_mask &= !button;
                }
            }
            None => {}
        }
    }

    /// Acts on what the menu made of a key or a controller button.
    fn apply_menu(&mut self, outcome: menu::Outcome) {
        if let Some(message) = outcome.message {
            self.message = message;
        }
        if outcome.changed.display {
            self.reconfigure = true;
        }
        if outcome.write {
            self.save_settings();
        }
        if outcome.closed {
            self.menu = None;
            self.reconfigure = true;
        }
    }

    fn handle_key(&mut self, event: KeyEvent, exact: bool) -> Outcome {
        if event.ctrl {
            if let Key::Char(c) = event.key {
                if matches!(c.to_ascii_lowercase(), 'q' | 'c') {
                    return Outcome::Quit;
                }
            }
        }

        if self.menu.is_some() {
            return self.handle_menu_key(event);
        }
        if event.key == Key::Escape {
            if event.kind == KeyKind::Press {
                self.buttons.clear();
                self.pad_mask = 0;
                let mut released = self.cc_keys.clear();
                released.extend(self.pad_cc.drain(..).map(|(_, slot)| slot));
                self.outputs.release_all(&released, &self.config);
                self.env = MenuEnv {
                    midi_ports: midi::ports(),
                };
                self.menu = Some(Menu::new(Setting::terminal()));
                self.repaint = true;
            }
            return Outcome::Continue;
        }

        if event.kind == KeyKind::Release {
            if self.sounding == Some(event.key) {
                self.sounding = None;
                self.send(|device| device.send_note_off());
            }
            if let Some(slot) = self.cc_keys.release(event.key) {
                self.outputs.release(slot, &self.config);
            }
            self.buttons.release(event.key);
            return Outcome::Continue;
        }

        if event.key == Key::Tab {
            if event.kind == KeyKind::Press {
                self.jazz.enabled = !self.jazz.enabled;
                self.repaint = true;
                if !self.jazz.enabled && self.sounding.take().is_some() {
                    self.send(|device| device.send_note_off());
                }
            }
            return Outcome::Continue;
        }

        if self.jazz.enabled {
            if let Key::Char(c) = event.key {
                if let Some(offset) = keys::note_offset(c) {
                    let (note, velocity) = (self.jazz.note(offset), self.jazz.velocity);
                    self.sounding = Some(event.key);
                    self.send(|device| device.send_note_on(note, velocity));
                    if !exact {
                        self.sounding = None;
                    }
                    return Outcome::Continue;
                }
                if self.adjust_keyjazz(c) {
                    self.repaint = true;
                    return Outcome::Continue;
                }
            }
        }

        let bound = key_name(event.key).and_then(|name| self.config.bound(&name));
        if let Some(Bound::Cc(slot)) = bound {
            if self.cc_keys.press(event.key, slot, exact) {
                self.outputs.press(slot, &self.config);
            }
            return Outcome::Continue;
        }

        let pressed = match bound {
            Some(Bound::Button(button)) => button,
            _ => 0,
        };
        let buttons = pressed | modifier_buttons(&event);
        if buttons != 0 {
            self.buttons.press(event.key, buttons, exact);
        }
        Outcome::Continue
    }

    /// Drives the settings menu. Escape leaves the open page, or the menu.
    fn handle_menu_key(&mut self, event: KeyEvent) -> Outcome {
        if event.kind == KeyKind::Release {
            return Outcome::Continue;
        }
        let Some(menu) = self.menu.as_mut() else {
            return Outcome::Continue;
        };
        self.repaint = true;

        let action = if menu.is_capturing() {
            match event.key {
                Key::Escape => menu::Action::Back,
                key => menu::Action::Bind(key_name(key)),
            }
        } else {
            let step = if event.shift { COARSE_STEP } else { 1 };
            let bound =
                key_name(event.key).and_then(|name| menu::bound_action(&self.config, &name, step));
            match bound {
                Some(action) => action,
                None => match event.key {
                    Key::Escape | Key::Tab => menu::Action::Back,
                    Key::Up => menu::Action::Previous,
                    Key::Down => menu::Action::Next,
                    Key::Left => menu::Action::Adjust(-step),
                    Key::Right => menu::Action::Adjust(step),
                    Key::Enter | Key::Char(' ') => menu::Action::Activate,
                    _ => return Outcome::Continue,
                },
            }
        };

        let outcome = menu.handle(action, &mut self.config, &self.env);
        self.apply_menu(outcome);
        Outcome::Continue
    }

    /// Writes the config out.
    fn save_settings(&mut self) {
        if let Err(e) = self.config.save() {
            self.message = format!("could not save settings: {e}");
        }
    }

    fn adjust_keyjazz(&mut self, c: char) -> bool {
        match c {
            '[' => self.jazz.octave_down(),
            ']' => self.jazz.octave_up(),
            '-' => self.jazz.velocity_down(false),
            '=' => self.jazz.velocity_up(false),
            '_' => self.jazz.velocity_down(true),
            '+' => self.jazz.velocity_up(true),
            _ => return false,
        }
        true
    }

    fn send(&mut self, action: impl FnOnce(&mut M8) -> Result<(), String>) {
        if let Some(device) = self.device.as_mut() {
            if let Err(e) = action(device) {
                self.message = e;
            }
        }
    }

    /// A line for the settings menu saying what the app is up to.
    fn status(&self, exact: bool) -> String {
        let mut status = match &self.device {
            Some(device) => device.path.clone(),
            None => "disconnected, retrying".to_string(),
        };
        if self.jazz.enabled {
            status += &format!(
                "   keyjazz oct {} vel {:02X}",
                self.jazz.octave, self.jazz.velocity
            );
        }
        if !exact {
            status += "   press-only terminal: Shift/Alt/Ctrl for chords";
        }
        let outputs = self.outputs.status(&self.config);
        if !outputs.is_empty() {
            status += &format!("   {outputs}");
        }
        let pads = self.pads.status();
        if !pads.is_empty() {
            status += &format!("   {pads}");
        }
        if !self.message.is_empty() {
            status += &format!("   {}", self.message);
        }
        status
    }
}

/// Runs the terminal frontend, errors and exit status and all.
pub fn main() {
    let options = match parse_args() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(e) => {
            eprintln!("uwot-tui: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) = run(options) {
        eprintln!("uwot-tui: {e}");
        std::process::exit(1);
    }
}

fn connect(device: Option<&str>) -> Result<M8, String> {
    let path = match device {
        Some(path) => path.to_string(),
        None => m8::find_devices()
            .into_iter()
            .next()
            .ok_or("no M8 found; is it plugged in and not in use by another program?")?,
    };
    let mut device = M8::open(&path)?;
    device.enable_display(true)?;
    Ok(device)
}

fn run(options: Options) -> Result<(), String> {
    let mut config = Config::load();
    if let Some(depth) = options.colors {
        config.color_depth = depth;
    }

    let mut terminal = Terminal::new(&config)?;
    let mut keyboard = Keyboard::new();

    let mut app = App {
        config,
        menu: None,
        env: MenuEnv::default(),
        device: None,
        outputs: Outputs::new(),
        screen: TextScreen::new()?,
        buttons: Buttons::new(),
        pads: Pads::new(),
        pad_mask: 0,
        pad_cc: Vec::new(),
        cc_keys: CcKeys::new(),
        jazz: Keyjazz::default(),
        sounding: None,
        sent_mask: 0,
        idle_frames: 0,
        last_reconnect: Instant::now() - RECONNECT_INTERVAL,
        message: String::new(),
        repaint: true,
        reconfigure: false,
    };

    match connect(options.device.as_deref()) {
        Ok(device) => app.device = Some(device),
        Err(e) => app.message = e,
    }

    loop {
        let exact = keyboard.reports_releases();

        for event in keyboard.poll() {
            if let Outcome::Quit = app.handle_key(event, exact) {
                return Ok(());
            }
        }
        for event in app.pads.poll() {
            app.handle_pad(event);
        }
        app.buttons.expire();
        let expired = app.cc_keys.expire();
        app.outputs.release_all(&expired, &app.config);

        app.outputs.sync(&app.config);
        app.outputs.tick();

        if app.mask() != app.sent_mask {
            let mask = app.mask();
            app.send(|device| device.send_keys(mask));
            app.sent_mask = mask;
            app.repaint = true;
        }

        let packets = app.device.as_ref().map(M8::poll).unwrap_or_default();
        let got_packets = !packets.is_empty();
        for packet in packets {
            match proto::parse(&packet, app.screen.last_color()) {
                Ok(command) => app.screen.apply(&command),
                Err(e) => app.message = e,
            }
        }

        app.screen.flush();

        if got_packets {
            app.idle_frames = 0;
        } else if app.device.is_some() {
            app.idle_frames += 1;
            if app.idle_frames >= PING_AFTER_IDLE_FRAMES {
                app.idle_frames = 0;
                let alive = app
                    .device
                    .as_mut()
                    .map(|device| device.ping().is_ok() && !device.reader_finished())
                    .unwrap_or(false);
                if !alive {
                    app.device = None;
                    app.buttons.clear();
                    app.sent_mask = 0;
                    app.message = "M8 disconnected".into();
                    app.repaint = true;
                }
            }
        }

        if app.device.as_ref().is_some_and(M8::reader_finished) {
            app.device = None;
            app.buttons.clear();
            app.sent_mask = 0;
            app.message = "M8 disconnected".into();
            app.repaint = true;
        }

        if app.device.is_none() && app.last_reconnect.elapsed() >= RECONNECT_INTERVAL {
            app.last_reconnect = Instant::now();
            if let Ok(device) = connect(options.device.as_deref()) {
                app.device = Some(device);
                app.screen.clear();
                app.message.clear();
                app.repaint = true;
            }
        }

        if std::mem::take(&mut app.reconfigure) {
            terminal.reconfigure(&app.config);
            app.repaint = true;
        }

        let resized = terminal.poll_resize();
        let full = resized || app.repaint || std::mem::take(&mut app.screen.reshaped);
        if full || app.screen.dirty {
            app.screen.dirty = false;
            app.repaint = false;
            terminal.paint(&app.screen, app.mask(), app.config.show_controls, full)?;
            if let Some(menu) = &app.menu {
                let items = menu.items(&app.config);
                terminal.paint_menu(
                    menu.title(),
                    &items,
                    menu.selected(),
                    menu.footer(),
                    &app.status(exact),
                )?;
            }
        }

        std::thread::sleep(FRAME);
    }
}
