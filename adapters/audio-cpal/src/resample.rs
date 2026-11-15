//! Small, dependency-free audio conversions: good enough for speech.

/// Averages interleaved channels into one.
pub fn to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let c = usize::from(channels.max(1));
    if c == 1 {
        return interleaved.to_vec();
    }
    interleaved.chunks(c).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect()
}

/// Linear-interpolation resampling.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let k = pos.floor() as usize;
            let frac = (pos - k as f64) as f32;
            let a = samples[k];
            let b = *samples.get(k + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_to_mono() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }

    #[test]
    fn resamples_length_and_keeps_shape() {
        let s: Vec<f32> = (0..480).map(|i| i as f32 / 480.0).collect();
        let r = resample(&s, 48_000, 16_000);
        assert_eq!(r.len(), 160);
        assert!((r[80] - 0.5).abs() < 0.01);
        assert_eq!(resample(&s, 16_000, 16_000).len(), 480);
    }
}
