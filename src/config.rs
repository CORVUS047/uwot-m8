//! Settings shared by both frontends, and the file they persist in.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::audio;
use crate::cc::{self, Binding, Curve, Shape, Trigger};
use crate::keys::bits;

/// Where the M8's screen takes its colours from.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    /// The colours the device sends, i.e. whatever theme the M8 is set to.
    #[default]
    M8,
    /// The terminal's own colours, with reverse video for the M8's highlights.
    Terminal,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Theme::M8 => "M8",
            Theme::Terminal => "terminal",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Theme::M8 => Theme::Terminal,
            Theme::Terminal => Theme::M8,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "m8" => Some(Theme::M8),
            "terminal" => Some(Theme::Terminal),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::M8 => "m8",
            Theme::Terminal => "terminal",
        }
    }
}

/// How much colour to send, when the M8's own theme is in use.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum ColorDepth {
    /// Work it out from the environment.
    #[default]
    Auto,
    TrueColor,
    Ansi256,
}

impl ColorDepth {
    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::Auto => "auto",
            ColorDepth::TrueColor => "truecolor",
            ColorDepth::Ansi256 => "256",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ColorDepth::Auto => ColorDepth::TrueColor,
            ColorDepth::TrueColor => ColorDepth::Ansi256,
            ColorDepth::Ansi256 => ColorDepth::Auto,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(ColorDepth::Auto),
            "truecolor" | "24bit" => Some(ColorDepth::TrueColor),
            "256" => Some(ColorDepth::Ansi256),
            _ => None,
        }
    }
}

/// An M8 button that can be bound to a key.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Select,
    Start,
    Option,
    Edit,
    Delete,
}

impl Button {
    pub const ALL: [Button; 9] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::Select,
        Button::Start,
        Button::Option,
        Button::Edit,
        Button::Delete,
    ];

    /// The bit this button sets in a controller message.
    pub fn bit(self) -> u8 {
        match self {
            Button::Up => bits::UP,
            Button::Down => bits::DOWN,
            Button::Left => bits::LEFT,
            Button::Right => bits::RIGHT,
            Button::Select => bits::SELECT,
            Button::Start => bits::START,
            Button::Option => bits::OPT,
            Button::Edit => bits::EDIT,
            Button::Delete => bits::DELETE,
        }
    }

    /// How the button is named in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Button::Up => "Up",
            Button::Down => "Down",
            Button::Left => "Left",
            Button::Right => "Right",
            Button::Select => "Select",
            Button::Start => "Start",
            Button::Option => "Option",
            Button::Edit => "Edit",
            Button::Delete => "Delete",
        }
    }

    /// How the binding is named in the config file.
    fn config_key(self) -> &'static str {
        match self {
            Button::Up => "key_up",
            Button::Down => "key_down",
            Button::Left => "key_left",
            Button::Right => "key_right",
            Button::Select => "key_select",
            Button::Start => "key_start",
            Button::Option => "key_option",
            Button::Edit => "key_edit",
            Button::Delete => "key_delete",
        }
    }

    fn from_config_key(key: &str) -> Option<Self> {
        Button::ALL
            .into_iter()
            .find(|button| button.config_key() == key)
    }
}

/// Which keys press which buttons.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bindings(BTreeMap<Button, Vec<String>>);

impl Default for Bindings {
    fn default() -> Self {
        let defaults: [(Button, &[&str]); 9] = [
            (Button::Up, &["Up", "PadUp"]),
            (Button::Down, &["Down", "PadDown"]),
            (Button::Left, &["Left", "PadLeft"]),
            (Button::Right, &["Right", "PadRight"]),
            (Button::Select, &["z", "LeftShift", "PadWest"]),
            (Button::Start, &["x", "Space", "PadSouth"]),
            (Button::Option, &["a", "LeftAlt", "PadNorth"]),
            (Button::Edit, &["s", "LeftCtrl", "PadEast"]),
            (Button::Delete, &["Delete", "Backspace", "PadR1"]),
        ];
        Self(
            defaults
                .into_iter()
                .map(|(button, keys)| (button, keys.iter().map(|k| (*k).to_string()).collect()))
                .collect(),
        )
    }
}

impl Bindings {
    /// The button a key name presses, if any.
    pub fn button_named(&self, name: &str) -> Option<Button> {
        self.0
            .iter()
            .find(|(_, keys)| keys.iter().any(|key| key == name))
            .map(|(button, _)| *button)
    }

    /// The same button as the bit it sets in a controller message.
    pub fn button_for(&self, name: &str) -> Option<u8> {
        self.button_named(name).map(Button::bit)
    }

    /// The keys bound to a button, for display.
    pub fn keys(&self, button: Button) -> String {
        match self.0.get(&button) {
            Some(keys) if !keys.is_empty() => keys.join(", "),
            _ => "unbound".to_string(),
        }
    }

    /// Binds a key to a button.
    pub fn bind(&mut self, button: Button, name: &str) {
        self.forget(name);
        self.0.insert(button, vec![name.to_string()]);
    }

    /// Takes a key off every button that had it.
    fn forget(&mut self, name: &str) {
        for keys in self.0.values_mut() {
            keys.retain(|key| key != name);
        }
    }
}

/// What a name does when it goes down.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Bound {
    /// Presses M8 buttons, as a bitmask.
    Button(u8),
    /// Fires a control-change slot.
    Cc(usize),
}

