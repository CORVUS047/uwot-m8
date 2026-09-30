//! Game controllers, treated as another kind of key.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, MappingSource};

/// How far a stick has to be pushed to count as a press.
const ON: f32 = 0.5;
const OFF: f32 = 0.3;

/// One axis: which way it is pushed, and whether it has been seen at rest.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct AxisState {
    /// -1, 0 or 1.
    pushed: i8,
    /// Set once the axis has been seen near the middle.
    armed: bool,
}

/// A controller button going down or coming up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PadEvent {
    /// The name the config knows it by, e.g. `PadSouth`.
    pub name: String,
    pub down: bool,
}

/// Every controller the system offers, polled as one.
pub struct Pads {
    /// `None` when there is no way to read controllers.
    gilrs: Option<Gilrs>,
    /// Why controllers are unavailable, if they are.
    error: Option<String>,
    /// What each axis is doing.
    axes: Vec<(GamepadId, Axis, AxisState)>,
    /// Buttons currently held, per controller.
    held: Vec<(GamepadId, String)>,
}

impl Default for Pads {
    fn default() -> Self {
        Self::new()
    }
}

impl Pads {
    pub fn new() -> Self {
        match Gilrs::new() {
            Ok(gilrs) => Self { gilrs: Some(gilrs), error: None, axes: Vec::new(), held: Vec::new() },
            Err(e) => Self {
                gilrs: None,
                error: Some(format!("no controller support: {e}")),
                axes: Vec::new(),
                held: Vec::new(),
            },
        }
    }

    /// Everything the controllers have done since the last look.
    pub fn poll(&mut self) -> Vec<PadEvent> {
        let raw: Vec<gilrs::Event> = match self.gilrs.as_mut() {
            Some(gilrs) => std::iter::from_fn(|| gilrs.next_event()).collect(),
            None => return Vec::new(),
        };

        let mut events = Vec::new();
        for event in raw {
            let pad = event.id;
            let mapped = self.gilrs.as_ref().is_some_and(|gilrs| {
                matches!(
                    gilrs.gamepad(pad).mapping_source(),
                    MappingSource::SdlMappings | MappingSource::Driver
                )
            });
            match event.event {
                EventType::ButtonPressed(button, code) => {
                    let name = button_name(button, code.into_u32());
                    self.held.push((pad, name.clone()));
                    events.push(PadEvent { name, down: true });
                }
                EventType::ButtonReleased(button, code) => {
                    let name = button_name(button, code.into_u32());
                    self.held.retain(|(id, held)| !(*id == pad && *held == name));
                    events.push(PadEvent { name, down: false });
                }
                EventType::AxisChanged(axis, value, _) => {
                    self.axis_moved(pad, axis, value, mapped, &mut events);
                }
                EventType::Disconnected => {
                    for (id, name) in std::mem::take(&mut self.held) {
                        if id == pad {
                            events.push(PadEvent { name, down: false });
                        } else {
                            self.held.push((id, name));
                        }
                    }
                    self.axes.retain(|(id, _, _)| *id != pad);
                }
                _ => {}
            }
        }
        events
    }

    /// Turns a stick or hat position into presses and releases.
    fn axis_moved(
        &mut self,
        pad: GamepadId,
        axis: Axis,
        value: f32,
        mapped: bool,
        events: &mut Vec<PadEvent>,
    ) {
        let Some((negative, positive)) = axis_directions(axis) else { return };
        let was = self
            .axes
            .iter()
            .find(|(id, known, _)| *id == pad && *known == axis)
            .map(|(_, _, state)| *state)
            .unwrap_or(AxisState { pushed: 0, armed: mapped });
        let now = step_axis(was, value);
        if now == was {
            return;
        }
        match self.axes.iter_mut().find(|(id, known, _)| *id == pad && *known == axis) {
            Some(entry) => entry.2 = now,
            None => self.axes.push((pad, axis, now)),
        }
        if now.pushed == was.pushed {
            return;
        }

        let (was, now) = (was.pushed, now.pushed);
        let name_of = |direction: i8| match direction {
            -1 => Some(negative.to_string()),
            1 => Some(positive.to_string()),
            _ => None,
        };
        if let Some(name) = name_of(was) {
            events.push(PadEvent { name, down: false });
        }
        if let Some(name) = name_of(now) {
            events.push(PadEvent { name, down: true });
        }
    }

    /// The controllers currently plugged in, by name.
    pub fn names(&self) -> Vec<String> {
        let Some(gilrs) = self.gilrs.as_ref() else { return Vec::new() };
        gilrs.gamepads().map(|(_, pad)| pad.name().to_string()).collect()
    }

