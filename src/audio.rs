use rodio::source::Source;
use rodio::{OutputStream, OutputStreamHandle, Sink};
use std::sync::Arc;
use std::time::Duration;

/// Brown noise generator source
pub struct BrownNoiseSource {
    sample_rate: u32,
    last_value: f32,
}

impl BrownNoiseSource {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            last_value: 0.0,
        }
    }
}

impl Iterator for BrownNoiseSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Generate white noise
        let white: f32 = rand::random::<f32>() * 2.0 - 1.0;

        // Integrate (accumulate) to get brown noise
        // Scale factor controls the "brownness" - smaller = smoother
        self.last_value += white * 0.02;

        // Soft clamp to prevent runaway
        self.last_value = self.last_value.clamp(-1.0, 1.0);

        // Apply slight decay to keep it centered around zero
        self.last_value *= 0.999;

        Some(self.last_value * 0.3) // Volume scaling
    }
}

impl Source for BrownNoiseSource {
    fn current_frame_len(&self) -> Option<usize> {
        None // Infinite source
    }

    fn channels(&self) -> u16 {
        1 // Mono
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None // Infinite
    }
}

/// Audio player handle that manages the output stream and sink
pub struct AudioPlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Arc<Sink>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        // Add brown noise source
        let source = BrownNoiseSource::new(44100);
        sink.append(source);

        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            sink: Arc::new(sink),
        })
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }
}
