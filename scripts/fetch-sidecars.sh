#!/usr/bin/env bash
# Télécharge yt-dlp et ffmpeg dans src-tauri/binaries/ au format Tauri externalBin : <nom>-<triplet>[.exe]
# Usage : scripts/fetch-sidecars.sh [triplet]   (défaut : celui de rustc)
set -euo pipefail
cd "$(dirname "$0")/../src-tauri/binaries"
TRIPLE="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
EXT=""; case "$TRIPLE" in *windows*) EXT=".exe";; esac

echo "→ yt-dlp ($TRIPLE)"
case "$TRIPLE" in
  *apple-darwin)  YT="yt-dlp_macos" ;;
  *windows*)      YT="yt-dlp.exe" ;;
  aarch64*linux*) YT="yt-dlp_linux_aarch64" ;;
  *linux*)        YT="yt-dlp_linux" ;;
  *) echo "triplet non géré : $TRIPLE" >&2; exit 1 ;;
esac
curl -fL "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YT" -o "yt-dlp-$TRIPLE$EXT"
chmod +x "yt-dlp-$TRIPLE$EXT"

echo "→ ffmpeg ($TRIPLE)"
tmp="$(mktemp -d)"
case "$TRIPLE" in
  aarch64-apple-darwin|x86_64-apple-darwin)
    # Builds statiques de evermeet.cx (x86_64, tourne via Rosetta sur Apple Silicon). Remplacez par votre build ARM si besoin.
    curl -fL "https://evermeet.cx/ffmpeg/get/zip" -o "$tmp/ff.zip"
    unzip -o -q "$tmp/ff.zip" -d "$tmp"; mv "$tmp/ffmpeg" "ffmpeg-$TRIPLE" ;;
  *windows*)
    curl -fL "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip" -o "$tmp/ff.zip"
    unzip -o -q "$tmp/ff.zip" -d "$tmp"; mv "$tmp"/ffmpeg-*/bin/ffmpeg.exe "ffmpeg-$TRIPLE.exe" ;;
  *linux*)
    curl -fL "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz" -o "$tmp/ff.tar.xz"
    tar -xf "$tmp/ff.tar.xz" -C "$tmp"; mv "$tmp"/ffmpeg-*/ffmpeg "ffmpeg-$TRIPLE" ;;
esac
chmod +x ffmpeg-$TRIPLE$EXT
rm -rf "$tmp"
ls -lh .