/// What a key being bound is being bound to.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BindTarget {
    Button(Button),
    /// One of the MIDI control-change slots.
    Cc(usize),
}

/// What the app has to do about a setting that just changed.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// The display has to be set up again rather than just redrawn.
    pub display: bool,
    pub audio: bool,
    pub midi: bool,
}

impl Changed {
    fn display() -> Self {
        Self {
            display: true,
            ..Self::default()
        }
    }

    fn audio() -> Self {
        Self {
            audio: true,
            ..Self::default()
        }
    }

    fn midi() -> Self {
        Self {
            midi: true,
            ..Self::default()
        }
    }
}

/// What the menu needs to know that the config does not hold.
#[derive(Clone, Debug, Default)]
pub struct MenuEnv {
    pub midi_ports: Vec<String>,
    /// The capture devices the M8's audio can be played from.
    pub audio_inputs: Vec<String>,
    /// The devices audio can be played through, either way round.
    pub audio_outputs: Vec<String>,
    /// The devices audio can be sent into the M8 from.
    pub audio_sources: Vec<String>,
    /// The applications one of which can be sent into the M8 on its own.
    pub audio_apps: Vec<String>,
}

impl MenuEnv {
    /// Everything the menu can offer, looked up now.
    pub fn scan() -> Self {
        let mut env = Self {
            midi_ports: crate::midi::ports(),
            ..Self::default()
        };
        env.rescan_audio();
        env
    }

    /// The audio lists again, for when the audio page is opened: devices are
    /// plugged in and applications started while the menu sits there.
    pub fn rescan_audio(&mut self) {
        self.audio_inputs = audio::inputs();
        self.audio_outputs = audio::outputs();
        self.audio_sources = audio::sources();
        self.audio_apps = audio::apps();
    }
}

/// How the control-change slots are named, in the menu and in the file.
const CC_LABELS: [&str; cc::SLOTS] = [
    "CC 1", "CC 2", "CC 3", "CC 4", "CC 5", "CC 6", "CC 7", "CC 8",
];
const CC_KEYS: [&str; cc::SLOTS] = ["cc1", "cc2", "cc3", "cc4", "cc5", "cc6", "cc7", "cc8"];

/// One row of a settings menu.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Setting {
    Controls,
    Theme,
    ColorDepth,
    /// Opens the audio page.
    Audio,
    /// Play the M8's audio through this computer, or do not.
    AudioOn,
    /// The device the M8's audio is taken from.
    AudioInput,
    /// The device it is played through.
    AudioOutput,
    /// Send audio from this computer into the M8, or do not.
    AudioSendOn,
    /// The device that audio is taken from.
    AudioSendInput,
    /// The M8 device it is sent into.
    AudioSendOutput,
    /// Opens the page of button bindings.
    Buttons,
    /// The key that presses a button.
    Key(Button),
    /// Puts every button binding back to its default.
    ResetKeys,
    /// Opens the MIDI page.
    Midi,
    MidiPort,
    /// Opens the page for one control-change binding.
    CcSlot(usize),
    CcKey(usize),
    CcChannel(usize),
    CcController(usize),
    CcShape(usize),
    CcTrigger(usize),
    CcFrom(usize),
    CcTo(usize),
    CcTime(usize),
    CcCurve(usize),
    /// Unbinds one control-change slot and puts its fields back.
    CcClear(usize),
    /// Leaves the current page.
    Back,
}

impl Setting {
    /// The terminal frontend's top-level menu.
    pub fn terminal() -> Vec<Setting> {
        vec![
            Setting::Controls,
            Setting::Theme,
            Setting::ColorDepth,
            Setting::Audio,
            Setting::Buttons,
            Setting::Midi,
        ]
    }

    /// The windowed frontend's top-level menu.
    pub fn window() -> Vec<Setting> {
        vec![
            Setting::Controls,
            Setting::Audio,
            Setting::Buttons,
            Setting::Midi,
        ]
    }

    /// The rows this setting opens, for the rows that open a page.
    pub fn page(self) -> Option<Vec<Setting>> {
        match self {
            Setting::Buttons => {
                let mut rows: Vec<Setting> = Button::ALL.map(Setting::Key).into();
                rows.push(Setting::ResetKeys);
                rows.push(Setting::Back);
                Some(rows)
            }
            Setting::Audio => Some(vec![
                Setting::AudioOn,
                Setting::AudioInput,
                Setting::AudioOutput,
                Setting::AudioSendOn,
                Setting::AudioSendInput,
                Setting::AudioSendOutput,
                Setting::Back,
            ]),
            Setting::Midi => {
                let mut rows = vec![Setting::MidiPort];
                rows.extend((0..cc::SLOTS).map(Setting::CcSlot));
                rows.push(Setting::Back);
                Some(rows)
            }
            Setting::CcSlot(slot) => Some(vec![
                Setting::CcKey(slot),
                Setting::CcChannel(slot),
                Setting::CcController(slot),
                Setting::CcShape(slot),
                Setting::CcTrigger(slot),
                Setting::CcFrom(slot),
                Setting::CcTo(slot),
                Setting::CcTime(slot),
                Setting::CcCurve(slot),
                Setting::CcClear(slot),
                Setting::Back,
            ]),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Setting::Controls => "Controls",
            Setting::Theme => "Theme",
            Setting::ColorDepth => "Colours",
            Setting::Audio => "Audio",
            Setting::AudioOn => "Play",
            Setting::AudioInput => "Play from",
            Setting::AudioOutput => "Play to",
            Setting::AudioSendOn => "Send",
            Setting::AudioSendInput => "Send from",
            Setting::AudioSendOutput => "Send into",
            Setting::Buttons => "Buttons",
            Setting::Key(button) => button.label(),
            Setting::ResetKeys => "Reset keys",
            Setting::Midi => "MIDI",
            Setting::MidiPort => "Port",
            Setting::CcSlot(slot) => CC_LABELS[slot.min(cc::SLOTS - 1)],
            Setting::CcKey(_) => "Key",
            Setting::CcChannel(_) => "Channel",
            Setting::CcController(_) => "Controller",
            Setting::CcShape(_) => "Shape",
            Setting::CcTrigger(_) => "Trigger",
            Setting::CcFrom(_) => "From",
            Setting::CcTo(_) => "To",
            Setting::CcTime(_) => "Time",
            Setting::CcCurve(_) => "Curve",
            Setting::CcClear(_) => "Clear",
            Setting::Back => "Back",
        }
    }

