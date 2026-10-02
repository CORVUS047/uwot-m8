# uwot-m8

**USB Window, Output & Terminal for M8.**

Mirrors a [Dirtywave M8](https://dirtywave.com/) tracker's screen and forwards
keyboard input back to it over the M8's USB serial connection. Two frontends are available:

- **`uwot-m8`** — a window, pixel-for-pixel with the device.
- **`uwot-tui`** — the same screen as text in your terminal.

Built for the M8 Model:02 (480x320 display); Model:01 and headless builds work
in theory, but I do not have those and have not tested it.

## Platforms

| | `uwot-m8` | `uwot-tui` |
|---|---|---|
| Linux | yes | yes |
| macOS | yes | yes |
| Windows | untested | no |

## Installing

Tagged releases carry prebuilt binaries: Linux and macOS on x86_64 and arm64,
Windows on x86_64. See [Releases](../../releases), and the `INSTALL.txt` inside
the archive. To build it yourself instead:

### Linux

Needs Rust, `libudev` (serial enumeration), ALSA (`libasound`, for audio and
MIDI) and SDL2 for the window. On Debian or Ubuntu:

```sh
sudo apt install build-essential pkg-config libudev-dev libasound2-dev libsdl2-dev
packaging/install.sh          # into ~/.local, or set PREFIX=
```

That installs both frontends to `~/.local/bin`, and sets up a .desktop file
for you.

### macOS

Needs Rust and SDL2:

```sh
brew install sdl2
packaging/install.sh          # both binaries to ~/.local/bin
```

The first time you turn audio on, macOS asks for microphone access: the M8
arrives as an input device, so that is the permission its audio needs.

### Windows

Needs Rust, plus [cmake](https://cmake.org/download/) to build SDL2 into the
executable rather than beside it:

```bat
packaging\install.bat
```

That builds `uwot-m8`, copies it to `%LOCALAPPDATA%\Programs\uwot-m8` and makes
a Start Menu shortcut. Without cmake it links against an SDL2 you provide, and
`SDL2.dll` must sit next to the executable or on your `PATH`.

### From source

```sh
cargo build --release                                       # both frontends
cargo build --release --no-default-features --bin uwot-tui   # terminal only, no SDL2
cargo build --release --features bundled-sdl                 # build SDL2 in (needs cmake)
```

Binaries land in `target/release/`.

## Device permissions

Linux only: you dont need to worry about this if you use the install script. 
The M8's serial port is `root:dialout`, so a plain user cannot open it. 
Install the bundled udev rule, which hands access to whoever is logged in on
the local seat:

```sh
sudo install -m644 packaging/70-m8.rules /etc/udev/rules.d/70-m8.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=tty
```

Replug the M8 if it was already connected. Alternatively add yourself to the
`dialout` group (`sudo usermod -aG dialout $USER`) and log out and back in.

## Running the window

```sh
uwot-m8                  # autodetect the M8
uwot-m8 --list           # print detected M8 serial devices
uwot-m8 --device /dev/ttyACM1
uwot-m8 --scale 4        # initial window scale (default 3)
uwot-m8 --fullscreen
```

Resizable, with nearest-neighbour integer scaling, so pixels stay square and
sharp. Unplugging the M8 leaves the window open; it reconnects automatically.

## Running in the terminal

```sh
uwot-tui                      # autodetect the M8
uwot-tui --device /dev/ttyACM1
uwot-tui --colors 256         # force 256-colour output (default: detected)
```

The screen appears as a centred 40x24 character grid (Model:02, large font), so
the terminal must be at least that big. Colour depth comes from
`COLORTERM`/`TERM`, falling back to the 256-colour palette.

## Audio

The M8 is a USB audio interface as well as a serial device: its headphone mix
arrives as a capture device, and it takes a stream back on its USB audio input.
The **Audio** settings page runs a passthrough each way, and the two are
independent — sending something into the M8 does not cost you hearing it.

| Row | Meaning |
|---|---|
| `Play` | the M8 heard here: on or off. Off by default, since many setups already route the M8 to an interface or mixer, and hearing it twice is worse than not hearing it at all imo |
| `Play from` | which capture device to take it from. `the M8` finds the M8 by name, which is what you want unless you have two of them |
| `Play to` | where to play it. `default` follows this computer's default output |
| `Send` | this computer heard on the M8: on or off |
| `Send from` | what to send. `default` is this computer's default input |
| `Send into` | which device to send it to. `the M8` finds the M8 by name |

`Send from` is how either direction gets in: a microphone or an interface input
is a capture device, and so is the **monitor** of an output — `Monitor of ...`
carries whatever this computer is playing, so picking one sends another
application into the M8. What the M8 then does with it is up to the M8's own
input settings.

Every device row lists what the system offers and stores the name, so a
passthrough comes back on the same device after a replug. A name that is no
longer there stays among the choices and is named in the status line until it
turns up again. The M8's own devices are left out of `Send from`, since sending
the M8 into itself is a feedback loop — and mind that `Monitor of X` while
`Play to` is also `X` makes that same loop the long way round.

## MIDI control changes

The **MIDI** settings page holds eight control-change bindings. Each sends to the
port chosen at the top of that page — the M8's own port by default, since it is a
class-compliant USB MIDI device — and fires when its bound key or controller
button is pressed.

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

`from` rests and `to` is where a press heads, which lets the trigger modes work
the same for a single value as for a ramp:

- **`oneshot`** — press sends it, or starts the ramp and lets it finish. Release
  does nothing; a second press starts over.
- **`gate`** — press goes out, release comes back. A ramp released part way turns
  round from where it got to, taking only as long as the distance left needs, so
  out and back travel at the same rate.
- **`toggle`** — press goes out, next press comes back.

A `linear` ramp takes equal steps per unit time; a `log` one is quick at the
start and slows towards the end, which reads as even for a filter or volume
sweep. Both land exactly on `from` and `to`.

Ramps run off the frontend's own loop, and only changed values are sent, so a
slow ramp does not flood the port with repeats.

## Controllers

A game controller works with no setting-up. Its buttons are bound by name like
any key, so the d-pad and four face buttons press M8 buttons out of the box:

| Controller | M8 |
|---|---|
| D-pad, left stick | Directions |
| North (`Y` / triangle) | OPTION |
| East (`B` / circle) | EDIT |
| West (`X` / square) | SELECT (shift) |
| South (`A` / cross) | START (play) |
| Right shoulder | OPTION+EDIT (delete) |

## Settings

Escape opens the settings in either frontend. Up/down move, left/right change a
value (hold Shift for larger steps), Enter opens a page or starts binding a key,
Escape goes back — out of the page, then out of the menu.

```
Settings
  Controls   shown        the face buttons below the screen
  Theme      M8           uwot-tui only
  Colours    auto         uwot-tui only
  Audio      off          a page: the M8 heard here, and this computer sent to it
  Buttons    >            a page: which key presses which M8 button
  MIDI       no port      a page: the MIDI port, and the eight CC bindings
```

Changes save as they are made, to `~/.config/uwot-m8/config.conf`
(`%APPDATA%\uwot-m8\config.conf` on Windows), shared by both frontends:

```
show_controls = true    # draw the face buttons below the screen
theme = m8              # m8 (the device's own colours) or terminal
colors = auto           # auto, truecolor or 256
audio = false           # play the M8's own output through this computer
audio_input = none      # the device to take it from; none finds the M8 by name
audio_output = none     # where to play it; none is this computer's default
audio_send = false      # play a device here into the M8's USB audio input
audio_send_input = none # what to send; none is this computer's default input
audio_send_output = none    # which M8 device; none finds the M8 by name

key_up = Up,PadUp       # which keys press which M8 buttons
key_select = z,LeftShift,PadWest
key_start = x,Space,PadSouth
...

midi_port = none        # by name; a partial name matches

cc1 = key=F1, channel=1, controller=74, shape=ramp, trigger=gate, from=0, to=127, time=2000, curve=log
cc2 = key=none, channel=1, controller=1, shape=static, trigger=oneshot, from=0, to=127, time=1000, curve=linear
...
```

### Rebinding keys

The **Buttons** page lists every M8 button, and each CC slot has its own **Key**
row. Select one, press Enter, then press the key or controller button you want it
on; Escape cancels. Binding takes that key off whatever else had it, so one key
never both presses a button and sends a control change. "Reset keys" puts the
buttons back.

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

The terminal frontend binds the same face buttons, with differences forced by how
terminals report keys:

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

Tab toggles keyjazz, turning the keyboard into a midi controller sending notes
to the m8.

| Key                                   | Note                     |
|---------------------------------------|--------------------------|
| `Z S X D C V G B H N J M`             | Chromatic from C         |
| `Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`   | The octave above         |
| Keypad `/` and `*`                    | Octave down / up         |
| Keypad `-` and `+`                    | Velocity down / up       |

Holding Alt makes velocity steps fine (1 instead of 16). The letter keys play
notes instead of acting as face buttons while keyjazz is on, so drive the UI with
the Shift/Space/Alt/Ctrl bindings or a controller.

### Held buttons in a terminal

The M8 wants the complete set of buttons currently held, so a client must see a
key being *released*, and terminals normally report only presses. `uwot-tui` asks
for release events with the kitty keyboard protocol — terminals without it simply
ignore the request — and notices whether any release actually arrives:

- **Releases reported** (kitty, foot, ghostty, WezTerm, recent Alacritty):
  buttons are tracked exactly, and holding a chord works as on the device.
- **Presses only** (xterm, VTE terminals, tmux, a bare pty): each keypress becomes
  a brief press instead, and the status line says so. Holding a chord is
  impossible, which is why Shift/Alt/Ctrl are offered as a way to press
  combinations in one keystroke; a gated control change is a brief out-and-back
  for the same reason. A controller always reports releases, so it has no such
  trouble.

## Debugging

`uwot-probe` dumps what the device is actually sending, which is how the text
grid `uwot-tui` draws was worked out:

```sh
uwot-probe 4              # sample 4s and report coordinate and size statistics
uwot-probe --text 2       # print the recovered text grid as ASCII
uwot-probe --rects 2      # list rectangles with the cells they land on
uwot-probe --trace 70     # print rectangles in arrival order, which is how a
                        # composite element like the cursor becomes visible
uwot-probe --boxes 2      # report what the cursor heuristics made of each outline
uwot-probe --ink 2        # report the sub-cell ink in each cell and its character
uwot-probe --audio 4      # play the M8's audio for 4s, saying where from and to
uwot-probe --audio-devices         # list the devices each row can be set to
uwot-probe --audio-in "M8 Analog" --audio-out Scarlett 4   # play between two of them
uwot-probe --audio-send 4           # send this computer's default input to the M8
uwot-probe --audio --send-from "Monitor of Scarlett" 4      # both ways at once
uwot-probe --midi         # list MIDI output ports and which one is the default
uwot-probe --cc "Midi Through" 2   # send a 2s ramp to a port, printing every value
uwot-probe --pads 10      # name every controller button and direction for 10s
```

## Licence

MIT, see [LICENSE](LICENSE). [NOTICE](NOTICE) carries the terms for the
bitmap fonts, which are compiled into both binaries.

## Credits

Protocol details and bitmap fonts come from
[m8c](https://github.com/laamaa/m8c) by Jonne Kokkonen (MIT).
