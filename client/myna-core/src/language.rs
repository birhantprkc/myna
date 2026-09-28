//! Which model family transcribes a language best, from the published model
//! cards, and the recommendation Settings shows for the user's language.

use crate::locale::language_code;

/// A backend family Myna knows, whatever its snap currently ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    Parakeet,
    Whisper,
    FunAsr,
}

impl ModelFamily {
    /// Every family, in the order Settings lists them.
    pub const ALL: [ModelFamily; 3] = [Self::Parakeet, Self::Whisper, Self::FunAsr];

    pub fn snap_name(self) -> &'static str {
        match self {
            Self::Parakeet => "myna-parakeet",
            Self::FunAsr => "myna-funasr",
            Self::Whisper => "myna-whisper",
        }
    }

    pub fn from_snap_name(snap_name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.snap_name() == snap_name)
    }
}

/// A published error rate (WER or CER, percent) of one family on one
/// language. Rates are ordinal and cross-dataset even within a language, so
/// they only order families; the margin means nothing.
struct Measured {
    language: &'static str,
    family: ModelFamily,
    error_rate: f32,
}

const fn measured(language: &'static str, family: ModelFamily, error_rate: f32) -> Measured {
    Measured {
        language,
        family,
        error_rate,
    }
}

use ModelFamily::{FunAsr, Parakeet, Whisper};

/// Parakeet: nvidia/parakeet-tdt-0.6b-v3 card, FLEURS WER, its full language
/// list. FunASR: SenseVoice-Small against Whisper-small, Common Voice, from
/// the SenseVoice paper (arXiv 2407.04051) table 6. Whisper covers every
/// other language unmeasured.
const TABLE: &[Measured] = &[
    measured("bg", Parakeet, 12.64),
    measured("cs", Parakeet, 11.01),
    measured("da", Parakeet, 18.41),
    measured("de", Parakeet, 5.04),
    measured("el", Parakeet, 20.70),
    measured("en", Parakeet, 4.85),
    measured("es", Parakeet, 3.45),
    measured("et", Parakeet, 17.73),
    measured("fi", Parakeet, 13.21),
    measured("fr", Parakeet, 5.15),
    measured("hr", Parakeet, 12.46),
    measured("hu", Parakeet, 15.72),
    measured("it", Parakeet, 3.00),
    measured("lt", Parakeet, 20.35),
    measured("lv", Parakeet, 22.84),
    measured("mt", Parakeet, 20.46),
    measured("nl", Parakeet, 7.48),
    measured("pl", Parakeet, 7.31),
    measured("pt", Parakeet, 4.76),
    measured("ro", Parakeet, 12.44),
    measured("ru", Parakeet, 5.51),
    measured("sk", Parakeet, 8.82),
    measured("sl", Parakeet, 24.03),
    measured("sv", Parakeet, 15.08),
    measured("uk", Parakeet, 6.79),
    measured("en", FunAsr, 14.71),
    measured("ja", FunAsr, 11.96),
    measured("ko", FunAsr, 8.28),
    measured("yue", FunAsr, 7.09),
    measured("zh", FunAsr, 10.78),
    measured("en", Whisper, 14.85),
    measured("ja", Whisper, 19.51),
    measured("ko", Whisper, 10.48),
    measured("yue", Whisper, 38.97),
    measured("zh", Whisper, 19.60),
];

/// The family to recommend for `preferred` languages, most preferred first,
/// out of every family Myna knows.
pub fn recommend<S: AsRef<str>>(preferred: &[S]) -> ModelFamily {
    recommend_among(preferred, &ModelFamily::ALL).unwrap_or(Whisper)
}

