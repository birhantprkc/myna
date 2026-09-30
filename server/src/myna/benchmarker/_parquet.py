"""AIB-shaped Parquet export of a results file.

    myna-bench export --parquet out/ --in results.jsonl

ai-inference-benchmark (AIB) reads a unit as three files keyed by ``unit_id``:
``<unit>.parquet`` (one summary row), ``<unit>.samples.parquet`` (one row per
request) and ``<unit>.load.parquet`` (the load time series). Here a unit is a
cell - one (machine, label) row of ``summarize``, every repeat of it - a
sample is one clip run, and the load is the cell's telemetry trace. Columns
take AIB's name where the quantity is the same; the mapping and this file's
own ``schema_version`` are in server/.kb/benchmarking.md.

pyarrow is optional: the pyz does not carry it, and ``export`` says so.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections.abc import Iterable, Sequence
from datetime import datetime, timedelta
from pathlib import Path
from statistics import fmean
from typing import Any

from myna.benchmarker._schedule import COLD, MEASURED
from myna.benchmarker._summarize import (
    Record,
    RowKey,
    SummaryRow,
    _bootstrap,
    _load_latest,
    _summarize,
    clip_samples,
    machine_of,
    resources_path_for,
    row_key,
)

# Version of the Parquet schema below, independent of the JSONL one: bump it
# when a column changes meaning or type, never for an added column.
EXPORT_SCHEMA_VERSION = 1

# AIB's ``hashing.ID_LENGTH``: a unit id is this many hex digits of a sha256.
ID_LENGTH = 16

# The provenance keys that describe the machine and the harness rather than
# the cell. Their digest is part of the unit id: a number measured on another
# box, or by another build of the harness, is another unit.
ENVIRONMENT_KEYS = (
    "cpu",
    "ram_gb",
    "gpu",
    "gpu_vram_gb",
    "provision",
    "hardware",
    "harness",
    "os",
    "gpus",
)

# myna status -> AIB ``RunStatus``. AIB's merge reports a campaign complete
# only when every unit says "completed"; a cell with no status record never
# said it finished, so it is incomplete, not completed.
AIB_STATUS = {"ok": "completed", "usability_fail": "incomplete", "broken": "failed"}
INCOMPLETE = "incomplete"

# Row fields copied into the samples table as they are, with their type.
SAMPLE_FIELDS = {
    "clip": "str",
    "language": "str",
    "cold": "bool",
    "reference": "str",
    "transcript": "str",
    "audio_seconds": "f64",
    "wer": "f64",
    "cer": "f64",
    "wer_edits": "i64",
    "ref_words": "i64",
    "cer_edits": "i64",
    "ref_chars": "i64",
    "wer_whisper_norm": "f64",
    "cer_whisper_norm": "f64",
    "wer_whisper_norm_edits": "i64",
    "ref_words_whisper_norm": "i64",
    "cer_whisper_norm_edits": "i64",
    "ref_chars_whisper_norm": "i64",
    "time_to_first_event": "f64",
    "time_to_ready": "f64",
    "time_to_first_snippet": "f64",
    "time_to_first_final": "f64",
    "time_to_first_committed": "f64",
    "time_to_first_unstable": "f64",
    "time_to_terminal": "f64",
    "finalize_latency": "f64",
    "rtf": "f64",
    "commit_stability": "bool",
    "committed_segments": "i64",
    "pace": "str",
    "pace_lag": "f64",
    "pace_starved": "bool",
    "usability_fail": "bool",
}

SAMPLES_COLUMNS = {
    "unit_id": "str",
    "request_class": "str",
    "phase": "str",
    "repeat": "i32",
    "ok": "bool",
    "error": "str",
    "error_message": "str",
    "started_at": "ts",
    "edits_sub": "i64",
    "edits_del": "i64",
    "edits_ins": "i64",
    **SAMPLE_FIELDS,
}

GPU_READING = {
    "index": "i32",
    "sm_mhz": "f64",
    "mem_mhz": "f64",
    "temp_c": "f64",
    "power_w": "f64",
    "util_pct": "f64",
    "power_limit_w": "f64",
    "thermal_margin_c": "f64",
    "throttle": "i64",
}

LOAD_COLUMNS = {
    "unit_id": "str",
    "t_offset_s": "f64",
    "cpu_pct": "f64",
    "ram_bytes": "i64",
    "gpu_pct": "f64",
    "vram_bytes": "i64",
    "power_w": "f64",
    "cpu_mhz": "list_f64",
    "cpu_temp_c": "f64",
    "cpu_energy_j": "f64",
    "cpu_throttle_count": "i64",
    "gpus": "gpus",
}

# (column, key in ``cell_intervals``) of the metrics with a 95% interval.
INTERVALS = (
    ("wer", "wer"),
    ("cer", "cer"),
    ("wer_whisper", "wer_whisper"),
    ("cer_whisper", "cer_whisper"),
    ("rtfx", "rtfx"),
    ("finalize_s_p50", "p50_final"),
    ("finalize_s_p95", "p95_final"),
)

SUMMARY_COLUMNS = {
    # AIB's columns, where the quantity is the same.
    "schema_version": "i32",
    "unit_id": "str",
    "campaign_id": "str",
    "status": "str",
    "model_repo_id": "str",
    "model_revision": "str",
    "engine_kind": "str",
    "env_hostname": "str",
    "env_gpu_name": "str",
    "env_gpu_count": "i32",
    "env_driver": "str",
    "env_cpu": "str",
    "env_ram_total_bytes": "i64",
    "env_os": "str",
    "started_at": "ts",
    "ended_at": "ts",
    "duration_s": "i32",
    "requests_total": "i64",
    "requests_ok": "i64",
    "requests_error": "i64",
    "ram_peak_bytes": "i64",
    "ram_mean_bytes": "i64",
    "vram_peak_bytes": "i64",
    "vram_mean_bytes": "i64",
    "cpu_load_mean_pct": "f64",
    "gpu_load_mean_pct": "f64",
    "energy_joules": "f64",
    # What the cell was.
    "myna_status": "str",
    "status_reason": "str",
    "label": "str",
    "snap": "str",
    "mode": "str",
    "pace": "str",
    "settings": "str",
    "artifacts": "str",
    "served_models": "list_str",
    "served_runtime": "str",
    "corpus_id": "str",
    "corpus_manifest": "str",
    "normalizer_version": "i32",
    "secondary_normalizer_version": "str",
    "results_schema_version": "i32",
    "repeats": "i32",
    "warmup_clips": "i32",
    "seed": "i64",
    "environment": "str",
    "environment_digest": "str",
    # What it measured, as `summarize` prints it.
    "clips": "i64",
    **{
        name: "f64"
        for column, _ in INTERVALS
        for name in (column, f"{column}_ci_low", f"{column}_ci_high")
    },
    "finalize_s_p99": "f64",
    "timed_clips": "i64",
    "repeat_cv": "f64",
    "rtf_median": "f64",
    "cold_ready_s": "f64",
    "warm_ready_s": "f64",
    "audio_seconds": "f64",
    "starved_clips": "i64",
    "ci_resamples": "i64",
    "ci_seed": "i64",
    # Telemetry verdict.
    "telemetry_seconds": "f64",
    "j_per_audio_s": "f64",
    "cpu_energy_j": "f64",
    "gpu_energy_j": "f64",
    "max_cpu_temp_c": "f64",
    "max_gpu_temp_c": "f64",
    "throttled_cpu": "bool",
    "throttled_gpu": "bool",
    "gpu_throttle_reasons": "list_str",
    "telemetry_error": "str",
}

MB = 1_000_000  # rss_mb: psutil bytes / 1e6
MIB = 1 << 20  # vram_mb: nvidia-smi's MiB


def _arrow() -> tuple[Any, Any]:
    """(pyarrow, pyarrow.parquet), or a way forward where it is missing."""
    try:
        import pyarrow as pa
        import pyarrow.parquet as pq
    except ImportError as exc:
        raise SystemExit(
            f"export --parquet needs pyarrow ({exc}): pip install pyarrow in a venv "
            "and run the pyz with its python, or on Ubuntu 26.04+ sudo apt install "
            "python3-pyarrow"
        ) from None
    return pa, pq


def _schema(pa: Any, columns: dict[str, str]) -> Any:
    types = {
        "str": pa.string(),
        "f64": pa.float64(),
        "i32": pa.int32(),
        "i64": pa.int64(),
        "bool": pa.bool_(),
        "ts": pa.timestamp("us", tz="UTC"),
        "list_str": pa.list_(pa.string()),
        "list_f64": pa.list_(pa.float64()),
    }
    types["gpus"] = pa.list_(pa.struct([(k, types[t]) for k, t in GPU_READING.items()]))
    return pa.schema([(name, types[kind]) for name, kind in columns.items()])


def _canonical(value: object) -> str:
    """AIB's ``canonical_json``: sorted keys, no whitespace, UTF-8."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _hash(value: object) -> str:
    return hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def _provenance(row: Record) -> Record:
    provenance = row.get("provenance")
    return provenance if isinstance(provenance, dict) else {}


