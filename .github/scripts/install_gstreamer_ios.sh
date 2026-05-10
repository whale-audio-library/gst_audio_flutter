#!/usr/bin/env bash

set -euo pipefail

cache_dir="$HOME/.cache/gstreamer-ios"
mkdir -p "$cache_dir"

archive_name="gstreamer-${GSTREAMER_IOS_VERSION}-xcframework.tar.xz"
archive_path="$cache_dir/$archive_name"
archive_url="https://gstreamer.freedesktop.org/pkg/ios/${GSTREAMER_IOS_VERSION}/$archive_name"
extract_dir="$cache_dir/gstreamer-${GSTREAMER_IOS_VERSION}-xcframework"

needs_extract=0
if [[ ! -d "$extract_dir/GStreamer.xcframework" ]]; then
  needs_extract=1
elif [[ ! -f "$extract_dir/GStreamer.xcframework/ios-arm64/libGStreamer.a" ]]; then
  needs_extract=1
elif ! find "$extract_dir/GStreamer.xcframework" \
    -maxdepth 2 \
    -path '*/ios-*simulator/libGStreamer.a' \
    -print -quit | grep -q .; then
  needs_extract=1
fi

if [[ "$needs_extract" -eq 1 ]]; then
  rm -rf "$extract_dir"
  mkdir -p "$extract_dir"
  if [[ ! -f "$archive_path" ]]; then
    curl --fail --location --retry 5 --retry-delay 10 \
      --output "$archive_path" \
      "$archive_url"
  fi
  echo "${GSTREAMER_IOS_XCFRAMEWORK_SHA256}  ${archive_path}" | shasum --algorithm 256 --check -
  df -h "$HOME" || true
  members_file="$RUNNER_TEMP/gstreamer-ios-members.txt"
  tar -tf "$archive_path" \
    | grep -E '^GStreamer\.xcframework/(Info\.plist|ios-arm64/|ios-arm64_x86_64-simulator/|ios-arm64-simulator/|ios-x86_64-simulator/)' \
    > "$members_file"
  if ! grep -q '^GStreamer\.xcframework/ios-arm64/' "$members_file"; then
    echo "GStreamer.xcframework ios-arm64 slice was not found in archive"
    exit 1
  fi
  if ! grep -E '^GStreamer\.xcframework/ios-.*simulator/' "$members_file"; then
    echo "GStreamer.xcframework iOS simulator slice was not found in archive"
    exit 1
  fi
  tar_log="$RUNNER_TEMP/gstreamer-tar.log"
  set +e
  tar -xJvf "$archive_path" -C "$extract_dir" -T "$members_file" > "$tar_log" 2>&1
  tar_status=$?
  set -e
  tail -n 80 "$tar_log" || true
  if [[ "$tar_status" -ne 0 ]]; then
    df -h "$HOME" || true
    du -sh "$cache_dir" || true
    exit "$tar_status"
  fi
else
  echo "${GSTREAMER_IOS_XCFRAMEWORK_SHA256}  ${archive_path}" | shasum --algorithm 256 --check -
fi

gstreamer_xcframework="$(find "$extract_dir" -maxdepth 4 -type d -name 'GStreamer.xcframework' -print -quit)"
if [[ -z "$gstreamer_xcframework" ]]; then
  find "$extract_dir" -maxdepth 4 -print
  exit 1
fi

device_library_dir="$gstreamer_xcframework/ios-arm64"
simulator_library_file="$(
  find "$gstreamer_xcframework" \
    -maxdepth 2 \
    -path '*/ios-*simulator/libGStreamer.a' \
    -print -quit
)"
simulator_library_dir="${simulator_library_file%/libGStreamer.a}"
if [[ ! -f "$device_library_dir/libGStreamer.a" ]]; then
  find "$gstreamer_xcframework" -maxdepth 3 -type f -name 'libGStreamer.a' -print
  exit 1
fi
if [[ -z "$simulator_library_dir" || ! -f "$simulator_library_dir/libGStreamer.a" ]]; then
  find "$gstreamer_xcframework" -maxdepth 3 -type f -name 'libGStreamer.a' -print
  exit 1
fi

{
  echo "GSTREAMER_IOS_XCFRAMEWORK=$gstreamer_xcframework"
  echo "GSTREAMER_IOS_DEVICE_LIBRARY_DIR=$device_library_dir"
  echo "GSTREAMER_IOS_SIMULATOR_LIBRARY_DIR=$simulator_library_dir"
} >> "$GITHUB_ENV"
