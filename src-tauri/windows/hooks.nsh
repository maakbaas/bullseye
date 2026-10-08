; Hooks into Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).
; The installer runs as an administrator (installMode "perMachine"). The work itself is in the
; app (src/setup.rs); these only run it and act on its exit code:
; 0 done, 1 failed, 2 refused because a session is locked.
;
; Bullseye is only installed on a clean machine: there is no update path.

!macro NSIS_HOOK_POSTINSTALL
  ; Registers the service, locks it down and starts it; it starts the app for whoever is logged on.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --install' $0
  ${If} $0 <> 0
    MessageBox MB_ICONEXCLAMATION "Bullseye is installed, but its service could not be set up, so it will not block anything. See %LOCALAPPDATA%\bullseye\app.log." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; The service cannot be stopped by anyone but itself, so this asks it to remove itself over its
  ; pipe. It runs before the template's own "is the app running" check, which is why that check
  ; finds nothing: the service has ended and has ended the app with it.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall' $0
  ${If} $0 = 2
    MessageBox MB_ICONSTOP "A block is locked, so Bullseye cannot be uninstalled now.$\n$\nWait for its lock to end, or end it with the unlock password in the app, then uninstall again." /SD IDOK
    SetErrorLevel 2
    Quit
  ${ElseIf} $0 <> 0
    ; The service is registered but did not answer or did not end. Stopping here, rather than
    ; deleting files under a service that may still be running, avoids a half-removed install.
    ; (No service at all is exit code 0, so that case goes on.)
    MessageBox MB_ICONSTOP "Bullseye's service did not respond, so nothing was removed.$\n$\nSee C:\ProgramData\bullseye\service.log." /SD IDOK
    SetErrorLevel 1
    Quit
  ${EndIf}
!macroend
