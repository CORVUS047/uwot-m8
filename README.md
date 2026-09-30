# uwot-m8

**USB Window, Output & Terminal for M8.**

Mirrors a [Dirtywave M8](https://dirtywave.com/) tracker's screen and forwards
keyboard input back to it, over the M8's USB serial connection. Two frontends
share one client:

- **`uwot-m8`** — a window, pixel-for-pixel with the device.
- **`uwot-tui`** — the same screen as text in your terminal, in colour.

Both draw the M8's face buttons below its screen, lit up as you press them, and
both open a settings menu with Escape. Beyond the screen, the app can play the
M8's own audio through your computer, send MIDI control changes from bound keys,
and be driven from a game controller.

Built for the M8 Model:02 (480x320 display); the Model:01 and headless builds
are handled too, since the device reports its own model.

## Platforms

| | `uwot-m8` | `uwot-tui` |
|---|---|---|
| Linux | yes | yes |
| macOS | yes | yes |
| Windows | yes | no |

The terminal frontend needs raw mode, the kitty keyboard protocol and a signal
handler to put the terminal back as it was found, none of which a Windows
console provides. On Windows it builds as a stub that tells you to use the
window instead.

## Installing

### Linux

Needs Rust, `libudev` (serial enumeration), ALSA (`libasound`, for audio and
MIDI) and SDL2 for the window. On Debian or Ubuntu:

```sh
sudo apt install build-essential pkg-config libudev-dev libasound2-dev libsdl2-dev
```

Then:

```sh
packaging/install.sh          # into ~/.local, or set PREFIX=
```

That builds both frontends in release mode, installs them to `~/.local/bin`,
drops the icon into the hicolor theme, and writes `uwot-m8.desktop` with an
absolute `Exec` (a launcher does not necessarily share your `PATH`). The entry's
`StartupWMClass` is `uwot-m8`, which is the class SDL gives the window, so
compositors match the two up.

### macOS

Needs Rust and SDL2:

```sh
brew install sdl2
packaging/install.sh          # installs both binaries to ~/.local/bin
```

The first time you turn the audio on, macOS asks for microphone access: the M8
arrives as an input device, so that is the permission playing its audio needs.

### Windows

Needs Rust, and [cmake](https://cmake.org/download/) if you want SDL2 built
into the executable rather than sitting beside it:

```bat
packaging\install.bat
```

That builds `uwot-m8`, copies it to `%LOCALAPPDATA%\Programs\uwot-m8` and
makes a Start Menu shortcut. Without cmake it links against an SDL2 you provide
yourself, and `SDL2.dll` has to be next to the executable or on your `PATH`.

The M8 appears as a USB serial device and needs no driver on Windows 10 or
later.

### From source

```sh
cargo build --release                                       # both frontends
cargo build --release --no-default-features --bin uwot-tui   # terminal only, no SDL2
cargo build --release --features bundled-sdl                 # build SDL2 in (needs cmake)
```

Binaries land in `target/release/`.

## Device permissions

Only Linux needs anything here: the M8's serial port is `root:dialout` by
default, so a plain user cannot open it. Install the bundled udev rule, which
hands access to whoever is logged in on the local seat:

```sh
sudo install -m644 packaging/70-m8.rules /etc/udev/rules.d/70-m8.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=tty
```

Unplug and replug the M8 if it was already connected. Alternatively, add
yourself to the `dialout` group (`sudo usermod -aG dialout $USER`) and log out
and back in.

## Running the window

```sh
uwot-m8                  # autodetect the M8
uwot-m8 --list           # print detected M8 serial devices
uwot-m8 --device /dev/ttyACM1
uwot-m8 --scale 4        # initial window scale (default 3)
uwot-m8 --fullscreen
```

The window is resizable and scales with nearest-neighbour integer scaling, so
pixels stay square and sharp. If the M8 is unplugged the window stays open and
reconnects automatically when it comes back.

## Running in the terminal

```sh
uwot-tui                      # autodetect the M8
uwot-tui --device /dev/ttyACM1
uwot-tui --colors 256         # force 256-colour output (default: detected)
```

The M8's screen appears as a 40x24 character grid (on a Model:02 in its large
font), centred. You need a terminal at least that big. Colour depth is taken
from `COLORTERM`/`TERM` and falls back to the 256-colour palette.

## Audio

The M8 is a USB audio interface as well as a serial device, so its headphone mix
arrives as an ordinary capture device. Turning **Audio** on in the settings plays
it through your default output.

It is off unless you ask for it: plenty of setups already have the M8 going to an
interface or a mixer, and hearing it twice is worse than not hearing it here.

Which capture device that is depends on the machine, so the hosts are tried in
order — PipeWire, PulseAudio, CoreAudio, WASAPI, then ALSA — and the first that
gives working streams wins. On a desktop the sound server normally holds the M8's
own ALSA device, so going straight underneath it only finds it busy; asking the
server for the same device by name works. If the M8 is not plugged in yet, the
setting stays on and the audio starts when it appears.

The two ends run off different clocks — the M8's crystal and your output
device's — so the buffer would otherwise creep towards full or empty over a long
session. It aims to keep 30 ms buffered, plays silence below that while it fills,
and throws frames away above 120 ms to catch up.

## MIDI control changes

The **MIDI** page in the settings holds eight control-change bindings. Each sends
to the port chosen at the top of that page — the M8's own MIDI port by default,
since it is a class-compliant USB MIDI device — and fires when the key or
controller button bound to it is pressed.

A binding has these fields:

| Field | Meaning |
|---|---|
| `key` | the key or controller button that fires it |
| `channel` | 1 to 16 |
| `controller` | the CC number, 0 to 127 |
| `shape` | `static` (one value) or `ramp` (travel over time) |
| `trigger` | `oneshot`, `gate` or `toggle` |
| `from` | the resting value |
| `to` | the value a press heads for |
| `time` | how long a ramp takes |
| `curve` | `linear` or `log` |

`from` is where it rests and `to` is where a press heads, which is what lets the
three trigger modes work the same way for a single value as for a ramp:

- **`oneshot`** — press sends it, or starts the ramp and lets it finish on its
  own. The release does nothing, and a second press starts over.
- **`gate`** — press goes out, release comes back. A ramp released part way turns
  round from where it had got to, and takes only as long as the distance left
  needs, so out and back travel at the same rate.
- **`toggle`** — press goes out, the next press comes back.

A `linear` ramp takes equal steps per unit time. A `log` one is quick at the
start and slows towards the end, which is how a filter or volume sweep reads to
the ear as even. Both land exactly on `from` and `to`, so a ramp always arrives
at the value you aimed it at.

Ramps are driven from the frontend's own loop, and only values that actually
change are sent, so a slow ramp does not flood the port with repeats.

Function keys `F1` upwards are the obvious things to bind, since nothing on the
M8 wants one. On a controller, the buttons the M8 does not use — the left
shoulder, the triggers, the thumbs — are free for the same job.

## Controllers

A game controller works with no setting-up. Its buttons are bound by name like
any key, so the d-pad and the four face buttons press M8 buttons out of the box:

| Controller | M8 |
|---|---|
| D-pad, left stick | Directions |
| North (`Y` / triangle) | OPTION |
| East (`B` / circle) | EDIT |
| West (`X` / square) | SELECT (shift) |
| South (`A` / cross) | START (play) |
| Right shoulder | OPTION+EDIT (delete) |

The four face buttons sit where the M8's own four do — Option top left, Edit top
right, Shift bottom left, Play bottom right — and are named by position rather
than by what is printed on them, since that differs between makes. A controller
can drive the settings menu too: d-pad to move, South to activate, East to go
back.

Anything the mapping database does not recognise still works: its buttons are
named by their platform code (`PadBtn0x130`), which the `--pads` probe below will
tell you. Axes on such a device only steer once they have been seen near the
middle, so a throttle or a pedal resting at one end of its travel does not hold a
direction down from the moment the app starts.

## Settings

Escape opens the settings in either frontend. Up and down move, left and right
change a value (hold Shift for larger steps), Enter opens a page or starts
binding a key, and Escape goes back — out of the page, then out of the menu.

```
Settings
  Controls   shown        the face buttons below the screen
  Theme      M8           uwot-tui only
  Colours    auto         uwot-tui only
  Audio      off          play the M8's own output through this computer
  Buttons    >            a page: which key presses which M8 button
  MIDI       no port      a page: the MIDI port, and the eight CC bindings
```

The menu is a stack of pages rather than one list, because a terminal is commonly
twenty-four rows tall and the bindings and CC slots between them are far more rows
than that.

Changes are saved as they are made, to `~/.config/uwot-m8/config.conf`
(`%APPDATA%\uwot-m8\config.conf` on Windows), which both frontends share:

```
show_controls = true    # draw the face buttons below the screen
theme = m8              # m8 (the device's own colours) or terminal
colors = auto           # auto, truecolor or 256
audio = false           # play the M8's own output through this computer

key_up = Up,PadUp       # which keys press which M8 buttons
key_select = z,LeftShift,PadWest
key_start = x,Space,PadSouth
...

midi_port = none        # by name; a partial name matches

cc1 = key=F1, channel=1, controller=74, shape=ramp, trigger=gate, from=0, to=127, time=2000, curve=log
cc2 = key=none, channel=1, controller=1, shape=static, trigger=oneshot, from=0, to=127, time=1000, curve=linear
...
```

A broken line costs you that one setting rather than the whole file, and a broken
field costs you that one field.

`theme = terminal` leaves your terminal's own foreground and background alone and
shows the M8's highlights as reverse video instead. It applies to `uwot-tui` only;
the window always uses the device's colours.

### Rebinding keys

The **Buttons** page lists every M8 button, and each CC slot has a **Key** row of
its own. Select one, press Enter, then press the key — or the controller button —
you want it on; Escape cancels. Binding takes that key off whatever else had it,
so one key never both presses a button and sends a control change. "Reset keys"
puts the buttons back.

Keys are stored by name so one config file serves both frontends, which work in
different key types. A name is a single character (`z`), a spelled-out key (`Up`,
`Space`, `Backspace`, `LeftShift`, `F1`) or a controller button (`PadSouth`,
`PadUp`, `PadR1`). Separate several with commas to bind more than one to a
button. Modifier names like `LeftShift` only reach the windowed frontend —
terminals don't report modifiers as keys in their own right, which is why
`uwot-tui` offers them as chords instead.

Rebinding covers the M8's buttons and the CC slots. The app's own keys — Escape
for settings, Tab for keyjazz, Ctrl+Q to quit — are fixed.

## Controls

| Key                 | M8                                |
|---------------------|-----------------------------------|
| Arrow keys          | Directions                        |
| `Z` / Left Shift    | SELECT (shift)                    |
| `X` / Space         | START (play)                      |
| `A` / Left Alt      | OPTION                            |
| `S` / Left Ctrl     | EDIT                              |
| Delete              | OPTION+EDIT (delete)              |
| Tab                 | Toggle keyjazz                    |
| Escape              | Open the settings menu            |
| Alt+Enter           | Toggle fullscreen                 |
| Alt+F4              | Quit                              |

The terminal frontend binds the same face buttons, with a few differences forced
by how terminals report keys:

| Key                   | uwot-tui                                  |
|-----------------------|-----------------------------------------|
| `Z` `X` `A` `S`       | SELECT, START, OPTION, EDIT             |
| Space                 | START                                   |
| Backspace             | OPTION+EDIT (delete)                    |
| Shift/Alt/Ctrl + key  | Adds SELECT / OPTION / EDIT to that key |
| Tab                   | Toggle keyjazz                          |
| Escape                | Open the settings menu                  |
| Ctrl+Q                | Quit                                    |

In keyjazz, `[` and `]` change octave and `-`/`=` change velocity (`_`/`+` for
single steps), since terminals can't be relied on for the keypad.

### Keyjazz

Tab toggles keyjazz, which turns the keyboard into a piano that plays notes on
the M8's current instrument. The title bar shows the octave and velocity while it
is on.

| Key                                   | Note                     |
|---------------------------------------|--------------------------|
| `Z S X D C V G B H N J M`             | Chromatic from C         |
| `Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`   | The octave above         |
| Keypad `/` and `*`                    | Octave down / up         |
| Keypad `-` and `+`                    | Velocity down / up       |

Holding Alt makes velocity steps fine (1 instead of 16). While keyjazz is on, the
letter keys play notes instead of acting as face buttons, so use the
Shift/Space/Alt/Ctrl bindings — or a controller — to drive the UI.

### Held buttons in a terminal

The M8 wants the complete set of buttons currently held, so a client has to be
able to see a key being *released*. Terminals normally report only presses.
`uwot-tui` asks for release events using the kitty keyboard protocol, which
terminals that don't implement it simply ignore, and notices whether any release
actually arrives:

- **Releases reported** (kitty, foot, ghostty, WezTerm, recent Alacritty):
  buttons are tracked exactly, and holding a chord works as on the device.
- **Presses only** (xterm, VTE terminals, tmux, a bare pty): each keypress
  becomes a brief press instead, and the status line says so. Holding a chord is
  impossible, which is why Shift/Alt/Ctrl are offered as a way to press
  combinations in one keystroke. A gated control change is a brief out-and-back
  there for the same reason; a controller has no such trouble, since it always
  reports its releases.

## How it works

The M8 speaks a small binary protocol over USB CDC serial, framed with SLIP
(`0xC0` delimiters). The host enables the stream by sending `E`, and the device
then pushes drawing commands:

| Byte   | Command             | Payload                                            |
|--------|---------------------|----------------------------------------------------|
| `0xFE` | Draw rectangle      | x, y (u16le); optional w, h (u16le); optional RGB  |
| `0xFD` | Draw character      | char, x, y (u16le), foreground RGB, background RGB |
| `0xFC` | Oscilloscope        | RGB, then one height sample per column             |
| `0xFB` | Device key state    | 3 bytes (ignored by this client)                   |
| `0xFF` | System info         | hardware id, firmware major/minor/patch, font mode |

Rectangles omit the size when it is 1x1 and omit the colour when it is unchanged
from the previous rectangle, so the command is 5, 8, 9 or 12 bytes long. A
rectangle covering the entire screen is how the M8 announces a theme background
colour change.

The system-info packet selects the screen size and font: the Model:02 has a
480x320 display and its own font set, the Model:01 has 320x240.

The two frontends consume that same command stream differently.

`src/screen.rs` rasterises it into a pixel framebuffer, drawing glyphs from the
M8's own bitmap fonts (see `assets/README.md`), and uploads that to an SDL2
texture each frame.

`src/text.rs` instead recovers the character grid the firmware laid its UI out
on, so the terminal shows real text rather than a picture of text. The grid pitch
is not something the font metrics give away — a Model:02 in its large font spaces
10x10 glyphs 12px apart across and 14px down — so it is derived from the
coordinates the device actually draws at: positions on an axis are
`phase + n * pitch`, so the pitch is the most common gap between neighbours.
Rectangles and the oscilloscope don't land on the grid at all, and what becomes
of one depends on what it is:

- A solid shape at least half a row tall — a key of the MIDI keyboard, a fader —
  fills the cells it covers with a full block (`█`). A shape narrower than a cell
  goes to the one cell it overlaps most, so a bar lands on a row of its own, and
  a shape that overhangs a cell by a pixel doesn't claim it.
- A solid shape shorter than that is decoration too small to place on a character
  grid, like the battery gauge, and is dropped.
- Small pieces are gathered into the shape they belong to before being judged,
  because the M8 builds one widget out of many rectangles. Each output meter is
  forty-nine `1x6` columns, every column a different shade of a magenta-to-cyan
  gradient, so no single piece of it looks like anything. Pieces join only when
  they touch *and* match on the other axis: the meter's columns all share a
  height, so they join each other but not the panel frame they happen to touch.
  Grouping on touch alone chains a whole panel into one shape and then nothing in
  it can be recognised.
- A gathered shape that fills its own bounds and is either half a cell across
  both ways, or long enough one way to be a bar, becomes blocks. A meter is only
  six pixels tall against a fourteen pixel row, but it runs four cells wide.
- Everything else is dropped: panel frames, keyboard key separators, and the
  meters' peak-hold marker, which is a lone `1x6` column that moves every frame.
  On a character grid a line can only land in whichever cell it happens to cross,
  so a moving one jitters instead of telling you anything.
- A single small square is an indicator dot, and is drawn as one (`•`). It has to
  be a single rectangle: a corner of the cursor is three pieces occupying much
  the same space.
- The oscilloscope is the exception: its samples are single pixels, and the
  quadrant blocks (`▘▄▌▞`) are what turn them into a trace.

The cursor gets its own treatment. The M8 draws it as four corner brackets —
twelve rectangles of 3x1, 1x2 and 1x1 — surrounding a box slightly taller than
one text row, so rendering those pieces directly puts stray marks either side of
the selection and spills onto the row below. Instead the pieces are collected and
grouped by runs of a single colour (the corners are far apart, so grouping them
by position cannot work), and a group small enough to be the cursor becomes a
background highlight on the cells whose centres it encloses, with the text
inverted if it would otherwise be unreadable against it. A larger outline is a
panel frame, and is still drawn as a border.

Host-to-device messages are equally small: `C` plus a one-byte button bitmask,
`K` plus note and velocity for keyjazz, `R` to force a redraw, `X` as a ping, and
`D` to stop the stream.

The rest hangs off the same loop. `src/audio.rs` runs the capture and playback
streams, `src/cc.rs` holds the control-change bindings and steps their ramps,
`src/midi.rs` owns the output port, and `src/pad.rs` turns controller events into
the key names the bindings already understand. Anything that depends on the M8
being plugged in — the serial port, the audio device, the MIDI port — is retried
a second at a time, so unplugging the device and plugging it back in needs
nothing from you.

## Debugging

`uwot-probe` dumps what the device is actually sending, which is how the grid above
was worked out:

```sh
uwot-probe 4              # sample 4s and report coordinate and size statistics
uwot-probe --text 2       # print the recovered text grid as ASCII
uwot-probe --rects 2      # list rectangles with the cells they land on
uwot-probe --trace 70     # print rectangles in arrival order, which is how a
                        # composite element like the cursor becomes visible
uwot-probe --boxes 2      # report what the cursor heuristics made of each outline
uwot-probe --ink 2        # report the sub-cell ink in each cell and its character
uwot-probe --audio 4      # play the M8's audio for 4s, saying where from and to
uwot-probe --midi         # list MIDI output ports and which one is the default
uwot-probe --cc "Midi Through" 2   # send a 2s ramp to a port, printing every value
uwot-probe --pads 10      # name every controller button and direction for 10s
```

## Credits

The protocol details and the bitmap fonts come from
[m8c](https://github.com/laamaa/m8c) by Jonne Kokkonen (MIT).
