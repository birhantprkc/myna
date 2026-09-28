//! What Settings calls each model family Myna knows, keyed by backend snap,
//! and which installed one it recommends for the user's language.

use myna_core::language::{recommend_among, ModelFamily as Family};

use crate::domain::BackendIdentity;

/// Families the store publishes, so the user can get them without
/// sideloading.
pub const PUBLISHED: [Family; 2] = [Family::Parakeet, Family::Whisper];

/// A backend as the model list shows it: a name and, for a family Myna
/// knows, a one-line description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFamily {
    pub name: String,
    pub description: Option<String>,
}

pub fn model_family(snap_name: &str) -> ModelFamily {
    let known = |name: &str, description: String| ModelFamily {
        name: name.to_owned(),
        description: Some(description),
    };
    match Family::from_snap_name(snap_name) {
        Some(Family::Parakeet) => known(
            "Parakeet",
            gettextrs::gettext("Fastest, good support for European languages"),
        ),
        Some(Family::Whisper) => known("Whisper", gettextrs::gettext("Widest language support")),
        Some(Family::FunAsr) => known(
            "FunASR",
            gettextrs::gettext("Good support for English, Chinese, Japanese and Korean"),
        ),
        None => ModelFamily {
            name: title_from_snap(snap_name),
            description: None,
        },
    }
}

/// The family to recommend for `languages` among those installed or
/// published, whether or not it is installed yet.
pub fn recommended_family(backends: &[BackendIdentity], languages: &[String]) -> Option<Family> {
    let mut candidates = PUBLISHED.to_vec();
    for backend in backends {
        if let Some(family) = Family::from_snap_name(backend.snap_name()) {
            if !candidates.contains(&family) {
                candidates.push(family);
            }
        }
    }
    recommend_among(languages, &candidates)
}

/// Whether `backend` belongs to the `recommended` family; a backend of no
/// known family never does.
pub fn is_recommended(backend: &BackendIdentity, recommended: Option<Family>) -> bool {
    recommended.is_some_and(|family| Family::from_snap_name(backend.snap_name()) == Some(family))
}

/// `backends` with the recommended family's first, the rest in order.
pub fn recommended_first(
    backends: &[BackendIdentity],
    recommended: Option<Family>,
) -> Vec<BackendIdentity> {
    let (first, rest): (Vec<&BackendIdentity>, Vec<&BackendIdentity>) = backends
        .iter()
        .partition(|backend| is_recommended(backend, recommended));
    first.into_iter().chain(rest).cloned().collect()
}

