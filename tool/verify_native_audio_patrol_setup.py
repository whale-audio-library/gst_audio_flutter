#!/usr/bin/env python3
"""Verify native audio and Patrol device-test project wiring."""

from __future__ import annotations

import plistlib
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


ANDROID_NS = "{http://schemas.android.com/apk/res/android}"
PACKAGE = "com.example.gst_audio_flutter"


def main() -> int:
    errors: list[str] = []
    errors.extend(verify_android())
    errors.extend(verify_ios())
    errors.extend(verify_pubspec())
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print("PASS: native audio permissions and Patrol runners are configured")
    return 0


def verify_android() -> list[str]:
    errors: list[str] = []
    manifest = ET.parse("android/app/src/main/AndroidManifest.xml").getroot()
    permissions = {
        item.attrib.get(f"{ANDROID_NS}name")
        for item in manifest.findall("uses-permission")
    }
    expected_permissions = {
        "android.permission.INTERNET",
        "android.permission.WAKE_LOCK",
        "android.permission.FOREGROUND_SERVICE",
        "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
        "android.permission.POST_NOTIFICATIONS",
    }
    for permission in sorted(expected_permissions - permissions):
        errors.append(f"Android permission missing: {permission}")

    application = manifest.find("application")
    if application is None:
        errors.append("Android application node missing")
        return errors

    services = application.findall("service")
    audio_service = next(
        (
            service
            for service in services
            if service.attrib.get(f"{ANDROID_NS}name")
            == "com.ryanheise.audioservice.AudioService"
        ),
        None,
    )
    if audio_service is None:
        errors.append("Android AudioService is not declared")
    elif audio_service.attrib.get(f"{ANDROID_NS}foregroundServiceType") != "mediaPlayback":
        errors.append("Android AudioService lacks foregroundServiceType=mediaPlayback")

    receivers = application.findall("receiver")
    media_button_receiver = next(
        (
            receiver
            for receiver in receivers
            if receiver.attrib.get(f"{ANDROID_NS}name")
            == "com.ryanheise.audioservice.MediaButtonReceiver"
        ),
        None,
    )
    if media_button_receiver is None:
        errors.append("Android MediaButtonReceiver is not declared")

    gradle = Path("android/app/build.gradle.kts").read_text()
    if "pl.leancode.patrol.PatrolJUnitRunner" not in gradle:
        errors.append("Android PatrolJUnitRunner is not configured")
    if "ANDROIDX_TEST_ORCHESTRATOR" not in gradle:
        errors.append("Android test orchestrator is not configured")

    activity = Path(
        "android/app/src/main/kotlin/com/example/gst_audio_flutter/MainActivity.kt",
    ).read_text()
    if "AudioServiceActivity" not in activity:
        errors.append("MainActivity does not extend AudioServiceActivity")
    if (
        "native_audio_test" not in activity
        or "dispatchMediaKey" not in activity
        or "BuildConfig.DEBUG" not in activity
    ):
        errors.append("Debug-gated native audio test bridge is missing")

    test_runner = Path(
        "android/app/src/androidTest/java/com/example/gst_audio_flutter/MainActivityTest.java",
    )
    if not test_runner.exists() or "PatrolJUnitRunner" not in test_runner.read_text():
        errors.append("Android Patrol instrumentation test runner is missing")
    return errors


def verify_ios() -> list[str]:
    errors: list[str] = []
    with Path("ios/Runner/Info.plist").open("rb") as file:
        plist = plistlib.load(file)
    if "audio" not in plist.get("UIBackgroundModes", []):
        errors.append("iOS UIBackgroundModes does not include audio")

    app_delegate = Path("ios/Runner/AppDelegate.swift").read_text()
    if "AVAudioSession" not in app_delegate or ".playback" not in app_delegate:
        errors.append("iOS AppDelegate does not configure AVAudioSession playback")

    project = Path("ios/Runner.xcodeproj/project.pbxproj").read_text()
    if "RunnerUITests" not in project:
        errors.append("iOS RunnerUITests target is missing")
    if "xcode_backend build" not in project or "xcode_backend embed_and_thin" not in project:
        errors.append("iOS RunnerUITests Flutter build phases are missing")
    if project.count("FLUTTER_BUILD_DIR = build;") < 3:
        errors.append("iOS RunnerUITests lacks FLUTTER_BUILD_DIR for Flutter scripts")

    ui_test = Path("ios/RunnerUITests/RunnerUITests.m")
    if not ui_test.exists():
        errors.append("iOS Patrol RunnerUITests.m is missing")
    else:
        ui_test_text = ui_test.read_text()
        if "PATROL_INTEGRATION_TEST_IOS_RUNNER" not in ui_test_text:
            errors.append("iOS Patrol RunnerUITests.m is missing")
        if "CLEAR_PERMISSIONS" not in ui_test_text or "FULL_ISOLATION" not in ui_test_text:
            errors.append("iOS Patrol runner compile-time flags are missing")

    podfile = Path("ios/Podfile")
    if not podfile.exists():
        errors.append("iOS Podfile is missing")
    else:
        podfile_text = podfile.read_text()
        if (
            "target 'RunnerUITests'" not in podfile_text
            or "inherit! :complete" not in podfile_text
        ):
            errors.append("iOS Podfile does not link RunnerUITests to Flutter pods")
    return errors


def verify_pubspec() -> list[str]:
    errors: list[str] = []
    pubspec = Path("pubspec.yaml").read_text()
    for expected in [
        "audio_service:",
        "audio_session:",
        "patrol:",
        "package_name: com.example.gst_audio_flutter",
        "bundle_id: com.example.gstAudioFlutter",
    ]:
        if expected not in pubspec:
            errors.append(f"pubspec missing {expected}")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
