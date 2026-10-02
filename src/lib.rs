//! Shared M8 client: serial transport, protocol parsing, and the two ways of
//! reproducing the device's screen ([`screen`] as pixels, [`text`] as text).

/// Capturing one application, which only a sound server can do.
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
pub mod apps;
pub mod audio;
pub mod cc;
pub mod config;
pub mod font;
pub mod keys;
pub mod m8;
pub mod menu;
pub mod midi;
pub mod outputs;
pub mod pad;
pub mod proto;
pub mod screen;
pub mod slip;
#[cfg(unix)]
pub mod term;
pub mod text;
#[cfg(unix)]
pub mod tui;
pub mod ui;