/// `myna-fake-backend` reads as "Fake Backend".
fn title_from_snap(snap_name: &str) -> String {
    let stripped = snap_name.strip_prefix("myna-").unwrap_or(snap_name);
    let mut title = String::with_capacity(stripped.len());
    let mut capitalize = true;
    for ch in stripped.chars() {
        if ch == '-' || ch == '_' {
            title.push(' ');
            capitalize = true;
        } else if capitalize {
            title.extend(ch.to_uppercase());
            capitalize = false;
        } else {
            title.push(ch);
        }
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn described(snap: &str) -> (String, Option<String>) {
        let family = model_family(snap);
        (family.name, family.description)
    }

    #[test]
    fn known_families_have_a_name_and_a_description() {
        assert_eq!(
            described("myna-parakeet"),
            (
                "Parakeet".to_owned(),
                Some("Fastest, good support for European languages".to_owned())
            )
        );
        assert_eq!(
            described("myna-whisper"),
            (
                "Whisper".to_owned(),
                Some("Widest language support".to_owned())
            )
        );
        assert_eq!(
            described("myna-funasr"),
            (
                "FunASR".to_owned(),
                Some("Good support for English, Chinese, Japanese and Korean".to_owned())
            )
        );
    }

    #[test]
    fn an_unknown_backend_is_named_after_its_snap_without_a_description() {
        assert_eq!(
            described("myna-fake-backend"),
            ("Fake Backend".to_owned(), None)
        );
        assert_eq!(
            described("myna-whisper-small"),
            ("Whisper Small".to_owned(), None)
        );
        assert_eq!(described("other_asr"), ("Other Asr".to_owned(), None));
        assert_eq!(described("myna-éclair"), ("Éclair".to_owned(), None));
    }

    fn installed(snaps: &[&str]) -> Vec<BackendIdentity> {
        snaps
            .iter()
            .map(|snap| BackendIdentity::new(*snap, "provider"))
            .collect()
    }

    fn recommended(snaps: &[&str], language: &str) -> Option<Family> {
        recommended_family(&installed(snaps), &[language.to_owned()])
    }

    #[test]
    fn the_recommendation_follows_the_language_among_installed_models() {
        let both = ["myna-parakeet", "myna-whisper"];
        assert_eq!(recommended(&both, "en_US"), Some(Family::Parakeet));
        assert_eq!(recommended(&both, "de"), Some(Family::Parakeet));
        assert_eq!(recommended(&both, "zh_CN"), Some(Family::Whisper));
        assert_eq!(recommended(&both, "cy_GB"), Some(Family::Whisper));
    }

    #[test]
    fn a_sideloaded_family_competes_but_an_unpublished_one_does_not() {
        assert_eq!(
            recommended(&["myna-funasr", "myna-whisper"], "zh_CN"),
            Some(Family::FunAsr)
        );
        assert_eq!(
            recommended(&["myna-parakeet"], "ja_JP"),
            Some(Family::Whisper)
        );
    }

    #[test]
    fn a_published_family_is_recommended_before_it_is_installed() {
        assert_eq!(
            recommended(&["myna-whisper"], "fr_FR"),
            Some(Family::Parakeet)
        );
        assert_eq!(
            recommended(&["myna-fake-backend"], "pt_BR"),
            Some(Family::Parakeet)
        );
        assert_eq!(recommended(&[], "ko"), Some(Family::Whisper));
    }

    fn snaps(backends: &[BackendIdentity]) -> Vec<&str> {
        backends.iter().map(BackendIdentity::snap_name).collect()
    }

    #[test]
    fn the_recommended_model_sorts_first_and_the_rest_keep_their_order() {
        let backends = installed(&["myna-fake-backend", "myna-parakeet", "myna-whisper"]);
        assert_eq!(
            snaps(&recommended_first(&backends, Some(Family::Whisper))),
            ["myna-whisper", "myna-fake-backend", "myna-parakeet"]
        );
        assert_eq!(
            snaps(&recommended_first(&backends, Some(Family::Parakeet))),
            ["myna-parakeet", "myna-fake-backend", "myna-whisper"]
        );
        assert_eq!(
            snaps(&recommended_first(&backends, Some(Family::FunAsr))),
            ["myna-fake-backend", "myna-parakeet", "myna-whisper"]
        );
        assert_eq!(
            snaps(&recommended_first(&backends, None)),
            ["myna-fake-backend", "myna-parakeet", "myna-whisper"]
        );
        let unknown_later = installed(&["myna-parakeet", "myna-fake-backend", "myna-whisper"]);
        assert_eq!(
            snaps(&recommended_first(&unknown_later, None)),
            ["myna-parakeet", "myna-fake-backend", "myna-whisper"],
            "no recommendation leaves the order alone"
        );
    }

    #[test]
    fn a_backend_of_no_known_family_is_never_recommended() {
        let fake = BackendIdentity::new("myna-fake-backend", "provider");
        let parakeet = BackendIdentity::new("myna-parakeet", "provider");
        assert!(!is_recommended(&fake, None));
        assert!(!is_recommended(&fake, Some(Family::Parakeet)));
        assert!(!is_recommended(&parakeet, None));
        assert!(!is_recommended(&parakeet, Some(Family::Whisper)));
        assert!(is_recommended(&parakeet, Some(Family::Parakeet)));
    }
}
