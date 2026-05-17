@echo off
setlocal
cd /d "%~dp0"

tasklist /FI "IMAGENAME eq anime-player.exe" 2>nul | find /I "anime-player.exe" >nul
if not errorlevel 1 (
  echo Close Anime Player before updating.
  pause
  exit /b 1
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0_update.ps1" %*
set EXITCODE=%ERRORLEVEL%
if %EXITCODE% neq 0 (
  echo.
  echo Update failed.
  pause
  exit /b %EXITCODE%
)

endlocal
