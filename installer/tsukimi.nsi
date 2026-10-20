; NSIS installer for the portable tsukimi Windows bundle.
;
; Build (from the repository root, after build-aux/windows/package.sh):
;   makensis "-DAPP_VERSION=26.9.2" installer/tsukimi.nsi
;
; The installer expects the bundle directory `tsukimi-x86_64-windows/` to be
; present in the current working directory. Everything is installed per-user
; (no administrator rights required); the bundle itself remains relocatable.

Unicode true

!ifndef APP_VERSION
  !define APP_VERSION "26.9.2"
!endif

!define APP_NAME "Tsukimi"
!define APP_PUBLISHER "tsukimi contributors"
!define APP_URL "https://github.com/tsukinaha/tsukimi"
!define APP_EXE "bin\tsukimi.exe"
!define BUNDLE_DIR "tsukimi-x86_64-windows"
!define REG_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
!define REG_ROOT "HKCU"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "tsukimi-x86_64-windows-setup.exe"
InstallDir "$LOCALAPPDATA\${APP_NAME}"
InstallDirRegKey "${REG_ROOT}" "${REG_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma

; ---------------------------------------------------------------------------
; Modern UI
; ---------------------------------------------------------------------------
!include "MUI2.nsh"
!include "LogicLib.nsh"

!define MUI_ABORTWARNING
!define MUI_ICON "${BUNDLE_DIR}\tsukimi.ico"
!define MUI_UNICON "${BUNDLE_DIR}\tsukimi.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------------------
; Installer
; ---------------------------------------------------------------------------
Section "Install"
  SectionIn RO

  ; The whole portable bundle (bin/, lib/, share/, etc/, tsukimi.ico).
  SetOutPath "$INSTDIR"
  File /r "${BUNDLE_DIR}\*.*"

  ; Keep the uninstaller inside the bundle-free install location.
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\Uninstall ${APP_NAME}.lnk" "$INSTDIR\uninstall.exe"
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  ; Add/Remove Programs entry (per-user).
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr ${REG_ROOT} "${REG_KEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegDWORD ${REG_ROOT} "${REG_KEY}" "NoModify" 1
  WriteRegDWORD ${REG_ROOT} "${REG_KEY}" "NoRepair" 1
SectionEnd

; ---------------------------------------------------------------------------
; Uninstaller
; ---------------------------------------------------------------------------
Section "Uninstall"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\Uninstall ${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"

  ; The bundle contains read-only DLLs after extraction on some systems.
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\lib"
  RMDir /r "$INSTDIR\share"
  RMDir /r "$INSTDIR\etc"
  Delete "$INSTDIR\tsukimi.ico"
  RMDir "$INSTDIR"

  DeleteRegKey ${REG_ROOT} "${REG_KEY}"
SectionEnd
