//! Wires [`Engine`] to a live `cpal` output stream.

use std::sync::Arc;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use piano_core::SampleRate;
use piano_params::Tuning;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::commands::Command;
use crate::denormals;
use crate::engine::Engine;
use crate::error::AudioError;
use crate::timing::CallbackTimer;

/// Commands the ring can hold before the producer starts dropping notes.
const COMMAND_QUEUE_CAPACITY: usize = 256;

/// Frames processed per interior chunk. Bounds the scratch buffer so the
/// callback never allocates, regardless of how many frames the host
/// requests in one call.
const MAX_CHUNK_FRAMES: usize = 4096;

/// Opens the default output device and starts playback.
///
/// Returns the live stream (drop it to stop playback), a producer for
/// sending it commands, and the sample rate the engine was tuned for.
pub(crate) fn start(
    tuning: Tuning,
    timer: Arc<CallbackTimer>,
) -> Result<(cpal::Stream, Producer<Command>, SampleRate), AudioError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(AudioError::NoOutputDevice)?;
    let supported = device.default_output_config()?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = device_sample_rate(&config)?;

    let (producer, consumer) = RingBuffer::new(COMMAND_QUEUE_CAPACITY);
    let engine = Engine::new(sample_rate, tuning);
    let stream = build_stream(sample_format, &device, &config, engine, consumer, timer)?;
    stream.play()?;
    Ok((stream, producer, sample_rate))
}

fn device_sample_rate(config: &StreamConfig) -> Result<SampleRate, AudioError> {
    let hertz = config.sample_rate.0 as f32;
    SampleRate::new(hertz).map_err(|_| AudioError::InvalidSampleRate(hertz))
}

fn build_stream(
    sample_format: SampleFormat,
    device: &cpal::Device,
    config: &StreamConfig,
    engine: Engine,
    consumer: Consumer<Command>,
    timer: Arc<CallbackTimer>,
) -> Result<cpal::Stream, AudioError> {
    match sample_format {
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, engine, consumer, timer),
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, engine, consumer, timer),
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, engine, consumer, timer),
        other => Err(AudioError::UnsupportedSampleFormat(other)),
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    engine: Engine,
    consumer: Consumer<Command>,
    timer: Arc<CallbackTimer>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels).max(1);
    let mut callback = AudioCallback::new(engine, consumer, timer, channels);
    let data_callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| callback.run(data);
    Ok(device.build_output_stream(config, data_callback, report_stream_error, None)?)
}

/// Runs on a `cpal`-owned helper thread, never the audio callback itself, so
/// printing here is not a realtime-safety violation.
#[allow(clippy::needless_pass_by_value)] // signature fixed by cpal's error_callback bound
fn report_stream_error(error: cpal::StreamError) {
    eprintln!("audio stream error: {error}");
}

/// Everything the audio callback touches, bundled so the callback closure
/// only has one thing to move and one method to call.
struct AudioCallback {
    engine: Engine,
    consumer: Consumer<Command>,
    timer: Arc<CallbackTimer>,
    channels: usize,
    scratch: [f32; MAX_CHUNK_FRAMES],
    denormals_enabled: bool,
}

impl AudioCallback {
    fn new(
        engine: Engine,
        consumer: Consumer<Command>,
        timer: Arc<CallbackTimer>,
        channels: usize,
    ) -> Self {
        Self {
            engine,
            consumer,
            timer,
            channels,
            scratch: [0.0; MAX_CHUNK_FRAMES],
            denormals_enabled: false,
        }
    }

    /// The audio callback body. Allocates nothing, locks nothing, panics
    /// nowhere: `drain_commands` and `process_block` are the two calls that
    /// carry that guarantee, and everything else here is arithmetic on
    /// values already owned by `self`.
    fn run<T>(&mut self, data: &mut [T])
    where
        T: SizedSample + FromSample<f32>,
    {
        if !self.denormals_enabled {
            denormals::enable_flush_to_zero();
            self.denormals_enabled = true;
        }
        let started = Instant::now();
        self.engine.drain_commands(&mut self.consumer);
        write_frames(data, self.channels, &mut self.scratch, &mut self.engine);
        self.timer.record(started.elapsed());
    }
}

/// Fills `data` (interleaved, `channels` per frame) from `engine`, chunking
/// through `scratch` so the block size is bounded regardless of `data`'s
/// length.
fn write_frames<T>(data: &mut [T], channels: usize, scratch: &mut [f32], engine: &mut Engine)
where
    T: SizedSample + FromSample<f32>,
{
    for chunk in data.chunks_mut(channels * scratch.len()) {
        let frames = chunk.len() / channels;
        let Some(block) = scratch.get_mut(..frames) else {
            continue;
        };
        engine.process_block(block);
        mix_into(chunk, channels, block);
    }
}

fn mix_into<T>(chunk: &mut [T], channels: usize, block: &[f32])
where
    T: SizedSample + FromSample<f32>,
{
    for (frame, &sample) in chunk.chunks_mut(channels).zip(block.iter()) {
        for slot in frame {
            *slot = T::from_sample(sample);
        }
    }
}