def _phase(row: Record) -> str:
    return str(row.get("phase") or (COLD if row.get("cold") else MEASURED))


def environment(row: Record) -> Record:
    """The machine and harness a row was measured on."""
    provenance = _provenance(row)
    return {"machine": machine_of(row), **{key: provenance.get(key) for key in ENVIRONMENT_KEYS}}


def _row_identity(row: Record) -> Record:
    provenance = _provenance(row)
    return {
        "label": row["label"],
        "artifacts": provenance.get("artifacts"),
        "engine": provenance.get("engine"),
        "model": row.get("served_models"),
        "mode": row.get("streaming_strategy"),
        "pace": row.get("pace"),
        "settings": provenance.get("settings"),
        "corpus_id": row.get("corpus_id"),
        "normalizer_version": row.get("normalizer_version"),
        "secondary_normalizer_version": row.get("secondary_normalizer_version"),
        "environment": _hash(environment(row)),
    }


def unit_identity(rows: Sequence[Record]) -> Record:
    """What a cell's rows say ran: artifacts, engine, model, mode, pace,
    settings, corpus, normalisers, environment digest and schedule.

    Every row of a cell must say the same, or the cell blends two runs (a
    partial rerun with ``--keep-results``), and no one id describes it. The
    cold sample carries no schedule, so only the other rows are held to one.
    """
    identities = {_canonical(_row_identity(row)) for row in rows}
    schedules = {
        _canonical(_provenance(row).get("schedule")) for row in rows if _phase(row) != COLD
    }
    if len(identities) > 1 or len(schedules) > 1:
        machine, label = row_key(rows[0])
        raise SystemExit(
            f"{label} on {machine} holds rows from more than one run (what ran, where, or "
            "how it was scored differs between them) - rerun the cell without "
            "--keep-results, or split the file"
        )
    identity: Record = json.loads(identities.pop())
    identity["schedule"] = json.loads(schedules.pop()) if schedules else None
    return identity


