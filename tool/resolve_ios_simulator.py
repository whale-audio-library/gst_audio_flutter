#!/usr/bin/env python3
"""Resolve a stable iOS simulator configuration from the current Xcode image."""

from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import sys
import time
from typing import Any


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--format",
        choices=("json", "shell"),
        default="json",
        help="Output format.",
    )
    args = parser.parse_args()

    try:
        runtimes = _load_simctl_json("runtimes")["runtimes"]
        device_types = _load_simctl_json("devicetypes")["devicetypes"]
    except subprocess.CalledProcessError as error:
        print(error, file=sys.stderr)
        return error.returncode

    runtime = _choose_runtime(runtimes)
    device_type = _choose_device_type(device_types)
    simulator_name = f"CI {device_type['name']} {int(time.time())}"

    payload = {
        "simulator_name": simulator_name,
        "simulator_device_name": device_type["name"],
        "simulator_device_type": device_type["identifier"],
        "simulator_runtime": runtime["identifier"],
        "simulator_runtime_name": runtime["name"],
    }

    if args.format == "json":
        print(json.dumps(payload))
    else:
        for key, value in payload.items():
            print(f"{key.upper()}={shlex.quote(str(value))}")
    return 0


def _load_simctl_json(section: str) -> dict[str, Any]:
    output = subprocess.check_output(
        ["xcrun", "simctl", "list", section, "-j"],
        text=True,
    )
    return json.loads(output)


def _choose_runtime(runtimes: list[dict[str, Any]]) -> dict[str, Any]:
    available = [
        runtime
        for runtime in runtimes
        if runtime.get("isAvailable", False)
        and runtime.get("identifier", "").startswith("com.apple.CoreSimulator.SimRuntime.iOS-")
    ]
    if not available:
        raise SystemExit("No available iOS simulator runtimes were found")

    return max(available, key=_runtime_version_key)


def _runtime_version_key(runtime: dict[str, Any]) -> tuple[int, ...]:
    name = runtime.get("name", "")
    match = re.search(r"iOS\s+(\d+(?:\.\d+)*)", name)
    if match:
        return tuple(int(part) for part in match.group(1).split("."))
    identifier = runtime.get("identifier", "")
    suffix = identifier.removeprefix("com.apple.CoreSimulator.SimRuntime.iOS-")
    parts = [part for part in suffix.split("-") if part.isdigit()]
    return tuple(int(part) for part in parts)


def _choose_device_type(device_types: list[dict[str, Any]]) -> dict[str, Any]:
    available = [item for item in device_types if item.get("identifier")]
    if not available:
        raise SystemExit("No simulator device types were found")

    preferred_names = [
        "iPhone 16 Pro",
        "iPhone 16",
        "iPhone 15 Pro",
        "iPhone 15",
    ]
    by_name = {item["name"]: item for item in available}
    for name in preferred_names:
        if name in by_name:
            return by_name[name]

    iphones = [item for item in available if item.get("name", "").startswith("iPhone")]
    if not iphones:
        raise SystemExit("No iPhone simulator device types were found")

    return max(iphones, key=_device_type_key)


def _device_type_key(device_type: dict[str, Any]) -> tuple[int, int, int, str]:
    name = device_type.get("name", "")
    match = re.match(r"iPhone\s+(\d+)(.*)", name)
    if not match:
        return (-1, -1, -1, name)

    generation = int(match.group(1))
    suffix = match.group(2)
    pro_score = 2 if "Pro Max" in suffix else 1 if "Pro" in suffix else 0
    plus_score = 1 if "Plus" in suffix else 0
    return (generation, pro_score, plus_score, name)


if __name__ == "__main__":
    raise SystemExit(main())
