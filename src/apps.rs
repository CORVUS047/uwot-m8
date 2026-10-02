//! Capturing one application instead of a whole device.
//!
//! A monitor source carries everything a device plays, mixed together. The sound
//! server can do better than that: a record stream can be tied to a single
//! playback stream — one application — and hear only it, which is what sampling
//! a game off its own output wants. cpal never asks for that, so this speaks the
//! PulseAudio protocol itself, which PipeWire serves too.
//!
//! The application carries on playing where it was: this listens in rather than
//! taking it over, so you keep hearing the game while the M8 samples it.

use std::ffi::CString;
use std::io::{BufReader, Read};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use pulseaudio::protocol;

/// The name this appears under in a patchbay.
const CLIENT: &str = "uwot-m8";

/// cpal names its PulseAudio client this, so a stream called it is this app's
/// own playing of the M8 — which is a feedback loop to send back in.
const OWN_PLAYBACK: &str = "cpal-pulseaudio";

/// What is asked for, and all that is handled.
const CHANNELS: u8 = 2;
const FORMAT: protocol::SampleFormat = protocol::SampleFormat::Float32Le;
const BYTES_PER_SAMPLE: u32 = 4;

/// Sequence numbers. The server echoes them; they only have to differ.
const SEQ_AUTH: u32 = 0;
const SEQ_NAME: u32 = 1;
const SEQ_LIST: u32 = 2;
const SEQ_SINK: u32 = 3;
const SEQ_STREAM: u32 = 4;
const SEQ_SUBSCRIBE: u32 = 5;
const SEQ_MODULE: u32 = 6;
const SEQ_MOVE: u32 = 7;
const SEQ_UNLOAD: u32 = 8;

/// The sink made to take an application off the speakers, and the module that
/// provides it.
///
/// One name, not one per process: two copies of this app both sending the same
/// application is not a thing anyone wants, and a fixed name is what lets a
/// crashed run's leftovers be cleared up on the next one.
const QUIET_SINK: &str = "uwot_m8";
const NULL_SINK_MODULE: &str = "module-null-sink";

/// What `LoadModule` replies with.
struct ModuleIndex(u32);

impl protocol::CommandReply for ModuleIndex {}

impl protocol::TagStructRead for ModuleIndex {
    fn read(
        ts: &mut protocol::TagStructReader<'_>,
        _version: u16,
    ) -> Result<Self, protocol::ProtocolError> {
        Ok(Self(ts.read_u32()?))
    }
}

impl protocol::TagStructWrite for ModuleIndex {
    fn write(
        &self,
        ts: &mut protocol::TagStructWriter<'_>,
        _version: u16,
    ) -> Result<(), protocol::ProtocolError> {
        ts.write_u32(self.0)
    }
}

/// An application taken off the speakers, and what it takes to put it back.
struct Quieted {
    /// The application's playback stream.
    stream: u32,
    /// Where it was playing before, to be put back on.
    was_on: u32,
    /// The module behind the sink it was moved to.
    module: u32,
}

