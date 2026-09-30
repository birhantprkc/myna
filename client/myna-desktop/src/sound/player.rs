//! The [`Chime`] that plays Myna's own cues, compiled into the daemon from
//! `sounds/` (made by `dev/synth_cues.py`).
//!
//! Playing happens on a thread of its own, one cue at a time: a cue takes as
//! long as its sound, and the controller must not wait for it. A cue that
//! cannot be played is logged and dropped; dictation never depends on it.

use std::sync::mpsc;

use myna_audio::playback::{self, Clip};

use super::{Chime, Cue};

/// Cues waiting behind the one playing; more than a session's worth is a
/// burst, and the excess is dropped.
const BACKLOG: usize = 3;

/// The Ogg Vorbis file each cue plays.
fn sound(cue: Cue) -> &'static [u8] {
    match cue {
        Cue::Start => include_bytes!("../../sounds/start.oga"),
        Cue::Stop => include_bytes!("../../sounds/stop.oga"),
        Cue::Error => include_bytes!("../../sounds/error.oga"),
    }
}

fn decode(cue: Cue) -> Result<Clip, String> {
    Clip::decode_ogg(std::io::Cursor::new(sound(cue))).map_err(|e| e.to_string())
}

pub struct Player {
    cues: mpsc::SyncSender<Cue>,
}

impl Player {
    /// Start the player thread, playing through PipeWire.
    pub fn spawn() -> std::io::Result<Self> {
        Self::spawn_with(|clip| playback::play(clip).map_err(|e| e.to_string()))
    }

    fn spawn_with(
        mut output: impl FnMut(&Clip) -> Result<(), String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (cues, queue) = mpsc::sync_channel::<Cue>(BACKLOG);
        std::thread::Builder::new()
            .name("myna-sound".into())
            .spawn(move || {
                for cue in queue {
                    if let Err(why) = decode(cue).and_then(|clip| output(&clip)) {
                        myna_core::info_log!("sound", "{cue:?} cue not played: {why}");
                    }
                }
            })?;
        Ok(Self { cues })
    }
}

impl Chime for Player {
    fn play(&self, cue: Cue) {
        let _ = self.cues.try_send(cue);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const CUES: [Cue; 3] = [Cue::Start, Cue::Stop, Cue::Error];

    /// Every cue decodes, is short, and starts and ends in near silence: a
    /// clip that begins or stops above zero clicks.
    #[test]
    fn each_cue_is_a_short_clip_that_fades_in_and_out() {
        for cue in CUES {
            let clip = decode(cue).unwrap_or_else(|e| panic!("{cue:?}: {e}"));
            let ms = clip.duration().as_millis();
            assert!((300..=600).contains(&ms), "{cue:?} lasts {ms} ms");
            let edge = (clip.rate() / 1000 * clip.channels()) as usize;
            let samples = clip.samples();
            let loudest = |part: &[f32]| part.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(loudest(&samples[..edge]) < 0.01, "{cue:?} starts loud");
            assert!(
                loudest(&samples[samples.len() - edge..]) < 0.001,
                "{cue:?} ends loud"
            );
            assert!(loudest(samples) > 0.3, "{cue:?} is too quiet");
        }
    }

    #[test]
    fn the_cues_are_three_different_sounds() {
        assert_ne!(sound(Cue::Start), sound(Cue::Stop));
        assert_ne!(sound(Cue::Start), sound(Cue::Error));
        assert_ne!(sound(Cue::Stop), sound(Cue::Error));
    }

    /// Each cue reaches the output as its own clip, in order, and one that
    /// fails to play does not stop the ones after it.
    #[test]
    fn the_player_plays_each_cue_in_turn_past_a_failure() {
        let (played, heard) = mpsc::channel();
        let mut first = true;
        let chime = Player::spawn_with(move |clip| {
            played.send(clip.duration()).unwrap();
            if std::mem::take(&mut first) {
                return Err("no sink".into());
            }
            Ok(())
        })
        .unwrap();

        for cue in CUES {
            chime.play(cue);
        }
        let got: Vec<_> = (0..3)
            .map(|_| {
                heard
                    .recv_timeout(Duration::from_secs(5))
                    .expect("a cue plays")
            })
            .collect();
        let want: Vec<_> = CUES.map(|cue| decode(cue).unwrap().duration()).into();
        assert_eq!(got, want);
    }
}
