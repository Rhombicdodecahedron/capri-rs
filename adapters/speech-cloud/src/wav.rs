//! WAV in and out, and raw PCM decoding.

use std::io::Cursor;

use capri_application::ports::{Audio, SpeechError};

/// 16-bit mono WAV, what every transcription API accepts.
pub fn encode(audio: &Audio) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.rate.max(1),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::new());
    if let Ok(mut w) = hound::WavWriter::new(&mut out, spec) {
        for s in &audio.samples {
            let _ = w.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
        }
        let _ = w.finalize();
    }
    out.into_inner()
}

/// Any PCM WAV to mono f32.
pub fn decode(bytes: &[u8]) -> Result<Audio, SpeechError> {
    let mut r = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| SpeechError(format!("not a WAV answer: {e}")))?;
    let spec = r.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            r.samples::<i32>().map(|s| s.unwrap_or(0) as f32 / scale).collect()
        }
    };
    let c = usize::from(spec.channels.max(1));
    let samples = if c == 1 { raw } else { raw.chunks(c).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect() };
    Ok(Audio { rate: spec.sample_rate, samples })
}

/// Little-endian signed 16-bit PCM to f32.
pub fn pcm16_le(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip() {
        let a = Audio { rate: 16_000, samples: vec![0.0, 0.5, -0.5, 0.25] };
        let back = decode(&encode(&a)).unwrap();
        assert_eq!(back.rate, 16_000);
        assert_eq!(back.samples.len(), 4);
        assert!((back.samples[1] - 0.5).abs() < 0.001);
    }

    #[test]
    fn raw_pcm() {
        assert_eq!(pcm16_le(&[0x00, 0x40, 0x00, 0xC0]), vec![0.5, -0.5]);
    }
}
