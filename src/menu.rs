//! The settings menu's state: the open page, the selected row, and whether it
//! is waiting for a key to bind.

use crate::config::{Button, Changed, Config, MenuEnv, Setting};

/// What a keypress means to the menu, once the frontend has decoded it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Previous,
    Next,
    /// Left or right: a step of one, or a coarser one with a modifier held.
    Adjust(i32),
    /// Enter or space: opens a page, starts a binding, or does the row's job.
    Activate,
    /// Escape: cancels a binding, leaves a page, or closes the menu.
    Back,
    /// The name of the key just pressed, while the menu is capturing one.
    Bind(Option<String>),
}

/// What the app has to do once the menu has handled a key.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub changed: Changed,
    /// The config was altered and should be written out.
    pub write: bool,
    /// The menu has closed.
    pub closed: bool,
    /// The audio devices and applications want looking up again, because the
    /// page that lists them has just been opened.
    pub rescan: bool,
    /// Something worth telling the user, e.g. a key that cannot be bound.
    pub message: Option<String>,
}

/// What a key or button bound to a direction means to the menu.
///
/// The menu answers to whatever drives the M8, so rebinding Up moves the
/// selection up too. Escape is left out on purpose: there has to be one way
/// back out of the menu that no binding can take away.
pub fn bound_action(config: &Config, name: &str, step: i32) -> Option<Action> {
    if name == "Escape" {
        return None;
    }
    Some(match config.button_bound(name)? {
        Button::Up => Action::Previous,
        Button::Down => Action::Next,
        Button::Left => Action::Adjust(-step),
        Button::Right => Action::Adjust(step),
        _ => return None,
    })
}

/// What a controller button means to the menu.
///
/// Its binding first, then the face buttons, so a pad still drives the menu
/// when its d-pad has been bound to something else.
pub fn pad_action(config: &Config, name: &str, capturing: bool) -> Option<Action> {
    if capturing {
        return Some(Action::Bind(Some(name.to_string())));
    }
    if let Some(action) = bound_action(config, name, 1) {
        return Some(action);
    }
    Some(match name {
        "PadUp" => Action::Previous,
        "PadDown" => Action::Next,
        "PadLeft" => Action::Adjust(-1),
        "PadRight" => Action::Adjust(1),
        "PadSouth" | "PadStart" => Action::Activate,
        "PadEast" | "PadSelect" => Action::Back,
        _ => return None,
    })
}

struct Page {
    title: &'static str,
    rows: Vec<Setting>,
    selected: usize,
}

pub struct Menu {
    /// The page stack, the open one last. Never empty.
    pages: Vec<Page>,
    /// Set while every key means itself rather than its usual job here.
    capturing: bool,
}

impl Menu {
    /// Opens the menu at its top-level page.
    pub fn new(rows: Vec<Setting>) -> Self {
        Self {
            pages: vec![Page {
                title: "Settings",
                rows,
                selected: 0,
            }],
            capturing: false,
        }
    }

