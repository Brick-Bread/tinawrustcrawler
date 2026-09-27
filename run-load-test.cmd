@echo off
cd /d "%~dp0"
title TINAW Bounded Load Test
echo Starting a maximum one-hour test at up to five requests per second.
if exist "%~dp0target\release\backstage_hunter.exe" (
    "%~dp0target\release\backstage_hunter.exe" --load-test --rate 5 --results load_test_results.jsonl --successes load_test_successes.txt
) else (
    cargo run --release -- --load-test --rate 5 --results load_test_results.jsonl --successes load_test_successes.txt
)
echo.
echo Load test stopped. Press any key to close this window.
pause >nul