/// The applications playing audio, each named once.
///
/// This app's own playing of the M8 is left out: sending that back into the M8
/// is a feedback loop, and it is never what someone means by "the game".
pub fn playing() -> Vec<String> {
    let Ok((mut sock, version)) = connect() else {
        return Vec::new();
    };
    let Ok(streams) = sink_inputs(&mut sock, version) else {
        return Vec::new();
    };

    let mut names: Vec<String> = Vec::new();
    for stream in &streams {
        let name = app_name(stream);
        if name.is_empty() || name.contains(OWN_PLAYBACK) || name.contains(CLIENT) {
            continue;
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A capture of one application, resolved and open but not yet pumping.
///
/// Opening comes first because the rate is the server's to decide, and the rest
/// of the passthrough is sized from it.
pub struct Opened {
    sock: BufReader<UnixStream>,
    version: u16,
    app: String,
    /// The playback stream this is tied to, watched so its going away can be
    /// noticed.
    stream: u32,
    /// Set when the application was taken off this computer's speakers, and so
    /// has to be put back.
    quieted: Option<Quieted>,
    rate: u32,
    channels: usize,
    format: protocol::SampleFormat,
}

impl Opened {
    /// The rate the server is sending at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How many channels the samples are interleaved in.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The application this is listening to, as the server names it.
    pub fn app(&self) -> &str {
        &self.app
    }

    /// Starts reading, handing every batch of samples to `on_samples` as floats.
    pub fn start(self, mut on_samples: impl FnMut(&[f32]) + Send + 'static) -> Capture {
        let failed = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_failed = Arc::clone(&failed);
        let thread_stopping = Arc::clone(&stopping);

        // Kept so dropping the capture can shut the socket down under the
        // reader, which is the only thing that unblocks its read.
        let socket = self.sock.get_ref().try_clone().ok();

        let Opened {
            mut sock,
            version,
            format,
            stream,
            quieted,
            ..
        } = self;
        let thread = std::thread::spawn(move || {
            let mut bytes: Vec<u8> = Vec::new();
            let mut samples: Vec<f32> = Vec::new();
            loop {
                match read_batch(&mut sock, version, &mut bytes, format, &mut samples, stream) {
                    Ok(Batch::Samples) => on_samples(&samples),
                    Ok(Batch::Nothing) => {}
                    Ok(Batch::Gone) | Err(_) => {
                        if !thread_stopping.load(Ordering::Relaxed) {
                            thread_failed.store(true, Ordering::Relaxed);
                        }
                        return;
                    }
                }
            }
        });

        Capture {
            failed,
            stopping,
            socket,
            thread: Some(thread),
            quieted,
        }
    }
}

/// A running capture. Dropping it stops the reading and closes the stream.
pub struct Capture {
    failed: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    socket: Option<UnixStream>,
    thread: Option<JoinHandle<()>>,
    /// What to undo, if the application was taken off the speakers.
    quieted: Option<Quieted>,
}

impl Capture {
    /// True once the application went away, or the server stopped sending.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(quieted) = self.quieted.take() {
            // Dropped or not, the application has to come back to the speakers,
            // so this goes through a fresh connection: ours is shut by now.
            let _ = restore(&quieted);
        }
    }
}

/// Opens a capture of the application whose name matches `wanted`.
///
/// It is listened to where it already plays, so it stays audible here — unless
/// `exclusive`, which moves it onto a sink of its own that goes nowhere, leaving
/// the M8 the only way it is heard. Dropping the capture puts it back.
pub fn open(wanted: &str, period: std::time::Duration, exclusive: bool) -> Result<Opened, String> {
    let (mut sock, version) = connect()?;
    if exclusive {
        // A sink left behind by a run that did not get to clean up: unloading it
        // hands whatever is stranded on it back to the default output.
        if let Some(stale) = find_sink(&mut sock, version, QUIET_SINK)? {
            if let Some(module) = stale.owner_module_index {
                unload(&mut sock, version, module)?;
            }
        }
    }
    let streams = sink_inputs(&mut sock, version)?;

    let needle = wanted.to_lowercase();
    let target = streams
        .iter()
        .find(|stream| app_name(stream) == wanted)
        .or_else(|| {
            streams
                .iter()
                .find(|stream| app_name(stream).to_lowercase().contains(&needle))
        })
        .ok_or_else(|| format!("{wanted} is not playing anything"))?;
    let app = app_name(target);
    let stream_index = target.index;
    let app_rate = target.sample_spec.sample_rate;

    let quieted = if exclusive {
        Some(quieten(
            &mut sock,
            version,
            stream_index,
            target.sink_index,
        )?)
    } else {
        None
    };

    // Where to listen: the sink it was moved to when it was quietened, else the
    // one it was already playing to.
    let playing_to = match &quieted {
        Some(_) => match find_sink(&mut sock, version, QUIET_SINK)? {
            Some(sink) => sink.index,
            None => {
                let _ = restore(quieted.as_ref().expect("quieted"));
                return Err("the sink made to quieten the application went missing".into());
            }
        },
        None => target.sink_index,
    };

    let opened = finish(
        sock,
        version,
        &app,
        stream_index,
        playing_to,
        period,
        app_rate,
    );
    match opened {
        Ok(mut opened) => {
            opened.quieted = quieted;
            Ok(opened)
        }
        Err(e) => {
            // Half a passthrough must not leave an application mute.
            if let Some(quieted) = &quieted {
                let _ = restore(quieted);
            }
            Err(e)
        }
    }
}

/// Everything from the monitor source on: the record stream, and asking to be
/// told when the application goes away.
fn finish(
    mut sock: BufReader<UnixStream>,
    version: u16,
    app: &str,
    stream_index: u32,
    playing_to: u32,
    period: std::time::Duration,
    app_rate: u32,
) -> Result<Opened, String> {
    // The stream is tied to one playback stream, but it still has to be made on
    // the monitor of the device that is playing it.
    write(
        &mut sock,
        SEQ_SINK,
        &protocol::Command::GetSinkInfo(protocol::GetSinkInfo {
            index: Some(playing_to),
            ..Default::default()
        }),
        version,
    )?;
    let sink = read::<protocol::SinkInfo>(&mut sock, version)?;
    let monitor = sink
        .monitor_source_index
        .ok_or_else(|| format!("what {app} plays to cannot be listened in on"))?;

    let rate = app_rate;
    let fragment = (rate as f32 * period.as_secs_f32()) as u32 * CHANNELS as u32 * BYTES_PER_SAMPLE;
    write(
        &mut sock,
        SEQ_STREAM,
        &protocol::Command::CreateRecordStream(protocol::RecordStreamParams {
            source_index: Some(monitor),
            direct_on_input_index: Some(stream_index),
            sample_spec: protocol::SampleSpec {
                format: FORMAT,
                channels: CHANNELS,
                sample_rate: rate,
            },
            channel_map: protocol::ChannelMap::stereo(),
            cvolume: Some(protocol::ChannelVolume::norm(CHANNELS)),
            buffer_attr: protocol::stream::BufferAttr {
                max_length: u32::MAX,
                // Left to itself the server sends seconds at a time, which is
                // seconds of delay before the M8 hears any of it.
                fragment_size: fragment.max(1),
                ..Default::default()
            },
            flags: protocol::stream::StreamFlags {
                adjust_latency: true,
                ..Default::default()
            },
            props: client_props(),
            ..Default::default()
        }),
        version,
    )?;
    let stream = read::<protocol::CreateRecordStreamReply>(&mut sock, version)?;

    // Once the application closes, the server leaves the stream running and
    // quietly feeds it the device's whole mix instead — which, with the M8
    // playing through this computer, is a feedback loop. So ask to be told.
    write(
        &mut sock,
        SEQ_SUBSCRIBE,
        &protocol::Command::Subscribe(protocol::SubscriptionMask::SINK_INPUT),
        version,
    )?;
    protocol::read_ack_message(&mut sock)
        .map_err(|e| format!("the sound server refused to report closures: {e}"))?;

    Ok(Opened {
        sock,
        version,
        app: app.to_string(),
        stream: stream_index,
        quieted: None,
        rate: stream.sample_spec.sample_rate,
        channels: stream.channel_map.num_channels() as usize,
        format: stream.sample_spec.format,
    })
}

/// Takes an application off the speakers: a sink of its own that goes nowhere,
/// and the application moved onto it.
fn quieten(
    sock: &mut BufReader<UnixStream>,
    version: u16,
    stream: u32,
    was_on: u32,
) -> Result<Quieted, String> {
    let arguments = format!(
        "sink_name={QUIET_SINK} sink_properties=device.description=\"uwot-m8 (into the M8)\""
    );
    write(
        sock,
        SEQ_MODULE,
        &protocol::Command::LoadModule(protocol::LoadModuleParams {
            name: CString::new(NULL_SINK_MODULE).map_err(|e| e.to_string())?,
            arguments: CString::new(arguments).ok(),
        }),
        version,
    )?;
    let module = read::<ModuleIndex>(sock, version)
        .map_err(|e| format!("cannot make a sink to quieten the application: {e}"))?
        .0;

    let sink = match find_sink(sock, version, QUIET_SINK)? {
        Some(sink) => sink,
        None => {
            unload(sock, version, module)?;
            return Err("the sound server made no sink to quieten the application".into());
        }
    };
    if let Err(e) = move_stream(sock, version, stream, sink.index) {
        unload(sock, version, module)?;
        return Err(e);
    }
    Ok(Quieted {
        stream,
        was_on,
        module,
    })
}

/// Puts an application back where it was playing, and clears the sink away.
fn restore(quieted: &Quieted) -> Result<(), String> {
    let (mut sock, version) = connect()?;
    // Back to its own device first: unloading alone would land it on whatever
    // the default output happens to be now.
    let moved = move_stream(&mut sock, version, quieted.stream, quieted.was_on);
    unload(&mut sock, version, quieted.module)?;
    moved
}

/// The sink of a given name, if the server has one.
fn find_sink(
    sock: &mut BufReader<UnixStream>,
    version: u16,
    name: &str,
) -> Result<Option<protocol::SinkInfo>, String> {
    write(
        sock,
        SEQ_SINK,
        &protocol::Command::GetSinkInfo(protocol::GetSinkInfo {
            name: CString::new(name).ok(),
            ..Default::default()
        }),
        version,
    )?;
    // Asking after a sink that is not there is an error, not an empty answer.
    Ok(read::<protocol::SinkInfo>(sock, version).ok())
}

fn move_stream(
    sock: &mut BufReader<UnixStream>,
    version: u16,
    stream: u32,
    sink: u32,
) -> Result<(), String> {
    write(
        sock,
        SEQ_MOVE,
        &protocol::Command::MoveSinkInput(protocol::MoveStreamParams {
            index: Some(stream),
            device_index: Some(sink),
            device_name: None,
        }),
        version,
    )?;
    protocol::read_ack_message(sock)
        .map(|_| ())
        .map_err(|e| format!("cannot move the application's audio: {e}"))
}

fn unload(sock: &mut BufReader<UnixStream>, version: u16, module: u32) -> Result<(), String> {
    write(
        sock,
        SEQ_UNLOAD,
        &protocol::Command::UnloadModule(module),
        version,
    )?;
    protocol::read_ack_message(sock)
        .map(|_| ())
        .map_err(|e| format!("cannot clear away the quietening sink: {e}"))
}

/// What one message off the socket turned out to be.
enum Batch {
    /// Audio, left behind in the sample buffer.
    Samples,
    /// The server saying something that does not concern us.
    Nothing,
    /// The application has closed, so there is nothing left to listen to.
    Gone,
}

/// Reads one message, audio or otherwise.
fn read_batch(
    sock: &mut BufReader<UnixStream>,
    version: u16,
    bytes: &mut Vec<u8>,
    format: protocol::SampleFormat,
    samples: &mut Vec<f32>,
    stream: u32,
) -> Result<Batch, String> {
    let descriptor = protocol::read_descriptor(sock).map_err(|e| e.to_string())?;

    // Everything but this channel is audio; this one is the server talking.
    if descriptor.channel == u32::MAX {
        let (_, command) =
            protocol::Command::read_tag_prefixed(sock, version).map_err(|e| e.to_string())?;
        let gone = matches!(
            command,
            protocol::Command::SubscribeEvent(event)
                if event.event_facility == protocol::SubscriptionEventFacility::SinkInput
                    && event.event_type == protocol::SubscriptionEventType::Removed
                    && event.index == Some(stream)
        );
        return Ok(if gone { Batch::Gone } else { Batch::Nothing });
    }

    bytes.resize(descriptor.length as usize, 0);
    sock.read_exact(bytes).map_err(|e| e.to_string())?;

    samples.clear();
    match format {
        protocol::SampleFormat::Float32Le => samples.extend(
            bytes
                .chunks_exact(4)
                .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]])),
        ),
        protocol::SampleFormat::S16Le => samples.extend(
            bytes
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / i16::MAX as f32),
        ),
        protocol::SampleFormat::S32Le => samples.extend(
            bytes
                .chunks_exact(4)
                .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / i32::MAX as f32),
        ),
        other => {
            return Err(format!(
                "the server is sending {other:?}, which we cannot read"
            ))
        }
    }
    Ok(Batch::Samples)
}

