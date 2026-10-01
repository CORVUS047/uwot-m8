//! uwot-m8: the M8's screen in a window, with the keyboard forwarded back.

mod input;

use std::time::{Duration, Instant};

use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Mod, Scancode};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{BlendMode, Texture, TextureCreator};
use sdl2::video::WindowContext;

use input::Action;
use uwot_m8::config::{Config, MenuEnv, Setting};
use uwot_m8::m8::{self, M8};
use uwot_m8::menu::{self, Menu};
use uwot_m8::outputs::Outputs;
use uwot_m8::proto::{self, Command};
use uwot_m8::screen::Screen;
use uwot_m8::{audio, font, midi, pad, ui};

const DEFAULT_SCALE: u32 = 3;
/// How far a number in the settings moves with Shift held.
const COARSE_STEP: i32 = 10;
/// Empty poll cycles before we ping the device to see if it is still there.
const PING_AFTER_IDLE_FRAMES: u32 = 240;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

struct Options {
    device: Option<String>,
    scale: u32,
    fullscreen: bool,
}

const HELP: &str = "\
uwot-m8 - show a Dirtywave M8's display on the desktop

Usage: uwot-m8 [options]

Options:
  -d, --device <path>   Serial device to use (default: autodetect)
  -s, --scale <n>       Initial window scale factor (default: 3)
  -f, --fullscreen      Start fullscreen
  -l, --list            List detected M8 devices and exit
  -h, --help            Show this help

A game controller works too, with no setting-up: the four face buttons press
the M8's own four in the positions they sit in, the d-pad and left stick move,
and the right shoulder is delete. Any button can be rebound, and the ones left
over take control-change bindings.

Controls:
  Arrow keys            Directions
  Z / Left Shift        SELECT (shift)
  X / Space             START (play)
  A / Left Alt          OPTION
  S / Left Ctrl         EDIT
  Delete                OPTION+EDIT (delete)
  Tab                   Toggle keyjazz (play notes with the keyboard)
  Escape                Open the settings menu
  Alt+Enter             Toggle fullscreen
  Alt+F4 / window close Quit

Settings are kept in ~/.config/uwot-m8/config.conf (%APPDATA%\\uwot-m8\\config.conf
on Windows), shared with uwot-tui, and can be changed from inside the app with
Escape: whether the face buttons are
drawn, whether to play the M8's own audio through this computer, and what the
keys do.

In the settings, up and down move, left and right change a value (hold Shift
for larger steps), Enter opens a page or starts binding a key, and Escape goes
back. The MIDI page holds eight control-change bindings: each sends either one
value or a ramp between two, over a time and a curve of its own, when the key
bound to it is pressed.

Keyjazz (while enabled):
  Z S X D C V G B H N J M   Lower octave, chromatic from C
  Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P   Octave above
  Keypad / and *        Octave down / up
  Keypad - and +        Velocity down / up (hold Alt for fine steps)
";

fn parse_args() -> Result<Option<Options>, String> {
    let mut options = Options {
        device: None,
        scale: DEFAULT_SCALE,
        fullscreen: false,
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
                } else {
                    for device in devices {
                        println!("{device}");
                    }
                }
                return Ok(None);
            }
            "-d" | "--device" => {
                options.device = Some(args.next().ok_or("--device needs a path")?);
            }
            "-s" | "--scale" => {
                let value = args.next().ok_or("--scale needs a number")?;
                options.scale = value.parse().map_err(|_| format!("bad scale: {value}"))?;
                if options.scale == 0 {
                    return Err("scale must be at least 1".into());
                }
            }
            "-f" | "--fullscreen" => options.fullscreen = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(options))
}

fn connect(device: Option<&str>) -> Result<M8, String> {
    let path = match device {
        Some(path) => path.to_string(),
        None => m8::find_devices()
            .into_iter()
            .next()
            .ok_or("no M8 found; is it plugged in and not in use by another program?")?,
    };

    let mut m8 = M8::open(&path)?;
    m8.enable_display(true)?;
    Ok(m8)
}

fn make_texture<'a>(
    creator: &'a TextureCreator<WindowContext>,
    screen: &Screen,
) -> Result<Texture<'a>, String> {
    creator
        .create_texture_streaming(PixelFormatEnum::ARGB8888, screen.width, screen.height)
        .map_err(|e| format!("cannot create texture: {e}"))
}

