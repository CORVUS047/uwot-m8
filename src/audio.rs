//! Playing the M8's audio through the computer's own output.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, FromSample, Host, HostId, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig, I24, U24,
};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;

const DEVICE_NEEDLE: &str = "m8";
/// A monitor device is the M8 being *played*.
const MONITOR_NEEDLE: &str = "monitor";

const CHANNELS: usize = 2;

/// One callback's worth of audio.
///
/// Left to itself a sound server sizes these for a music player, not a monitor:
/// PulseAudio's default is "something like 2s", which is where the seconds of
/// delay and, once the ring overflowed, the chop came from. So we ask.
const PERIOD: Duration = Duration::from_millis(10);

/// Enough to ride out a scheduling hiccup, little enough not to be heard.
const TARGET_LATENCY: Duration = Duration::from_millis(30);
/// Past this the input clock is outrunning the output's, so frames go.
const MAX_LATENCY: Duration = Duration::from_millis(120);
const CAPACITY: Duration = Duration::from_millis(500);
/// So a wedged audio server cannot hang the app.
const BUILD_TIMEOUT: Duration = Duration::from_secs(2);

/// A running passthrough. Dropping it stops the audio.
pub struct Audio {
    /// Held only to keep the streams alive; the callbacks do the work.
    _input: Stream,
    _output: Stream,
    failed: Arc<AtomicBool>,
    description: String,
}

impl Audio {
    /// Starts playing the M8's audio through the default output.
    pub fn start() -> Result<Self, String> {
        let mut hosts = cpal::available_hosts();
        hosts.sort_by_key(|id| host_rank(id.name()));

        let mut last_error = None;
        for id in hosts {
            let Ok(host) = cpal::host_from_id(id) else {
                continue;
            };
            let Some(output) = host.default_output_device() else {
                continue;
            };
            for input in m8_inputs(&host) {
                match Self::connect(&input, &output, id) {
                    Ok(audio) => return Ok(audio),
                    Err(e) => last_error = Some(e),
                }
            }
        }
        Err(last_error.unwrap_or_else(|| "no M8 audio device; is the M8 plugged in?".into()))
    }

    fn connect(input: &Device, output: &Device, host: HostId) -> Result<Self, String> {
        let in_config = input_config(input)?;
        let rate = in_config.sample_rate();
        let out_config = output_config(output, rate)?;

        let frames = |duration: Duration| frames_at(rate, duration);
        let in_channels = in_config.channels() as usize;
        let out_channels = out_config.channels() as usize;
        let out_rate = out_config.sample_rate();

        let in_period = period(&in_config);
        let out_period = period(&out_config);

        // What one output callback eats, counted in input frames, plus the
        // capture chunk that refills it. Playing cannot start on less than this
        // or the very first callback runs dry.
        let out_frames = out_period.unwrap_or_else(|| frames_at(out_rate, PERIOD) as u32) as f64
            * rate as f64
            / out_rate.max(1) as f64;
        let one_pass = out_frames.ceil() as usize
            + in_period.unwrap_or_else(|| frames(PERIOD) as u32) as usize;

        let target = frames(TARGET_LATENCY).max(one_pass) * CHANNELS;
        let ceiling = frames(MAX_LATENCY).max(target / CHANNELS + one_pass) * CHANNELS;
        let ring = HeapRb::<f32>::new(frames(CAPACITY).max(ceiling / CHANNELS * 2) * CHANNELS);
        let (mut producer, mut consumer) = ring.split();

        let failed = Arc::new(AtomicBool::new(false));
        let input_failed = Arc::clone(&failed);
        let output_failed = Arc::clone(&failed);

        let mut capture = move |frame: [f32; CHANNELS]| {
            let _ = producer.push_slice(&frame);
        };
        let input_stream = build_input(
            input,
            &in_config,
            in_period,
            move |samples: &[f32]| {
                for frame in samples.chunks(in_channels) {
                    capture(widen(frame));
                }
            },
            move |_| input_failed.store(true, Ordering::Relaxed),
        )?;

        let mut resampler = Resampler::new(rate, out_rate);
        let mut playing = false;
        let mut play = move |out: &mut [f32]| {
            if !playing && consumer.occupied_len() < target {
                out.fill(0.0);
                return;
            }
            playing = true;

            while consumer.occupied_len() > ceiling {
                let mut discard = [0.0f32; CHANNELS];
                if consumer.pop_slice(&mut discard) < CHANNELS {
                    break;
                }
            }

            for out_frame in out.chunks_mut(out_channels) {
                let mut pop = || {
                    let mut frame = [0.0f32; CHANNELS];
                    (consumer.pop_slice(&mut frame) == CHANNELS).then_some(frame)
                };
                match resampler.next_frame(&mut pop) {
                    Some(frame) => narrow(frame, out_frame),
                    None => {
                        playing = false;
                        out_frame.fill(0.0);
                    }
                }
            }
        };
        let output_stream = build_output(
            output,
            &out_config,
            out_period,
            move |samples: &mut [f32]| {
                play(samples);
            },
            move |_| output_failed.store(true, Ordering::Relaxed),
        )?;

        input_stream
            .play()
            .map_err(|e| format!("cannot start the M8 capture: {e}"))?;
        output_stream
            .play()
            .map_err(|e| format!("cannot start audio output: {e}"))?;

        let description = format!(
            "{} to {} at {rate} Hz via {}",
            name_of(input),
            name_of(output),
            host.name(),
        );
        Ok(Self {
            _input: input_stream,
            _output: output_stream,
            failed,
            description,
        })
    }

