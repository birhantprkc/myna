"""`myna-bench export --parquet`: the AIB-shaped Parquet files.

The files are the interface AIB reads, so the tests read them back with
pyarrow and hold every value against the JSONL it came from.
"""

from __future__ import annotations

import builtins
import json
from datetime import datetime
from statistics import fmean

import pytest
from _records import record

from myna.benchmarker import _parquet
from myna.benchmarker._bootstrap import SEED
from myna.benchmarker._parquet import (
    EXPORT_SCHEMA_VERSION,
    cmd_export,
    export,
    unit_id,
    unit_identity,
)

pq = pytest.importorskip("pyarrow.parquet")

LABEL = "myna-parakeet/cpu/tdt/batch"
PROVENANCE = {
    "machine": "zephyrus",
    "cpu": "AMD Ryzen 9",
    "ram_gb": 33.3,
    "gpu": "RTX 4090 Laptop",
    "gpu_vram_gb": 16.0,
    "provision": "snap",
    "hardware": {"cuda": True},
    "engine": "cpu",
    "artifacts": {
        "snap": "myna-parakeet",
        "version": "0.2.0",
        "revision": "x1",
        "components": {"model-tdt": {"version": "1", "revision": "x2"}},
        "files": [{"file": "myna-parakeet.snap", "sha3_384": "ab" * 48, "size": 10}],
    },
    "harness": {"version": "0.2.0", "pyz": "myna-bench.pyz", "pyz_sha256": "cd" * 32},
    "os": {"kernel_release": "6.8.0-40-generic", "cpu_governor": "performance"},
    "gpus": [{"index": 0, "name": "RTX 4090 Laptop", "driver_version": "575.1"}],
    "settings": {"num-threads": "4"},
    "pace": "max",
}
SCHEDULE = {"repeats": 2, "warmup_clips": 1, "seed": 0}


def _at(seconds):
    return f"2026-08-20T00:00:{seconds:02d}+00:00"


def _row(clip, repeat=0, phase="measured", **overrides):
    provenance = {**PROVENANCE, **({} if phase == "cold" else {"schedule": SCHEDULE})}
    fields = {
        "label": LABEL,
        "clip": clip,
        "repeat": repeat,
        "phase": phase,
        "cold": phase == "cold",
        "pace": "max",
        "pace_lag": None,
        "pace_starved": False,
        "provenance": provenance,
        "served_models": ["parakeet-tdt-0.6b-v3@int8"],
        "served_runtime": {"onnxruntime": "1.27.0"},
        "corpus_manifest": "manifest.json",
        "schema_version": 3,
    }
    return record(**{**fields, **overrides})


ROWS = [
    _row("clip-1", phase="cold", time_to_ready=3.5, started_at=_at(0), schema_version=2),
    _row("clip-2", phase="warmup", started_at=_at(5)),
    _row("clip-1", repeat=0, wer_edits=1, ref_words=10, finalize_latency=0.3, started_at=_at(10)),
    _row("clip-2", repeat=0, wer_edits=0, ref_words=5, finalize_latency=0.5, started_at=_at(20)),
    _row("clip-1", repeat=1, wer_edits=2, ref_words=10, finalize_latency=0.4, started_at=_at(30)),
    _row(
        "clip-2",
        repeat=1,
        error={"code": "model_error", "message": "weights missing"},
        transcript="",
        started_at=_at(40),
        time_to_terminal=7.4,
        served_runtime={"onnxruntime": "1.27.1"},
    ),
]
ERROR = ROWS[-1]
STATUS = {"schema_version": 3, "machine": "zephyrus", "label": LABEL, "status": "ok", "reason": ""}
STAMP = {"schema_version": 3, "machine": "zephyrus", "label": LABEL, "snap": "myna-parakeet"}
TRACE_GPU = {
    "sm_mhz": 1500.0,
    "mem_mhz": 8000.0,
    "temp_c": 50.0,
    "power_limit_w": 100.0,
    "thermal_margin_c": None,
    "throttle": 0,
}
TRACE = [
    {
        "t": 0.0,
        "wall": 1.0,
        "cpu_mhz": [3000.0, 3100.0],
        "cpu_temp_c": 55.0,
        "cpu_energy_j": 0.0,
        "cpu_throttle_count": None,
        "rss_mb": 800.5,
        "vram_mb": 1024.0,
        "gpus": [
            {
                "index": 0,
                "sm_mhz": 1500.0,
                "mem_mhz": 8000.0,
                "temp_c": 50.0,
                "power_w": 30.0,
                "util_pct": 40.0,
                "power_limit_w": 100.0,
                "thermal_margin_c": None,
                "throttle": 4,
            }
        ],
    },
    {
        "t": 1.0,
        "wall": 2.0,
        "cpu_mhz": [3200.0, 3300.0],
        "cpu_temp_c": 60.0,
        "cpu_energy_j": 20.0,
        "cpu_throttle_count": None,
        "rss_mb": 900.0,
        "vram_mb": None,
        "gpus": [],
    },
    {
        "t": 2.0,
        "wall": 3.0,
        "cpu_mhz": [3200.0, 3300.0],
        "cpu_temp_c": 58.0,
        "cpu_energy_j": 30.0,
        "cpu_throttle_count": None,
        "rss_mb": 1000.0,
        "vram_mb": 2048.0,
        "gpus": [
            {**TRACE_GPU, "index": 0, "power_w": 50.0, "util_pct": 80.0},
            # Read, but its power sensor was not: the sum over GPUs is unknown.
            {**TRACE_GPU, "index": 1, "power_w": None, "util_pct": 20.0},
        ],
    },
]
CELL = {
    "audio_seconds": 6.0,
    "peak_rss_mb": 1000.0,
    "peak_vram_mb": 2048.0,
    "telemetry_seconds": 1.0,
    "cpu_energy_j": 20.0,
    "gpu_energy_j": None,
    "gpu_energy_j_by_index": None,
    "energy_j": None,
    "energy_sources": [],
    "j_per_audio_s": None,
    "max_cpu_temp_c": 60.0,
    "max_gpu_temp_c": 50.0,
    "throttled": {"cpu": None, "gpu": False},
    "gpu_throttle_reasons": [],
    "gpu_throttle_unverified": [],
    "telemetry_error": None,
}
HEADER = {"type": "machine", "schema_version": 3, "hostname": "zephyrus"}


