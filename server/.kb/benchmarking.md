# Preface

Read this document when running, extending, or interpreting Myna benchmarks. Benchmark runs install and purge snaps and must not be executed on a machine whose Myna installation must be preserved.

Read the top-level `.kb/agents.md` file before continuing below.

# Overview

`myna.benchmarker` is the only benchmark implementation. `make bench-*` uses the same standalone `myna-bench.pyz` artifact distributed to external test machines, so local and remote measurements have the same semantics.

# Important

- Run `make bench-check` and `make bench-plan` before a sweep.
- `make bench-run` uses sudo, installs snap artifacts, and removes them with purge.
- Use real generated corpora for accuracy. Synthetic fixtures test plumbing and latency only.
- Compare rows only when their `corpus_id` values match.
- Treat `USABILITY_FAIL` as a product result, not a transient test failure.
- Explicit engine requests must fail rather than silently fall back.
- `myna.testbed.metrics.normalize` casefolds and strips punctuation
  (NFKC + casefold, drop non-word/non-apostrophe chars, keep intra-word
  apostrophes) - matching NVIDIA's FLEURS-card convention of
  punctuation-and-case removal only, not Whisper's `EnglishTextNormalizer`/
  `BasicTextNormalizer` (decided 2026-09-17: fold apostrophes only, no style
  switch). Typographic apostrophes (U+2019, U+2018, U+02BC) fold to ASCII `'`
  before that, because FLEURS French references use U+2019 for elisions and
  an ASCII hypothesis otherwise splits "l'accident" into two words against
  them. `normalizer_version` is stamped on every row; `one_normalizer_version`
  refuses a file whose rows were scored under different versions, same as
  `one_corpus` does for `corpus_id`.
- Full FLEURS test set, Parakeet v3 fp32 CUDA, 2026-09-17 (de 862 / es 908 /
  fr 676 clips): the apostrophe fold moved fr 7.78 -> 5.46 WER (published
  5.15), de 5.16 -> 5.14 (published 5.04), es unchanged at 3.62 (published
  3.45) - all within bootstrap CI of NVIDIA's published numbers. The bundled
  40-clip FLEURS subset's bootstrap CI half-width is ~2-4 WER points, so a
  point estimate on it is not comparable to a published number; use the full
  test set for that comparison.

# Architecture

The local workflow is:

```shell
make bench-check
make bench-plan
make bench-corpus
make bench-run-whisper
make bench-aggregate
```

For another machine, build `myna-bench.pyz` with `make build-bench`, copy it with the selected `.snap` and `.comp` artifacts and a configuration based on `dev/bench.yaml.example`, then run:

```shell
python3 myna-bench.pyz download-corpus --out ./corpus \
    --select balanced -n 80 --manifest-name manifest-balanced.json \
    --long-form-minutes 5
python3 myna-bench.pyz check --sweep
python3 myna-bench.pyz plan --config bench.yaml
sudo python3 myna-bench.pyz run --config bench.yaml
python3 myna-bench.pyz summarize --in results.jsonl --by-category
```

Merge returned submissions with:

```shell
make bench-merge SUBMISSIONS="incoming/a.jsonl incoming/b.jsonl"
```

Rows are identified by machine and label. Re-running a machine replaces its rows. Labels describe snap, engine, model, mode, and optional config; `provenance.settings` records the effective setting values.

Every row (clip, status and `*-resources.jsonl`) and the machine header carry `schema_version` (absent means 1, the unstamped schema); readers treat a missing field as unknown, never as an error. From schema 2 each sweep row's `provenance` also names the engine read back, the installed artifacts (file sha3-384 and size, snap version and revision, installed components), the harness (the `dev/version.sh` string `build-bench` bakes into the pyz, and the pyz's sha256), the OS state (kernel cmdline, microcode, governor, boost, SMT, snapd) and every NVIDIA GPU (driver, CUDA driver version, persistence, clocks, ECC). `served_runtime` is the server's own report of its inference libraries and execution provider, from capabilities re-read after the clips ran: the server names a library version only once a model load has imported it. Rows repeat all of this because `merge` keeps rows and drops headers.

`run` and `bench` also write `<out>-events.jsonl.gz`: one line per clip run, keyed like its row by (label, clip, repeat, phase), holding every server event and the audio-feed schedule (chunk send time, audio position reached), all timed from the first chunk sent; `audio_start` converts back to the row's session-open origin. It exists so a latency metric invented later is computed from data on disk, not a rerun; `summarize` and `merge` ignore it and `_events.load_events` reads it. About 195 KB per 83-clip balanced-corpus cell (streaming parakeet, 2026-09-29), so it is always on.

Each cell runs `warmup_clips` first, then `repeats` full passes over the clips (config keys, global with per-target overrides; defaults 1 and 0, so a tester's run is one pass in manifest order). Rows carry `repeat` (0..N-1) and `phase` (`cold`, `warmup` or `measured`); `summarize` drops warmup rows and dedups by (label, clip, repeat, phase), and a row without the fields reads as repeat 0 in the phase its `cold` flag names. With more than one repeat, each pass is shuffled by `random.Random(f"{seed}/{repeat}")` (SHA-512 string seeding, so the order survives a new interpreter) and `provenance.schedule` records repeats, warmup and seed: drift then spreads across clips instead of biasing one. `sweep_budget_seconds` is per pass, so a cell's deadline is budget x repeats; warmup runs outside it, like the cold sample. A rerun with `--keep-results` and fewer repeats leaves the earlier run's higher repeats in the file, so rerun without it when the schedule changes.

WER and CER are micro-averaged. Speed is audio duration divided by decode time. Final latency measures end-of-audio to committed text; cold load measures session open to ready. Incomplete rows sort behind successful rows because partial metrics are not comparable.
