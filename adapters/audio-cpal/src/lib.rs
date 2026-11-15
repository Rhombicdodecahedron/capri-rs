//! The sound card. cpal streams are not `Send` on every platform, so each one lives on its own thread
//! and is driven through shared buffers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use capri_application::ports::{Audio, Microphone, Speaker, SpeechError};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, StreamConfig};

mod resample;

pub use resample::{resample, to_mono};

/// Speech recognisers want 16 kHz mono.
pub const SPEECH_RATE: u32 = 16_000;

fn err(e: impl std::fmt::Display) -> SpeechError {
    SpeechError(e.to_string())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Push-to-talk recording from the default input device.
#[derive(Default)]
pub struct CpalMicrophone {
    recording: Mutex<Option<Recording>>,
}

impl std::fmt::Debug for CpalMicrophone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CpalMicrophone")
    }
}

struct Recording {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<Result<Audio, SpeechError>>,
}

impl Microphone for CpalMicrophone {
    fn start(&self) -> Result<(), SpeechError> {
        let mut slot = lock(&self.recording);
        if slot.is_some() {
            return Ok(());
        }
        let (stop, stopped) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || record(&stopped, &ready_tx));
        // wait until the stream really runs, so the first words are not lost
        match ready_rx.recv() {
            Ok(Ok(())) => {
                *slot = Some(Recording { stop, thread });
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(err("the microphone thread stopped")),
        }
    }

    fn stop(&self) -> Result<Audio, SpeechError> {
        let rec = lock(&self.recording).take().ok_or_else(|| err("not recording"))?;
        let _ = rec.stop.send(());
        rec.thread.join().map_err(|_| err("the microphone thread panicked"))?
    }
}

/// A running input stream, its sample buffer, its rate and its channel count.
type Capture = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32, u16);

fn record(stopped: &mpsc::Receiver<()>, ready: &mpsc::Sender<Result<(), SpeechError>>) -> Result<Audio, SpeechError> {
    let setup = || -> Result<Capture, SpeechError> {
        let device = cpal::default_host().default_input_device().ok_or_else(|| err("no microphone found"))?;
        let supported = device.default_input_config().map_err(err)?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let stream = match format {
            SampleFormat::F32 => input::<f32>(&device, &config, buf.clone()),
            SampleFormat::I16 => input::<i16>(&device, &config, buf.clone()),
            SampleFormat::U16 => input::<u16>(&device, &config, buf.clone()),
            SampleFormat::I32 => input::<i32>(&device, &config, buf.clone()),
            other => return Err(err(format!("unsupported microphone format {other:?}"))),
        }?;
        stream.play().map_err(err)?;
        Ok((stream, buf, config.sample_rate.0, config.channels))
    };
    let (stream, buf, rate, channels) = match setup() {
        Ok(s) => {
            let _ = ready.send(Ok(()));
            s
        }
        Err(e) => {
            let _ = ready.send(Err(SpeechError(e.0.clone())));
            return Err(e);
        }
    };
    let _ = stopped.recv();
    drop(stream);
    let raw = std::mem::take(&mut *lock(&buf));
    let mono = to_mono(&raw, channels);
    Ok(Audio { rate: SPEECH_RATE, samples: resample(&mono, rate, SPEECH_RATE) })
}

fn input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, SpeechError>
where
    T: SizedSample,
    f32: cpal::FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                lock(&buf).extend(data.iter().map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s)));
            },
            |e| tracing::warn!(error = %e, "microphone stream error"),
            None,
        )
        .map_err(err)
}

/// Her voice: a queue of samples played on the default output device by a long-lived stream.
pub struct CpalSpeaker {
    queue: Arc<Mutex<VecDeque<f32>>>,
    volume: Arc<Mutex<f32>>,
    device_rate: Mutex<Option<u32>>,
    alive: Arc<AtomicBool>,
}

impl std::fmt::Debug for CpalSpeaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalSpeaker").field("queued", &lock(&self.queue).len()).finish_non_exhaustive()
    }
}

impl CpalSpeaker {
    pub fn new(volume: f32) -> Self {
        Self {
            queue: Arc::default(),
            volume: Arc::new(Mutex::new(volume.clamp(0.0, 2.0))),
            device_rate: Mutex::new(None),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        *lock(&self.volume) = volume.clamp(0.0, 2.0);
    }

    /// Opens the output stream on first use; returns the device's sample rate.
    fn rate(&self) -> Result<u32, SpeechError> {
        let mut rate = lock(&self.device_rate);
        if let Some(r) = *rate {
            return Ok(r);
        }
        let (tx, rx) = mpsc::channel();
        let (queue, volume, alive) = (self.queue.clone(), self.volume.clone(), self.alive.clone());
        std::thread::spawn(move || {
            let opened = open_output(queue, volume);
            match opened {
                Ok((stream, r)) => {
                    let _ = tx.send(Ok(r));
                    while alive.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
        let r = rx.recv().map_err(|_| err("the speaker thread stopped"))??;
        *rate = Some(r);
        Ok(r)
    }
}

impl Drop for CpalSpeaker {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

fn open_output(queue: Arc<Mutex<VecDeque<f32>>>, volume: Arc<Mutex<f32>>) -> Result<(cpal::Stream, u32), SpeechError> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| err("no speakers found"))?;
    let supported = device.default_output_config().map_err(err)?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => output::<f32>(&device, &config, queue, volume),
        SampleFormat::I16 => output::<i16>(&device, &config, queue, volume),
        SampleFormat::U16 => output::<u16>(&device, &config, queue, volume),
        SampleFormat::I32 => output::<i32>(&device, &config, queue, volume),
        other => return Err(err(format!("unsupported speaker format {other:?}"))),
    }?;
    stream.play().map_err(err)?;
    Ok((stream, config.sample_rate.0))
}

fn output<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    volume: Arc<Mutex<f32>>,
) -> Result<cpal::Stream, SpeechError>
where
    T: SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let gain = *lock(&volume);
                let mut q = lock(&queue);
                for frame in data.chunks_mut(channels) {
                    let s = q.pop_front().unwrap_or(0.0) * gain;
                    let v = T::from_sample_(s.clamp(-1.0, 1.0));
                    for out in frame.iter_mut() {
                        *out = v;
                    }
                }
            },
            |e| tracing::warn!(error = %e, "speaker stream error"),
            None,
        )
        .map_err(err)
}

impl Speaker for CpalSpeaker {
    fn play(&self, audio: Audio) -> Result<(), SpeechError> {
        let rate = self.rate()?;
        let samples = resample(&audio.samples, audio.rate, rate);
        lock(&self.queue).extend(samples);
        Ok(())
    }

    fn stop(&self) {
        lock(&self.queue).clear();
    }

    fn is_playing(&self) -> bool {
        !lock(&self.queue).is_empty()
    }
}