/// Connects to the sound server and says who we are.
fn connect() -> Result<(BufReader<UnixStream>, u16), String> {
    let path = pulseaudio::socket_path_from_env()
        .ok_or("no PulseAudio or PipeWire to ask about applications")?;
    let stream =
        UnixStream::connect(path).map_err(|e| format!("cannot reach the sound server: {e}"))?;
    let mut sock = BufReader::new(stream);

    let cookie = pulseaudio::cookie_path_from_env()
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    write(
        &mut sock,
        SEQ_AUTH,
        &protocol::Command::Auth(protocol::AuthParams {
            version: protocol::MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie,
        }),
        protocol::MAX_VERSION,
    )?;
    let auth = read::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION)?;
    let version = protocol::MAX_VERSION.min(auth.version);

    write(
        &mut sock,
        SEQ_NAME,
        &protocol::Command::SetClientName(client_props()),
        version,
    )?;
    read::<protocol::SetClientNameReply>(&mut sock, version)?;
    Ok((sock, version))
}

/// Every playback stream the server currently has.
fn sink_inputs(
    sock: &mut BufReader<UnixStream>,
    version: u16,
) -> Result<protocol::SinkInputInfoList, String> {
    write(
        sock,
        SEQ_LIST,
        &protocol::Command::GetSinkInputInfoList,
        version,
    )?;
    read::<protocol::SinkInputInfoList>(sock, version)
}