    /// The title of the page this setting opens.
    pub fn page_title(self) -> &'static str {
        match self {
            Setting::Midi => "MIDI",
            Setting::Audio => "Audio",
            other => other.label(),
        }
    }

    /// What a key pressed on this row gets bound to.
    pub fn bind_target(self) -> Option<BindTarget> {
        match self {
            Setting::Key(button) => Some(BindTarget::Button(button)),
            Setting::CcKey(slot) => Some(BindTarget::Cc(slot)),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// Draw the M8's face buttons below its screen.
    pub show_controls: bool,
    pub theme: Theme,
    pub color_depth: ColorDepth,
    /// Play the M8's own audio through this computer's output.
    pub audio: bool,
    /// The capture device that audio is taken from, by name. `None` finds the
    /// M8 by name instead.
    pub audio_input: Option<String>,
    /// The device it is played through, by name. `None` uses the default.
    pub audio_output: Option<String>,
    /// Send audio from this computer into the M8's USB audio input.
    pub audio_send: bool,
    /// The device that audio is taken from, by name. `None` uses this
    /// computer's default input.
    pub audio_send_input: Option<String>,
    /// The M8 device it is sent into, by name. `None` finds the M8 by name.
    pub audio_send_output: Option<String>,
    /// The MIDI output the control-change bindings send to, by name.
    pub midi_port: Option<String>,
    pub bindings: Bindings,
    /// The control-change bindings, always [`cc::SLOTS`] of them.
    pub cc: Vec<Binding>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_controls: true,
            theme: Theme::default(),
            color_depth: ColorDepth::default(),
            audio: false,
            audio_input: None,
            audio_output: None,
            audio_send: false,
            audio_send_input: None,
            audio_send_output: None,
            midi_port: None,
            bindings: Bindings::default(),
            cc: vec![Binding::default(); cc::SLOTS],
        }
    }
}

impl fmt::Display for Config {
    /// Writes the config file, comments and all.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "# Settings for uwot-m8 and uwot-tui. Press Escape in either app to"
        )?;
        writeln!(f, "# change these.")?;
        writeln!(f, "# show_controls: true or false")?;
        writeln!(f, "show_controls = {}", self.show_controls)?;
        writeln!(f, "# theme: m8 (the device's own colours) or terminal")?;
        writeln!(f, "theme = {}", self.theme.key())?;
        writeln!(f, "# colors: auto, truecolor or 256")?;
        writeln!(f, "colors = {}", self.color_depth.label())?;
        writeln!(f, "# audio: play the M8's own output through this computer")?;
        writeln!(f, "audio = {}", self.audio)?;
        writeln!(
            f,
            "# audio_input: the device that audio is taken from, or none to find"
        )?;
        writeln!(
            f,
            "# the M8 by name; audio_output: where to play it, or none"
        )?;
        writeln!(f, "# for this computer's default. Part of a name will do.")?;
        writeln!(f, "audio_input = {}", name_or_none(&self.audio_input))?;
        writeln!(f, "audio_output = {}", name_or_none(&self.audio_output))?;
        writeln!(
            f,
            "# audio_send: play a device here into the M8's USB audio input."
        )?;
        writeln!(
            f,
            "# audio_send_input: what to send, or none for this computer's default"
        )?;
        writeln!(
            f,
            "# input; audio_send_output: which M8 device, or none to find it by name."
        )?;
        writeln!(f, "audio_send = {}", self.audio_send)?;
        writeln!(
            f,
            "audio_send_input = {}",
            name_or_none(&self.audio_send_input)
        )?;
        writeln!(
            f,
            "audio_send_output = {}",
            name_or_none(&self.audio_send_output)
        )?;
        writeln!(f)?;
        writeln!(
            f,
            "# Which keys press which M8 buttons. A key is either a single"
        )?;
        writeln!(
            f,
            "# character, a name such as Up, Space, Backspace, LeftShift or F1, or"
        )?;
        writeln!(
            f,
            "# a controller button such as PadSouth, PadUp or PadR1; separate"
        )?;
        writeln!(f, "# several with commas.")?;
        for button in Button::ALL {
            let keys = self.bindings.0.get(&button).cloned().unwrap_or_default();
            writeln!(f, "{} = {}", button.config_key(), keys.join(","))?;
        }
        writeln!(f)?;
        writeln!(
            f,
            "# The MIDI output the control-change bindings send to, by name; a"
        )?;
        writeln!(f, "# partial name matches, and none sends nothing.")?;
        writeln!(
            f,
            "midi_port = {}",
            self.midi_port.clone().unwrap_or_else(|| "none".into())
        )?;
        writeln!(f)?;
        writeln!(
            f,
            "# Control-change bindings, one per slot. A slot with key=none does"
        )?;
        writeln!(f, "# nothing. Fields:")?;
        writeln!(f, "#   key         the key that fires it")?;
        writeln!(f, "#   channel     1 to 16")?;
        writeln!(f, "#   controller  0 to 127")?;
        writeln!(
            f,
            "#   shape       static (one value) or ramp (travel over time)"
        )?;
        writeln!(
            f,
            "#   trigger     oneshot (press fires it and it finishes on its own),"
        )?;
        writeln!(
            f,
            "#               gate (press goes out, release comes back) or"
        )?;
        writeln!(
            f,
            "#               toggle (press goes out, the next press comes back)"
        )?;
        writeln!(
            f,
            "#   from, to    the resting value and the one a press heads for"
        )?;
        writeln!(f, "#   time        how long a ramp takes, in milliseconds")?;
        writeln!(f, "#   curve       linear or log")?;
        for (slot, binding) in self.cc.iter().enumerate().take(cc::SLOTS) {
            writeln!(f, "{} = {}", CC_KEYS[slot], write_binding(binding))?;
        }
        Ok(())
    }
}

