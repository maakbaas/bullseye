; Hooks into Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).
; The installer runs as an administrator (installMode "perMachine"). The work itself is in the
; app (src/setup.rs); these only run it and act on its exit code:
; 0 done, 1 failed, 2 refused because a block is running.
;
; The service cannot be stopped by anyone but itself, so each hook asks it to let go over its
; pipe. Both hooks run before the template's own "is the app running" check, which is why that
; check finds nothing: the service has ended and has ended the app with it.

!macro NSIS_HOOK_PREINSTALL
  ; An update replaces the exe the service runs from. Nothing to ask on a first install.
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --prepare-update' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A block is running, so Site Blocker cannot be updated now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then run the installer again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      MessageBox MB_ICONSTOP "Site Blocker could not be stopped for the update. See C:\ProgramData\SiteBlocker\service.log." /SD IDOK
      SetErrorLevel 1
      Quit
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Registers the service, locks it down and starts it; it starts the app for whoever is logged on.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --install' $0
  ${If} $0 <> 0
    MessageBox MB_ICONEXCLAMATION "Site Blocker is installed, but its service could not be set up, so it will not block anything. See %LOCALAPPDATA%\SiteBlocker\app.log." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; An update replaces the files only (and has asked the service already, in PREINSTALL).
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A timed block is still running, so Site Blocker cannot be uninstalled now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then uninstall again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      MessageBox MB_ICONEXCLAMATION "The Site Blocker service could not be removed. Some files may be left behind in the install folder." /SD IDOK
    ${EndIf}
  ${EndIf}
!macroend
