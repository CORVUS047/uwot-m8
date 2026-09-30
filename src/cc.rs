//! MIDI control-change bindings: what a key sends, and the ramps it starts.

use std::time::{Duration, Instant};

/// How many bindings there are.
pub const SLOTS: usize = 8;

pub const MAX_VALUE: u8 = 127;
pub const MAX_CONTROLLER: u8 = 127;
pub const MIN_CHANNEL: u8 = 1;
pub const MAX_CHANNEL: u8 = 16;
/// Longest ramp offered in the settings menu.
pub const MAX_TIME_MS: u32 = 60_000;
/// What one press of left or right changes a ramp time by.
pub const TIME_STEP_MS: u32 = 50;

/// Whether a binding sends one value or walks between two.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// One value, sent the moment the key goes down.
    #[default]
    Static,
    /// A walk from `from` to `to` over `time_ms`.
    Ramp,
}

impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Shape::Static => "static",
            Shape::Ramp => "ramp",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Shape::Static => Shape::Ramp,
            Shape::Ramp => Shape::Static,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "static" => Some(Shape::Static),
            "ramp" => Some(Shape::Ramp),
            _ => None,
        }
    }
}

/// The shape of a ramp's travel over time.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Curve {
    /// Equal steps per unit time.
    #[default]
    Linear,
    /// Quick at the start and slowing towards the end.
    Log,
}

impl Curve {
    pub fn label(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Log => "log",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Curve::Linear => Curve::Log,
            Curve::Log => Curve::Linear,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "linear" | "lin" => Some(Curve::Linear),
            "log" | "logarithmic" => Some(Curve::Log),
            _ => None,
        }
    }

    /// Maps progress through a ramp onto how far along its travel it is.
    fn ease(self, progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Curve::Linear => progress,
            Curve::Log => (1.0 + 9.0 * progress).log10(),
        }
    }
}

/// What a key press does, and what a release does about it.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Trigger {
    /// Press sends the value, or starts the ramp and lets it finish.
    #[default]
    Oneshot,
    /// Press goes out, release comes back.
    Gate,
    /// Press goes out, the next press comes back.
    Toggle,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Oneshot => "oneshot",
            Trigger::Gate => "gate",
            Trigger::Toggle => "toggle",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Trigger::Oneshot => Trigger::Gate,
            Trigger::Gate => Trigger::Toggle,
            Trigger::Toggle => Trigger::Oneshot,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "oneshot" | "once" => Some(Trigger::Oneshot),
            "gate" | "hold" => Some(Trigger::Gate),
            "toggle" | "latch" => Some(Trigger::Toggle),
            _ => None,
        }
    }
}

/// One control change a key can send.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    /// The key that fires it, named as in [`crate::config::Bindings`].
    pub key: Option<String>,
    pub channel: u8,
    pub controller: u8,
    pub shape: Shape,
    pub trigger: Trigger,
    pub from: u8,
    pub to: u8,
    pub time_ms: u32,
    pub curve: Curve,
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            key: None,
            channel: MIN_CHANNEL,
            controller: 1,
            shape: Shape::Static,
            trigger: Trigger::Oneshot,
            from: 0,
            to: MAX_VALUE,
            time_ms: 1000,
            curve: Curve::Linear,
        }
    }
}

impl Binding {
    pub fn is_bound(&self) -> bool {
        self.key.is_some()
    }

    /// A one-line description for the menu row that opens this binding.
    pub fn summary(&self) -> String {
        let Some(key) = &self.key else { return "unbound".into() };
        match self.shape {
            Shape::Static => {
                format!("{key} ch{} cc{} = {}", self.channel, self.controller, self.to)
            }
            Shape::Ramp => format!(
                "{key} ch{} cc{} {}>{} {}",
                self.channel,
                self.controller,
                self.from,
                self.to,
                format_time(self.time_ms),
            ),
        }
    }

    /// How long a full journey takes. Static bindings arrive at once.
    fn duration(&self) -> Duration {
        match self.shape {
            Shape::Static => Duration::ZERO,
            Shape::Ramp => Duration::from_millis(self.time_ms as u64),
        }
    }
}

/// A ramp time as the menu and the summary show it.
pub fn format_time(ms: u32) -> String {
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// One control change, ready to send.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Message {
    pub channel: u8,
    pub controller: u8,
    pub value: u8,
}

impl Message {
    /// The three MIDI bytes.
    pub fn bytes(self) -> [u8; 3] {
        let channel = self.channel.clamp(MIN_CHANNEL, MAX_CHANNEL) - 1;
        [0xB0 | channel, self.controller & 0x7F, self.value & 0x7F]
    }
}

#[derive(Copy, Clone, Debug)]
struct Ramp {
    channel: u8,
    controller: u8,
    from: u8,
    to: u8,
    started: Instant,
    duration: Duration,
    curve: Curve,
}

