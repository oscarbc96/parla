//! cpal capture and playback at the device rate; resampling to and from
//! the pipeline's 48 kHz mono happens at this edge.
//!
//! Neither side polls: cpal calls the capture callback when the device has
//! audio and the playback callback when it wants more. Both callbacks only
//! touch preallocated buffers and lock-free rings: no allocation, no locks.

use super::SAMPLE_RATE;
use super::pipeline::{
    AudioSink, AudioSource, CaptureTx, PipelineError, PlayoutTx, SINK_TARGET_SAMPLES, SinkFactory,
    SourceFactory,
};
use super::resample::Resampler;
use super::ring::Ring;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Playback ring bound, in milliseconds of device audio. Beyond it new
/// audio is dropped (and counted).
const SINK_MAX_MS: usize = 200;
/// Queue level around which the sink slips or duplicates a sample to absorb
/// clock drift: 20 ms either side of the target.
const SLIP_MARGIN_MS: usize = 20;
/// Largest block the callbacks handle at once; bigger buffers loop.
const CALLBACK_BLOCK: usize = 4096;
/// Device latency assumed while the backend reports none.
const DEFAULT_LATENCY_MS: u32 = 20;

fn device_err(e: impl std::fmt::Display) -> PipelineError {
    PipelineError::Device(e.to_string())
}

/// A device's human-readable name, as the picker and the settings spell it.
fn device_name(d: &Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_string())
}

/// The input device called `name`, or the system default for `None` or a
/// name that is not connected now (the choice comes back when it is).
fn pick_input(name: Option<&str>) -> Result<Device, PipelineError> {
    let host = cpal::default_host();
    if let Some(name) = name
        && let Ok(mut all) = host.input_devices()
        && let Some(d) = all.find(|d| device_name(d).as_deref() == Some(name))
    {
        return Ok(d);
    }
    host.default_input_device()
        .ok_or_else(|| device_err("no input device"))
}

/// The output device called `name`, or the system default (as for input).
fn pick_output(name: Option<&str>) -> Result<Device, PipelineError> {
    let host = cpal::default_host();
    if let Some(name) = name
        && let Ok(mut all) = host.output_devices()
        && let Some(d) = all.find(|d| device_name(d).as_deref() == Some(name))
    {
        return Ok(d);
    }
    host.default_output_device()
        .ok_or_else(|| device_err("no output device"))
}

/// Names of the connected microphones, in the host's order, without
/// duplicates. Enumerates once per call: callers ask when a picker opens,
/// never on a timer.
pub fn input_device_names() -> Vec<String> {
    names(cpal::default_host().input_devices().ok())
}

/// Names of the connected speakers (as [`input_device_names`]).
pub fn output_device_names() -> Vec<String> {
    names(cpal::default_host().output_devices().ok())
}

fn names(devices: Option<impl Iterator<Item = Device>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in devices
        .into_iter()
        .flatten()
        .filter_map(|d| device_name(&d))
    {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The default microphone.
pub struct CpalSource {
    device: Device,
    config: StreamConfig,
    format: SampleFormat,
    stream: Option<Stream>,
}

impl CpalSource {
    /// A factory for the media thread: opens the default input device there.
    pub fn factory() -> SourceFactory {
        Self::factory_for(None)
    }

    /// Opens the input device called `name` (the system default for `None`
    /// or when it is not connected).
    pub fn factory_for(name: Option<String>) -> SourceFactory {
        Box::new(move || {
            let device = pick_input(name.as_deref())?;
            let supported = device.default_input_config().map_err(device_err)?;
            Ok(Box::new(Self {
                device,
                format: supported.sample_format(),
                config: supported.into(),
                stream: None,
            }) as Box<dyn AudioSource>)
        })
    }
}

impl AudioSource for CpalSource {
    fn start(&mut self, out: CaptureTx) -> Result<(), PipelineError> {
        let (device, config) = (&self.device, self.config);
        let stream = match self.format {
            SampleFormat::F32 => build_input::<f32>(device, config, out),
            SampleFormat::I16 => build_input::<i16>(device, config, out),
            SampleFormat::I32 => build_input::<i32>(device, config, out),
            SampleFormat::U16 => build_input::<u16>(device, config, out),
            SampleFormat::I8 => build_input::<i8>(device, config, out),
            SampleFormat::U8 => build_input::<u8>(device, config, out),
            other => Err(device_err(format!("unsupported sample format {other}"))),
        }?;
        stream.play().map_err(device_err)?;
        self.stream = Some(stream);
        Ok(())
    }
}

fn build_input<T>(
    device: &Device,
    config: StreamConfig,
    out: CaptureTx,
) -> Result<Stream, PipelineError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels).max(1);
    let mut resampler = Resampler::new(config.sample_rate, SAMPLE_RATE);
    // Sized up front so the callback does not allocate.
    let mut mono: Vec<f32> = Vec::with_capacity(CALLBACK_BLOCK * 2);
    let mut resampled: Vec<f32> = Vec::with_capacity(CALLBACK_BLOCK * 4);
    let error_tx = CaptureTx::clone_for_errors(&out);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                mono.clear();
                mono.extend(data.chunks(channels).map(|frame| {
                    frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32
                }));
                resampled.clear();
                resampler.process(&mono, &mut resampled);
                if !resampled.is_empty() {
                    out.send(&resampled);
                }
            },
            move |e| error_tx.report_error(e.to_string()),
            None,
        )
        .map_err(device_err)
}