fn upload(texture: &mut Texture, screen: &Screen) -> Result<(), String> {
    let width = screen.width as usize;
    texture
        .with_lock(None, |dst, pitch| {
            for (y, row) in screen.pixels.chunks_exact(width).enumerate() {
                let line = &mut dst[y * pitch..];
                for (x, pixel) in row.iter().enumerate() {
                    line[x * 4..x * 4 + 4].copy_from_slice(&pixel.to_le_bytes());
                }
            }
        })
        .map_err(|e| format!("cannot upload frame: {e}"))
}

/// The overlay texture holds the keypad and the settings panel.
fn make_overlay<'a>(
    creator: &'a TextureCreator<WindowContext>,
    width: u32,
    height: u32,
) -> Result<Texture<'a>, String> {
    let mut texture = creator
        .create_texture_streaming(PixelFormatEnum::ARGB8888, width, height)
        .map_err(|e| format!("cannot create overlay texture: {e}"))?;
    texture.set_blend_mode(BlendMode::Blend);
    Ok(texture)
}

fn upload_canvas(texture: &mut Texture, canvas: &ui::Canvas) -> Result<(), String> {
    texture
        .with_lock(None, |dst, pitch| {
            for (y, row) in canvas.pixels.chunks_exact(canvas.width).enumerate() {
                let line = &mut dst[y * pitch..];
                for (x, pixel) in row.iter().enumerate() {
                    line[x * 4..x * 4 + 4].copy_from_slice(&pixel.to_le_bytes());
                }
            }
        })
        .map_err(|e| format!("cannot upload overlay: {e}"))
}

/// Acts on what the menu made of a key or a controller button.
fn apply_menu(outcome: menu::Outcome, config: &Config, menu: &mut Option<Menu>) {
    if let Some(message) = &outcome.message {
        eprintln!("uwot-m8: {message}");
    }
    if outcome.write {
        save_settings(config);
    }
    if outcome.closed {
        *menu = None;
    }
}

/// Writes the config out.
fn save_settings(config: &Config) {
    if let Err(e) = config.save() {
        eprintln!("uwot-m8: could not save settings: {e}");
    }
}

/// The settings menu's status line: audio, MIDI and controllers.
fn status(outputs: &Outputs, pads: &pad::Pads, config: &Config) -> String {
    let parts = [outputs.status(config), pads.status()];
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("   ")
}

fn title(m8: Option<&M8>, keyjazz: &input::Input) -> String {
    match m8 {
        None => "M8 - disconnected".to_string(),
        Some(m8) if keyjazz.jazz.enabled => format!(
            "M8 - {} - keyjazz oct {} vel {:02X}",
            m8.path, keyjazz.jazz.octave, keyjazz.jazz.velocity
        ),
        Some(m8) => format!("M8 - {}", m8.path),
    }
}