def unit_id(identity: Record) -> str:
    """AIB's ``hashing.unit_id``: 16 hex digits of the identity's sha256."""
    return _hash(identity)[:ID_LENGTH]


def campaign_id(unit_ids: Iterable[str]) -> str:
    """AIB's ``hashing.campaign_id``: the hash of the sorted unit ids."""
    return _hash(sorted(unit_ids))[:ID_LENGTH]


def _traces(path: Path) -> dict[RowKey, tuple[list[Record], Record | None]]:
    """Each cell's last telemetry trace and its verdict row, by row key.

    A sweep appends a cell's samples, then its verdict; a rerun appends
    another block, and the last one wins, as rows do. Samples with no
    verdict after them are a trace cut short, kept with no verdict.
    """
    if not path.exists():
        return {}
    done: dict[RowKey, tuple[list[Record], Record | None]] = {}
    pending: dict[RowKey, list[Record]] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip():
            continue
        rec = json.loads(raw)
        key = row_key(rec)
        if rec.get("kind") == "sample":
            pending.setdefault(key, []).append(rec)
        else:
            done[key] = (pending.pop(key, []), rec)
    for key, samples in pending.items():
        done[key] = (samples, None)
    return done


def _scaled(value: object, unit: int) -> int | None:
    return round(value * unit) if isinstance(value, (int, float)) else None


