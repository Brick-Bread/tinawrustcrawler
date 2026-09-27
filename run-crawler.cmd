@echo off
cd /d "%~dp0"
title TINAW Crawler
echo Starting TINAW Crawler. Press Ctrl+C to stop.
if exist "%~dp0target\release\backstage_hunter.exe" (
    "%~dp0target\release\backstage_hunter.exe"
) else (
    cargo run --release
)
echo.
echo Crawler stopped. Press any key to close this window.
pause >nul