impl Ramp {
    /// Where the ramp is at `now`, and whether it has arrived.
    fn value_at(&self, now: Instant) -> (u8, bool) {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= self.duration {
            return (self.to, true);
        }
        let progress = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let travelled = self.curve.ease(progress);
        let span = self.to as f32 - self.from as f32;
        let value = self.from as f32 + span * travelled;
        (value.round().clamp(0.0, MAX_VALUE as f32) as u8, false)
    }
}

#[derive(Clone, Debug, Default)]
struct Slot {
    /// The value last sent, so a ramp can turn round from where it got to.
    sent: Option<u8>,
    ramp: Option<Ramp>,
    /// Set while a toggle binding is in its pressed position.
    latched: bool,
}

/// Holds the bindings that are currently doing something.
#[derive(Debug)]
pub struct Engine {
    slots: Vec<Slot>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self { slots: vec![Slot::default(); SLOTS] }
    }

    /// Whether anything is mid-ramp, i.e. whether [`Engine::tick`] has work.
    pub fn is_active(&self) -> bool {
        self.slots.iter().any(|slot| slot.ramp.is_some())
    }

    /// Handles the key going down.
    pub fn press(&mut self, slot: usize, binding: &Binding, now: Instant) -> Option<Message> {
        match binding.trigger {
            Trigger::Oneshot | Trigger::Gate => self.travel_out(slot, binding, now),
            Trigger::Toggle => {
                if self.latched(slot) {
                    self.set_latched(slot, false);
                    self.travel_back(slot, binding, now)
                } else {
                    self.set_latched(slot, true);
                    self.travel_out(slot, binding, now)
                }
            }
        }
    }

    /// Handles the key coming up. Only a gate does anything with it.
    pub fn release(&mut self, slot: usize, binding: &Binding, now: Instant) -> Option<Message> {
        match binding.trigger {
            Trigger::Gate => self.travel_back(slot, binding, now),
            Trigger::Oneshot | Trigger::Toggle => None,
        }
    }

    /// Advances every running ramp, returning whatever changed.
    pub fn tick(&mut self, now: Instant) -> Vec<Message> {
        let mut messages = Vec::new();
        for slot in 0..self.slots.len() {
            let Some(ramp) = self.slots[slot].ramp else { continue };
            let (value, arrived) = ramp.value_at(now);
            if arrived {
                self.slots[slot].ramp = None;
            }
            if let Some(message) = self.emit(slot, ramp.channel, ramp.controller, value) {
                messages.push(message);
            }
        }
        messages
    }

    /// Forgets every ramp and every value sent.
    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = Slot::default();
        }
    }

    /// Heads for the binding's `to`.
    fn travel_out(&mut self, slot: usize, binding: &Binding, now: Instant) -> Option<Message> {
        self.start(slot, binding, binding.from, binding.to, binding.duration(), now)
    }

    /// Heads back to the binding's `from`, from wherever the slot actually is.
    fn travel_back(&mut self, slot: usize, binding: &Binding, now: Instant) -> Option<Message> {
        let current = self.slots[slot].sent.unwrap_or(binding.to);
        let span = (binding.to as i32 - binding.from as i32).unsigned_abs();
        let remaining = (current as i32 - binding.from as i32).unsigned_abs();
        let duration = match (binding.duration(), span) {
            (duration, 0) => duration,
            (duration, span) => duration.mul_f32(remaining as f32 / span as f32),
        };
        self.start(slot, binding, current, binding.from, duration, now)
    }

    fn start(
        &mut self,
        slot: usize,
        binding: &Binding,
        from: u8,
        to: u8,
        duration: Duration,
        now: Instant,
    ) -> Option<Message> {
        if slot >= self.slots.len() {
            return None;
        }
        if duration.is_zero() || from == to {
            self.slots[slot].ramp = None;
            return self.emit(slot, binding.channel, binding.controller, to);
        }
        self.slots[slot].ramp = Some(Ramp {
            channel: binding.channel,
            controller: binding.controller,
            from,
            to,
            started: now,
            duration,
            curve: binding.curve,
        });
        self.emit(slot, binding.channel, binding.controller, from)
    }

    fn emit(&mut self, slot: usize, channel: u8, controller: u8, value: u8) -> Option<Message> {
        if self.slots[slot].sent == Some(value) {
            return None;
        }
        self.slots[slot].sent = Some(value);
        Some(Message { channel, controller, value })
    }

    fn latched(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|slot| slot.latched)
    }

    fn set_latched(&mut self, slot: usize, latched: bool) {
        if let Some(slot) = self.slots.get_mut(slot) {
            slot.latched = latched;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(time_ms: u32, curve: Curve, trigger: Trigger) -> Binding {
        Binding {
            key: Some("F1".into()),
            channel: 2,
            controller: 74,
            shape: Shape::Ramp,
            trigger,
            from: 0,
            to: 100,
            time_ms,
            curve,
        }
    }

    #[test]
    fn a_status_byte_carries_the_channel_counted_from_one() {
        let message = Message { channel: 1, controller: 74, value: 64 };
        assert_eq!(message.bytes(), [0xB0, 74, 64]);
        let message = Message { channel: 16, controller: 1, value: 127 };
        assert_eq!(message.bytes(), [0xBF, 1, 127]);
    }

    #[test]
    fn a_static_binding_sends_its_target_on_press_and_nothing_on_release() {
        let binding = Binding {
            key: Some("F1".into()),
            controller: 11,
            to: 64,
            ..Binding::default()
        };
        let mut engine = Engine::new();
        let now = Instant::now();
        assert_eq!(
            engine.press(0, &binding, now),
            Some(Message { channel: 1, controller: 11, value: 64 })
        );
        assert_eq!(engine.release(0, &binding, now), None);
        assert!(engine.tick(now).is_empty());
    }

    #[test]
    fn a_static_gate_returns_to_its_resting_value_on_release() {
        let binding = Binding {
            key: Some("F1".into()),
            trigger: Trigger::Gate,
            from: 20,
            to: 90,
            ..Binding::default()
        };
        let mut engine = Engine::new();
        let now = Instant::now();
        assert_eq!(engine.press(0, &binding, now).map(|m| m.value), Some(90));
        assert_eq!(engine.release(0, &binding, now).map(|m| m.value), Some(20));
    }

    #[test]
    fn a_toggle_goes_out_on_one_press_and_back_on_the_next() {
        let binding = Binding {
            key: Some("F1".into()),
            trigger: Trigger::Toggle,
            from: 0,
            to: 127,
            ..Binding::default()
        };
        let mut engine = Engine::new();
        let now = Instant::now();
        assert_eq!(engine.press(0, &binding, now).map(|m| m.value), Some(127));
        assert_eq!(engine.release(0, &binding, now), None);
        assert_eq!(engine.press(0, &binding, now).map(|m| m.value), Some(0));
        assert_eq!(engine.press(0, &binding, now).map(|m| m.value), Some(127));
    }

    #[test]
    fn a_ramp_starts_where_it_says_and_arrives_when_its_time_is_up() {
        let binding = ramp(1000, Curve::Linear, Trigger::Oneshot);
        let mut engine = Engine::new();
        let start = Instant::now();

        let first = engine.press(0, &binding, start).expect("the starting value");
        assert_eq!((first.channel, first.controller, first.value), (2, 74, 0));
        assert!(engine.is_active());

        let half = engine.tick(start + Duration::from_millis(500));
        assert_eq!(half.last().map(|m| m.value), Some(50));

        let done = engine.tick(start + Duration::from_millis(1000));
        assert_eq!(done.last().map(|m| m.value), Some(100));
        assert!(!engine.is_active());
        assert!(engine.tick(start + Duration::from_secs(2)).is_empty());
    }

    #[test]
    fn a_log_ramp_is_ahead_of_a_linear_one_in_the_middle() {
        let start = Instant::now();
        let midpoint = |curve| {
            let mut engine = Engine::new();
            let binding = ramp(1000, curve, Trigger::Oneshot);
            engine.press(0, &binding, start);
            engine
                .tick(start + Duration::from_millis(500))
                .last()
                .map(|m| m.value)
                .expect("a value part way along")
        };
        let linear = midpoint(Curve::Linear);
        let log = midpoint(Curve::Log);
        assert_eq!(linear, 50);
        assert!(log > linear, "log at the midpoint was {log}, linear was {linear}");
        assert_eq!(Curve::Log.ease(0.0), 0.0);
        assert_eq!(Curve::Log.ease(1.0), 1.0);
    }

    #[test]
    fn a_gate_released_part_way_turns_round_from_where_it_got_to() {
        let binding = ramp(1000, Curve::Linear, Trigger::Gate);
        let mut engine = Engine::new();
        let start = Instant::now();
        engine.press(0, &binding, start);
        let quarter = start + Duration::from_millis(250);
        assert_eq!(engine.tick(quarter).last().map(|m| m.value), Some(25));

        engine.release(0, &binding, quarter);
        assert_eq!(engine.tick(quarter + Duration::from_millis(125)).last().map(|m| m.value), Some(13));
        assert_eq!(engine.tick(quarter + Duration::from_millis(250)).last().map(|m| m.value), Some(0));
        assert!(!engine.is_active());
    }

    #[test]
    fn a_retriggered_ramp_sets_off_from_the_start_again() {
        let binding = ramp(1000, Curve::Linear, Trigger::Oneshot);
        let mut engine = Engine::new();
        let start = Instant::now();
        engine.press(0, &binding, start);
        let half = start + Duration::from_millis(500);
        engine.tick(half);
        assert_eq!(engine.press(0, &binding, half).map(|m| m.value), Some(0));
        assert_eq!(engine.tick(half + Duration::from_millis(500)).last().map(|m| m.value), Some(50));
        assert_eq!(engine.tick(half + Duration::from_millis(1000)).last().map(|m| m.value), Some(100));
    }

    #[test]
    fn a_time_reads_as_seconds_when_it_divides_evenly() {
        assert_eq!(format_time(2000), "2s");
        assert_eq!(format_time(1500), "1500ms");
    }
}