def _write(tmp_path, rows=ROWS, statuses=(STATUS,), trace=TRACE, cell=CELL):
    results = tmp_path / "results.jsonl"
    results.write_text(
        "\n".join(json.dumps(r) for r in [HEADER, *rows, *statuses]) + "\n", encoding="utf-8"
    )
    if trace is not None:
        lines = [{**STAMP, "kind": "sample", **s} for s in trace]
        lines.append({**STAMP, "kind": "cell", **cell})
        (tmp_path / "results-resources.jsonl").write_text(
            "\n".join(json.dumps(r) for r in lines) + "\n", encoding="utf-8"
        )
    return results


def _export(tmp_path, ci=True, **kwargs):
    ids = export(_write(tmp_path, **kwargs), tmp_path / "out", ci=ci, resamples=200)
    return ids, tmp_path / "out"


def _read(out, uid, table=""):
    return pq.read_table(out / f"{uid}{table}.parquet").to_pylist()


# ─── round trip ──────────────────────────────────────────────────────────────


def test_one_cell_is_one_unit_with_its_three_tables(tmp_path):
    ids, out = _export(tmp_path)
    assert len(ids) == 1
    uid = ids[0]
    assert sorted(p.name for p in out.iterdir()) == sorted(
        [f"{uid}.parquet", f"{uid}.samples.parquet", f"{uid}.load.parquet"]
    )


def test_every_sample_reads_back_as_the_row_it_came_from(tmp_path):
    ids, out = _export(tmp_path)
    samples = _read(out, ids[0], ".samples")
    # Warmup rows are not a score; every other row is a sample, errors included.
    expected = [r for r in ROWS if r["phase"] != "warmup"]
    assert len(samples) == len(expected)
    by_key = {(s["clip"], s["repeat"], s["phase"]): s for s in samples}
    for row in expected:
        got = by_key[(row["clip"], row["repeat"], row["phase"])]
        assert got["unit_id"] == ids[0]
        for column in _parquet.SAMPLE_FIELDS:
            assert got[column] == row.get(column), column
        assert got["request_class"] == row["category"]
        assert got["started_at"] == datetime.fromisoformat(row["started_at"])
        assert got["edits_sub"] == row["edits"]["sub"]
        assert got["edits_del"] == row["edits"]["del"]
        assert got["edits_ins"] == row["edits"]["ins"]
        assert got["ok"] is (row["error"] is None)
        error = row["error"] or {}
        assert got["error"] == error.get("code")
        assert got["error_message"] == error.get("message")


