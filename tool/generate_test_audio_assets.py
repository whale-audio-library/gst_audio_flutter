#!/usr/bin/env python3
"""Generate deterministic large WAV fixtures for HTTP integration tests."""

from __future__ import annotations

import argparse
import math
import struct
import wave
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output-dir",
        default="test-assets/generated",
        help="Directory for generated audio files.",
    )
    args = parser.parse_args()

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    write_sine_wav(
        output_dir / "long-30s.wav",
        duration_seconds=30.0,
        frequency_hz=440.0,
        sample_rate=44100,
        channels=1,
    )
    write_sine_wav(
        output_dir / "large-60s-stereo.wav",
        duration_seconds=60.0,
        frequency_hz=330.0,
        sample_rate=44100,
        channels=2,
    )

    for path in sorted(output_dir.glob("*.wav")):
        print(f"{path} {path.stat().st_size} bytes")


def write_sine_wav(
    path: Path,
    *,
    duration_seconds: float,
    frequency_hz: float,
    sample_rate: int,
    channels: int,
) -> None:
    frame_count = int(duration_seconds * sample_rate)
    amplitude = 0.35 * 32767

    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(channels)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)

        frames = bytearray()
        for index in range(frame_count):
            sample = int(amplitude * math.sin(2 * math.pi * frequency_hz * index / sample_rate))
            packed = struct.pack("<h", sample)
            frames.extend(packed * channels)
        wav.writeframes(frames)


if __name__ == "__main__":
    main()