fn write(
    sock: &mut BufReader<UnixStream>,
    seq: u32,
    command: &protocol::Command,
    version: u16,
) -> Result<(), String> {
    protocol::write_command_message(sock.get_mut(), seq, command, version)
        .map_err(|e| format!("cannot talk to the sound server: {e}"))
}

fn read<T: protocol::CommandReply>(
    sock: &mut BufReader<UnixStream>,
    version: u16,
) -> Result<T, String> {
    protocol::read_reply_message::<T>(sock, version)
        .map(|(_, reply)| reply)
        .map_err(|e| format!("the sound server refused: {e}"))
}

fn client_props() -> protocol::Props {
    let mut props = protocol::Props::new();
    if let Ok(name) = CString::new(CLIENT) {
        props.set(protocol::Prop::ApplicationName, name);
    }
    props
}

/// What to call an application: the name it gave, else the stream's own.
fn app_name(stream: &protocol::SinkInputInfo) -> String {
    let from_props = stream
        .props
        .get(protocol::Prop::ApplicationName)
        .map(text)
        .filter(|name| !name.is_empty());
    from_props.unwrap_or_else(|| text(stream.name.as_bytes()))
}

/// A property's bytes as a string, without the trailing NUL the server sends.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_property_loses_the_trailing_nul_the_server_sends() {
        assert_eq!(text(b"Zen\0"), "Zen");
        assert_eq!(text(b"osu!"), "osu!");
        assert_eq!(text(b""), "");
    }
}
