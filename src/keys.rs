//! Input vocabulary shared by every frontend: the M8's button bitmask, the
//! face-button layout, and keyjazz.

/// Bit positions the M8 expects in a `C` (controller) message.
pub mod bits {
    pub const EDIT: u8 = 1 << 0;
    pub const OPT: u8 = 1 << 1;
    pub const RIGHT: u8 = 1 << 2;
    pub const START: u8 = 1 << 3;
    pub const SELECT: u8 = 1 << 4;
    pub const DOWN: u8 = 1 << 5;
    pub const UP: u8 = 1 << 6;
    pub const LEFT: u8 = 1 << 7;
    /// The M8 has no delete button; it is OPT and EDIT pressed together.
    pub const DELETE: u8 = OPT | EDIT;
}

/// One of the M8's face buttons, and where it sits in the drawn keypad.
pub struct FaceButton {
    /// The bit it sets in a controller message.
    pub mask: u8,
    pub col: usize,
    pub row: usize,
    /// How far off its slot it sits, in nudge steps, right and down positive.
    /// Each frontend picks the step; every button here takes a whole one.
    pub nudge: (i32, i32),
}

/// The M8's face buttons, laid out the way they sit on the device: the d-pad's
/// cross on the left with OPTION and EDIT alongside its top, SHIFT and PLAY
/// under its foot. The pair top right sits a step up and right of the grid, the
/// pair at the bottom a step below it.
///
/// ```text
/// .      UP     OPTION  EDIT
/// LEFT   DOWN   RIGHT
/// SHIFT  PLAY
/// ```
pub const FACE_BUTTONS: [FaceButton; 8] = [
    face(bits::UP, 1, 0, (0, 0)),
    face(bits::OPT, 2, 0, (1, -1)),
    face(bits::EDIT, 3, 0, (1, -1)),
    face(bits::LEFT, 0, 1, (0, 0)),
    face(bits::DOWN, 1, 1, (0, 0)),
    face(bits::RIGHT, 2, 1, (0, 0)),
    face(bits::SELECT, 0, 2, (0, 1)),
    face(bits::START, 1, 2, (0, 1)),
];

const fn face(mask: u8, col: usize, row: usize, nudge: (i32, i32)) -> FaceButton {
    FaceButton {
        mask,
        col,
        row,
        nudge,
    }
}

/// Columns and rows of slots the face buttons occupy.
pub const FACE_SLOT_COLS: usize = 4;
pub const FACE_SLOT_ROWS: usize = 3;

pub const MIN_OCTAVE: u8 = 0;
pub const MAX_OCTAVE: u8 = 8;
pub const MAX_VELOCITY: u8 = 0x7F;
pub const FINE_VELOCITY_STEP: u8 = 1;
pub const COARSE_VELOCITY_STEP: u8 = 0x10;

/// Semitone offset of a key in the piano-style layout on the letter rows.
pub fn note_offset(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        'z' => 0,
        's' => 1,
        'x' => 2,
        'd' => 3,
        'c' => 4,
        'v' => 5,
        'g' => 6,
        'b' => 7,
        'h' => 8,
        'n' => 9,
        'j' => 10,
        'm' => 11,
        'q' => 12,
        '2' => 13,
        'w' => 14,
        '3' => 15,
        'e' => 16,
        'r' => 17,
        '5' => 18,
        't' => 19,
        '6' => 20,
        'y' => 21,
        '7' => 22,
        'u' => 23,
        'i' => 24,
        '9' => 25,
        'o' => 26,
        '0' => 27,
        'p' => 28,
        _ => return None,
    })
}

/// Octave and velocity for keyboard note entry.
#[derive(Copy, Clone, Debug)]
pub struct Keyjazz {
    pub enabled: bool,
    pub octave: u8,
    pub velocity: u8,
}

impl Default for Keyjazz {
    fn default() -> Self {
        Self {
            enabled: false,
            octave: 2,
            velocity: MAX_VELOCITY,
        }
    }
}

impl Keyjazz {
    /// MIDI note for a semitone offset at the current octave.
    pub fn note(&self, offset: u8) -> u8 {
        offset.saturating_add(self.octave.saturating_mul(12))
    }

    pub fn octave_down(&mut self) {
        self.octave = self.octave.saturating_sub(1).max(MIN_OCTAVE);
    }

    pub fn octave_up(&mut self) {
        self.octave = (self.octave + 1).min(MAX_OCTAVE);
    }

    pub fn velocity_down(&mut self, fine: bool) {
        self.velocity = self.velocity.saturating_sub(self.step(fine));
    }

    pub fn velocity_up(&mut self, fine: bool) {
        self.velocity = self
            .velocity
            .saturating_add(self.step(fine))
            .min(MAX_VELOCITY);
    }

    fn step(&self, fine: bool) -> u8 {
        if fine {
            FINE_VELOCITY_STEP
        } else {
            COARSE_VELOCITY_STEP
        }
    }
}
