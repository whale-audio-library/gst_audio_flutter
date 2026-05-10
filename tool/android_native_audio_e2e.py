#!/usr/bin/env python3
"""Run Android native audio control checks on an installed debug APK."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from typing import Iterable


PACKAGE = "com.example.gst_audio_flutter"
ACTIVITY = f"{PACKAGE}/.MainActivity"
SERVICE = "com.ryanheise.audioservice.AudioService"
RECEIVER = "com.ryanheise.audioservice.MediaButtonReceiver"


class CommandError(RuntimeError):
    pass


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--device", default="emulator-5554")
    parser.add_argument(
        "--gstreamer-root-android",
        default=os.getenv("GSTREAMER_ROOT_ANDROID"),
    )
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()

    env = os.environ.copy()
    if args.gstreamer_root_android:
        env["GSTREAMER_ROOT_ANDROID"] = args.gstreamer_root_android

    run(["adb", "-s", args.device, "wait-for-device"])
    print_device_summary(args.device)
    run(["adb", "-s", args.device, "reverse", "tcp:8765", "tcp:8765"])
    ensure_http_fixture_server()

    if not args.skip_build:
        run(
            [
                "flutter",
                "build",
                "apk",
                "--debug",
                "--target-platform",
                "android-x64",
                "--target",
                "lib/android_native_audio_e2e_main.dart",
            ],
            env=env,
        )

    install_apk(args.device)
    assert_manifest(args.device)
    print("PASS: APK/package declare native audio permissions, service and receiver")
    run(["adb", "-s", args.device, "logcat", "-c"])

    logcat = subprocess.Popen(
        ["adb", "-s", args.device, "logcat", "-v", "time"],
        cwd=Path.cwd(),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        bufsize=1,
    )

    try:
        run(["adb", "-s", args.device, "shell", "am", "start", "-W", "-n", ACTIVITY])
        return drive_from_logcat(args.device, logcat)
    finally:
        if logcat.poll() is None:
            logcat.terminate()
            try:
                logcat.wait(timeout=5)
            except subprocess.TimeoutExpired:
                logcat.kill()
        run(["adb", "-s", args.device, "shell", "am", "force-stop", PACKAGE], check=False)


def install_apk(device: str) -> None:
    apk = Path("build/app/outputs/flutter-apk/app-debug.apk")
    require(apk.exists(), f"APK does not exist: {apk}")
    run(["adb", "-s", device, "install", "-r", str(apk)])


def drive_from_logcat(device: str, logcat: subprocess.Popen[str]) -> int:
    stages_seen: set[str] = set()
    required_stages = {
        "playing",
        "paused_by_media_key",
        "resumed_by_media_key",
        "next_by_media_key",
        "previous_by_media_key",
        "complete",
    }
    actions = {
        "playing": lambda: verify_playing_native_state(device),
        "paused_by_media_key": lambda: dispatch_media_key(device, "play-pause"),
        "resumed_by_media_key": lambda: dispatch_media_key(device, "next"),
        "next_by_media_key": lambda: dispatch_media_key(device, "previous"),
    }

    assert logcat.stdout is not None
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        line = logcat.stdout.readline()
        if line == "":
            if logcat.poll() is not None:
                break
            time.sleep(0.1)
            continue

        if "ANDROID_NATIVE_AUDIO_STAGE" in line:
            print(line, end="")
        if "StateError" in line or "lastError" in line:
            print(line, end="")

        match = re.search(r"ANDROID_NATIVE_AUDIO_STAGE:(\w+)", line)
        if match is None:
            continue

        stage = match.group(1)
        stages_seen.add(stage)
        action = actions.get(stage)
        if action is not None:
            action()
        if stage == "complete":
            missing = required_stages - stages_seen
            if missing:
                raise CommandError(
                    f"Missing native audio test stages: {', '.join(sorted(missing))}"
                )
            return 0

    raise CommandError(
        "Timed out waiting for Android native audio E2E completion. "
        f"Stages seen: {', '.join(sorted(stages_seen)) or 'none'}\n"
        f"{runtime_snapshot(device)}"
    )


def verify_playing_native_state(device: str) -> None:
    run(["adb", "-s", device, "shell", "input", "keyevent", "KEYCODE_HOME"])
    wait_for(lambda: assert_package_installed(device), "installed package visibility")
    print("PASS: package remains visible while playback continues in background")
    wait_for(lambda: assert_audio_playback_active(device), "native audio playback")
    print("PASS: dumpsys audio reports active USAGE_MEDIA playback")
    wait_for(lambda: assert_media_notification_or_session(device), "media notification/session")
    print("PASS: media notification/session is visible to Android")
    dispatch_media_key(device, "play-pause")


def assert_manifest(device: str) -> None:
    dump = adb_text(device, ["shell", "dumpsys", "package", PACKAGE])
    if not dump.strip():
        dump = dump_badging()

    require("android.permission.WAKE_LOCK" in dump, "WAKE_LOCK is missing")
    require(
        "android.permission.FOREGROUND_SERVICE" in dump,
        "FOREGROUND_SERVICE is missing",
    )
    require(
        "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK" in dump,
        "FOREGROUND_SERVICE_MEDIA_PLAYBACK is missing",
    )
    require(SERVICE in dump, "AudioService is not declared")
    require(RECEIVER in dump, "MediaButtonReceiver is not declared")

    xmltree = dump_xmltree()
    require(
        "android:foregroundServiceType" in xmltree and "0x2" in xmltree,
        "AudioService does not declare foregroundServiceType=mediaPlayback",
    )


def assert_package_installed(device: str) -> bool:
    packages = adb_text(device, ["shell", "pm", "list", "packages"])
    require(PACKAGE in packages, f"{PACKAGE} is not visible to package manager")
    return True


def assert_audio_playback_active(device: str) -> bool:
    dump = adb_text(device, ["shell", "dumpsys", "audio"])
    require(
        "usage=USAGE_MEDIA" in dump and "state:started" in dump,
        "native audio playback was not active in dumpsys audio",
    )
    return True


def assert_media_notification_or_session(device: str) -> bool:
    notification = adb_text(device, ["shell", "dumpsys", "notification", "--noredact"])
    media_session = adb_text(device, ["shell", "dumpsys", "media_session"])
    combined = f"{notification}\n{media_session}"
    require(
        PACKAGE in combined or "Audio playback" in combined,
        "media notification/session did not mention the app",
    )
    return True


def dispatch_media_key(device: str, key: str) -> None:
    print(f"DISPATCH: cmd media_session dispatch {key}")
    run(["adb", "-s", device, "shell", "cmd", "media_session", "dispatch", key])


def print_device_summary(device: str) -> None:
    manufacturer = adb_text(device, ["shell", "getprop", "ro.product.manufacturer"]).strip()
    model = adb_text(device, ["shell", "getprop", "ro.product.model"]).strip()
    release = adb_text(device, ["shell", "getprop", "ro.build.version.release"]).strip()
    hardware = adb_text(device, ["shell", "getprop", "ro.boot.hardware"]).strip()
    print(
        "Device: "
        f"manufacturer={manufacturer or 'unknown'} "
        f"model={model or 'unknown'} "
        f"android={release or 'unknown'} "
        f"hardware={hardware or 'unknown'}"
    )


def runtime_snapshot(device: str) -> str:
    media_sessions = adb_text(
        device,
        ["shell", "dumpsys", "media_session"],
    )
    notifications = adb_text(
        device,
        ["shell", "dumpsys", "notification", "--noredact"],
    )
    audio = adb_text(device, ["shell", "dumpsys", "audio"])
    return "\n".join(
        [
            "Runtime snapshot after timeout:",
            "-- media_session --",
            interesting_lines(
                media_sessions,
                ["Audio playback", PACKAGE, "MediaButtonReceiver", "PlaybackState"],
            ),
            "-- notification --",
            interesting_lines(
                notifications,
                [PACKAGE, "NotificationChannel", "mFgServiceShown", "audio"],
            ),
            "-- audio --",
            interesting_lines(audio, ["USAGE_MEDIA", "state:started", PACKAGE]),
        ]
    )


def interesting_lines(text: str, needles: list[str], limit: int = 40) -> str:
    lines = [
        line.rstrip()
        for line in text.splitlines()
        if any(needle in line for needle in needles)
    ]
    if not lines:
        return "(no matching lines)"
    return "\n".join(lines[:limit])


def wait_for(assertion, label: str, timeout_seconds: float = 8.0) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            assertion()
            return
        except Exception as error:  # noqa: BLE001 - preserve assertion text.
            last_error = error
            time.sleep(0.4)
    raise CommandError(f"Timed out waiting for {label}: {last_error}")


def ensure_http_fixture_server() -> None:
    try:
        with urllib.request.urlopen(
            "http://127.0.0.1:8765/generated/long-30s.wav",
            timeout=2,
        ) as response:
            if response.status == 200:
                return
    except Exception:
        pass
    raise CommandError(
        "Expected http://127.0.0.1:8765/generated/long-30s.wav to be served. "
        "Start tool/audio_test_server.py before running this script."
    )


def dump_badging() -> str:
    aapt = android_build_tool("aapt")
    return run([aapt, "dump", "badging", "build/app/outputs/flutter-apk/app-debug.apk"]).stdout


def dump_xmltree() -> str:
    aapt = android_build_tool("aapt")
    return run(
        [
            aapt,
            "dump",
            "xmltree",
            "build/app/outputs/flutter-apk/app-debug.apk",
            "AndroidManifest.xml",
        ]
    ).stdout


def android_build_tool(name: str) -> str:
    android_home = os.getenv("ANDROID_HOME")
    require(android_home is not None, "ANDROID_HOME is not set")
    candidates = sorted(Path(android_home).glob(f"build-tools/*/{name}"))
    require(bool(candidates), f"{name} was not found under ANDROID_HOME")
    return str(candidates[-1])


def adb_text(device: str, args: Iterable[str]) -> str:
    return run(["adb", "-s", device, *args]).stdout


def run(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        args,
        cwd=Path.cwd(),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if check and result.returncode != 0:
        raise CommandError(f"{' '.join(args)} failed with {result.returncode}\n{result.stdout}")
    return result


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CommandError(message)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except CommandError as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