    /// A line for the settings menu: what is plugged in, or why nothing is.
    pub fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        match self.names() {
            names if names.is_empty() => String::new(),
            names => format!("pad: {}", names.join(", ")),
        }
    }
}

/// Where an axis has got to, given where it was.
fn step_axis(was: AxisState, value: f32) -> AxisState {
    let armed = was.armed || value.abs() <= OFF;
    if !armed {
        return AxisState { pushed: 0, armed };
    }
    let holds = match was.pushed {
        1 => value > OFF,
        -1 => value < -OFF,
        _ => false,
    };
    let pushed = if holds {
        was.pushed
    } else if value >= ON {
        1
    } else if value <= -ON {
        -1
    } else {
        0
    };
    AxisState { pushed, armed }
}

/// The directions an axis stands for, negative first.
fn axis_directions(axis: Axis) -> Option<(&'static str, &'static str)> {
    match axis {
        Axis::LeftStickX | Axis::DPadX => Some(("PadLeft", "PadRight")),
        Axis::LeftStickY | Axis::DPadY => Some(("PadDown", "PadUp")),
        _ => None,
    }
}

/// The name a controller button is known by in the config.
pub fn button_name(button: Button, code: u32) -> String {
    let name = match button {
        Button::South => "PadSouth",
        Button::East => "PadEast",
        Button::North => "PadNorth",
        Button::West => "PadWest",
        Button::C => "PadC",
        Button::Z => "PadZ",
        Button::LeftTrigger => "PadL1",
        Button::LeftTrigger2 => "PadL2",
        Button::RightTrigger => "PadR1",
        Button::RightTrigger2 => "PadR2",
        Button::Select => "PadSelect",
        Button::Start => "PadStart",
        Button::Mode => "PadMode",
        Button::LeftThumb => "PadL3",
        Button::RightThumb => "PadR3",
        Button::DPadUp => "PadUp",
        Button::DPadDown => "PadDown",
        Button::DPadLeft => "PadLeft",
        Button::DPadRight => "PadRight",
        Button::Unknown => return format!("PadBtn{code:#X}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_buttons_are_named_by_where_they_sit() {
        assert_eq!(button_name(Button::South, 0), "PadSouth");
        assert_eq!(button_name(Button::DPadUp, 0), "PadUp");
        assert_eq!(button_name(Button::RightTrigger, 0), "PadR1");
    }

    #[test]
    fn an_unrecognised_button_falls_back_to_its_code() {
        assert_eq!(button_name(Button::Unknown, 0x130), "PadBtn0x130");
    }

    /// An axis that has been centred at least once, i.e. an ordinary stick.
    fn armed(pushed: i8) -> AxisState {
        AxisState { pushed, armed: true }
    }

    #[test]
    fn a_stick_has_to_be_pushed_past_the_threshold_to_count() {
        assert_eq!(step_axis(armed(0), 0.4).pushed, 0);
        assert_eq!(step_axis(armed(0), 0.5).pushed, 1);
        assert_eq!(step_axis(armed(0), -0.9).pushed, -1);
    }

    #[test]
    fn a_stick_resting_near_the_edge_does_not_chatter() {
        assert_eq!(step_axis(armed(1), 0.45).pushed, 1);
        assert_eq!(step_axis(armed(1), 0.31).pushed, 1);
        assert_eq!(step_axis(armed(1), 0.29).pushed, 0);
        assert_eq!(step_axis(armed(-1), -0.35).pushed, -1);
    }

    #[test]
    fn a_recognised_controller_steers_from_the_first_movement() {
        let start = AxisState { pushed: 0, armed: true };
        assert_eq!(step_axis(start, -1.0).pushed, -1);
    }

    #[test]
    fn an_axis_that_rests_at_one_end_holds_nothing_until_it_is_centred() {
        let resting = step_axis(AxisState::default(), -1.0);
        assert_eq!(resting, AxisState { pushed: 0, armed: false });
        assert_eq!(step_axis(resting, -0.8).pushed, 0);
        let centred = step_axis(resting, 0.0);
        assert!(centred.armed);
        assert_eq!(step_axis(centred, -1.0).pushed, -1);
    }

    #[test]
    fn only_the_left_stick_and_the_hat_steer() {
        assert_eq!(axis_directions(Axis::LeftStickY), Some(("PadDown", "PadUp")));
        assert_eq!(axis_directions(Axis::DPadX), Some(("PadLeft", "PadRight")));
        assert_eq!(axis_directions(Axis::RightStickX), None);
    }
}