    /// True once either stream has failed.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// What is being played where, for the settings menu.
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// The M8's capture devices on one host, monitors left out.
fn m8_inputs(host: &Host) -> Vec<Device> {
    let Ok(inputs) = host.input_devices() else {
        return Vec::new();
    };
    inputs
        .filter(|device| {
            let Ok(about) = device.description() else {
                return false;
            };
            let name = about.name().to_lowercase();
            name.contains(DEVICE_NEEDLE) && !name.contains(MONITOR_NEEDLE)
        })
        .collect()
}

/// Which hosts to try first.
///
/// Three hosts, five ALSA names for one card, and the name the kernel hands you
/// is the one the sound server already took — so you ask ALSA for the M8 and get
/// told it is busy BY THE THING PLAYING IT FOR YOU. Servers first, then.
fn host_rank(name: &str) -> u8 {
    match name.to_lowercase() {
        name if name.contains("pipewire") => 0,
        name if name.contains("pulse") => 1,
        name if name.contains("coreaudio") || name.contains("wasapi") => 1,
        name if name.contains("alsa") => 2,
        _ => 3,
    }
}

fn name_of(device: &Device) -> String {
    device
        .description()
        .map(|about| about.name().to_string())
        .unwrap_or_else(|_| "unknown device".into())
}

/// Any channel count onto stereo: mono is heard both sides, wider keeps two.
fn widen(frame: &[f32]) -> [f32; CHANNELS] {
    match frame {
        [] => [0.0, 0.0],
        [mono] => [*mono, *mono],
        [left, right, ..] => [*left, *right],
    }
}

/// Stereo onto any channel count: mono gets the mix, wider gets silence.
fn narrow(frame: [f32; CHANNELS], out: &mut [f32]) {
    match out {
        [] => {}
        [mono] => *mono = (frame[0] + frame[1]) / 2.0,
        [left, right, rest @ ..] => {
            *left = frame[0];
            *right = frame[1];
            rest.fill(0.0);
        }
    }
}

/// Reads frames at one rate and hands them back at another.
struct Resampler {
    step: f32,
    phase: f32,
    prev: [f32; CHANNELS],
    next: [f32; CHANNELS],
}

impl Resampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            step: in_rate as f32 / out_rate.max(1) as f32,
            phase: 1.0,
            prev: [0.0; CHANNELS],
            next: [0.0; CHANNELS],
        }
    }

    fn next_frame(
        &mut self,
        pop: &mut impl FnMut() -> Option<[f32; CHANNELS]>,
    ) -> Option<[f32; CHANNELS]> {
        while self.phase >= 1.0 {
            self.prev = self.next;
            self.next = pop()?;
            self.phase -= 1.0;
        }
        let t = self.phase;
        self.phase += self.step;
        Some([
            self.prev[0] + (self.next[0] - self.prev[0]) * t,
            self.prev[1] + (self.next[1] - self.prev[1]) * t,
        ])
    }
}

