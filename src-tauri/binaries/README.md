# Sidecars

Ce dossier reçoit `yt-dlp-<triplet>[.exe]` et `ffmpeg-<triplet>[.exe]` (convention Tauri `externalBin`).
Ils sont téléchargés par `scripts/fetch-sidecars.sh` (macOS/Linux) ou `scripts/fetch-sidecars.ps1` (Windows)
et NE sont PAS versionnés (voir `.gitignore`).
