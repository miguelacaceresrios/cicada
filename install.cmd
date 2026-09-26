@echo off
rem Doble clic para instalar CICADA.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
pause
