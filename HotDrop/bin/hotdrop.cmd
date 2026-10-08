@echo off
rem HotDrop launcher for Windows. Uses JAVA_HOME if set, otherwise java from PATH.
setlocal
set "DIR=%~dp0.."
if defined JAVA_HOME (set "JAVA=%JAVA_HOME%\bin\java.exe") else (set "JAVA=java")
"%JAVA%" -jar "%DIR%\build\hotdrop-daemon.jar" %*