/// The family among `candidates` to recommend for `preferred` languages,
/// or `None` when no candidate can transcribe the most preferred one.
///
/// Only the first language decides: the rest are fallbacks the user settled
/// for, and a Welsh speaker listing English after Welsh still speaks Welsh.
fn recommend_among<S: AsRef<str>>(
    preferred: &[S],
    candidates: &[ModelFamily],
) -> Option<ModelFamily> {
    let language = preferred
        .iter()
        .find_map(|locale| language_code(locale.as_ref()));
    let measured = language.as_deref().and_then(|language| {
        TABLE
            .iter()
            .filter(|entry| entry.language == language && candidates.contains(&entry.family))
            .min_by(|a, b| a.error_rate.total_cmp(&b.error_rate))
            .map(|entry| entry.family)
    });
    measured.or_else(|| candidates.contains(&Whisper).then_some(Whisper))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn european_languages_go_to_parakeet_whatever_the_region() {
        for locale in [
            "pt_BR",
            "pt_PT",
            "de_CH.UTF-8",
            "en_US",
            "en_GB.UTF-8",
            "fr_CA",
            "uk_UA",
        ] {
            assert_eq!(recommend(&[locale]), Parakeet, "{locale}");
        }
    }

    #[test]
    fn chinese_japanese_and_korean_go_to_funasr_whatever_the_region() {
        for locale in [
            "zh_CN.UTF-8",
            "zh_TW",
            "zh_HK",
            "zh_SG",
            "ja_JP",
            "ko_KR.UTF-8",
            "yue",
        ] {
            assert_eq!(recommend(&[locale]), FunAsr, "{locale}");
        }
    }

    #[test]
    fn a_language_no_card_measures_falls_back_to_whisper() {
        for locale in ["sr_RS", "cy_GB", "ar_EG.UTF-8", "hi_IN", "tr_TR", "xx"] {
            assert_eq!(recommend(&[locale]), Whisper, "{locale}");
        }
    }

    #[test]
    fn nothing_to_go_on_falls_back_to_whisper() {
        assert_eq!(recommend::<&str>(&[]), Whisper);
        assert_eq!(recommend(&["C", "POSIX", ""]), Whisper);
    }

    #[test]
    fn only_the_most_preferred_language_decides() {
        assert_eq!(recommend(&["cy_GB", "en_GB", "en"]), Whisper);
        assert_eq!(recommend(&["ja_JP", "en"]), FunAsr);
        assert_eq!(recommend(&["en_GB", "ja"]), Parakeet);
    }

    #[test]
    fn locale_names_that_are_not_languages_are_skipped() {
        assert_eq!(recommend(&["C", "ja_JP"]), FunAsr);
        assert_eq!(recommend(&["POSIX", "C.UTF-8", "de"]), Parakeet);
    }

    #[test]
    fn the_lowest_error_rate_breaks_a_tie_between_candidates() {
        assert_eq!(recommend_among(&["en"], &[Whisper, FunAsr]), Some(FunAsr));
        assert_eq!(recommend_among(&["en"], &[FunAsr, Whisper]), Some(FunAsr));
        assert_eq!(
            recommend_among(&["en_US"], &ModelFamily::ALL),
            Some(Parakeet)
        );
        assert_eq!(recommend_among(&["ko"], &[Whisper, FunAsr]), Some(FunAsr));
    }

    #[test]
    fn without_the_best_family_the_next_measured_one_wins() {
        assert_eq!(recommend_among(&["zh_TW"], &[Whisper]), Some(Whisper));
        assert_eq!(recommend_among(&["de"], &[FunAsr, Whisper]), Some(Whisper));
    }

    #[test]
    fn no_candidate_for_the_language_recommends_nothing() {
        assert_eq!(recommend_among(&["de"], &[FunAsr]), None);
        assert_eq!(recommend_among(&["ja"], &[Parakeet]), None);
        assert_eq!(recommend_among(&["en"], &[]), None);
        assert_eq!(recommend_among::<&str>(&[], &[Parakeet]), None);
        assert_eq!(
            recommend_among::<&str>(&[], &[Parakeet, Whisper]),
            Some(Whisper)
        );
    }

    #[test]
    fn every_measured_language_is_a_bare_lowercase_code() {
        for entry in TABLE {
            assert_eq!(
                language_code(entry.language).as_deref(),
                Some(entry.language)
            );
            assert!(entry.error_rate > 0.0 && entry.error_rate < 100.0);
        }
    }

    #[test]
    fn families_are_named_by_their_snap() {
        for family in ModelFamily::ALL {
            assert_eq!(
                ModelFamily::from_snap_name(family.snap_name()),
                Some(family)
            );
        }
        assert_eq!(ModelFamily::from_snap_name("myna-fake-backend"), None);
        assert_eq!(Parakeet.snap_name(), "myna-parakeet");
        assert_eq!(FunAsr.snap_name(), "myna-funasr");
        assert_eq!(Whisper.snap_name(), "myna-whisper");
    }
}
