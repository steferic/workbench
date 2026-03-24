use rodio::source::Source;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::BufReader;
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

/// Looping audio player for ambient sounds (cross-platform via rodio).
/// Replaces ffplay subprocess approach.
pub struct LoopingAudio {
    player: Option<LoopingPlayer>,
    was_playing: bool,
    wav_path: &'static str,
}

struct LoopingPlayer {
    _stream: OutputStream,
    sink: Sink,
}

impl LoopingAudio {
    pub fn new(wav_path: &'static str) -> Self {
        Self {
            player: None,
            was_playing: false,
            wav_path,
        }
    }

    /// Sync the player with the desired play state.
    pub fn sync(&mut self, should_play: bool) {
        if should_play == self.was_playing {
            return;
        }
        if should_play {
            if self.player.is_none() {
                self.player = Self::create_player(self.wav_path);
            }
            if let Some(ref player) = self.player {
                player.sink.play();
            }
        } else if let Some(ref player) = self.player {
            player.sink.pause();
        }
        self.was_playing = should_play;
    }

    /// Kill the player (for shutdown).
    pub fn kill(&mut self) {
        if let Some(player) = self.player.take() {
            player.sink.stop();
        }
    }

    fn create_player(path: &str) -> Option<LoopingPlayer> {
        let (stream, stream_handle) = OutputStream::try_default().ok()?;
        let sink = Sink::try_new(&stream_handle).ok()?;
        let file = std::fs::File::open(path).ok()?;
        let source = Decoder::new(BufReader::new(file)).ok()?;
        let channels = source.channels();
        let sample_rate = source.sample_rate();
        let samples: Vec<i16> = source.collect();
        let buffer = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
        sink.append(buffer.repeat_infinite());
        Some(LoopingPlayer {
            _stream: stream,
            sink,
        })
    }
}

/// Play a one-shot sound file (cross-platform, non-blocking).
/// Spawns a thread so it doesn't block the main loop.
pub fn play_sound(path: &'static str) {
    std::thread::spawn(move || {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let reader = BufReader::new(file);
        let source = match Decoder::new(reader) {
            Ok(s) => s,
            Err(_) => return,
        };
        // OutputStream must live until playback completes
        if let Ok((_stream, handle)) = OutputStream::try_default() {
            if let Ok(sink) = Sink::try_new(&handle) {
                sink.append(source);
                sink.sleep_until_end();
            }
        }
    });
}
