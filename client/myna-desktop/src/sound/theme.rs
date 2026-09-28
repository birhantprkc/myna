//! The [`Chime`] that plays the desktop sound theme (freedesktop Sound Theme
//! and Naming specifications).
//!
//! The theme the user picked lives in the host's `org.gnome.desktop.sound`,
//! which a confined daemon cannot read, so the search is Ubuntu's default,
//! Yaru, then freedesktop, the theme every other inherits from. The
//! directories are `$XDG_DATA_HOME/sounds` and each `$XDG_DATA_DIRS/sounds`;
//! in the snap, `sound-themes` mounts the themes under `$SNAP/data-dir`,
//! which the daemon's environment adds to `XDG_DATA_DIRS`.
//!
//! Playing happens on a thread of its own, one cue at a time: a cue takes as
//! long as its sound, and the controller must not wait for it. A cue that
//! cannot be found or played is logged and dropped; dictation never depends
//! on it.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use myna_audio::playback::{self, Clip};

use super::{Chime, Cue};

pub const THEMES: [&str; 2] = ["Yaru", "freedesktop"];

/// Cues waiting behind the one playing; more than a session's worth is a
/// burst, and the excess is dropped.
const BACKLOG: usize = 3;

/// The sound directories the environment names, most specific first.
pub fn sound_dirs(
    data_home: Option<&Path>,
    home: Option<&Path>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let home_dir = data_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".local/share")));
    let system = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    home_dir
        .into_iter()
        .chain(
            system
                .split(':')
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
        )
        .map(|dir| dir.join("sounds"))
        .collect()
}

/// The file a theme search finds for `event_id`: each theme in order, and
/// within one the first directory holding it.
pub fn find(event_id: &str, themes: &[&str], dirs: &[PathBuf]) -> Option<PathBuf> {
    themes.iter().find_map(|theme| {
        dirs.iter().find_map(|dir| {
            ["oga", "ogg"]
                .iter()
                .map(|ext| {
                    dir.join(theme)
                        .join("stereo")
                        .join(format!("{event_id}.{ext}"))
                })
                .find(|path| path.is_file())
        })
    })
}

pub struct ThemeChime {
    cues: mpsc::SyncSender<Cue>,
}

impl ThemeChime {
    /// Start the player thread, searching the directories this process's
    /// environment names and playing through PipeWire.
    pub fn spawn() -> std::io::Result<Self> {
        let dirs = sound_dirs(
            std::env::var_os("XDG_DATA_HOME").as_deref().map(Path::new),
            std::env::var_os("HOME").as_deref().map(Path::new),
            std::env::var("XDG_DATA_DIRS").ok().as_deref(),
        );
        Self::spawn_with(dirs, |clip| playback::play(clip).map_err(|e| e.to_string()))
    }

    fn spawn_with(
        dirs: Vec<PathBuf>,
        mut output: impl FnMut(&Clip) -> Result<(), String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (cues, queue) = mpsc::sync_channel::<Cue>(BACKLOG);
        std::thread::Builder::new()
            .name("myna-sound".into())
            .spawn(move || {
                for cue in queue {
                    if let Err(why) = load(cue, &dirs).and_then(|clip| output(&clip)) {
                        myna_core::info_log!("sound", "{cue:?} cue not played: {why}");
                    }
                }
            })?;
        Ok(Self { cues })
    }
}

impl Chime for ThemeChime {
    fn play(&self, cue: Cue) {
        let _ = self.cues.try_send(cue);
    }
}

fn load(cue: Cue, dirs: &[PathBuf]) -> Result<Clip, String> {
    let path = find(cue.event_id(), &THEMES, dirs)
        .ok_or_else(|| format!("no {} sound in {THEMES:?} under {dirs:?}", cue.event_id()))?;
    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Clip::decode_ogg(std::io::BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("myna-sounds-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, theme: &str, file: &str) -> PathBuf {
        let path = dir.join(theme).join("stereo").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn the_first_theme_that_has_the_sound_wins_over_directory_order() {
        let root = scratch("order");
        let (user, system) = (root.join("user"), root.join("system"));
        put(&user, "freedesktop", "dialog-error.oga");
        let yaru = put(&system, "Yaru", "dialog-error.oga");
        let dirs = [user.clone(), system.clone()];

        assert_eq!(find("dialog-error", &THEMES, &dirs), Some(yaru));
        let only_freedesktop = put(&user, "freedesktop", "device-added.oga");
        assert_eq!(find("device-added", &THEMES, &dirs), Some(only_freedesktop));
        assert_eq!(find("device-removed", &THEMES, &dirs), None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_ogg_extension_is_found_too_and_a_directory_is_not_a_sound() {
        let root = scratch("ext");
        let ogg = put(&root, "freedesktop", "device-removed.ogg");
        std::fs::create_dir_all(root.join("Yaru/stereo/device-removed.oga")).unwrap();
        assert_eq!(
            find("device-removed", &THEMES, std::slice::from_ref(&root)),
            Some(ogg)
        );
        std::fs::remove_dir_all(&root).ok();
    }

    const CUE: &[u8] = include_bytes!("../../../myna-audio/tests/sounds/cue.oga");

    /// Each cue reaches the output decoded from its own theme file; a cue
    /// with no file, or one that does not decode, is dropped without
    /// stopping the ones after it.
    #[test]
    fn the_player_plays_what_the_theme_has_and_skips_the_rest() {
        let root = scratch("player");
        std::fs::write(put(&root, "Yaru", "device-added.oga"), CUE).unwrap();
        std::fs::write(put(&root, "freedesktop", "dialog-error.oga"), b"not vorbis").unwrap();
        let (played, heard) = mpsc::channel();
        let chime = ThemeChime::spawn_with(vec![root.clone()], move |clip| {
            played.send(clip.duration()).unwrap();
            Ok(())
        })
        .unwrap();

        for cue in [Cue::Error, Cue::Stop, Cue::Start] {
            chime.play(cue);
        }
        let duration = heard
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the start cue plays");
        assert_eq!(duration.as_millis(), 250);
        assert!(heard
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_missing_sound_names_what_was_looked_for() {
        let why = load(Cue::Stop, &[PathBuf::from("/nonexistent")]).unwrap_err();
        assert!(why.contains("device-removed"), "{why}");
    }

    #[test]
    fn the_directories_follow_the_base_directory_spec() {
        assert_eq!(
            sound_dirs(
                Some(Path::new("/data")),
                Some(Path::new("/home/u")),
                Some("/snap/myna/x1/usr/share:/snap/myna/x1/data-dir:relative")
            ),
            [
                "/data/sounds",
                "/snap/myna/x1/usr/share/sounds",
                "/snap/myna/x1/data-dir/sounds"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            sound_dirs(
                Some(Path::new("relative")),
                Some(Path::new("/home/u")),
                Some("")
            ),
            [
                "/home/u/.local/share/sounds",
                "/usr/local/share/sounds",
                "/usr/share/sounds"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            sound_dirs(None, None, None),
            ["/usr/local/share/sounds", "/usr/share/sounds"].map(PathBuf::from)
        );
    }
}
