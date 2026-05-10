#!/usr/bin/env bash

set -euo pipefail

mkdir -p .cache/gstreamer-android
cd .cache/gstreamer-android

archive_name="gstreamer-1.0-android-universal-${GSTREAMER_VERSION}.tar.xz"
archive_path="$PWD/$archive_name"
extract_dir="$PWD/gstreamer-1.0-android-universal-${GSTREAMER_VERSION}"
archive_url="https://gstreamer.freedesktop.org/pkg/android/${GSTREAMER_VERSION}/${archive_name}"

curl -fsSLo "$archive_path" "$archive_url"
echo "${GSTREAMER_SHA256}  ${archive_path}" | sha256sum -c -
mkdir -p "$extract_dir"
tar -xJf "$archive_path" -C "$extract_dir"
rm "$archive_path"
