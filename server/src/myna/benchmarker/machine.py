"""Collect machine summary for the benchmarker header record.

Written once as the first line of the results JSONL so every submission
carries its own hardware context, and recipients can group/filter without
trusting hand-annotated provenance. The sweep also stamps the environment onto
every row, because ``merge`` keeps rows and drops headers.

Every probe degrades to None (or an empty list) rather than raising: a
container, a non-Ubuntu host or a machine with no nvidia-smi still benchmarks.
"""

from __future__ import annotations

import hashlib
import importlib.resources
import platform
import socket
import subprocess
import sys
import zipfile
from datetime import UTC, datetime
from pathlib import Path
from typing import TypedDict

from myna.benchmarker._summarize import SCHEMA_VERSION

SYS_CPU = Path("/sys/devices/system/cpu")

# Written into the zipapp by dev/build-bench.sh; absent from the source tree.
BUILD_VERSION_FILE = "VERSION"


class Smt(TypedDict):
    control: str | None
    active: bool | None


class OsState(TypedDict):
    kernel_release: str
    kernel_cmdline: str | None
    cpu_microcode: str | None
    cpu_governor: str | None
    cpu_boost: bool | None
    smt: Smt
    snapd: str | None


class Gpu(TypedDict):
    index: int
    name: str
    driver_version: str | None
    cuda_driver_version: str | None
    persistence_mode: str | None
    sm_clock_mhz: int | None
    sm_clock_max_mhz: int | None
    mem_clock_mhz: int | None
    mem_clock_max_mhz: int | None
    ecc_mode: str | None
    memory_total_mib: int | None


class Harness(TypedDict):
    version: str | None
    pyz: str | None
    pyz_sha256: str | None


class Machine(TypedDict):
    type: str
    schema_version: int
    hostname: str
    cpu: str | None
    cpu_cores: int | None
    ram_gb: float | None
    gpu: str | None
    gpu_vram_gb: float | None
    ubuntu: str | None
    kernel: str
    os: OsState
    gpus: list[Gpu]
    harness: Harness
    collected_at: str


