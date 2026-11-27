//! Native huddle audio: device capture and playback, echo
//! cancellation, Opus and a fixed-target jitter buffer on one media thread.
//!
//! The pipeline is provider-neutral and sans-network: encoded frames leave
//! through a callback and received packets enter through
//! [`MediaPipeline::push_packet`]. Task 6 wires those to the RTP stack.
//! Audio devices sit behind [`AudioSource`] and [`AudioSink`] so tests and CI
//! feed synthetic PCM.

mod apm;
mod call;
mod codec;
mod device;
mod jitter;
mod pipeline;
mod resample;
mod ring;
mod watch;

#[cfg(test)]
mod call_tests;

pub use apm::EchoProcessor;
pub use call::{CallEvent, CallEvents, Devices, NativeCall, Participant};
pub use codec::{FrameDecoder, FrameEncoder};
pub use device::{CpalSink, CpalSource, input_device_names, output_device_names};
pub use jitter::{JitterBuffer, Playout};
pub use pipeline::{
    AudioSink, AudioSource, CaptureTx, EncodedFrame, EventOut, MediaPipeline, PacketOut,
    PipelineError, PipelineEvent, PipelineStats, PlayoutTx, RtpPacket, SINK_TARGET_SAMPLES,
    SinkFactory, SourceFactory,
};
pub use resample::Resampler;
pub use watch::{DeviceChange, DeviceWatcher, OnDeviceChange};

/// Pipeline sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples per 20 ms Opus frame at [`SAMPLE_RATE`], mono.
pub const FRAME_SAMPLES: usize = 960;
/// Samples per 10 ms echo-processing block.
pub const APM_SAMPLES: usize = 480;