def test_the_load_table_is_the_telemetry_trace(tmp_path):
    ids, out = _export(tmp_path)
    load = _read(out, ids[0], ".load")
    assert [s["t_offset_s"] for s in load] == [0.0, 1.0, 2.0]
    first, second, third = load
    assert first["ram_bytes"] == 800_500_000  # rss_mb is decimal megabytes
    assert first["vram_bytes"] == 1024 * 2**20  # nvidia-smi reports MiB
    assert second["vram_bytes"] is None
    assert first["power_w"] == 30.0
    assert second["power_w"] is None  # no GPU read: unknown, not 0 W
    assert first["gpu_pct"] == 40.0
    assert first["cpu_pct"] is None  # not sampled
    assert first["cpu_mhz"] == [3000.0, 3100.0]
    assert first["cpu_energy_j"] == 0.0
    assert first["cpu_temp_c"] == 55.0
    assert first["gpus"] == TRACE[0]["gpus"]
    assert second["gpus"] == []
    assert third["power_w"] is None  # GPU 1 went unread: unknown, not 50 W
    assert third["gpu_pct"] == 50.0
    assert third["gpus"] == TRACE[2]["gpus"]


def test_the_summary_holds_the_cells_aggregates_with_intervals(tmp_path):
    ids, out = _export(tmp_path)
    (summary,) = _read(out, ids[0])
    assert summary["schema_version"] == EXPORT_SCHEMA_VERSION
    assert summary["unit_id"] == ids[0]
    assert summary["status"] == "completed"
    assert summary["myna_status"] == "ok"
    assert summary["label"] == LABEL
    assert summary["engine_kind"] == "cpu"
    assert summary["model_repo_id"] == "parakeet-tdt-0.6b-v3@int8"
    assert summary["model_revision"] == "x1"
    assert summary["corpus_id"] == "v1:testcorpus"
    assert summary["repeats"] == 2
    assert summary["seed"] == 0
    assert summary["warmup_clips"] == 1
    assert summary["results_schema_version"] == 3  # the newest row's
    # First session open to the last terminal event, the errored run's.
    assert summary["started_at"] == datetime.fromisoformat(_at(0))
    assert summary["ended_at"] == datetime.fromisoformat("2026-08-20T00:00:47.400+00:00")
    assert summary["duration_s"] == 47
    assert json.loads(summary["served_runtime"]) == ERROR["served_runtime"]  # last run's
    assert summary["served_models"] == ["parakeet-tdt-0.6b-v3@int8"]
    # 3 measured rows scored (the errored one is not): 3 edits / 25 words.
    assert summary["clips"] == 3
    assert summary["wer"] == pytest.approx(3 / 25)
    assert summary["wer_ci_low"] <= summary["wer"] <= summary["wer_ci_high"]
    assert (summary["ci_resamples"], summary["ci_seed"]) == (200, SEED)
    assert summary["finalize_s_p50"] == pytest.approx(0.4)
    assert summary["finalize_s_p95"] is None  # below the sample floor
    assert summary["finalize_s_p95_ci_low"] is None
    assert summary["cold_ready_s"] == 3.5
    assert summary["requests_total"] == 5
    assert summary["requests_ok"] == 4
    assert summary["requests_error"] == 1
    assert summary["ram_peak_bytes"] == 1_000_000_000
    assert summary["ram_mean_bytes"] == round(fmean([800.5, 900.0, 1000.0]) * 1_000_000)
    assert summary["vram_peak_bytes"] == 2048 * 2**20
    assert summary["vram_mean_bytes"] == 1536 * 2**20  # the unread sample left out
    assert summary["gpu_load_mean_pct"] == pytest.approx(fmean([40.0, 80.0, 20.0]))
    assert summary["energy_joules"] is None
    assert summary["cpu_energy_j"] == 20.0
    assert summary["throttled_gpu"] is False
    assert summary["env_hostname"] == "zephyrus"
    assert summary["env_gpu_count"] == 1
    assert summary["env_driver"] == "575.1"
    assert summary["env_ram_total_bytes"] == 33_300_000_000
    assert json.loads(summary["settings"]) == {"num-threads": "4"}
    assert json.loads(summary["environment"])["os"] == PROVENANCE["os"]
    assert summary["campaign_id"] == _parquet.campaign_id(ids)


def test_without_intervals_the_interval_columns_are_null(tmp_path):
    """--no-ci exists so export runs where numpy does not."""
    ids, out = _export(tmp_path, ci=False)
    (summary,) = _read(out, ids[0])
    assert summary["wer"] == pytest.approx(3 / 25)
    for column, _ in _parquet.INTERVALS:
        assert summary[f"{column}_ci_low"] is None, column
        assert summary[f"{column}_ci_high"] is None, column
    assert summary["ci_resamples"] is None
    assert summary["ci_seed"] is None


