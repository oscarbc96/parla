//! Echo cancellation, noise suppression and gain (sonora, 10 ms blocks).

use super::APM_SAMPLES;
use sonora::config::{EchoCanceller, GainController2, NoiseSuppression};
use sonora::{AudioProcessing, Config, StreamConfig};

pub struct EchoProcessor {
    apm: AudioProcessing,
    scratch: [f32; APM_SAMPLES],
}

impl EchoProcessor {
    pub fn new() -> Self {
        let cfg = StreamConfig::new(super::SAMPLE_RATE, 1);
        let config = Config {
            echo_canceller: Some(EchoCanceller::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            gain_controller2: Some(GainController2 {
                adaptive_digital: Some(Default::default()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let apm = AudioProcessing::builder()
            .config(config)
            .capture_config(cfg)
            .render_config(cfg)
            .build();
        Self {
            apm,
            scratch: [0.0; APM_SAMPLES],
        }
    }

    /// Sets the render-to-capture delay: audio queued for the speaker plus
    /// the device latency.
    pub fn set_stream_delay_ms(&mut self, ms: u32) {
        if let Err(e) = self.apm.set_stream_delay_ms(ms.min(500) as i32) {
            tracing::warn!("echo stream delay failed: {e}");
        }
    }

    /// Tells the canceller what is being played (the far end).
    pub fn render(&mut self, block: &[f32]) {
        debug_assert_eq!(block.len(), APM_SAMPLES);
        if let Err(e) = self
            .apm
            .process_render_f32(&[block], &mut [&mut self.scratch[..]])
        {
            tracing::warn!("echo render failed: {e}");
        }
    }

    /// Cleans one microphone block in place.
    pub fn capture(&mut self, block: &mut [f32]) {
        debug_assert_eq!(block.len(), APM_SAMPLES);
        self.scratch.copy_from_slice(block);
        if let Err(e) = self
            .apm
            .process_capture_f32(&[&self.scratch[..]], &mut [block])
        {
            tracing::warn!("echo capture failed: {e}");
        }
    }
}

impl Default for EchoProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.6
        }
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32
    }

    #[test]
    fn echo_fixture_loses_energy() {
        let mut apm = EchoProcessor::new();
        let mut rng = Lcg(7);
        // Far end noise; the "microphone" hears it 20 ms late at half level.
        let blocks = 600; // 6 s
        let far: Vec<f32> = (0..blocks * APM_SAMPLES).map(|_| rng.next()).collect();
        let delay = 960;
        let (mut in_e, mut out_e, mut n) = (0.0f32, 0.0f32, 0usize);
        for b in 0..blocks {
            let r = &far[b * APM_SAMPLES..(b + 1) * APM_SAMPLES];
            apm.render(r);
            let mut mic: Vec<f32> = (0..APM_SAMPLES)
                .map(|i| {
                    let idx = b * APM_SAMPLES + i;
                    if idx >= delay {
                        0.5 * far[idx - delay]
                    } else {
                        0.0
                    }
                })
                .collect();
            let before = energy(&mic);
            apm.capture(&mut mic);
            if b >= blocks - 100 {
                in_e += before;
                out_e += energy(&mic);
                n += 1;
            }
        }
        assert!(n > 0);
        assert!(
            out_e < in_e * 0.3,
            "echo energy not reduced: in {in_e} out {out_e}"
        );
    }
}
