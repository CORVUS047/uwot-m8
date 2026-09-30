//! The terminal frontend's binary.

#[cfg(unix)]
fn main() {
    uwot_m8::tui::main();
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "uwot-tui needs a Unix terminal, which this platform does not have. \
         Use uwot-m8 instead."
    );
    std::process::exit(2);
}
