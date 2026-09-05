use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct Track {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub name: String,
}

impl Track {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|err| err.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|err| err.to_string())?;
        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| "No audio track in that file".to_string())?;
        let track_id = track.id;
        let codec_params = track.codec_params.clone();
        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| "Track has no sample rate".to_string())?;
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|err| err.to_string())?;

        let mut samples = Vec::new();
        let mut sample_buf = None;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                    continue;
                }
                Err(err) => return Err(err.to_string()),
            };
            if packet.track_id() != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let capacity = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(capacity, spec));
                    }
                    if let Some(buf) = sample_buf.as_mut() {
                        buf.copy_interleaved_ref(decoded);
                        let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1).max(1);
                        let frames = buf.samples();
                        for frame in frames.chunks(channels) {
                            let sum: f32 = frame.iter().sum();
                            samples.push(sum / channels as f32);
                        }
                    }
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => return Err(err.to_string()),
            }
        }

        if samples.is_empty() {
            return Err("Could not decode any audio from that file".to_string());
        }

        Ok(Self {
            samples,
            sample_rate,
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("track")
                .to_string(),
        })
    }

    pub fn sample_at(&self, frame: usize, play_rate: u32) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        if self.sample_rate == play_rate {
            return self.samples.get(frame).copied().unwrap_or(0.0);
        }
        let pos = frame as f64 * self.sample_rate as f64 / play_rate as f64;
        let i = pos.floor() as usize;
        let frac = (pos - i as f64) as f32;
        let a = self.samples.get(i).copied().unwrap_or(0.0);
        let b = self.samples.get(i + 1).copied().unwrap_or(0.0);
        a + (b - a) * frac
    }

    pub fn frame_count(&self, play_rate: u32) -> usize {
        if self.sample_rate == play_rate {
            return self.samples.len();
        }
        (self.samples.len() as u64 * play_rate as u64 / self.sample_rate as u64) as usize
    }
}
