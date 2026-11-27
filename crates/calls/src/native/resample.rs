//! Streaming mono resampler: windowed sinc through a fixed kernel table.

/// Zero crossings kept on each side of the sinc kernel.
const LOBES: usize = 8;
/// Kernel table entries per unit of normalised distance.
const TABLE_RES: usize = 128;

/// Streaming mono f32 resampler between two fixed rates.
pub struct Resampler {
    step: f64,
    cutoff: f64,
    /// Kernel half width in input samples.
    half: usize,
    table: Vec<f32>,
    buf: Vec<f32>,
    pos: f64,
    /// Equal rates: copy through, no filter and no delay.
    passthrough: bool,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let cutoff = (f64::from(out_rate) / f64::from(in_rate)).min(1.0);
        let half = (LOBES as f64 / cutoff).ceil() as usize;
        let len = LOBES * TABLE_RES + 2;
        let table = (0..len)
            .map(|i| {
                let u = i as f64 / TABLE_RES as f64;
                let sinc = if u == 0.0 {
                    1.0
                } else {
                    (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u)
                };
                let t = (u / LOBES as f64).min(1.0);
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * t).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * t).cos();
                (sinc * w) as f32
            })
            .collect();
        Self {
            step: f64::from(in_rate) / f64::from(out_rate),
            cutoff,
            half,
            table,
            // Zeros before the first sample so early outputs are defined;
            // room for a large device callback so the audio thread does not
            // grow it.
            buf: {
                let mut b = Vec::with_capacity(half + 8192);
                b.resize(half, 0.0);
                b
            },
            pos: half as f64,
            passthrough: in_rate == out_rate,
        }
    }

    fn kernel(&self, x: f64) -> f32 {
        let u = (x * self.cutoff).abs() * TABLE_RES as f64;
        let i = u as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let f = (u - i as f64) as f32;
        (self.table[i] * (1.0 - f) + self.table[i + 1] * f) * self.cutoff as f32
    }

    /// Appends the resampled `input` to `out`. Output sample `j` is aligned
    /// with input sample `j`; the last `half` input samples stay pending
    /// until more arrive.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        loop {
            let centre = self.pos.floor() as usize;
            if centre + self.half >= self.buf.len() {
                break;
            }
            let first = centre + 1 - self.half;
            let mut acc = 0.0f32;
            for (k, s) in self.buf[first..=centre + self.half].iter().enumerate() {
                acc += s * self.kernel(self.pos - (first + k) as f64);
            }
            out.push(acc);
            self.pos += self.step;
        }
        let keep_from = (self.pos.floor() as usize + 1).saturating_sub(self.half);
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    fn round_trip_error(rate: u32, hz: f32) -> f32 {
        let input = sine(rate, hz, rate as usize / 2);
        let mut up = Resampler::new(rate, 48_000);
        let mut down = Resampler::new(48_000, rate);
        let mut mid = Vec::new();
        for chunk in input.chunks(173) {
            up.process(chunk, &mut mid);
        }
        let mut back = Vec::new();
        for chunk in mid.chunks(311) {
            down.process(chunk, &mut back);
        }
        // Find the delay, then compare the steady-state middle.
        let mut best = f32::MAX;
        for lag in 0..400usize {
            let n = 4000;
            let start = 2000;
            if start + lag + n > back.len() {
                break;
            }
            let err: f32 = (0..n)
                .map(|i| (back[start + lag + i] - input[start + i]).powi(2))
                .sum::<f32>()
                / n as f32;
            best = best.min(err);
        }
        best.sqrt()
    }

    #[test]
    fn round_trip_44100_is_close() {
        assert!(round_trip_error(44_100, 1_000.0) < 0.01);
    }

    #[test]
    fn round_trip_16000_is_close() {
        assert!(round_trip_error(16_000, 440.0) < 0.01);
    }

    #[test]
    fn output_length_matches_ratio() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        r.process(&vec![0.0; 44_100], &mut out);
        let expected = 48_000i64;
        assert!((out.len() as i64 - expected).abs() < 40, "{}", out.len());
    }

    #[test]
    fn equal_rates_copy_exactly_without_delay() {
        let input = sine(48_000, 300.0, 500);
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn same_rate_is_passthrough() {
        let input = sine(48_000, 300.0, 4800);
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&input, &mut out);
        for i in 200..1000 {
            assert!((out[i] - input[i]).abs() < 1e-3);
        }
    }
}