def _all_read(values: list[Any]) -> list[Any] | None:
    """The values, or None when there are none or any went unread."""
    return values if values and all(v is not None for v in values) else None


def _load_row(uid: str, sample: Record) -> Record:
    gpus = sample.get("gpus") or []
    power = _all_read([g.get("power_w") for g in gpus])
    util = _all_read([g.get("util_pct") for g in gpus])
    return {
        "unit_id": uid,
        "t_offset_s": sample.get("t"),
        # Not sampled: the trace follows the served process, not the host.
        "cpu_pct": None,
        "ram_bytes": _scaled(sample.get("rss_mb"), MB),
        "gpu_pct": fmean(util) if util else None,
        "vram_bytes": _scaled(sample.get("vram_mb"), MIB),
        # Every GPU, where AIB reads GPU 0 alone; unknown if any went unread.
        "power_w": sum(power) if power else None,
        "cpu_mhz": sample.get("cpu_mhz"),
        "cpu_temp_c": sample.get("cpu_temp_c"),
        "cpu_energy_j": sample.get("cpu_energy_j"),
        "cpu_throttle_count": sample.get("cpu_throttle_count"),
        "gpus": [{key: g.get(key) for key in GPU_READING} for g in gpus],
    }


def _sample_row(uid: str, row: Record) -> Record:
    error = row.get("error") or {}
    edits = row.get("edits") or {}
    return {
        "unit_id": uid,
        # AIB's name for the class of request a workload mixes; ours is the category.
        "request_class": row.get("category"),
        "phase": _phase(row),
        "repeat": int(row.get("repeat") or 0),
        "ok": not row.get("error"),
        "error": error.get("code"),
        "error_message": error.get("message"),
        "started_at": datetime.fromisoformat(row["started_at"]),
        "edits_sub": edits.get("sub"),
        "edits_del": edits.get("del"),
        "edits_ins": edits.get("ins"),
        **{field: row.get(field) for field in SAMPLE_FIELDS},
    }


def _span(rows: Sequence[Record]) -> tuple[datetime, datetime]:
    """First session open to the last terminal event across the cell."""
    starts = [datetime.fromisoformat(r["started_at"]) for r in rows]
    ends = [
        start + timedelta(seconds=r.get("time_to_terminal") or 0.0)
        for start, r in zip(starts, rows, strict=True)
    ]
    return min(starts), max(ends)


def _json(value: object) -> str | None:
    return None if value is None else _canonical(value)


def _environment_columns(row: Record) -> Record:
    provenance = _provenance(row)
    gpus = provenance.get("gpus")
    gpus = gpus if isinstance(gpus, list) else None
    os_state = provenance.get("os")
    return {
        "env_hostname": machine_of(row),
        "env_gpu_name": provenance.get("gpu"),
        "env_gpu_count": len(gpus) if gpus is not None else None,
        "env_driver": gpus[0].get("driver_version") if gpus else None,
        "env_cpu": provenance.get("cpu"),
        "env_ram_total_bytes": _scaled(provenance.get("ram_gb"), 1_000_000_000),
        "env_os": os_state.get("kernel_release") if isinstance(os_state, dict) else None,
        "environment": _json(environment(row)),
        "environment_digest": _hash(environment(row)),
    }


