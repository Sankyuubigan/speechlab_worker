@echo off
REM Switch console to UTF-8
chcp 65001 >nul
cd /d "%~dp0"

REM Auto-detect and initialize MSVC
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

REM Remove sccache wrappers that break cc-rs
set "CC="
set "CXX="
set "CMAKE_C_COMPILER_LAUNCHER="
set "CMAKE_CXX_COMPILER_LAUNCHER="
set "RUSTC_WRAPPER="
set "CMAKE_POLICY_VERSION_MINIMUM=3.5"

REM Run only Rust tests (без полной сборки Tauri/exe)
cd src-tauri
cargo test %*
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   TEST ERROR! Press any key to exit...
    echo ========================================
    pause >nul
    exit /b 1
)
