//! What Settings calls each model family Myna knows, keyed by backend snap.

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
    match snap_name {
        "myna-parakeet" => known(
            "Parakeet",
            gettextrs::gettext("Fastest, good support for European languages"),
        ),
        "myna-whisper" => known("Whisper", gettextrs::gettext("Widest language support")),
        "myna-funasr" => known(
            "FunASR",
            gettextrs::gettext("Good support for English, Chinese, Japanese and Korean"),
        ),
        _ => ModelFamily {
            name: title_from_snap(snap_name),
            description: None,
        },
    }
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
}
