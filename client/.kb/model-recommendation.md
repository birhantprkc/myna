# Preface

Read this before changing which model family Myna recommends, the language table behind it, or how the user's language is read.

Read the top-level `.kb/agents.md` file before continuing below.

# Overview

`myna_core::language` holds a table of published error rates per (language, family): Parakeet's FLEURS WER for its 25 languages, and SenseVoice-Small (the FunASR snap) against Whisper-small on Common Voice for zh, yue, ja, ko and en. The lowest rate among the candidate families wins; Whisper covers every language the table does not measure. `myna_core::locale` turns the session into an ordered list of `ll` or `ll_CC` languages behind the `LocaleSource` port.

Sources, checked 2026-09-28:

- Parakeet: the `nvidia/parakeet-tdt-0.6b-v3` model card, FLEURS WER, its full 25-language list.
- FunASR and Whisper: the SenseVoice paper (arXiv 2407.04051) Table 6, SenseVoice-Small against Whisper-small on Common Voice (CER for zh, yue, ja and ko, WER for en).

Caveats the table does not show:

- myna-funasr ships SenseVoice-Small, not Fun-ASR-Nano. The released Small checkpoint covers only zh, en, yue, ja and ko; the "50+ languages" on its card belong to the unreleased Large.
- zh_TW goes to FunASR although SenseVoice writes Simplified script. Whether Traditional-script users are better served by Whisper with a language hint is untested.
- The Whisper figures are for small, the GPU default; CPU users get tiny, which is worse, so the table flatters Whisper for them.

# Important

- Only the most preferred language decides. Later `LANGUAGE` entries are fallbacks the user settled for: `cy:en` is a Welsh speaker, not a Parakeet user.
- Rates are ordinal and cross-dataset even within a language (en sets Parakeet's FLEURS against Common Voice figures). Use them only to order families for one language; never trust the margin or compare across languages.
- Read the language from `g_get_language_names()` (the list that also picks the UI translation), never from the `LC_*` format categories, which carry the Formats region. AccountsService `User.Language` is read only when that list has no language (`LANG=C`), and English is assumed after that.
- Recommend among families the user can actually get: `recommend_among` takes the candidates, since a family that is not published (myna-funasr at the time of writing) must not be offered.
