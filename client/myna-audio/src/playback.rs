//! One-shot playback of a short sound through PipeWire: the dictation cues.
//!
//! A [`Clip`] is decoded whole, up front, from Ogg Vorbis (what sound themes
//! ship). [`play`] then opens its own loop and output stream, hands the graph
//! the clip at its own rate and layout (PipeWire converts), and returns once
//! the stream reports it drained. The caller owns the thread: playing blocks
//! for the clip's length.

use std::cell::RefCell;
use std::fmt;
use std::io::{Read, Seek};
use std::rc::Rc;
use std::time::Duration;

use pipewire::{
    context::ContextRc,
    keys,
    main_loop::MainLoopRc,
    properties::properties,
    spa::{
        param::{
            audio::{AudioFormat, AudioInfoRaw},
            ParamType,
        },
        pod::{serialize::PodSerializer, Object, Pod, Value},
        utils::{Direction, SpaTypes},
    },
    stream::{Stream, StreamFlags, StreamRc, StreamState},
};

/// Longer than any cue a theme means by its event sounds; the rest of a
/// longer file is dropped rather than played over the user's dictation.
pub const MAX_CLIP: Duration = Duration::from_secs(3);

/// How long past the clip's own length the graph may take to play it before
/// [`play`] gives up.
const SLACK: Duration = Duration::from_secs(2);

const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackError(String);

impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlaybackError {}

/// Decoded audio: interleaved `f32` frames, `channels` wide.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    rate: u32,
    channels: u32,
    samples: Vec<f32>,
}

