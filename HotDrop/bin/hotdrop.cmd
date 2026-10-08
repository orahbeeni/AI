@echo off
rem HotDrop launcher for Windows. Uses JAVA_HOME if set, otherwise java from PATH.
rem Builds the jars on first use. 'hotdrop up' switches to the running server's JDK by itself.
setlocal
set "DIR=%~dp0.."
if defined JAVA_HOME (set "JAVA=%JAVA_HOME%\bin\java.exe") else (set "JAVA=java")
if not exist "%DIR%\build\hotdrop-daemon.jar" (
  echo first run: building HotDrop ... 1>&2
  pushd "%DIR%"
  "%JAVA%" Build.java || exit /b 1
  popd
)
"%JAVA%" -jar "%DIR%\build\hotdrop-daemon.jar" %*
