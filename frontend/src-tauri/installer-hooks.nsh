; installer-hooks.nsh
;
; If a `models\` folder sits next to the installer executable (i.e. the
; installer was run from an extracted "with-models" distribution zip),
; copy those files straight into the app-data models directory that
; models_seed.rs expects, so the app finds them already present on first
; launch and skips the "download a model" onboarding step / first-run
; network fetch.
;
; This runs as a runtime file copy (SHFileOperation via CopyFiles), not a
; compile-time embed, so it is not subject to the ~2GB single-file / total
; working-set limits that the NSIS/WiX bundlers themselves hit when asked
; to embed multi-gigabyte model files directly into the installer image.
;
; $APPDATA\com.meetily.ai matches tauri::path::PathResolver::app_data_dir(),
; which joins dirs::data_dir() (== %APPDATA%) with the `identifier` from
; tauri.conf.json.

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  Push $1

  IfFileExists "$EXEDIR\models\*.*" 0 skip_bundled_models
    DetailPrint "Installing bundled AI models (this may take a few minutes)..."
    CreateDirectory "$APPDATA\com.meetily.ai\models"
    CopyFiles "$EXEDIR\models\*.*" "$APPDATA\com.meetily.ai\models\"

    ; Mirror models_seed.rs::link_summary_models: the summary engine reads
    ; GGUF models from models\summary\, not models\ itself.
    CreateDirectory "$APPDATA\com.meetily.ai\models\summary"
    FindFirst $0 $1 "$APPDATA\com.meetily.ai\models\*.gguf"
    gguf_loop:
      StrCmp $1 "" gguf_done
      CopyFiles /SILENT "$APPDATA\com.meetily.ai\models\$1" "$APPDATA\com.meetily.ai\models\summary\$1"
      FindNext $0 $1
      Goto gguf_loop
    gguf_done:
    FindClose $0
  skip_bundled_models:

  Pop $1
  Pop $0
!macroend