/// One binding as its config-file value.
fn write_binding(binding: &Binding) -> String {
    format!(
        "key={}, channel={}, controller={}, shape={}, trigger={}, from={}, to={}, time={}, curve={}",
        binding.key.clone().unwrap_or_else(|| "none".into()),
        binding.channel,
        binding.controller,
        binding.shape.label(),
        binding.trigger.label(),
        binding.from,
        binding.to,
        binding.time_ms,
        binding.curve.label(),
    )
}

/// Reads one binding back.
fn parse_binding(value: &str) -> Binding {
    let mut binding = Binding::default();
    for field in value.split(',') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        match name {
            "key" => {
                binding.key = match value {
                    "" | "none" => None,
                    key => Some(key.to_string()),
                }
            }
            "channel" => {
                if let Ok(channel) = value.parse::<u8>() {
                    binding.channel = channel.clamp(cc::MIN_CHANNEL, cc::MAX_CHANNEL);
                }
            }
            "controller" => {
                if let Ok(controller) = value.parse::<u8>() {
                    binding.controller = controller.min(cc::MAX_CONTROLLER);
                }
            }
            "shape" => {
                if let Some(shape) = Shape::parse(value) {
                    binding.shape = shape;
                }
            }
            "trigger" => {
                if let Some(trigger) = Trigger::parse(value) {
                    binding.trigger = trigger;
                }
            }
            "from" => {
                if let Ok(from) = value.parse::<u8>() {
                    binding.from = from.min(cc::MAX_VALUE);
                }
            }
            "to" => {
                if let Ok(to) = value.parse::<u8>() {
                    binding.to = to.min(cc::MAX_VALUE);
                }
            }
            "time" => {
                if let Ok(time) = value.parse::<u32>() {
                    binding.time_ms = time.min(cc::MAX_TIME_MS);
                }
            }
            "curve" => {
                if let Some(curve) = Curve::parse(value) {
                    binding.curve = curve;
                }
            }
            _ => {}
        }
    }
    binding
}