def _metric_columns(
    summary: SummaryRow | None, scored: list[Record], ci: bool, resamples: int | None
) -> Record:
    """The cell's `summarize` numbers, each with its 95% bootstrap interval."""
    point: Record = {
        "wer": None,
        "cer": None,
        "wer_whisper": None,
        "cer_whisper": None,
        "rtfx": None,
        "finalize_s_p50": None,
        "finalize_s_p95": None,
    }
    columns: Record = {"clips": 0, "ci_resamples": None, "ci_seed": None}
    if summary is not None:
        point.update(
            {
                "wer": summary["wer"],
                "cer": summary["cer"],
                "wer_whisper": summary["wer_whisper"],
                "cer_whisper": summary["cer_whisper"],
                "rtfx": summary["rtfx"],
                "finalize_s_p50": summary["median_final"],
                "finalize_s_p95": summary["p95_final"],
            }
        )
        columns.update(
            {
                "clips": summary["clips"],
                "finalize_s_p99": summary["p99_final"],
                "timed_clips": summary["timed_clips"],
                "repeat_cv": summary["repeat_cv"],
                "rtf_median": summary["rtf"],
                "cold_ready_s": summary["cold_ready"],
                "warm_ready_s": summary["warm_ready"],
                "audio_seconds": summary["audio"],
                "starved_clips": summary["starved"],
            }
        )
    intervals: dict[str, Any] = {}
    if ci and summary is not None:
        boot = _bootstrap()
        resamples = resamples or boot.RESAMPLES
        intervals = boot.cell_intervals(list(clip_samples(scored).values()), resamples=resamples)
        columns.update({"ci_resamples": resamples, "ci_seed": boot.SEED})
    for column, key in INTERVALS:
        interval = intervals.get(key)
        columns[column] = point[column]
        columns[f"{column}_ci_low"] = interval.low if interval else None
        columns[f"{column}_ci_high"] = interval.high if interval else None
    return columns


def _telemetry_columns(samples: list[Record], cell: Record | None) -> Record:
    cell = cell or {}
    rss = [s["rss_mb"] for s in samples if s.get("rss_mb") is not None]
    vram = [s["vram_mb"] for s in samples if s.get("vram_mb") is not None]
    util = [
        g["util_pct"] for s in samples for g in s.get("gpus") or [] if g.get("util_pct") is not None
    ]
    throttled = cell.get("throttled")
    throttled = throttled if isinstance(throttled, dict) else {}
    return {
        "ram_peak_bytes": _scaled(cell.get("peak_rss_mb"), MB),
        "ram_mean_bytes": _scaled(fmean(rss), MB) if rss else None,
        "vram_peak_bytes": _scaled(cell.get("peak_vram_mb"), MIB),
        "vram_mean_bytes": _scaled(fmean(vram), MIB) if vram else None,
        "cpu_load_mean_pct": None,
        "gpu_load_mean_pct": fmean(util) if util else None,
        "energy_joules": cell.get("energy_j"),
        "telemetry_seconds": cell.get("telemetry_seconds"),
        "j_per_audio_s": cell.get("j_per_audio_s"),
        "cpu_energy_j": cell.get("cpu_energy_j"),
        "gpu_energy_j": cell.get("gpu_energy_j"),
        "max_cpu_temp_c": cell.get("max_cpu_temp_c"),
        "max_gpu_temp_c": cell.get("max_gpu_temp_c"),
        "throttled_cpu": throttled.get("cpu"),
        "throttled_gpu": throttled.get("gpu"),
        "gpu_throttle_reasons": cell.get("gpu_throttle_reasons"),
        "telemetry_error": cell.get("telemetry_error"),
    }


