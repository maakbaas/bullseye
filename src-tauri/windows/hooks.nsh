; Hooks into Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).
; The work itself is in the app (src/setup.rs); these only run it and act on its exit code:
; 0 done, 1 failed, 2 refused because a timed block is running.

!macro NSIS_HOOK_POSTINSTALL
  ; Registers the login task and starts the app, as a logon would.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --install' $0
  ${If} $0 <> 0
    MessageBox MB_ICONEXCLAMATION "Site Blocker is installed, but its login task could not be registered, so it will not start by itself after a restart. See %LOCALAPPDATA%\SiteBlocker\launcher.log." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; An update replaces the files only: the task, the saved block and a running block all stay.
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A timed block is still running, so Site Blocker cannot be uninstalled now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then uninstall again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      MessageBox MB_ICONEXCLAMATION "The SiteBlocker login task could not be removed. You can delete it in Task Scheduler." /SD IDOK
    ${EndIf}
  ${EndIf}
!macroend