def test_rows_are_ordered_by_when_they_ran_not_where_they_sit_in_the_file(tmp_path):
    ids, out = _export(tmp_path, rows=ROWS[::-1])
    (summary,) = _read(out, ids[0])
    assert json.loads(summary["served_runtime"]) == ERROR["served_runtime"]
    assert summary["started_at"] == datetime.fromisoformat(_at(0))
    starts = [s["started_at"] for s in _read(out, ids[0], ".samples")]
    assert starts == sorted(starts)


def test_a_rerun_that_errors_does_not_displace_the_scored_run(tmp_path):
    """summarize scores the last success; export must score the same rows."""
    failed = {**ROWS[2], "error": {"code": "model_error", "message": "oom"}}
    ids, out = _export(tmp_path, rows=[*ROWS, {**failed, "started_at": _at(50)}])
    (summary,) = _read(out, ids[0])
    assert summary["wer"] == pytest.approx(3 / 25)
    assert summary["clips"] == 3
    assert (summary["requests_total"], summary["requests_error"]) == (6, 2)
    samples = _read(out, ids[0], ".samples")
    runs = [s for s in samples if (s["clip"], s["repeat"], s["phase"]) == ("clip-1", 0, "measured")]
    assert [s["ok"] for s in runs] == [True, False]


def test_a_cell_where_every_run_errored_exports_no_scores(tmp_path):
    ids, out = _export(tmp_path, rows=[ERROR])
    (summary,) = _read(out, ids[0])
    assert (summary["requests_total"], summary["requests_error"]) == (1, 1)
    assert summary["clips"] == 0
    assert summary["wer"] is None
    assert summary["wer_ci_low"] is None
    assert summary["ci_seed"] is None


def test_a_rerun_that_succeeds_clears_the_earlier_error(tmp_path):
    fixed = {**ERROR, "error": None, "transcript": "hello world", "started_at": _at(50)}
    ids, out = _export(tmp_path, rows=[*ROWS, fixed])
    (summary,) = _read(out, ids[0])
    assert (summary["requests_total"], summary["requests_error"]) == (5, 0)
    assert summary["clips"] == 4


def test_a_cell_with_no_status_record_is_not_reported_complete(tmp_path):
    ids, out = _export(tmp_path, statuses=())
    (summary,) = _read(out, ids[0])
    assert summary["status"] == "incomplete"
    assert summary["myna_status"] is None


@pytest.mark.parametrize(
    ("status", "aib"),
    [("usability_fail", "incomplete"), ("broken", "failed"), ("ok", "completed")],
)
def test_statuses_map_onto_aibs(tmp_path, status, aib):
    ids, out = _export(tmp_path, statuses=({**STATUS, "status": status, "reason": "why"},))
    (summary,) = _read(out, ids[0])
    assert summary["status"] == aib
    assert summary["myna_status"] == status


def test_a_cell_without_a_trace_has_an_empty_load_table(tmp_path):
    ids, out = _export(tmp_path, trace=None)
    assert _read(out, ids[0], ".load") == []
    (summary,) = _read(out, ids[0])
    assert summary["ram_peak_bytes"] is None


def test_the_last_trace_of_a_label_is_the_one_exported(tmp_path):
    results = _write(tmp_path)
    resources = tmp_path / "results-resources.jsonl"
    rerun = [{**STAMP, "kind": "sample", **TRACE[1], "t": 0.0}, {**STAMP, "kind": "cell", **CELL}]
    with resources.open("a", encoding="utf-8") as fp:
        fp.writelines(json.dumps(r) + "\n" for r in rerun)
    (uid,) = export(results, tmp_path / "out", resamples=200)
    assert [s["t_offset_s"] for s in _read(tmp_path / "out", uid, ".load")] == [0.0]


def test_a_trace_cut_short_is_exported_without_a_verdict(tmp_path):
    results = _write(tmp_path)
    resources = tmp_path / "results-resources.jsonl"
    with resources.open("a", encoding="utf-8") as fp:
        fp.write("\n" + json.dumps({**STAMP, "kind": "sample", **TRACE[0]}) + "\n")
    (uid,) = export(results, tmp_path / "out", resamples=200)
    assert [s["t_offset_s"] for s in _read(tmp_path / "out", uid, ".load")] == [0.0]
    (summary,) = _read(tmp_path / "out", uid)
    assert summary["ram_peak_bytes"] is None  # no verdict row
    assert summary["ram_mean_bytes"] == 800_500_000