impl Clip {
    /// Decode an Ogg Vorbis stream, keeping at most [`MAX_CLIP`] of it.
    pub fn decode_ogg(source: impl Read + Seek) -> Result<Self, PlaybackError> {
        let mut reader = lewton::inside_ogg::OggStreamReader::new(source)
            .map_err(|e| PlaybackError(format!("not an Ogg Vorbis stream: {e}")))?;
        let rate = reader.ident_hdr.audio_sample_rate;
        let channels = u32::from(reader.ident_hdr.audio_channels);
        let limit = (u64::from(rate) * MAX_CLIP.as_secs() * u64::from(channels)) as usize;
        let mut samples = Vec::new();
        while samples.len() < limit {
            match reader.read_dec_packet_itl() {
                Ok(Some(packet)) => samples.extend(
                    packet
                        .into_iter()
                        .map(|sample| f32::from(sample) / f32::from(i16::MAX)),
                ),
                Ok(None) => {
                    // The last page's granule position is the true length;
                    // the final packet decodes to a whole block past it.
                    if let Some(frames) = reader.get_last_absgp() {
                        samples.truncate((frames * u64::from(channels)) as usize);
                    }
                    break;
                }
                Err(e) => return Err(PlaybackError(format!("undecodable Vorbis packet: {e}"))),
            }
        }
        samples.truncate(limit);
        if samples.is_empty() {
            return Err(PlaybackError("the stream holds no samples".into()));
        }
        Ok(Self {
            rate,
            channels,
            samples,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    pub fn duration(&self) -> Duration {
        let frames = self.samples.len() as u64 / u64::from(self.channels);
        Duration::from_micros(frames * 1_000_000 / u64::from(self.rate))
    }

    fn frame_bytes(&self) -> usize {
        SAMPLE_BYTES * self.channels as usize
    }

    /// Copy the next frames into `out`, whole frames only, returning how many
    /// bytes were written.
    fn fill(&self, cursor: &mut usize, out: &mut [u8]) -> usize {
        let room = out.len() / self.frame_bytes() * self.channels as usize;
        let take = room.min(self.samples.len() - *cursor);
        for (slot, sample) in out
            .chunks_exact_mut(SAMPLE_BYTES)
            .zip(&self.samples[*cursor..*cursor + take])
        {
            slot.copy_from_slice(&sample.to_le_bytes());
        }
        *cursor += take;
        take * SAMPLE_BYTES
    }
}

/// Play `clip` on the default output and block until PipeWire has played it.
pub fn play(clip: &Clip) -> Result<(), PlaybackError> {
    play_on(clip, None)
}

/// [`play`] against the daemon named `remote` rather than the default one.
pub fn play_on(clip: &Clip, remote: Option<&str>) -> Result<(), PlaybackError> {
    let main_loop =
        MainLoopRc::new(None).map_err(|e| PlaybackError(format!("no PipeWire loop: {e}")))?;
    let context = ContextRc::new(&main_loop, None)
        .map_err(|e| PlaybackError(format!("no PipeWire context: {e}")))?;
    let core = match remote {
        Some(name) => {
            context.connect_rc(Some(properties! { *keys::REMOTE_NAME => name }.to_owned()))
        }
        None => context.connect_rc(None),
    }
    .map_err(|e| PlaybackError(format!("cannot connect to PipeWire: {e}")))?;

    let outcome: Rc<RefCell<Option<Result<(), PlaybackError>>>> = Rc::default();
    let finish = {
        let main_loop = main_loop.clone();
        let outcome = outcome.clone();
        move |result: Result<(), PlaybackError>| {
            outcome.borrow_mut().get_or_insert(result);
            main_loop.quit();
        }
    };

    let deadline = main_loop.loop_().add_timer({
        let finish = finish.clone();
        move |_| {
            finish(Err(PlaybackError(
                "the graph never played the sound".into(),
            )))
        }
    });
    let _ = deadline
        .update_timer(Some(clip.duration() + SLACK), None)
        .into_result();

    let props = properties! {
        *keys::MEDIA_TYPE => "Audio",
        *keys::MEDIA_CATEGORY => "Playback",
        *keys::MEDIA_ROLE => "Notification",
        *keys::NODE_NAME => "myna-cue",
    };
    let stream = StreamRc::new(core, "myna-cue", props)
        .map_err(|e| PlaybackError(format!("cannot create the output stream: {e}")))?;

    let listener = stream
        .add_local_listener_with_user_data(0usize)
        .state_changed({
            let finish = finish.clone();
            move |_stream, _cursor, _old, new| {
                if let StreamState::Error(message) = new {
                    finish(Err(PlaybackError(format!(
                        "output stream error: {message}"
                    ))));
                }
            }
        })
        .process({
            let clip = clip.clone();
            let mut flushed = false;
            move |stream: &Stream, cursor: &mut usize| {
                if flushed {
                    return;
                }
                if let Some(mut buffer) = stream.dequeue_buffer() {
                    if let Some(data) = buffer.datas_mut().first_mut() {
                        let written = data.data().map_or(0, |out| clip.fill(cursor, out));
                        let chunk = data.chunk_mut();
                        *chunk.offset_mut() = 0;
                        *chunk.stride_mut() = clip.frame_bytes() as i32;
                        *chunk.size_mut() = written as u32;
                    }
                }
                if *cursor == clip.samples.len() {
                    flushed = true;
                    let _ = stream.flush(true);
                }
            }
        })
        .drained({
            let finish = finish.clone();
            move |_stream, _cursor| finish(Ok(()))
        })
        .register()
        .map_err(|e| PlaybackError(format!("cannot listen to the output stream: {e}")))?;

    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(AudioFormat::F32LE);
    audio_info.set_rate(clip.rate);
    audio_info.set_channels(clip.channels);
    if let Some(position) = crate::native::channel_positions(None, clip.channels) {
        audio_info.set_position(position);
    }
    let format = PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: SpaTypes::ObjectParamFormat.as_raw(),
            id: ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        }),
    )
    .expect("serializing audio format pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&format).expect("valid format pod")];

    stream
        .connect(
            Direction::Output,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| PlaybackError(format!("cannot connect the output stream: {e}")))?;

    main_loop.run();

    let _ = stream.disconnect();
    drop(listener);
    drop(deadline);
    let result = outcome.borrow_mut().take();
    result.unwrap_or_else(|| Err(PlaybackError("playback ended without an outcome".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tests/sounds/*.oga` are ffmpeg sine sources encoded with libvorbis:
    /// cue 880 Hz, 0.25 s, 44.1 kHz stereo; long 440 Hz, 4 s, 8 kHz stereo.
    fn fixture() -> Clip {
        Clip::decode_ogg(std::io::Cursor::new(
            &include_bytes!("../tests/sounds/cue.oga")[..],
        ))
        .expect("the fixture decodes")
    }

    #[test]
    fn a_theme_sound_decodes_to_its_rate_layout_and_length() {
        let clip = fixture();
        assert_eq!((clip.rate(), clip.channels()), (44_100, 2));
        let ms = clip.duration().as_millis();
        assert!((240..=260).contains(&ms), "{ms} ms");
        // ffmpeg's sine source peaks at 1/8 of full scale.
        let peak = clip.samples.iter().fold(0f32, |peak, s| peak.max(s.abs()));
        assert!((0.08..0.2).contains(&peak), "peak {peak}");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for bytes in [&b""[..], b"OggS but not really", b"RIFF....WAVE"] {
            let error = Clip::decode_ogg(std::io::Cursor::new(bytes)).unwrap_err();
            assert!(
                error.to_string().starts_with("not an Ogg Vorbis stream"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_long_file_is_cut_to_the_cap() {
        let clip = Clip::decode_ogg(std::io::Cursor::new(
            &include_bytes!("../tests/sounds/long.oga")[..],
        ))
        .expect("the fixture decodes");
        assert_eq!((clip.rate(), clip.channels()), (8_000, 2));
        assert_eq!(clip.duration(), MAX_CLIP);
    }

    #[test]
    fn fill_writes_whole_frames_and_ends_exactly_at_the_clip() {
        let clip = Clip {
            rate: 8,
            channels: 2,
            samples: (0..10).map(|n| n as f32).collect(),
        };
        let mut cursor = 0;
        let mut out = [0u8; 12];
        assert_eq!(clip.fill(&mut cursor, &mut out), 8);
        assert_eq!(cursor, 2);
        assert_eq!(f32::from_le_bytes(out[4..8].try_into().unwrap()), 1.0);
        let mut big = [0u8; 64];
        assert_eq!(clip.fill(&mut cursor, &mut big), 32);
        assert_eq!(cursor, 10);
        assert_eq!(clip.fill(&mut cursor, &mut big), 0);
    }
}
