; NSIS hooks for LLM Translate (DESIGN 5.7 / 6).
; Saved as UTF-8 with BOM so NSIS Unicode compiles Chinese strings cleanly.

!macro NSIS_HOOK_POSTINSTALL
  ; Notify Windows Shell so upgraded shortcuts and exe icons refresh promptly.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Preserve autostart, settings and downloaded models across upgrade reinstalls.
  ${If} $UpdateMode <> 1
    Push $R0
    Push $R1

    ; Tauri's template already removes HKCU\...\Run\${PRODUCTNAME}; also remove
    ; Task Manager's per-user startup approval record if present.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"

    SetShellVarContext current

    ; Read the mirrored modelsDir (UTF-16 REG_SZ) before cleaning app data.
    ReadRegStr $R0 HKCU "Software\LLMTranslate" "ModelsDir"
    ${If} $R0 == ""
      StrCpy $R0 "$LOCALAPPDATA\LLMTranslate\models"
    ${EndIf}
    DeleteRegKey HKCU "Software\LLMTranslate"

    ; When "删除应用程序数据" is checked on the uninstall confirmation page,
    ; also remove our settings and logs (models are asked separately below).
    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "$APPDATA\LLMTranslate"
      RMDir /r "$LOCALAPPDATA\LLMTranslate\logs"
    ${EndIf}

    ; Check whether any known model file (or partial download) exists in $R0.
    StrCpy $R1 0
    ${If} ${FileExists} "$R0\Index-Translate-2B.Q4_K_M.gguf"
    ${OrIf} ${FileExists} "$R0\Index-Translate-2B.Q4_K_M.gguf.part"
    ${OrIf} ${FileExists} "$R0\Hy-MT2-7B.Q4_K_M.gguf"
    ${OrIf} ${FileExists} "$R0\Hy-MT2-7B.Q4_K_M.gguf.part"
    ${OrIf} ${FileExists} "$R0\Index-Translate-9B.Q4_K_M.gguf"
    ${OrIf} ${FileExists} "$R0\Index-Translate-9B.Q4_K_M.gguf.part"
      StrCpy $R1 1
    ${EndIf}

    ${If} $R1 = 1
    ${AndIfNot} ${Silent}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 \
        "检测到已下载的翻译模型：$\r$\n$R0$\r$\n$\r$\n是否同时删除已下载的模型文件？$\r$\n（选“否”将保留模型，重装后无需重新下载）" \
        IDNO _llmt_keep_models
      ; Delete only known model files so a custom shared directory is never harmed.
      Delete "$R0\Index-Translate-2B.Q4_K_M.gguf"
      Delete "$R0\Index-Translate-2B.Q4_K_M.gguf.part"
      Delete "$R0\Hy-MT2-7B.Q4_K_M.gguf"
      Delete "$R0\Hy-MT2-7B.Q4_K_M.gguf.part"
      Delete "$R0\Index-Translate-9B.Q4_K_M.gguf"
      Delete "$R0\Index-Translate-9B.Q4_K_M.gguf.part"
      RMDir "$R0"
      _llmt_keep_models:
    ${EndIf}

    ; Remove the parent data folder only if completely empty.
    RMDir "$LOCALAPPDATA\LLMTranslate"

    Pop $R1
    Pop $R0
  ${EndIf}
!macroend