//! Debug tool: samples the M8's command stream and reports what it drew.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use uwot_m8::cc::{Binding, Curve, Shape, Trigger};
use uwot_m8::{audio, cc, font, m8, midi, pad, proto};

/// Runs the passthrough, which is the only real test of it. Both legs at once
/// where both were asked for, since that is how they are usually run.
fn play_audio(
    seconds: u64,
    legs: &[(audio::Leg, Option<&str>, Option<&str>)],
) -> Result<(), String> {
    let running: Vec<audio::Audio> = legs
        .iter()
        .map(|(leg, input, output)| {
            // Exclusive when both legs run: the same rule the app follows.
            let exclusive = *leg == audio::Leg::Send && legs.len() > 1;
            let audio = audio::Audio::start(*leg, *input, *output, exclusive)?;
            println!(
                "{} {}",
                match leg {
                    audio::Leg::Play => "playing",
                    audio::Leg::Send => "sending",
                },
                audio.description()
            );
            Ok(audio)
        })
        .collect::<Result<_, String>>()?;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if running.iter().any(audio::Audio::failed) {
            return Err("the audio stream stopped".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("ran for {seconds}s without a fault");
    Ok(())
}

/// Names the devices the audio passthrough can be pointed at.
fn list_audio() -> Result<(), String> {
    for (what, names) in [
        ("in  ", audio::inputs()),
        ("out ", audio::outputs()),
        ("send", audio::sources()),
        ("app ", audio::apps()),
    ] {
        if names.is_empty() {
            println!("{what}  (none)");
        }
        for name in names {
            println!("{what}  {name}");
        }
    }
    Ok(())
}

/// Names every button and direction the controllers send.
fn watch_pads(seconds: u64) -> Result<(), String> {
    let mut pads = pad::Pads::new();
    let status = pads.status();
    println!(
        "{}",
        if status.is_empty() {
            "no controllers".into()
        } else {
            status
        }
    );
    println!("press buttons for {seconds}s");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        for event in pads.poll() {
            println!("{} {}", event.name, if event.down { "down" } else { "up" });
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

fn list_midi() -> Result<(), String> {
    let ports = midi::ports();
    if ports.is_empty() {
        println!("no MIDI outputs");
    }
    for port in &ports {
        println!("{port}");
    }
    println!("default: {:?}", midi::default_port());
    Ok(())
}

/// Runs one ramp binding into a port, reporting every value it sends.
fn send_ramp(port_name: &str, seconds: u64) -> Result<(), String> {
    let mut port = midi::Port::open(port_name)?;
    let binding = Binding {
        key: Some("F1".into()),
        channel: 1,
        controller: 74,
        shape: Shape::Ramp,
        trigger: Trigger::Oneshot,
        from: 0,
        to: 127,
        time_ms: (seconds * 1000) as u32,
        curve: Curve::Log,
    };
    let mut engine = cc::Engine::new();
    let mut sent = 0usize;
    let send = |port: &mut midi::Port, message: Option<cc::Message>| -> Result<(), String> {
        let Some(message) = message else {
            return Ok(());
        };
        port.send(message)?;
        println!(
            "cc {} = {} {:02X?}",
            message.controller,
            message.value,
            message.bytes()
        );
        Ok(())
    };

    let start = Instant::now();
    send(&mut port, engine.press(0, &binding, start))?;
    sent += 1;
    while engine.is_active() {
        for message in engine.tick(Instant::now()) {
            send(&mut port, Some(message))?;
            sent += 1;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    println!("sent {sent} messages in {:?}", start.elapsed());
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut text_mode = false;
    let mut rect_mode = false;
    let mut box_mode = false;
    let mut trace_mode = false;
    let mut ink_mode = false;
    let mut audio_mode = false;
    let mut send_mode = false;
    let mut audio_devices = false;
    let mut audio_input: Option<String> = None;
    let mut audio_output: Option<String> = None;
    let mut send_input: Option<String> = None;
    let mut send_output: Option<String> = None;
    let mut midi_mode = false;
    let mut pad_mode = false;
    let mut cc_port: Option<String> = None;
    let mut seconds = 3u64;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--text" => text_mode = true,
            "--rects" => rect_mode = true,
            "--boxes" => box_mode = true,
            "--trace" => trace_mode = true,
            "--ink" => ink_mode = true,
            "--audio" => audio_mode = true,
            "--audio-devices" => audio_devices = true,
            "--audio-send" => send_mode = true,
            "--send-from" => {
                send_input = Some(args.next().ok_or("--send-from needs a device name")?);
                send_mode = true;
            }
            "--send-into" => {
                send_output = Some(args.next().ok_or("--send-into needs a device name")?);
                send_mode = true;
            }
            "--audio-in" => {
                audio_input = Some(args.next().ok_or("--audio-in needs a device name")?);
                audio_mode = true;
            }
            "--audio-out" => {
                audio_output = Some(args.next().ok_or("--audio-out needs a device name")?);
                audio_mode = true;
            }
            "--midi" => midi_mode = true,
            "--pads" => pad_mode = true,
            "--cc" => cc_port = Some(args.next().ok_or("--cc needs a port name")?),
            other => seconds = other.parse().map_err(|_| format!("bad duration {other}"))?,
        }
    }
    if audio_devices {
        return list_audio();
    }
    if audio_mode || send_mode {
        let mut legs = Vec::new();
        if audio_mode {
            legs.push((
                audio::Leg::Play,
                audio_input.as_deref(),
                audio_output.as_deref(),
            ));
        }
        if send_mode {
            legs.push((
                audio::Leg::Send,
                send_input.as_deref(),
                send_output.as_deref(),
            ));
        }
        return play_audio(seconds, &legs);
    }
    if midi_mode {
        return list_midi();
    }
    if pad_mode {
        return watch_pads(seconds);
    }
    if let Some(port) = cc_port {
        return send_ramp(&port, seconds);
    }
    if text_mode {
        return dump_text(seconds);
    }
    if rect_mode {
        return dump_rects(seconds);
    }
    if box_mode {
        return dump_boxes(seconds);
    }
    if trace_mode {
        return trace_rects(seconds as usize);
    }
    if ink_mode {
        return dump_ink(seconds);
    }

    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;
    println!("probing {path} for {seconds}s");

    let mut chars: Vec<proto::Char> = Vec::new();
    let mut rects: Vec<proto::Rect> = Vec::new();
    let mut waveform_lengths = BTreeMap::new();
    let mut metrics = None;
    let mut last_color = proto::Rgb::default();

    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        for packet in device.poll() {
            match proto::parse(&packet, last_color) {
                Ok(proto::Command::Char(c)) => chars.push(c),
                Ok(proto::Command::Rect(r)) => {
                    last_color = r.color;
                    rects.push(r);
                }
                Ok(proto::Command::Waveform(w)) => {
                    *waveform_lengths.entry(w.samples.len()).or_insert(0usize) += 1;
                }
                Ok(proto::Command::System(info)) => {
                    let index = font::mode_index(info.is_model_02(), info.font_mode as usize);
                    metrics = index.and_then(font::metrics);
                    println!(
                        "system: {} fw {}.{}.{} font_mode {} -> font index {index:?}",
                        info.hardware_name(),
                        info.version.0,
                        info.version.1,
                        info.version.2,
                        info.font_mode,
                    );
                }
                Ok(proto::Command::Joypad) => {}
                Err(e) => println!("parse error: {e}"),
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();

    let metrics = metrics.ok_or("device never sent a system info packet")?;
    println!(
        "\nfont metrics: glyph {}x{} screen_offset_y {} text_offset_y {}",
        metrics.glyph_w, metrics.glyph_h, metrics.screen_offset_y, metrics.text_offset_y,
    );

    println!("\n{} character commands", chars.len());
    report("char x", chars.iter().map(|c| c.x as i32), 12);
    report("char y", chars.iter().map(|c| c.y as i32), 14);

    println!("\n{} rectangle commands", rects.len());
    report("rect x", rects.iter().map(|r| r.x as i32), 12);
    report("rect y", rects.iter().map(|r| r.y as i32), 14);
    let mut sizes: BTreeMap<(u16, u16), usize> = BTreeMap::new();
    for r in &rects {
        *sizes.entry((r.w, r.h)).or_insert(0) += 1;
    }
    let mut sizes: Vec<_> = sizes.into_iter().collect();
    sizes.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    println!("  most common sizes (w x h: count):");
    for ((w, h), count) in sizes.iter().take(12) {
        println!("    {w:>3} x {h:<3}: {count}");
    }

    println!("\nwaveform packet lengths: {waveform_lengths:?}");
    Ok(())
}

/// Prints the distinct values a coordinate takes, and the gaps between them.
fn report(label: &str, values: impl Iterator<Item = i32>, pitch: i32) {
    let values: Vec<i32> = values.collect();
    if values.is_empty() {
        println!("  {label}: none");
        return;
    }
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for v in &values {
        *counts.entry(*v).or_insert(0) += 1;
    }
    let distinct: Vec<i32> = counts.keys().copied().collect();
    let mut gaps: BTreeMap<i32, usize> = BTreeMap::new();
    for pair in distinct.windows(2) {
        *gaps.entry(pair[1] - pair[0]).or_insert(0) += 1;
    }
    println!(
        "  {label}: {} distinct, min {} max {}, nominal pitch {pitch}",
        distinct.len(),
        distinct.first().unwrap(),
        distinct.last().unwrap()
    );
    println!("    gaps between distinct values: {gaps:?}");
    println!("    values: {distinct:?}");
}

/// Prints the text grid as plain ASCII.
pub fn dump_text(seconds: u64) -> Result<(), String> {
    use uwot_m8::text::TextScreen;

    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;

    let mut screen = TextScreen::new()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        for packet in device.poll() {
            if let Ok(command) = proto::parse(&packet, screen.last_color()) {
                screen.apply(&command);
            }
        }
        screen.flush();
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();

    println!("grid {}x{}", screen.cols, screen.rows);
    println!("+{}+", "-".repeat(screen.cols));
    for row in 0..screen.rows {
        let line: String = (0..screen.cols)
            .map(|col| screen.glyph_at(col, row).0)
            .collect();
        let marks: String = (0..screen.cols)
            .map(|col| {
                if screen.cell(col, row).highlight.is_some() {
                    '#'
                } else {
                    ' '
                }
            })
            .collect();
        if marks.trim().is_empty() {
            println!("|{line}|");
        } else {
            println!("|{line}|  highlight: |{marks}|");
        }
    }
    println!("+{}+", "-".repeat(screen.cols));
    Ok(())
}

/// Lists the rectangles the device draws, with the cell grid they land on.
pub fn dump_rects(seconds: u64) -> Result<(), String> {
    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;

    let mut counts: BTreeMap<(u16, u16, u16, u16, u8, u8, u8), usize> = BTreeMap::new();
    let mut last_color = proto::Rgb::default();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        for packet in device.poll() {
            if let Ok(proto::Command::Rect(r)) = proto::parse(&packet, last_color) {
                last_color = r.color;
                *counts
                    .entry((r.x, r.y, r.w, r.h, r.color.r, r.color.g, r.color.b))
                    .or_insert(0) += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();

    const PITCH_X: u16 = 12;
    const PITCH_Y: u16 = 14;
    const PHASE_Y: i32 = 8;

    println!(
        "{:>4} {:>4} {:>4} {:>4}  {:>15}  {:>9}  {:>9} count",
        "x", "y", "w", "h", "rgb", "cols", "rows"
    );
    for ((x, y, w, h, r, g, b), count) in &counts {
        let top = *y as i32 - 2 - PHASE_Y;
        let bottom = top + *h as i32 - 1;
        println!(
            "{x:>4} {y:>4} {w:>4} {h:>4}  {:>15}  {:>9}  {:>9} {count}",
            format!("{r:3},{g:3},{b:3}"),
            format!("{}..{}", x / PITCH_X, (x + w - 1) / PITCH_X),
            format!(
                "{}..{}",
                top.div_euclid(PITCH_Y as i32),
                bottom.div_euclid(PITCH_Y as i32)
            ),
        );
    }
    println!("\n{} distinct rectangles", counts.len());
    Ok(())
}

/// Reports how the outline heuristics classified what the device drew.
pub fn dump_boxes(seconds: u64) -> Result<(), String> {
    use uwot_m8::text::TextScreen;

    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;

    let mut screen = TextScreen::new()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut frame = 0;
    while Instant::now() < deadline {
        for packet in device.poll() {
            if let Ok(command) = proto::parse(&packet, screen.last_color()) {
                screen.apply(&command);
            }
        }
        screen.flush();
        frame += 1;
        for outline in screen.outlines() {
            let (x0, y0, x1, y1) = outline.bounds;
            print!(
                "frame {frame:>4}: {} edge(s)  px {x0},{y0}..{x1},{y1}  ({}x{})  ",
                outline.lines,
                x1 - x0 + 1,
                y1 - y0 + 1
            );
            match outline.cells {
                Some((c0, r0, c1, r1)) => println!("-> highlight cols {c0}..{c1} rows {r0}..{r1}"),
                None => println!("-> drawn as border"),
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();
    Ok(())
}

/// Prints rectangles in the order they arrive.
pub fn trace_rects(limit: usize) -> Result<(), String> {
    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;

    let mut last_color = proto::Rgb::default();
    let mut shown = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while shown < limit && Instant::now() < deadline {
        for packet in device.poll() {
            if let Ok(proto::Command::Rect(r)) = proto::parse(&packet, last_color) {
                last_color = r.color;
                if r.w as u32 * r.h as u32 > 4000 {
                    continue; // skip the full-screen background fill
                }
                println!(
                    "{:>4},{:<4} {:>3}x{:<3}  rgb {:>3},{:>3},{:>3}",
                    r.x, r.y, r.w, r.h, r.color.r, r.color.g, r.color.b
                );
                shown += 1;
                if shown >= limit {
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();
    Ok(())
}

/// Reports the ink in every cell.
pub fn dump_ink(seconds: u64) -> Result<(), String> {
    use uwot_m8::text::TextScreen;

    let path = m8::find_devices().into_iter().next().ok_or("no M8 found")?;
    let mut device = m8::M8::open(&path)?;
    device.enable_display(true)?;

    let mut screen = TextScreen::new()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        for packet in device.poll() {
            if let Ok(command) = proto::parse(&packet, screen.last_color()) {
                screen.apply(&command);
            }
        }
        screen.flush();
        std::thread::sleep(Duration::from_millis(4));
    }
    device.disconnect();

    for row in 0..screen.rows {
        for col in 0..screen.cols {
            if let Some(what) = screen.ink_debug(col, row) {
                println!(
                    "col {col:>2} row {row:>2}  {what:<14} -> {:?}",
                    screen.glyph_at(col, row).0
                );
            }
        }
    }
    Ok(())
}