def test_a_cell_that_left_no_rows_is_named_not_exported(tmp_path, capsys):
    broken = {**STATUS, "label": "myna-whisper/cpu/batch", "status": "broken"}
    ids = export(_write(tmp_path, statuses=(STATUS, broken)), tmp_path / "out", resamples=200)
    assert len(ids) == 1
    assert "myna-whisper/cpu/batch on zephyrus" in capsys.readouterr().out


# ─── unit identity ───────────────────────────────────────────────────────────


def test_the_unit_id_is_stable_for_the_same_cell(tmp_path):
    first, _ = _export(tmp_path)
    again = export(_write(tmp_path), tmp_path / "again", resamples=200)
    assert first == again
    assert len(first[0]) == 16


@pytest.mark.parametrize(
    "change",
    [
        {"settings": {"num-threads": "8"}},
        {"engine": "cuda"},
        {"artifacts": {**PROVENANCE["artifacts"], "revision": "x9"}},
        {"gpus": [{"index": 0, "name": "RTX 4090 Laptop", "driver_version": "580.0"}]},
        {"harness": {"version": "0.3.0", "pyz": None, "pyz_sha256": None}},
    ],
)
def test_the_unit_id_moves_with_what_ran_and_where(change):
    base = _row("clip-1")
    changed = {**base, "provenance": {**base["provenance"], **change}}
    assert unit_id(unit_identity([base])) != unit_id(unit_identity([changed]))


@pytest.mark.parametrize(
    "change",
    [
        {"corpus_id": "v1:other"},
        {"normalizer_version": 2},
        {"secondary_normalizer_version": "whisper-v2"},
        {"streaming_strategy": "streaming"},
        {"pace": "realtime"},
        {"served_models": ["other"]},
    ],
)
def test_the_unit_id_moves_with_the_scoring_and_the_feed(change):
    base = _row("clip-1")
    assert unit_id(unit_identity([base])) != unit_id(unit_identity([{**base, **change}]))


def test_the_schedule_is_part_of_the_unit_but_cold_rows_do_not_carry_it():
    cold, warm = _row("clip-1", phase="cold"), _row("clip-1")
    identity = unit_identity([cold, warm])
    assert identity["schedule"] == SCHEDULE
    other = {**warm, "provenance": {**warm["provenance"], "schedule": {**SCHEDULE, "seed": 1}}}
    assert unit_id(identity) != unit_id(unit_identity([cold, other]))


def test_a_label_mixing_two_runs_is_refused(tmp_path):
    moved = {**PROVENANCE["artifacts"], "revision": "x9"}
    rows = [*ROWS, _row("clip-3", provenance={**PROVENANCE, "artifacts": moved})]
    with pytest.raises(SystemExit, match="more than one run"):
        export(_write(tmp_path, rows=rows), tmp_path / "out", resamples=200)


def test_rows_without_provenance_still_export(tmp_path):
    rows = [record(label="bench/socket", clip="a"), record(label="bench/socket", clip="b")]
    ids = export(_write(tmp_path, rows=rows, statuses=(), trace=None), tmp_path / "out")
    (summary,) = _read(tmp_path / "out", ids[0])
    assert summary["env_hostname"] == "unknown"
    assert summary["engine_kind"] is None


# ─── CLI ─────────────────────────────────────────────────────────────────────


def test_cmd_export_writes_and_reports(tmp_path, capsys):
    import argparse

    results = _write(tmp_path)
    out = tmp_path / "out"
    cmd_export(argparse.Namespace(infile=str(results), parquet=str(out), ci=False))
    printed = capsys.readouterr().out
    assert "1 unit(s)" in printed
    assert str(out) in printed
    (uid,) = (p.name.removesuffix(".samples.parquet") for p in out.glob("*.samples.parquet"))
    (summary,) = _read(out, uid)
    assert summary["ci_seed"] is None  # --no-ci reached the export


def test_export_without_pyarrow_says_how_to_get_it(tmp_path, monkeypatch):
    real_import = builtins.__import__

    def no_pyarrow(name, *args, **kwargs):
        if name.startswith("pyarrow"):
            raise ImportError("No module named 'pyarrow'")
        return real_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", no_pyarrow)
    with pytest.raises(SystemExit, match="pip install pyarrow in a venv"):
        export(_write(tmp_path), tmp_path / "out")


def test_an_empty_results_file_is_refused(tmp_path):
    results = tmp_path / "results.jsonl"
    results.write_text(json.dumps(HEADER) + "\n", encoding="utf-8")
    with pytest.raises(SystemExit, match="no clip records"):
        export(results, tmp_path / "out")
