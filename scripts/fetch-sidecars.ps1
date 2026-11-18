# Télécharge yt-dlp et ffmpeg dans src-tauri\binaries au format Tauri externalBin.
$ErrorActionPreference = "Stop"
$dir = Join-Path $PSScriptRoot "..\src-tauri\binaries"
New-Item -ItemType Directory -Force $dir | Out-Null
$triple = (rustc -vV | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
Write-Host "-> yt-dlp ($triple)"
Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile (Join-Path $dir "yt-dlp-$triple.exe")
Write-Host "-> ffmpeg ($triple)"
$tmp = Join-Path $env:TEMP "aurex-ffmpeg"
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $tmp | Out-Null
Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip" -OutFile "$tmp\ff.zip"
Expand-Archive "$tmp\ff.zip" -DestinationPath $tmp
Copy-Item (Get-ChildItem "$tmp\ffmpeg-*\bin\ffmpeg.exe").FullName (Join-Path $dir "ffmpeg-$triple.exe")
Get-ChildItem $dir