    pub fn title(&self) -> &'static str {
        self.page().title
    }

    pub fn selected(&self) -> usize {
        self.page().selected
    }

    /// True while the menu is waiting for a key to bind.
    pub fn is_capturing(&self) -> bool {
        self.capturing
    }

    /// The open page's rows as label and value.
    pub fn items(&self, config: &Config) -> Vec<(String, String)> {
        let page = self.page();
        page.rows
            .iter()
            .enumerate()
            .map(|(index, &setting)| {
                let value = if self.capturing && index == page.selected {
                    "press a key...".to_string()
                } else {
                    config.value(setting)
                };
                (setting.label().to_string(), value)
            })
            .collect()
    }

    /// The line along the bottom of the panel.
    pub fn footer(&self) -> &'static str {
        if self.capturing {
            return "press the key to bind   Esc cancel";
        }
        match self.selected_setting() {
            setting if setting.page().is_some() => "up/down select   Enter open   Esc back",
            setting if setting.bind_target().is_some() => "up/down select   Enter bind   Esc back",
            Setting::Back => "up/down select   Enter back   Esc back",
            _ => "up/down select   left/right change   Esc back",
        }
    }

    /// Handles one action, returning what the app has to do about it.
    pub fn handle(&mut self, action: Action, config: &mut Config, env: &MenuEnv) -> Outcome {
        if self.capturing {
            return self.capture(action, config);
        }

        match action {
            Action::Previous => {
                let count = self.page().rows.len();
                let page = self.page_mut();
                page.selected = (page.selected + count - 1) % count;
                Outcome::default()
            }
            Action::Next => {
                let count = self.page().rows.len();
                let page = self.page_mut();
                page.selected = (page.selected + 1) % count;
                Outcome::default()
            }
            Action::Adjust(delta) => {
                let setting = self.selected_setting();
                let changed = config.adjust(setting, delta, env);
                let write = !matches!(setting, Setting::Back)
                    && setting.page().is_none()
                    && setting.bind_target().is_none();
                Outcome {
                    changed,
                    write,
                    ..Outcome::default()
                }
            }
            Action::Activate => self.activate(config, env),
            Action::Back => self.leave(),
            Action::Bind(_) => Outcome::default(),
        }
    }

    /// Enter on a row: open its page, start binding, do its job, or change it.
    fn activate(&mut self, config: &mut Config, env: &MenuEnv) -> Outcome {
        let setting = self.selected_setting();
        if let Some(rows) = setting.page() {
            self.pages.push(Page {
                title: setting.page_title(),
                rows,
                selected: 0,
            });
            return Outcome {
                rescan: setting == Setting::Audio,
                ..Outcome::default()
            };
        }
        if setting == Setting::Back {
            return self.leave();
        }
        if setting.bind_target().is_some() {
            self.capturing = true;
            return Outcome::default();
        }
        if config.activate(setting) {
            return Outcome {
                write: true,
                ..Outcome::default()
            };
        }
        let changed = config.adjust(setting, 1, env);
        Outcome {
            changed,
            write: true,
            ..Outcome::default()
        }
    }

    /// Escape, or Enter on a Back row: out of the page, or out of the menu.
    fn leave(&mut self) -> Outcome {
        if self.pages.len() > 1 {
            self.pages.pop();
            return Outcome::default();
        }
        Outcome {
            closed: true,
            write: true,
            ..Outcome::default()
        }
    }

    /// While capturing, every key means itself.
    fn capture(&mut self, action: Action, config: &mut Config) -> Outcome {
        let Action::Bind(name) = action else {
            self.capturing = false;
            return Outcome::default();
        };
        self.capturing = false;
        let Some(target) = self.selected_setting().bind_target() else {
            return Outcome::default();
        };
        match name {
            Some(name) => {
                config.bind(target, &name);
                Outcome {
                    write: true,
                    ..Outcome::default()
                }
            }
            None => Outcome {
                message: Some("that key cannot be bound".into()),
                ..Outcome::default()
            },
        }
    }

    fn selected_setting(&self) -> Setting {
        let page = self.page();
        page.rows
            .get(page.selected)
            .copied()
            .unwrap_or(Setting::Back)
    }

    fn page(&self) -> &Page {
        self.pages.last().expect("the menu always has a page open")
    }

    fn page_mut(&mut self) -> &mut Page {
        self.pages
            .last_mut()
            .expect("the menu always has a page open")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc::{Shape, Trigger};
    use crate::config::BindTarget;

    fn open() -> (Menu, Config, MenuEnv) {
        (
            Menu::new(Setting::terminal()),
            Config::default(),
            MenuEnv::default(),
        )
    }

    /// Moves the selection onto the row holding `setting`.
    fn select(menu: &mut Menu, config: &mut Config, env: &MenuEnv, setting: Setting) {
        for _ in 0..64 {
            if menu.selected_setting() == setting {
                return;
            }
            menu.handle(Action::Next, config, env);
        }
        panic!("no row for {setting:?}");
    }

    #[test]
    fn a_controller_drives_the_menu_and_binds_itself() {
        let config = Config::default();
        assert_eq!(pad_action(&config, "PadDown", false), Some(Action::Next));
        assert_eq!(pad_action(&config, "PadEast", false), Some(Action::Back));
        assert_eq!(
            pad_action(&config, "PadRight", false),
            Some(Action::Adjust(1))
        );
        assert_eq!(pad_action(&config, "PadL2", false), None);
        assert_eq!(
            pad_action(&config, "PadDown", true),
            Some(Action::Bind(Some("PadDown".into())))
        );
    }

    #[test]
    fn the_keys_bound_to_the_directions_drive_the_menu() {
        let mut config = Config::default();
        config.bind(BindTarget::Button(Button::Up), "w");
        config.bind(BindTarget::Button(Button::Down), "s");
        config.bind(BindTarget::Button(Button::Left), "a");
        config.bind(BindTarget::Button(Button::Right), "d");

        assert_eq!(bound_action(&config, "w", 1), Some(Action::Previous));
        assert_eq!(bound_action(&config, "s", 1), Some(Action::Next));
        assert_eq!(bound_action(&config, "a", 10), Some(Action::Adjust(-10)));
        assert_eq!(bound_action(&config, "d", 10), Some(Action::Adjust(10)));
        // A button that is not a direction is left to the frontend.
        assert_eq!(bound_action(&config, "x", 1), None);
    }

    #[test]
    fn a_rebound_pad_still_drives_the_menu_by_its_d_pad() {
        let mut config = Config::default();
        config.bind(BindTarget::Button(Button::Up), "PadL1");
        assert_eq!(pad_action(&config, "PadL1", false), Some(Action::Previous));
        assert_eq!(pad_action(&config, "PadUp", false), Some(Action::Previous));
    }

    /// Whatever else it is bound to, Escape has to keep closing the menu.
    #[test]
    fn escape_is_never_taken_over_by_a_binding() {
        let mut config = Config::default();
        config.bind(BindTarget::Button(Button::Down), "Escape");
        assert_eq!(bound_action(&config, "Escape", 1), None);
    }

    #[test]
    fn a_key_that_fires_a_cc_does_not_move_the_selection() {
        let mut config = Config::default();
        config.bind(BindTarget::Button(Button::Up), "w");
        config.bind(BindTarget::Cc(0), "w");
        assert_eq!(bound_action(&config, "w", 1), None);
    }

    #[test]
    fn escape_from_the_top_closes_the_menu_and_saves() {
        let (mut menu, mut config, env) = open();
        let outcome = menu.handle(Action::Back, &mut config, &env);
        assert!(outcome.closed);
        assert!(outcome.write);
    }

    #[test]
    fn a_page_opens_and_closes_without_closing_the_menu() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Buttons);
        menu.handle(Action::Activate, &mut config, &env);
        assert_eq!(menu.title(), "Buttons");

        let outcome = menu.handle(Action::Back, &mut config, &env);
        assert!(!outcome.closed);
        assert_eq!(menu.title(), "Settings");
    }

    #[test]
    fn the_back_row_leaves_the_page_it_is_on() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Midi);
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcSlot(0));
        menu.handle(Action::Activate, &mut config, &env);
        assert_eq!(menu.title(), "CC 1");

        select(&mut menu, &mut config, &env, Setting::Back);
        assert!(!menu.handle(Action::Activate, &mut config, &env).closed);
        assert_eq!(menu.title(), "MIDI");
    }

    #[test]
    fn a_binding_row_takes_the_next_key_pressed() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Midi);
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcSlot(1));
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcKey(1));

        menu.handle(Action::Activate, &mut config, &env);
        assert!(menu.is_capturing());
        let items = menu.items(&config);
        assert_eq!(items[0].1, "press a key...");

        let outcome = menu.handle(Action::Bind(Some("F2".into())), &mut config, &env);
        assert!(outcome.write);
        assert!(!menu.is_capturing());
        assert_eq!(config.cc_slot_for("F2"), Some(1));
    }

    #[test]
    fn a_key_with_no_name_is_refused_rather_than_bound() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Buttons);
        menu.handle(Action::Activate, &mut config, &env);
        select(
            &mut menu,
            &mut config,
            &env,
            Setting::Key(crate::config::Button::Up),
        );
        menu.handle(Action::Activate, &mut config, &env);

        let outcome = menu.handle(Action::Bind(None), &mut config, &env);
        assert_eq!(outcome.message.as_deref(), Some("that key cannot be bound"));
        assert!(!outcome.write);
        assert_eq!(config.bindings.keys(crate::config::Button::Up), "Up, PadUp");
    }

    #[test]
    fn escape_while_capturing_cancels_the_binding_and_keeps_the_page() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Buttons);
        menu.handle(Action::Activate, &mut config, &env);
        menu.handle(Action::Activate, &mut config, &env);
        assert!(menu.is_capturing());

        let outcome = menu.handle(Action::Back, &mut config, &env);
        assert!(!menu.is_capturing());
        assert!(!outcome.closed);
        assert_eq!(menu.title(), "Buttons");
    }

    #[test]
    fn enter_on_a_value_row_moves_it_on_like_right_does() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Audio);
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::AudioOn);
        let outcome = menu.handle(Action::Activate, &mut config, &env);
        assert!(config.audio);
        assert!(outcome.changed.audio);
        assert!(outcome.write);
    }

    #[test]
    fn opening_the_audio_page_asks_for_the_devices_to_be_looked_up_again() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Audio);
        assert!(menu.handle(Action::Activate, &mut config, &env).rescan);

        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Buttons);
        assert!(!menu.handle(Action::Activate, &mut config, &env).rescan);
    }

    #[test]
    fn the_audio_page_lists_both_directions_and_their_devices() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Audio);
        menu.handle(Action::Activate, &mut config, &env);
        assert_eq!(menu.title(), "Audio");
        let items = menu.items(&config);
        let labels: Vec<&str> = items.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Play",
                "Play from",
                "Play to",
                "Send",
                "Send from",
                "Send into",
                "Back"
            ]
        );
        let values: Vec<&str> = items.iter().map(|(_, value)| value.as_str()).collect();
        assert_eq!(
            values[..6],
            ["off", "the M8", "default", "off", "default", "the M8"]
        );
    }

    #[test]
    fn a_coarse_step_moves_a_number_further() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Midi);
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcSlot(0));
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcController(0));

        menu.handle(Action::Adjust(10), &mut config, &env);
        assert_eq!(config.cc[0].controller, 11);
        menu.handle(Action::Adjust(-1), &mut config, &env);
        assert_eq!(config.cc[0].controller, 10);
    }

    #[test]
    fn the_footer_says_what_the_selected_row_does() {
        let (mut menu, mut config, env) = open();
        select(&mut menu, &mut config, &env, Setting::Buttons);
        assert!(menu.footer().contains("open"));
        menu.handle(Action::Activate, &mut config, &env);
        assert!(menu.footer().contains("bind"));
        menu.handle(Action::Activate, &mut config, &env);
        assert!(menu.footer().contains("press the key"));
    }

    #[test]
    fn a_static_slot_shows_no_time_or_curve() {
        let (mut menu, mut config, env) = open();
        config.cc[0].shape = Shape::Static;
        config.cc[0].trigger = Trigger::Oneshot;
        select(&mut menu, &mut config, &env, Setting::Midi);
        menu.handle(Action::Activate, &mut config, &env);
        select(&mut menu, &mut config, &env, Setting::CcSlot(0));
        menu.handle(Action::Activate, &mut config, &env);

        let items = menu.items(&config);
        let time = items
            .iter()
            .find(|(label, _)| label == "Time")
            .expect("a time row");
        assert_eq!(time.1, "-");
    }
}