def _read(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return None


def _cpuinfo_field(name: str) -> str | None:
    """The first ``name`` line of /proc/cpuinfo."""
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            key, _, value = line.partition(":")
            if key.strip() == name:
                return value.strip()
    except OSError:
        pass
    return None


def _cpu_model() -> str | None:
    return _cpuinfo_field("model name")


def _microcode() -> str | None:
    return _cpuinfo_field("microcode")


def _cpu_cores() -> int | None:
    try:
        count = sum(
            1
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines()
            if line.startswith("processor")
        )
        return count or None
    except OSError:
        return None


def _ram_gb() -> float | None:
    try:
        import psutil

        return round(psutil.virtual_memory().total / 1e9, 1)
    except Exception:  # noqa: BLE001
        return None


def _kernel_cmdline() -> str | None:
    return _read(Path("/proc/cmdline"))


def _governor() -> str | None:
    """The scaling governor, or every distinct one when the CPUs disagree."""
    found = {
        value
        for path in SYS_CPU.glob("cpu[0-9]*/cpufreq/scaling_governor")
        if (value := _read(path))
    }
    return ",".join(sorted(found)) or None


def _boost() -> bool | None:
    """Whether turbo/boost is enabled: cpufreq's switch, else intel_pstate's."""
    boost = _read(SYS_CPU / "cpufreq" / "boost")
    if boost in ("0", "1"):
        return boost == "1"
    no_turbo = _read(SYS_CPU / "intel_pstate" / "no_turbo")
    if no_turbo in ("0", "1"):
        return no_turbo == "0"
    return None


def _smt() -> Smt:
    active = _read(SYS_CPU / "smt" / "active")
    return {
        "control": _read(SYS_CPU / "smt" / "control"),
        "active": active == "1" if active in ("0", "1") else None,
    }


def _capture(cmd: list[str], timeout: float = 10) -> str | None:
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, check=True
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None


def _snapd_version() -> str | None:
    for line in (_capture(["snap", "version"]) or "").splitlines():
        key, _, value = line.partition(" ")
        if key == "snapd" and value.strip() not in ("", "unavailable"):
            return value.strip()
    return None


# nvidia-smi --query-gpu columns, in the order _gpus unpacks them.
_GPU_QUERY = (
    "index,name,driver_version,persistence_mode,clocks.sm,clocks.max.sm,"
    "clocks.mem,clocks.max.mem,ecc.mode.current,memory.total"
)


def _smi_value(raw: str) -> str | None:
    """nvidia-smi's "[N/A]"/"N/A" means the field does not apply: unknown."""
    value = raw.strip()
    return None if not value or "N/A" in value else value


def _smi_int(raw: str) -> int | None:
    value = _smi_value(raw)
    try:
        return int(float(value)) if value is not None else None
    except ValueError:
        return None


def _cuda_driver_version() -> str | None:
    """The CUDA version the driver supports; only ``nvidia-smi -q`` reports it."""
    for line in (_capture(["nvidia-smi", "-q"]) or "").splitlines():
        key, _, value = line.partition(":")
        if key.strip() == "CUDA Version":
            return _smi_value(value)
    return None


def _gpus() -> list[Gpu]:
    """Every NVIDIA GPU nvidia-smi reports; empty without a driver."""
    out = _capture(
        ["nvidia-smi", f"--query-gpu={_GPU_QUERY}", "--format=csv,noheader,nounits"], timeout=5
    )
    rows = [line.split(",") for line in (out or "").splitlines() if line.strip()]
    rows = [row for row in rows if len(row) == len(_GPU_QUERY.split(","))]
    if not rows:
        return []
    cuda = _cuda_driver_version()
    gpus: list[Gpu] = []
    for index, name, driver, persistence, sm, sm_max, mem, mem_max, ecc, total in rows:
        reported = _smi_int(index)
        gpus.append(
            {
                "index": len(gpus) if reported is None else reported,
                "name": name.strip(),
                "driver_version": _smi_value(driver),
                "cuda_driver_version": cuda,
                "persistence_mode": _smi_value(persistence),
                "sm_clock_mhz": _smi_int(sm),
                "sm_clock_max_mhz": _smi_int(sm_max),
                "mem_clock_mhz": _smi_int(mem),
                "mem_clock_max_mhz": _smi_int(mem_max),
                "ecc_mode": _smi_value(ecc),
                "memory_total_mib": _smi_int(total),
            }
        )
    return gpus


def _ubuntu_version() -> str | None:
    try:
        for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
            key, _, value = line.partition("=")
            if key.strip() == "PRETTY_NAME":
                return value.strip().strip('"')
    except OSError:
        pass
    return None


def _read_build_version() -> str | None:
    try:
        return (
            importlib.resources.files("myna.benchmarker")
            .joinpath(BUILD_VERSION_FILE)
            .read_text(encoding="utf-8")
        )
    except (OSError, ModuleNotFoundError):
        return None


def _harness() -> Harness:
    """Which myna-bench build ran: its version, and the zipapp's own hash."""
    version = _read_build_version()
    entry = Path(sys.argv[0]) if sys.argv and sys.argv[0] else None
    pyz = entry if entry is not None and entry.is_file() and zipfile.is_zipfile(entry) else None
    return {
        "version": version.strip() if version else None,
        "pyz": pyz.name if pyz else None,
        "pyz_sha256": hashlib.sha256(pyz.read_bytes()).hexdigest() if pyz else None,
    }


def collect() -> Machine:
    """Return a machine-summary dict to write as the JSONL header record."""
    gpus = _gpus()
    first = gpus[0] if gpus else None
    return {
        "type": "machine",
        "schema_version": SCHEMA_VERSION,
        "hostname": socket.gethostname(),
        "cpu": _cpu_model(),
        "cpu_cores": _cpu_cores(),
        "ram_gb": _ram_gb(),
        "gpu": first["name"] if first else None,
        "gpu_vram_gb": (
            round(first["memory_total_mib"] / 1024, 1)
            if first and first["memory_total_mib"]
            else None
        ),
        "ubuntu": _ubuntu_version(),
        "kernel": platform.release(),
        "os": {
            "kernel_release": platform.release(),
            "kernel_cmdline": _kernel_cmdline(),
            "cpu_microcode": _microcode(),
            "cpu_governor": _governor(),
            "cpu_boost": _boost(),
            "smt": _smt(),
            "snapd": _snapd_version(),
        },
        "gpus": gpus,
        "harness": _harness(),
        "collected_at": datetime.now(UTC).isoformat(),
    }