impl Config {
    /// `~/.config/uwot-m8/config.conf`, or the platform's own equivalent.
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("uwot-m8").join("config.conf"))
    }

    /// Where settings lived when this was called m8-display, still read so a
    /// rename does not cost anyone their bindings.
    fn legacy_path() -> Option<PathBuf> {
        Some(config_dir()?.join("m8-display").join("tui.conf"))
    }

    /// Reads the config file, falling back to defaults for anything broken.
    pub fn load() -> Self {
        for path in [Self::path(), Self::legacy_path()].into_iter().flatten() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Self::parse(&text);
            }
        }
        Self::default()
    }

    fn parse(text: &str) -> Self {
        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "show_controls" => match value {
                    "true" => config.show_controls = true,
                    "false" => config.show_controls = false,
                    _ => {}
                },
                "audio" => match value {
                    "true" => config.audio = true,
                    "false" => config.audio = false,
                    _ => {}
                },
                "theme" => {
                    if let Some(theme) = Theme::parse(value) {
                        config.theme = theme;
                    }
                }
                "colors" => {
                    if let Some(depth) = ColorDepth::parse(value) {
                        config.color_depth = depth;
                    }
                }
                "audio_input" => config.audio_input = parse_device(value),
                "audio_output" => config.audio_output = parse_device(value),
                "audio_send" => match value {
                    "true" => config.audio_send = true,
                    "false" => config.audio_send = false,
                    _ => {}
                },
                "audio_send_input" => config.audio_send_input = parse_device(value),
                "audio_send_output" => config.audio_send_output = parse_device(value),
                "midi_port" => {
                    config.midi_port = match value {
                        "" | "none" => None,
                        name => Some(name.to_string()),
                    }
                }
                other => {
                    if let Some(button) = Button::from_config_key(other) {
                        let keys: Vec<String> = value
                            .split(',')
                            .map(str::trim)
                            .filter(|key| !key.is_empty())
                            .map(str::to_string)
                            .collect();
                        config.bindings.0.insert(button, keys);
                    } else if let Some(slot) = CC_KEYS.iter().position(|key| *key == other) {
                        config.cc[slot] = parse_binding(value);
                    }
                }
            }
        }
        config
    }

    /// What a key or controller button does, if anything.
    pub fn bound(&self, name: &str) -> Option<Bound> {
        if let Some(slot) = self.cc_slot_for(name) {
            return Some(Bound::Cc(slot));
        }
        self.bindings.button_for(name).map(Bound::Button)
    }

    /// The M8 button a name presses, if it presses one rather than firing a CC.
    pub fn button_bound(&self, name: &str) -> Option<Button> {
        match self.bound(name) {
            Some(Bound::Button(_)) => self.bindings.button_named(name),
            _ => None,
        }
    }

    /// The control-change slot a key name fires, if any.
    pub fn cc_slot_for(&self, name: &str) -> Option<usize> {
        self.cc
            .iter()
            .position(|binding| binding.key.as_deref() == Some(name))
    }

    /// Binds a key, taking it off whatever else had it: one key, one job.
    pub fn bind(&mut self, target: BindTarget, name: &str) {
        self.bindings.forget(name);
        for binding in self.cc.iter_mut() {
            if binding.key.as_deref() == Some(name) {
                binding.key = None;
            }
        }
        match target {
            BindTarget::Button(button) => self.bindings.bind(button, name),
            BindTarget::Cc(slot) => {
                if let Some(binding) = self.cc.get_mut(slot) {
                    binding.key = Some(name.to_string());
                }
            }
        }
    }

    /// The current value of a setting, for display.
    pub fn value(&self, setting: Setting) -> String {
        match setting {
            Setting::Controls => {
                if self.show_controls {
                    "shown".into()
                } else {
                    "hidden".into()
                }
            }
            Setting::Theme => self.theme.label().into(),
            Setting::ColorDepth => self.color_depth.label().into(),
            Setting::Audio => match (self.audio, self.audio_send) {
                (true, true) => "play, send".into(),
                (true, false) => "play".into(),
                (false, true) => "send".into(),
                (false, false) => "off".into(),
            },
            Setting::AudioOn => if self.audio { "on" } else { "off" }.into(),
            Setting::AudioInput => match &self.audio_input {
                Some(name) => name.clone(),
                None => "the M8".into(),
            },
            Setting::AudioOutput => match &self.audio_output {
                Some(name) => name.clone(),
                None => "default".into(),
            },
            Setting::AudioSendOn => if self.audio_send { "on" } else { "off" }.into(),
            Setting::AudioSendInput => match &self.audio_send_input {
                Some(name) => match name.strip_prefix(audio::APP_PREFIX) {
                    Some(app) => format!("{app} (app)"),
                    None => name.clone(),
                },
                None => "default".into(),
            },
            Setting::AudioSendOutput => match &self.audio_send_output {
                Some(name) => name.clone(),
                None => "the M8".into(),
            },
            Setting::Buttons => ">".into(),
            Setting::Key(button) => self.bindings.keys(button),
            Setting::ResetKeys => "press to apply".into(),
            Setting::Midi => match &self.midi_port {
                Some(port) => port.clone(),
                None => "no port".into(),
            },
            Setting::MidiPort => match &self.midi_port {
                Some(port) => port.clone(),
                None => "none".into(),
            },
            Setting::CcSlot(slot) => self.binding(slot).summary(),
            Setting::CcKey(slot) => match &self.binding(slot).key {
                Some(key) => key.clone(),
                None => "unbound".into(),
            },
            Setting::CcChannel(slot) => self.binding(slot).channel.to_string(),
            Setting::CcController(slot) => self.binding(slot).controller.to_string(),
            Setting::CcShape(slot) => self.binding(slot).shape.label().into(),
            Setting::CcTrigger(slot) => self.binding(slot).trigger.label().into(),
            Setting::CcFrom(slot) => self.binding(slot).from.to_string(),
            Setting::CcTo(slot) => self.binding(slot).to.to_string(),
            Setting::CcTime(slot) => match self.binding(slot).shape {
                Shape::Static => "-".into(),
                Shape::Ramp => cc::format_time(self.binding(slot).time_ms),
            },
            Setting::CcCurve(slot) => match self.binding(slot).shape {
                Shape::Static => "-".into(),
                Shape::Ramp => self.binding(slot).curve.label().into(),
            },
            Setting::CcClear(_) => "press to apply".into(),
            Setting::Back => String::new(),
        }
    }

    fn binding(&self, slot: usize) -> &Binding {
        self.cc.get(slot).unwrap_or(&DEFAULT_BINDING)
    }

    /// Changes a setting by `delta` steps, positive or negative.
    pub fn adjust(&mut self, setting: Setting, delta: i32, env: &MenuEnv) -> Changed {
        if delta == 0 {
            return Changed::default();
        }
        let forwards = delta > 0;
        match setting {
            Setting::Controls => {
                self.show_controls = !self.show_controls;
                Changed::default()
            }
            Setting::Theme => {
                self.theme = self.theme.next();
                Changed::display()
            }
            Setting::ColorDepth => {
                self.color_depth = self.color_depth.next();
                Changed::display()
            }
            Setting::AudioOn => {
                self.audio = !self.audio;
                Changed::audio()
            }
            Setting::AudioInput => {
                self.audio_input =
                    next_port(self.audio_input.as_deref(), &env.audio_inputs, forwards);
                Changed::audio()
            }
            Setting::AudioOutput => {
                self.audio_output =
                    next_port(self.audio_output.as_deref(), &env.audio_outputs, forwards);
                Changed::audio()
            }
            Setting::AudioSendOn => {
                self.audio_send = !self.audio_send;
                Changed::audio()
            }
            Setting::AudioSendInput => {
                // One row for either, applications first: a particular one is
                // more often what someone is after than a whole device.
                let mut choices: Vec<String> = env
                    .audio_apps
                    .iter()
                    .map(|app| format!("{}{app}", audio::APP_PREFIX))
                    .collect();
                choices.extend(env.audio_sources.iter().cloned());
                self.audio_send_input =
                    next_port(self.audio_send_input.as_deref(), &choices, forwards);
                Changed::audio()
            }
            Setting::AudioSendOutput => {
                self.audio_send_output = next_port(
                    self.audio_send_output.as_deref(),
                    &env.audio_outputs,
                    forwards,
                );
                Changed::audio()
            }
            Setting::MidiPort => {
                self.midi_port = next_port(self.midi_port.as_deref(), &env.midi_ports, forwards);
                Changed::midi()
            }
            Setting::Key(_)
            | Setting::CcKey(_)
            | Setting::Buttons
            | Setting::Audio
            | Setting::Midi
            | Setting::CcSlot(_)
            | Setting::ResetKeys
            | Setting::CcClear(_)
            | Setting::Back => Changed::default(),
            Setting::CcChannel(slot) => self.edit(slot, |binding| {
                binding.channel = step(binding.channel, delta, cc::MIN_CHANNEL, cc::MAX_CHANNEL);
            }),
            Setting::CcController(slot) => self.edit(slot, |binding| {
                binding.controller = step(binding.controller, delta, 0, cc::MAX_CONTROLLER);
            }),
            Setting::CcShape(slot) => self.edit(slot, |binding| {
                binding.shape = binding.shape.next();
            }),
            Setting::CcTrigger(slot) => self.edit(slot, |binding| {
                binding.trigger = binding.trigger.next();
            }),
            Setting::CcFrom(slot) => self.edit(slot, |binding| {
                binding.from = step(binding.from, delta, 0, cc::MAX_VALUE);
            }),
            Setting::CcTo(slot) => self.edit(slot, |binding| {
                binding.to = step(binding.to, delta, 0, cc::MAX_VALUE);
            }),
            Setting::CcTime(slot) => self.edit(slot, |binding| {
                let moved = binding.time_ms as i32 + delta * cc::TIME_STEP_MS as i32;
                binding.time_ms = moved.clamp(0, cc::MAX_TIME_MS as i32) as u32;
            }),
            Setting::CcCurve(slot) => self.edit(slot, |binding| {
                binding.curve = binding.curve.next();
            }),
        }
    }

    fn edit(&mut self, slot: usize, change: impl FnOnce(&mut Binding)) -> Changed {
        if let Some(binding) = self.cc.get_mut(slot) {
            change(binding);
        }
        Changed::default()
    }

    /// Activates a row that does something rather than holding a value.
    pub fn activate(&mut self, setting: Setting) -> bool {
        match setting {
            Setting::ResetKeys => {
                self.bindings = Bindings::default();
                true
            }
            Setting::CcClear(slot) => {
                if let Some(binding) = self.cc.get_mut(slot) {
                    *binding = Binding::default();
                }
                true
            }
            _ => false,
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("cannot work out where to save the config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, self.to_string()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Stands in for a slot that is somehow missing.
static DEFAULT_BINDING: Binding = Binding {
    key: None,
    channel: cc::MIN_CHANNEL,
    controller: 1,
    shape: Shape::Static,
    trigger: Trigger::Oneshot,
    from: 0,
    to: cc::MAX_VALUE,
    time_ms: 1000,
    curve: Curve::Linear,
};

/// Where settings live: `~/.config` on Unix, `%APPDATA%` on Windows.
fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return Some(PathBuf::from(appdata));
        }
    }
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(home_dir()?).join(".config")),
    }
}

