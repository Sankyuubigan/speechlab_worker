@echo off
REM ASR tests for SpeechLab.
REM Thin wrapper over the toolkit's test.bat - ZERO build logic here
REM (desktop_rust_tauri/rules.md SS2: build and test only via the toolkit).
REM
REM Part 1: asr:: - self-contained preprocessor test, always runs.
REM Part 2: ogg_decode - #[ignore]d, needs a REAL ogg passed via
REM         SPEECHLAB_TEST_OGG. Skipped loudly (not silently) when unset,
REM         because a skipped test that reports "ok" is a lie (core rules SS2.2).
REM
REM Usage:
REM   set SPEECHLAB_TEST_OGG=D:\path\to\sample.ogg
REM   run_asr_test.bat
setlocal enableextensions

set "PROJ=%~dp0"
if not exist "%PROJ%src-tauri\tauri.conf.json" (
  echo [ERROR] This script must live in the project root, next to src-tauri\tauri.conf.json
  pause
  exit /b 1
)

set "TOOLKIT=%TAURI_BUILD_TOOLKIT%"
if "%TOOLKIT%"=="" set "TOOLKIT=%PROJ%..\my-tauri-plugins\tauri-build-toolkit\cli.cjs"
if not exist "%TOOLKIT%" (
  echo [ERROR] Tauri build toolkit not found: "%TOOLKIT%"
  echo Set env TAURI_BUILD_TOOLKIT to the toolkit cli.cjs, or place the
  echo toolkit folder at: my-tauri-plugins\tauri-build-toolkit
  pause
  exit /b 1
)

echo [1/2] ASR preprocessor tests (asr::)...
call "%PROJ%test.bat" asr:: -- --nocapture
if errorlevel 1 (
  echo [ERROR] asr:: tests failed.
  pause
  exit /b 1
)

echo.
echo [2/2] OGG decode test (needs a real file)...
if "%SPEECHLAB_TEST_OGG%"=="" (
  echo [SKIP] SPEECHLAB_TEST_OGG is not set - ogg_decode NOT verified.
  echo        It is #[ignore]d and will fail loudly once the variable is set.
  echo        Example: set SPEECHLAB_TEST_OGG=D:\path\to\sample.ogg
  echo.
  echo [+DONE] run_asr_test.bat finished (ogg_decode NOT checked).
  endlocal
  exit /b 0
)

call "%PROJ%test.bat" -- --ignored --nocapture ogg_decode
if errorlevel 1 (
  echo [ERROR] ogg_decode failed.
  pause
  exit /b 1
)

echo.
echo [+DONE] run_asr_test.bat finished.
endlocal