/// The M8's capture format: its own default, or the widest on offer.
fn input_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    if let Ok(config) = device.default_input_config() {
        return Ok(config);
    }
    let mut ranges: Vec<_> = device
        .supported_input_configs()
        .map_err(|e| format!("cannot read the M8's audio formats: {e}"))?
        .collect();
    ranges.sort_by_key(|range| {
        (
            range.channels() != CHANNELS as u16,
            u32::MAX - range.max_sample_rate(),
        )
    });
    ranges
        .into_iter()
        .next()
        .map(|range| range.with_max_sample_rate())
        .ok_or_else(|| "the M8 offers no audio format we can read".into())
}

/// An output format at `rate` if the device has one, else its default.
fn output_config(device: &Device, rate: u32) -> Result<SupportedStreamConfig, String> {
    let ranges = device
        .supported_output_configs()
        .map_err(|e| format!("cannot read the output's audio formats: {e}"))?;
    let matching = ranges
        .filter(|range| range.min_sample_rate() <= rate && rate <= range.max_sample_rate())
        .min_by_key(|range| (range.channels() as i32 - CHANNELS as i32).abs());
    if let Some(range) = matching {
        return Ok(range.with_sample_rate(rate));
    }
    device
        .default_output_config()
        .map_err(|e| format!("the output has no usable audio format: {e}"))
}

/// The capture stream, converting the device's format into floats.
fn build_input(
    device: &Device,
    config: &SupportedStreamConfig,
    period: Option<u32>,
    mut on_samples: impl FnMut(&[f32]) + Send + 'static,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream, String> {
    let stream_config = stream_config(config, period);

    fn build<T>(
        device: &Device,
        config: StreamConfig,
        mut on_samples: impl FnMut(&[f32]) + Send + 'static,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
    ) -> Result<Stream, cpal::Error>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let mut scratch: Vec<f32> = Vec::new();
        device.build_input_stream::<T, _, _>(
            config,
            move |samples, _| {
                scratch.clear();
                scratch.extend(samples.iter().map(|s| f32::from_sample_(*s)));
                on_samples(&scratch);
            },
            on_error,
            Some(BUILD_TIMEOUT),
        )
    }

    let result = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            stream_config,
            move |samples, _| on_samples(samples),
            on_error,
            Some(BUILD_TIMEOUT),
        ),
        SampleFormat::I8 => build::<i8>(device, stream_config, on_samples, on_error),
        SampleFormat::I16 => build::<i16>(device, stream_config, on_samples, on_error),
        SampleFormat::I24 => build::<I24>(device, stream_config, on_samples, on_error),
        SampleFormat::I32 => build::<i32>(device, stream_config, on_samples, on_error),
        SampleFormat::I64 => build::<i64>(device, stream_config, on_samples, on_error),
        SampleFormat::U8 => build::<u8>(device, stream_config, on_samples, on_error),
        SampleFormat::U16 => build::<u16>(device, stream_config, on_samples, on_error),
        SampleFormat::U24 => build::<U24>(device, stream_config, on_samples, on_error),
        SampleFormat::U32 => build::<u32>(device, stream_config, on_samples, on_error),
        SampleFormat::U64 => build::<u64>(device, stream_config, on_samples, on_error),
        SampleFormat::F64 => build::<f64>(device, stream_config, on_samples, on_error),
        other => return Err(format!("the M8's audio format ({other}) is not supported")),
    };
    result.map_err(|e| format!("cannot capture the M8's audio: {e}"))
}