fn home_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
}

/// Moves a number by `delta`, staying between `low` and `high`.
fn step(value: u8, delta: i32, low: u8, high: u8) -> u8 {
    (value as i32 + delta).clamp(low as i32, high as i32) as u8
}

/// A device name for the config file, or `none` where nothing is set.
fn name_or_none(name: &Option<String>) -> &str {
    name.as_deref().unwrap_or("none")
}

/// A device name read back out of the config file.
fn parse_device(value: &str) -> Option<String> {
    match value {
        "" | "none" => None,
        name => Some(name.to_string()),
    }
}

/// The next port or audio device along, with "none" among the choices.
fn next_port(current: Option<&str>, ports: &[String], forwards: bool) -> Option<String> {
    let mut choices: Vec<Option<&str>> = vec![None];
    choices.extend(ports.iter().map(|port| Some(port.as_str())));
    if let Some(current) = current {
        if !choices.contains(&Some(current)) {
            choices.push(Some(current));
        }
    }
    let count = choices.len();
    let at = choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or(0);
    let next = if forwards { at + 1 } else { at + count - 1 } % count;
    choices[next].map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_show_the_controls_in_the_devices_own_colours_and_stay_quiet() {
        let config = Config::default();
        assert!(config.show_controls);
        assert_eq!(config.theme, Theme::M8);
        assert_eq!(config.color_depth, ColorDepth::Auto);
        assert!(!config.audio);
        assert_eq!(config.midi_port, None);
        assert_eq!(config.cc.len(), cc::SLOTS);
        assert!(config.cc.iter().all(|binding| !binding.is_bound()));
    }

    #[test]
    fn a_saved_config_reads_back_the_same() {
        let mut config = Config {
            show_controls: false,
            theme: Theme::Terminal,
            color_depth: ColorDepth::Ansi256,
            audio: true,
            midi_port: Some("M8 MIDI 1".into()),
            ..Config::default()
        };
        config.bind(BindTarget::Button(Button::Edit), "q");
        config.cc[2] = Binding {
            key: Some("F3".into()),
            channel: 7,
            controller: 74,
            shape: Shape::Ramp,
            trigger: Trigger::Gate,
            from: 10,
            to: 120,
            time_ms: 2500,
            curve: Curve::Log,
        };
        assert_eq!(Config::parse(&config.to_string()), config);
    }

    #[test]
    fn binding_a_key_takes_it_off_whatever_else_had_it() {
        let mut config = Config::default();
        assert_eq!(config.bindings.button_for("z"), Some(bits::SELECT));

        config.bind(BindTarget::Button(Button::Edit), "z");
        assert_eq!(config.bindings.button_for("z"), Some(bits::EDIT));
        assert_eq!(config.bindings.keys(Button::Select), "LeftShift, PadWest");
    }

    #[test]
    fn a_key_never_both_presses_a_button_and_sends_a_control_change() {
        let mut config = Config::default();
        config.bind(BindTarget::Cc(0), "x");
        assert_eq!(config.cc_slot_for("x"), Some(0));
        assert_eq!(config.bindings.button_for("x"), None);
        assert_eq!(config.bindings.keys(Button::Start), "Space, PadSouth");

        config.bind(BindTarget::Button(Button::Start), "x");
        assert_eq!(config.cc_slot_for("x"), None);
        assert_eq!(config.bindings.button_for("x"), Some(bits::START));

        config.bind(BindTarget::Cc(1), "F1");
        config.bind(BindTarget::Cc(4), "F1");
        assert_eq!(config.cc_slot_for("F1"), Some(4));
    }

    #[test]
    fn an_unknown_key_name_presses_nothing() {
        assert_eq!(Bindings::default().button_for("F13"), None);
    }

    #[test]
    fn unknown_and_broken_lines_are_ignored() {
        let config = Config::parse(
            "# a comment\n\nshow_controls = false\nnonsense\nwidth = 3\ntheme = bogus\n",
        );
        assert!(!config.show_controls);
        assert_eq!(config.theme, Theme::M8);
    }

    #[test]
    fn a_broken_field_in_a_binding_costs_only_that_field() {
        let config = Config::parse("cc1 = key=F5, channel=99, controller=nonsense, shape=ramp\n");
        let binding = &config.cc[0];
        assert_eq!(binding.key.as_deref(), Some("F5"));
        assert_eq!(binding.channel, cc::MAX_CHANNEL);
        assert_eq!(binding.controller, Binding::default().controller);
        assert_eq!(binding.shape, Shape::Ramp);
    }

    #[test]
    fn numbers_move_by_the_step_and_stop_at_their_limits() {
        let mut config = Config::default();
        let env = MenuEnv::default();
        config.adjust(Setting::CcController(0), 10, &env);
        assert_eq!(config.cc[0].controller, 11);
        config.adjust(Setting::CcController(0), -100, &env);
        assert_eq!(config.cc[0].controller, 0);
        config.adjust(Setting::CcChannel(0), -1, &env);
        assert_eq!(config.cc[0].channel, cc::MIN_CHANNEL);
        config.adjust(Setting::CcTime(0), 2, &env);
        assert_eq!(config.cc[0].time_ms, 1000 + 2 * cc::TIME_STEP_MS);
    }

    #[test]
    fn the_port_row_cycles_through_what_is_plugged_in_plus_none() {
        let env = MenuEnv {
            midi_ports: vec!["M8 MIDI 1".into(), "Midi Through".into()],
            ..MenuEnv::default()
        };
        let mut config = Config::default();
        assert_eq!(config.adjust(Setting::MidiPort, 1, &env), Changed::midi());
        assert_eq!(config.midi_port.as_deref(), Some("M8 MIDI 1"));
        config.adjust(Setting::MidiPort, 1, &env);
        assert_eq!(config.midi_port.as_deref(), Some("Midi Through"));
        config.adjust(Setting::MidiPort, 1, &env);
        assert_eq!(config.midi_port, None);
        config.adjust(Setting::MidiPort, -1, &env);
        assert_eq!(config.midi_port.as_deref(), Some("Midi Through"));
    }

    #[test]
    fn a_port_that_is_not_plugged_in_is_one_of_the_choices_while_it_is_the_one_set() {
        let mut config = Config {
            midi_port: Some("Somewhere else".into()),
            ..Config::default()
        };
        config.adjust(Setting::MidiPort, 1, &MenuEnv::default());
        assert_eq!(config.midi_port, None);

        let env = MenuEnv {
            midi_ports: vec!["Somewhere else".into()],
            ..MenuEnv::default()
        };
        config.adjust(Setting::MidiPort, 1, &env);
        assert_eq!(config.midi_port.as_deref(), Some("Somewhere else"));
    }

    #[test]
    fn turning_audio_on_tells_the_app_to_start_it() {
        let mut config = Config::default();
        let changed = config.adjust(Setting::AudioOn, 1, &MenuEnv::default());
        assert!(config.audio);
        assert_eq!(changed, Changed::audio());
    }

    #[test]
    fn the_audio_device_rows_cycle_through_what_is_plugged_in_plus_automatic() {
        let env = MenuEnv {
            audio_inputs: vec!["M8 Analog Stereo".into(), "Scarlett Solo".into()],
            audio_outputs: vec!["Headphones".into()],
            ..MenuEnv::default()
        };
        let mut config = Config::default();
        assert_eq!(config.value(Setting::AudioInput), "the M8");

        assert_eq!(
            config.adjust(Setting::AudioInput, 1, &env),
            Changed::audio()
        );
        assert_eq!(config.audio_input.as_deref(), Some("M8 Analog Stereo"));
        config.adjust(Setting::AudioInput, -1, &env);
        assert_eq!(config.audio_input, None);

        config.adjust(Setting::AudioOutput, 1, &env);
        assert_eq!(config.audio_output.as_deref(), Some("Headphones"));
        config.adjust(Setting::AudioOutput, 1, &env);
        assert_eq!(config.audio_output, None);
    }

    #[test]
    fn the_audio_devices_survive_a_trip_through_the_config_file() {
        let config = Config {
            audio: true,
            audio_input: Some("M8 Analog Stereo".into()),
            audio_output: Some("Headphones".into()),
            audio_send: true,
            audio_send_input: Some("Monitor of Headphones".into()),
            audio_send_output: Some("M8, USB Audio".into()),
            ..Config::default()
        };
        let read_back = Config::parse(&config.to_string());
        assert_eq!(read_back, config);

        let automatic = Config::parse(&Config::default().to_string());
        assert_eq!(automatic, Config::default());
    }

    #[test]
    fn the_send_row_offers_applications_before_devices_and_says_which_is_which() {
        let env = MenuEnv {
            audio_apps: vec!["Zen".into()],
            audio_sources: vec!["Monitor of Headphones".into()],
            ..MenuEnv::default()
        };
        let mut config = Config::default();

        config.adjust(Setting::AudioSendInput, 1, &env);
        assert_eq!(config.audio_send_input.as_deref(), Some("app:Zen"));
        assert_eq!(config.value(Setting::AudioSendInput), "Zen (app)");

        config.adjust(Setting::AudioSendInput, 1, &env);
        assert_eq!(
            config.audio_send_input.as_deref(),
            Some("Monitor of Headphones")
        );
        assert_eq!(
            config.value(Setting::AudioSendInput),
            "Monitor of Headphones"
        );

        config.adjust(Setting::AudioSendInput, 1, &env);
        assert_eq!(config.audio_send_input, None);
        assert_eq!(config.value(Setting::AudioSendInput), "default");
    }

    #[test]
    fn an_application_survives_a_trip_through_the_config_file() {
        let config = Config {
            audio_send: true,
            audio_send_input: Some("app:Zen".into()),
            ..Config::default()
        };
        assert_eq!(Config::parse(&config.to_string()), config);
    }

    #[test]
    fn the_two_directions_are_turned_on_and_off_apart() {
        let mut config = Config::default();
        assert_eq!(config.value(Setting::Audio), "off");

        config.adjust(Setting::AudioSendOn, 1, &MenuEnv::default());
        assert!(config.audio_send);
        assert!(!config.audio);
        assert_eq!(config.value(Setting::Audio), "send");

        config.adjust(Setting::AudioOn, 1, &MenuEnv::default());
        assert_eq!(config.value(Setting::Audio), "play, send");
    }

    #[test]
    fn the_send_rows_cycle_through_the_sources_and_the_outputs() {
        let env = MenuEnv {
            audio_sources: vec!["Monitor of Headphones".into()],
            audio_outputs: vec!["M8, USB Audio".into()],
            ..MenuEnv::default()
        };
        let mut config = Config::default();
        assert_eq!(
            config.adjust(Setting::AudioSendInput, 1, &env),
            Changed::audio()
        );
        assert_eq!(
            config.audio_send_input.as_deref(),
            Some("Monitor of Headphones")
        );
        config.adjust(Setting::AudioSendOutput, 1, &env);
        assert_eq!(config.audio_send_output.as_deref(), Some("M8, USB Audio"));
        config.adjust(Setting::AudioSendOutput, 1, &env);
        assert_eq!(config.audio_send_output, None);
    }

    #[test]
    fn clearing_a_slot_unbinds_it_and_puts_its_fields_back() {
        let mut config = Config::default();
        config.bind(BindTarget::Cc(3), "F4");
        config.adjust(Setting::CcTo(3), -27, &MenuEnv::default());
        assert!(config.activate(Setting::CcClear(3)));
        assert_eq!(config.cc[3], Binding::default());
        assert_eq!(config.cc_slot_for("F4"), None);
    }

    #[test]
    fn every_page_offers_a_way_back_out() {
        for page in [
            Setting::Buttons,
            Setting::Audio,
            Setting::Midi,
            Setting::CcSlot(0),
        ] {
            let rows = page.page().expect("a page");
            assert_eq!(rows.last(), Some(&Setting::Back));
        }
        assert!(Setting::Controls.page().is_none());
    }
}
