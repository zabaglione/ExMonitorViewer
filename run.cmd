@echo off
setlocal
set "APP=%~dp0dist\exmonitor-viewer.exe"
if not exist "%APP%" set "APP=%~dp0target\debug\exmonitor-viewer.exe"
if not exist "%APP%" (
  echo Run build.cmd first.
  exit /b 1
)
"%APP%" %*