/// The playback stream, converting the floats back to the device's format.
fn build_output(
    device: &Device,
    config: &SupportedStreamConfig,
    period: Option<u32>,
    mut fill: impl FnMut(&mut [f32]) + Send + 'static,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream, String> {
    let stream_config = stream_config(config, period);

    fn build<T>(
        device: &Device,
        config: StreamConfig,
        mut fill: impl FnMut(&mut [f32]) + Send + 'static,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
    ) -> Result<Stream, cpal::Error>
    where
        T: SizedSample + FromSample<f32>,
    {
        let mut scratch: Vec<f32> = Vec::new();
        device.build_output_stream::<T, _, _>(
            config,
            move |out, _| {
                scratch.clear();
                scratch.resize(out.len(), 0.0);
                fill(&mut scratch);
                for (slot, sample) in out.iter_mut().zip(&scratch) {
                    *slot = T::from_sample_(*sample);
                }
            },
            on_error,
            Some(BUILD_TIMEOUT),
        )
    }

    let result = match config.sample_format() {
        SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            stream_config,
            move |out, _| fill(out),
            on_error,
            Some(BUILD_TIMEOUT),
        ),
        SampleFormat::I8 => build::<i8>(device, stream_config, fill, on_error),
        SampleFormat::I16 => build::<i16>(device, stream_config, fill, on_error),
        SampleFormat::I24 => build::<I24>(device, stream_config, fill, on_error),
        SampleFormat::I32 => build::<i32>(device, stream_config, fill, on_error),
        SampleFormat::I64 => build::<i64>(device, stream_config, fill, on_error),
        SampleFormat::U8 => build::<u8>(device, stream_config, fill, on_error),
        SampleFormat::U16 => build::<u16>(device, stream_config, fill, on_error),
        SampleFormat::U24 => build::<U24>(device, stream_config, fill, on_error),
        SampleFormat::U32 => build::<u32>(device, stream_config, fill, on_error),
        SampleFormat::U64 => build::<u64>(device, stream_config, fill, on_error),
        SampleFormat::F64 => build::<f64>(device, stream_config, fill, on_error),
        other => {
            return Err(format!(
                "the output's audio format ({other}) is not supported"
            ))
        }
    };
    result.map_err(|e| format!("cannot play audio: {e}"))
}

fn stream_config(config: &SupportedStreamConfig, period: Option<u32>) -> StreamConfig {
    StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size: period.map_or(BufferSize::Default, BufferSize::Fixed),
    }
}

fn frames_at(rate: u32, duration: Duration) -> usize {
    ((rate as f32 * duration.as_secs_f32()) as usize).max(1)
}

/// The period to ask a device for: `PERIOD`'s worth of frames, held inside what
/// it allows. `None` where the host will not say, and its default has to do.
fn period(config: &SupportedStreamConfig) -> Option<u32> {
    let wanted = frames_at(config.sample_rate(), PERIOD) as u32;
    match *config.buffer_size() {
        SupportedBufferSize::Range { min, max } => Some(wanted.clamp(min, max.max(min))),
        SupportedBufferSize::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mono_frame_is_heard_on_both_sides() {
        assert_eq!(widen(&[0.5]), [0.5, 0.5]);
        assert_eq!(widen(&[0.1, 0.2, 0.3, 0.4]), [0.1, 0.2]);
        assert_eq!(widen(&[]), [0.0, 0.0]);
    }

    #[test]
    fn a_stereo_frame_is_mixed_down_for_a_mono_output() {
        let mut mono = [0.0];
        narrow([1.0, 0.0], &mut mono);
        assert_eq!(mono, [0.5]);

        let mut surround = [9.0; 4];
        narrow([0.25, 0.75], &mut surround);
        assert_eq!(surround, [0.25, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn matching_rates_pass_frames_through_unchanged() {
        let mut resampler = Resampler::new(44_100, 44_100);
        let frames = [[1.0, -1.0], [0.5, -0.5], [0.25, -0.25]];
        let mut at = 0;
        let mut pop = || {
            let frame = frames.get(at).copied();
            at += 1;
            frame
        };
        assert_eq!(resampler.next_frame(&mut pop), Some([0.0, 0.0]));
        assert_eq!(resampler.next_frame(&mut pop), Some([1.0, -1.0]));
        assert_eq!(resampler.next_frame(&mut pop), Some([0.5, -0.5]));
    }

    #[test]
    fn halving_the_rate_takes_two_input_frames_per_output_frame() {
        let mut resampler = Resampler::new(48_000, 24_000);
        let frames = [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [4.0, 4.0]];
        let mut at = 0;
        let mut pop = || {
            let frame = frames.get(at).copied();
            at += 1;
            frame
        };
        resampler.next_frame(&mut pop);
        resampler.next_frame(&mut pop);
        assert_eq!(at, 3);
    }

    #[test]
    fn a_resampler_stops_when_the_input_runs_dry() {
        let mut resampler = Resampler::new(44_100, 44_100);
        let mut pop = || None;
        assert_eq!(resampler.next_frame(&mut pop), None);
    }
}
