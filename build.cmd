@echo off
setlocal
set "ROOT=%~dp0"
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
for /f "usebackq tokens=* delims=" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%I"
if not defined VSINSTALL (
  echo Install Visual Studio Build Tools with the x64 C++ build tools workload.
  exit /b 1
)
call "%VSINSTALL%\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 exit /b 1
if exist "%ROOT%.tools\cargo-home\bin\cargo.exe" (
  set "CARGO_HOME=%ROOT%.tools\cargo-home"
  set "RUSTUP_HOME=%ROOT%.tools\rustup-home"
  set "PATH=%ROOT%.tools\cargo-home\bin;%PATH%"
) else (
  where cargo >nul 2>nul
  if errorlevel 1 (
    echo Install Rust and Visual Studio Build Tools, then retry.
    exit /b 1
  )
)
cargo build --release --manifest-path "%ROOT%Cargo.toml"
if errorlevel 1 exit /b 1
if not exist "%ROOT%dist" mkdir "%ROOT%dist"
copy /Y "%ROOT%target\release\exmonitor-viewer.exe" "%ROOT%dist\exmonitor-viewer.exe" >nul
if errorlevel 1 exit /b 1
copy /Y "%ROOT%README.md" "%ROOT%dist\README.md" >nul
if errorlevel 1 exit /b 1
copy /Y "%ROOT%LICENSE" "%ROOT%dist\LICENSE" >nul
if errorlevel 1 exit /b 1
echo Built dist\exmonitor-viewer.exe