fn main() {
    let options = match parse_args() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(e) => {
            eprintln!("uwot-m8: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) = run(options) {
        eprintln!("uwot-m8: {e}");
        std::process::exit(1);
    }
}

fn run(options: Options) -> Result<(), String> {
    let mut config = Config::load();
    let mut screen = Screen::new()?;
    let ui_font = font::load(2)?;
    let mut menu: Option<Menu> = None;
    let mut env = MenuEnv::default();
    let mut outputs = Outputs::new();
    let mut pads = pad::Pads::new();

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let panel_height = |show: bool| if show { ui::panel_height() as u32 } else { 0 };
    let mut logical_height = screen.height + panel_height(config.show_controls);

    let mut window = video
        .window(
            "M8",
            screen.width * options.scale,
            logical_height * options.scale,
        )
        .position_centered()
        .resizable()
        .opengl()
        .build()
        .map_err(|e| e.to_string())?;

    if options.fullscreen {
        window
            .set_fullscreen(sdl2::video::FullscreenType::Desktop)
            .map_err(|e| e.to_string())?;
    }

    let mut canvas = window
        .into_canvas()
        .present_vsync()
        .build()
        .map_err(|e| e.to_string())?;
    canvas.set_integer_scale(true)?;
    canvas
        .set_logical_size(screen.width, logical_height)
        .map_err(|e| e.to_string())?;

    let creator = canvas.texture_creator();
    let mut texture = make_texture(&creator, &screen)?;
    let mut overlay_canvas = ui::Canvas::new(screen.width as usize, logical_height as usize);
    let mut overlay = make_overlay(&creator, screen.width, logical_height)?;
    let mut overlay_stale = true;
    screen.resized = false;

    let mut events = sdl.event_pump()?;
    let mut keys = input::Input::new();

    let mut m8 = match connect(options.device.as_deref()) {
        Ok(m8) => Some(m8),
        Err(e) => {
            eprintln!("uwot-m8: {e}");
            None
        }
    };
    if let Some(m8) = &m8 {
        println!("Connected to M8 on {}", m8.path);
    }

    canvas
        .window_mut()
        .set_title(&title(m8.as_ref(), &keys))
        .map_err(|e| e.to_string())?;

    let mut idle_frames = 0u32;
    let mut last_reconnect_attempt = Instant::now();
    let mut last_sent_keys = 0u8;
    let mut retitle = false;

    'main: loop {
        for event in events.poll_iter() {
            if let Some(open) = menu.as_mut() {
                match event {
                    Event::Quit { .. } => break 'main,
                    Event::KeyDown {
                        scancode: Some(scancode),
                        keymod,
                        repeat: false,
                        ..
                    } => {
                        overlay_stale = true;
                        let action = if open.is_capturing() {
                            match scancode {
                                Scancode::Escape => menu::Action::Back,
                                other => menu::Action::Bind(input::key_name(other)),
                            }
                        } else {
                            let coarse = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                            let step = if coarse { COARSE_STEP } else { 1 };
                            let bound = input::key_name(scancode)
                                .and_then(|name| menu::bound_action(&config, &name, step));
                            match bound {
                                Some(action) => action,
                                None => match scancode {
                                    Scancode::Up => menu::Action::Previous,
                                    Scancode::Down => menu::Action::Next,
                                    Scancode::Left => menu::Action::Adjust(-step),
                                    Scancode::Right => menu::Action::Adjust(step),
                                    Scancode::Return | Scancode::Space => menu::Action::Activate,
                                    Scancode::Escape | Scancode::Tab => menu::Action::Back,
                                    _ => continue,
                                },
                            }
                        };

                        let outcome = open.handle(action, &mut config, &env);
                        apply_menu(outcome, &config, &mut menu);
                    }
                    _ => {}
                }
                continue;
            }

            let action = match event {
                Event::Quit { .. } => break 'main,
                Event::KeyDown {
                    scancode: Some(scancode),
                    keymod,
                    repeat,
                    ..
                } => {
                    let before = keys.jazz.enabled;
                    let action = keys.key_down(&config, scancode, keymod, repeat);
                    retitle |= keys.jazz.enabled != before;
                    action
                }
                Event::KeyUp {
                    scancode: Some(scancode),
                    ..
                } => keys.key_up(&config, scancode),
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => {
                    let released = keys.release_all();
                    outputs.release_all(&released, &config);
                    Action::None
                }
                _ => Action::None,
            };

            match action {
                Action::Quit => break 'main,
                Action::CcPress(slot) => outputs.press(slot, &config),
                Action::CcRelease(slot) => outputs.release(slot, &config),
                Action::OpenSettings => {
                    let released = keys.release_all();
                    outputs.release_all(&released, &config);
                    env = MenuEnv {
                        midi_ports: midi::ports(),
                        audio_inputs: audio::inputs(),
                        audio_outputs: audio::outputs(),
                    };
                    menu = Some(Menu::new(Setting::window()));
                    overlay_stale = true;
                }
                Action::ToggleFullscreen => {
                    let window = canvas.window_mut();
                    let next = match window.fullscreen_state() {
                        sdl2::video::FullscreenType::Off => sdl2::video::FullscreenType::Desktop,
                        _ => sdl2::video::FullscreenType::Off,
                    };
                    window.set_fullscreen(next).map_err(|e| e.to_string())?;
                }
                other => {
                    if let Some(device) = m8.as_mut() {
                        let result = match other {
                            Action::NoteOn(note, velocity) => device.send_note_on(note, velocity),
                            Action::NoteOff => device.send_note_off(),
                            Action::None
                            | Action::Quit
                            | Action::CcPress(_)
                            | Action::CcRelease(_)
                            | Action::ToggleFullscreen
                            | Action::OpenSettings => Ok(()),
                        };
                        if let Err(e) = result {
                            eprintln!("uwot-m8: {e}");
                        }
                    }
                }
            }
        }

        for event in pads.poll() {
            if let Some(open) = menu.as_mut() {
                if !event.down {
                    continue;
                }
                let Some(action) = menu::pad_action(&config, &event.name, open.is_capturing())
                else {
                    continue;
                };
                let outcome = open.handle(action, &mut config, &env);
                apply_menu(outcome, &config, &mut menu);
                overlay_stale = true;
                continue;
            }
            let action = if event.down {
                keys.pad_down(&config, &event.name)
            } else {
                keys.pad_up(&config, &event.name)
            };
            match action {
                Action::CcPress(slot) => outputs.press(slot, &config),
                Action::CcRelease(slot) => outputs.release(slot, &config),
                _ => {}
            }
        }

        outputs.sync(&config);
        if outputs.tick() {
            overlay_stale |= menu.is_some();
        }

        if let Some(device) = m8.as_mut() {
            if keys.keys() != last_sent_keys {
                if let Err(e) = device.send_keys(keys.keys()) {
                    eprintln!("uwot-m8: {e}");
                } else {
                    last_sent_keys = keys.keys();
                    overlay_stale = true;
                }
            }
        }

        let mut got_packets = false;
        if let Some(device) = m8.as_ref() {
            for packet in device.poll() {
                got_packets = true;
                match proto::parse(&packet, screen.last_color()) {
                    Ok(command) => {
                        if let Command::System(info) = &command {
                            println!(
                                "M8 hardware: {}, firmware {}.{}.{}",
                                info.hardware_name(),
                                info.version.0,
                                info.version.1,
                                info.version.2
                            );
                        }
                        if let Err(e) = screen.apply(&command) {
                            eprintln!("uwot-m8: {e}");
                        }
                    }
                    Err(e) => eprintln!("uwot-m8: {e}"),
                }
            }
        }

        if let Some(device) = m8.as_mut() {
            if got_packets {
                idle_frames = 0;
            } else {
                idle_frames += 1;
                if idle_frames >= PING_AFTER_IDLE_FRAMES {
                    idle_frames = 0;
                    if device.ping().is_err() || device.reader_finished() {
                        eprintln!("uwot-m8: M8 disconnected");
                        m8 = None;
                        last_sent_keys = 0;
                        retitle = true;
                    }
                }
            }
            if m8.as_ref().is_some_and(M8::reader_finished) {
                eprintln!("uwot-m8: M8 disconnected");
                m8 = None;
                last_sent_keys = 0;
                retitle = true;
            }
        } else if last_reconnect_attempt.elapsed() >= RECONNECT_INTERVAL {
            last_reconnect_attempt = Instant::now();
            if let Ok(device) = connect(options.device.as_deref()) {
                println!("Reconnected to M8 on {}", device.path);
                screen.clear();
                m8 = Some(device);
                retitle = true;
            }
        }

        let wanted_height = screen.height + panel_height(config.show_controls);
        if screen.resized || wanted_height != logical_height {
            screen.resized = false;
            logical_height = wanted_height;
            texture = make_texture(&creator, &screen)?;
            overlay_canvas.resize(screen.width as usize, logical_height as usize);
            overlay = make_overlay(&creator, screen.width, logical_height)?;
            overlay_stale = true;
            canvas
                .set_logical_size(screen.width, logical_height)
                .map_err(|e| e.to_string())?;
            let window = canvas.window_mut();
            let (window_w, window_h) = window.size();
            if window_w < screen.width * 2 || window_h < logical_height * 2 {
                window
                    .set_size(screen.width * options.scale, logical_height * options.scale)
                    .map_err(|e| e.to_string())?;
            }
            screen.dirty = true;
        }

        if retitle {
            retitle = false;
            canvas
                .window_mut()
                .set_title(&title(m8.as_ref(), &keys))
                .map_err(|e| e.to_string())?;
        }

        if screen.dirty {
            screen.dirty = false;
            upload(&mut texture, &screen)?;
        }

        if overlay_stale {
            overlay_stale = false;
            overlay_canvas.clear();
            if config.show_controls {
                ui::draw_keypad(&mut overlay_canvas, screen.height as usize, keys.keys());
            }
            if let Some(open) = &menu {
                let items = open.items(&config);
                ui::draw_menu(
                    &mut overlay_canvas,
                    &ui_font,
                    open.title(),
                    &items,
                    open.selected(),
                    open.footer(),
                    &status(&outputs, &pads, &config),
                );
            }
            upload_canvas(&mut overlay, &overlay_canvas)?;
        }

        let bg = screen.background();
        canvas.set_draw_color(Color::RGB(bg.r, bg.g, bg.b));
        canvas.clear();
        canvas.copy(
            &texture,
            None,
            Some(Rect::new(0, 0, screen.width, screen.height)),
        )?;
        canvas.copy(&overlay, None, None)?;
        canvas.present();
    }

    if let Some(mut device) = m8.take() {
        device.disconnect();
    }
    Ok(())
}