def _summary_row(
    uid: str,
    identity: Record,
    rows: list[Record],
    status: tuple[str, str] | None,
    trace: tuple[list[Record], Record | None],
    *,
    ci: bool,
    resamples: int | None,
) -> Record:
    first, last = rows[0], rows[-1]
    provenance = _provenance(first)
    artifacts = provenance.get("artifacts")
    artifacts = artifacts if isinstance(artifacts, dict) else {}
    served = first.get("served_models") or []
    schedule = identity["schedule"] or {}
    scored = [r for r in rows if not r.get("error")]
    started, ended = _span(rows)
    versions = [r["schema_version"] for r in rows if isinstance(r.get("schema_version"), int)]
    return {
        "schema_version": EXPORT_SCHEMA_VERSION,
        "unit_id": uid,
        "status": AIB_STATUS.get(status[0], INCOMPLETE) if status else INCOMPLETE,
        "model_repo_id": served[0] if served else None,
        "model_revision": artifacts.get("revision"),
        "engine_kind": provenance.get("engine"),
        **_environment_columns(first),
        "started_at": started,
        "ended_at": ended,
        "duration_s": round((ended - started).total_seconds()),
        "requests_total": len(rows),
        "requests_ok": len(scored),
        "requests_error": len(rows) - len(scored),
        "myna_status": status[0] if status else None,
        "status_reason": (status[1] or None) if status else None,
        "label": first["label"],
        "snap": artifacts.get("snap"),
        "mode": first.get("streaming_strategy"),
        "pace": first.get("pace"),
        "settings": _json(provenance.get("settings")),
        "artifacts": _json(provenance.get("artifacts")),
        "served_models": served,
        "served_runtime": _json(last.get("served_runtime")),
        "corpus_id": first.get("corpus_id"),
        "corpus_manifest": first.get("corpus_manifest"),
        "normalizer_version": first.get("normalizer_version"),
        "secondary_normalizer_version": first.get("secondary_normalizer_version"),
        "results_schema_version": max(versions) if versions else None,
        "repeats": schedule.get("repeats"),
        "warmup_clips": schedule.get("warmup_clips"),
        "seed": schedule.get("seed"),
        **_metric_columns(_summarize(scored).get(row_key(first)), scored, ci, resamples),
        **_telemetry_columns(*trace),
    }


def export(infile: Path, out: Path, *, ci: bool = True, resamples: int | None = None) -> list[str]:
    """Write every cell of ``infile`` as an AIB unit under ``out``; return the ids."""
    pa, pq = _arrow()
    records, statuses = _load_latest(infile, keep_errors=True)
    if not records:
        raise SystemExit(f"no clip records in {infile}")
    cells: dict[RowKey, list[Record]] = {}
    for rec in sorted(records, key=lambda r: r["started_at"]):
        cells.setdefault(row_key(rec), []).append(rec)
    traces = _traces(resources_path_for(infile))
    identities = {key: unit_identity(rows) for key, rows in cells.items()}
    units = {key: unit_id(identity) for key, identity in identities.items()}
    campaign = campaign_id(units.values())
    out.mkdir(parents=True, exist_ok=True)
    summary_schema = _schema(pa, SUMMARY_COLUMNS)
    samples_schema = _schema(pa, SAMPLES_COLUMNS)
    load_schema = _schema(pa, LOAD_COLUMNS)
    for key, rows in cells.items():
        uid = units[key]
        trace = traces.get(key, ([], None))
        summary = _summary_row(
            uid,
            identities[key],
            rows,
            statuses.get(key),
            trace,
            ci=ci,
            resamples=resamples,
        )
        summary["campaign_id"] = campaign
        pq.write_table(
            pa.Table.from_pylist([summary], schema=summary_schema), out / f"{uid}.parquet"
        )
        pq.write_table(
            pa.Table.from_pylist([_sample_row(uid, r) for r in rows], schema=samples_schema),
            out / f"{uid}.samples.parquet",
        )
        pq.write_table(
            pa.Table.from_pylist([_load_row(uid, s) for s in trace[0]], schema=load_schema),
            out / f"{uid}.load.parquet",
        )
    unexported = sorted(f"{label} on {machine}" for machine, label in set(statuses) - set(cells))
    if unexported:
        print(f"no rows, so no unit, for: {', '.join(unexported)}")
    return list(units.values())


def cmd_export(args: argparse.Namespace) -> None:
    out = Path(args.parquet)
    ids = export(Path(args.infile), out, ci=args.ci)
    print(f"wrote {len(ids)} unit(s) of campaign {campaign_id(ids)} to {out}")
