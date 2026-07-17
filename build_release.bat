@echo off
set "VSROOT=C:\Program Files\Microsoft Visual Studio\18\Community"
call "%VSROOT%\Common7\Tools\VsDevCmd.bat" -arch=amd64 >nul 2>&1
set "LIB=%VSROOT%\VC\Tools\MSVC\14.52.36520\lib\x64;%LIB%"
cd /d "%~dp0"
cargo build --release --bin audio-switcher