/// The default speaker.
pub struct CpalSink {
    device: Device,
    config: StreamConfig,
    format: SampleFormat,
    ring: Arc<Ring>,
    latency_ms: Arc<AtomicU32>,
    resampler: Resampler,
    scratch: Vec<f32>,
    rate: u32,
    stream: Option<Stream>,
}

impl CpalSink {
    /// A factory for the media thread: opens the default output device there.
    pub fn factory() -> SinkFactory {
        Self::factory_for(None)
    }

    /// Opens the output device called `name` (the system default for `None`
    /// or when it is not connected).
    pub fn factory_for(name: Option<String>) -> SinkFactory {
        Box::new(move || {
            let device = pick_output(name.as_deref())?;
            let supported = device.default_output_config().map_err(device_err)?;
            let format = supported.sample_format();
            let config: StreamConfig = supported.into();
            let rate = config.sample_rate;
            Ok(Box::new(Self {
                device,
                format,
                ring: Arc::new(Ring::new(rate as usize * SINK_MAX_MS / 1000)),
                latency_ms: Arc::new(AtomicU32::new(0)),
                resampler: Resampler::new(SAMPLE_RATE, rate),
                scratch: Vec::with_capacity(4096),
                rate,
                config,
                stream: None,
            }) as Box<dyn AudioSink>)
        })
    }

    /// 48 kHz samples to device-rate samples.
    fn to_device(&self, samples: usize) -> usize {
        samples * self.rate as usize / SAMPLE_RATE as usize
    }
}

impl AudioSink for CpalSink {
    fn start(&mut self, demand: PlayoutTx) -> Result<(), PipelineError> {
        let out = Output {
            ring: self.ring.clone(),
            demand,
            latency_ms: self.latency_ms.clone(),
            target: self.to_device(SINK_TARGET_SAMPLES),
        };
        let (device, config) = (&self.device, self.config);
        let stream = match self.format {
            SampleFormat::F32 => build_output::<f32>(device, config, out),
            SampleFormat::I16 => build_output::<i16>(device, config, out),
            SampleFormat::I32 => build_output::<i32>(device, config, out),
            SampleFormat::U16 => build_output::<u16>(device, config, out),
            SampleFormat::I8 => build_output::<i8>(device, config, out),
            SampleFormat::U8 => build_output::<u8>(device, config, out),
            other => Err(device_err(format!("unsupported sample format {other}"))),
        }?;
        stream.play().map_err(device_err)?;
        self.stream = Some(stream);
        Ok(())
    }

    fn write(&mut self, pcm: &[f32]) {
        self.scratch.clear();
        self.resampler.process(pcm, &mut self.scratch);
        // The media thread tops the queue up on demand, so drift between the
        // clocks shows as a slow level change: slip or repeat one sample per
        // frame to hold the level.
        let level = self.ring.len();
        let target = self.to_device(SINK_TARGET_SAMPLES);
        let margin = self.rate as usize * SLIP_MARGIN_MS / 1000;
        if level > target + margin && self.scratch.len() > 1 {
            self.scratch.pop();
        } else if let (true, Some(&last)) =
            (level > 0 && level + margin < target, self.scratch.last())
        {
            self.scratch.push(last);
        }
        self.ring.push_slice(&self.scratch);
    }

    fn queued_samples(&self) -> usize {
        self.ring.len() * SAMPLE_RATE as usize / self.rate as usize
    }

    fn delay_ms(&self) -> u32 {
        let queued = (self.ring.len() * 1000 / self.rate as usize) as u32;
        let reported = self.latency_ms.load(Ordering::Relaxed);
        queued
            + if reported == 0 {
                DEFAULT_LATENCY_MS
            } else {
                reported
            }
    }
}

/// What the output callback owns.
struct Output {
    ring: Arc<Ring>,
    demand: PlayoutTx,
    latency_ms: Arc<AtomicU32>,
    /// Queue level to hold, in device samples.
    target: usize,
}

fn build_output<T>(
    device: &Device,
    config: StreamConfig,
    out: Output,
) -> Result<Stream, PipelineError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels).max(1);
    // Start playing once most of the target is queued (the resampler edge
    // keeps a few samples back), and again after every underrun: this is the
    // 40 ms prefill.
    let prime_at = out.target * 3 / 4;
    let mut primed = false;
    let mut block = vec![0.0f32; CALLBACK_BLOCK];
    let error_tx = out.demand.clone_for_errors();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info| {
                let playback = info.timestamp().playback;
                let latency = playback.duration_since(info.timestamp().callback);
                out.latency_ms
                    .store(latency.as_millis().min(500) as u32, Ordering::Relaxed);
                if !primed && out.ring.len() >= prime_at {
                    primed = true;
                }
                for chunk in data.chunks_mut(CALLBACK_BLOCK * channels) {
                    let want = chunk.len() / channels;
                    let got = if primed {
                        out.ring.pop_slice(&mut block[..want])
                    } else {
                        0
                    };
                    if got < want {
                        // Underrun (or still priming): silence, then refill.
                        block[got..want].fill(0.0);
                        primed = false;
                    }
                    for (frame, sample) in chunk.chunks_mut(channels).zip(&block[..want]) {
                        frame.fill(T::from_sample(*sample));
                    }
                }
                if out.ring.len() < out.target {
                    out.demand.request();
                }
            },
            move |e| error_tx.report_error(e.to_string()),
            None,
        )
        .map_err(device_err)
}
