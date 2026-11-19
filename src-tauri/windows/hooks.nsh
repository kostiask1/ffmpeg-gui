!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Checking AI model: ihnatov-yuri/parakeet-tdt-0.6b-v3-uk..."
  DetailPrint "Model credit: Yuri Ihnatov (https://ihnatov.nl), fine-tuned from NVIDIA Parakeet TDT 0.6B v3 (CC BY 4.0)"

  IfFileExists "$PROFILE\.cache\huggingface\hub\models--ihnatov-yuri--parakeet-tdt-0.6b-v3-uk\snapshots\5c4bcfaca501587beaedfd23b72a678bf7194de1\model.safetensors" parakeet_already_installed 0
  IfFileExists "$LOCALAPPDATA\Audio Forge\models\parakeet-tdt-0.6b-v3-uk\model.safetensors" parakeet_already_installed 0

  DetailPrint "AI model not found in default cache path. Checking and installing ihnatov-yuri/parakeet-tdt-0.6b-v3-uk..."
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\third-party\parakeet\ensure-parakeet-model.ps1"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Warning: AI model installer exited with code $0. You can retry later or let Audio Forge download it on first transcription."
  ${EndIf}
  Goto parakeet_check_done

  parakeet_already_installed:
  DetailPrint "AI model ihnatov-yuri/parakeet-tdt-0.6b-v3-uk is already installed. Skipping download."

  parakeet_check_done:
!macroend
