; YoluPainter の Windows インストーラー（NSIS。利用者ごとのインストールで、管理者権限は要らない）。
; ビルドは `cargo xtask installer --target x86_64-pc-windows-msvc`。xtask が -D で次を渡す。
;   VERSION          SemVer の文字列（プレリリースの識別子つき。exe のバージョン情報の ProductVersion と同じ値）
;   VERSION_NUMERIC  a.b.c.0（バージョン情報の数字の欄）
;   STAGE            入れるファイル（exe・LICENSE・README.md・THIRD_PARTY.md・DEPENDENCIES.md・THIRD_PARTY_LICENSES.txt）を集めたフォルダ
;   OUTFILE          書き出すインストーラー
;   ICON             アイコン（crates/yolu-app/assets/logo/yolupainter.ico。exe と同じロゴ）
;
; コマンドライン
;   インストーラー  /S            無音。画面を出さない
;                   /ASSOC=1|0    .ylp の関連付けを付ける・付けない（無音のとき。省略は今の状態のまま、初めてなら付けない）
;                   /RUN          入れ終わったらアプリを起こす（アプリの更新が使う）
;                   /D=PATH       入れ先（最後に置く。省略は前の入れ先、初めては %LOCALAPPDATA%\Programs\YoluPainter）
;   アンインストーラー  /S           無音
;                       /DELETEDATA  設定・ブラシ・復旧のデータも消す（無音のとき。省略は残す）
;
; 実行中のアプリは終了させない。exe が使われている間は待つ（無音は 60 秒まで。超えたら何も変えずに終わり、終了コード 5）。

; アプリは 64 ビット（x86_64）だけなので、インストーラーも 64 ビット（Unicode）で作る。Wine でも 32 ビットの環境なしで試せる。
Target amd64-unicode
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma
CRCCheck on

!ifndef VERSION
  !error "VERSION を -D で渡してください"
!endif
!ifndef VERSION_NUMERIC
  !error "VERSION_NUMERIC を -D で渡してください"
!endif
!ifndef STAGE
  !error "STAGE を -D で渡してください"
!endif
!ifndef OUTFILE
  !error "OUTFILE を -D で渡してください"
!endif
!ifndef ICON
  !error "ICON を -D で渡してください"
!endif

; 動いている exe を待つ長さ（無音のとき）。0.5 秒を 1 回として数え、超えたら終了コード 5。通常は 60 秒。
; 試験（tools/test-installer.py）だけが -DWAIT_STEPS=<回数> で短くする。
!ifndef WAIT_STEPS
  !define WAIT_STEPS 120
!endif

!define PRODUCT "YoluPainter"
!define PUBLISHER "Yozolab"
!define HOMEPAGE "https://github.com/YozoraKurage/YoluPainter-rs"
!define EXE "yolupainter.exe"
!define UNINSTALLER "uninstall.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"
!define EXT_KEY "Software\Classes\.ylp"
!define PROGID "YoluPainter.Project"
!define PROGID_KEY "Software\Classes\${PROGID}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"

!insertmacro GetParameters
!insertmacro GetOptions
!insertmacro un.GetParameters
!insertmacro un.GetOptions

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
BrandingText "${PRODUCT} ${VERSION}"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_FINISHPAGE_NOREBOOTSUPPORT
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"

!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; 先に書いた言語が、利用者の表示言語に合うものが無いときの既定になる。
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "English"

LangString NAME_CORE ${LANG_JAPANESE} "YoluPainter"
LangString NAME_CORE ${LANG_ENGLISH} "YoluPainter"
LangString NAME_ASSOC ${LANG_JAPANESE} ".ylp ファイルを YoluPainter で開く"
LangString NAME_ASSOC ${LANG_ENGLISH} "Open .ylp files with YoluPainter"
LangString CLOSE_APP ${LANG_JAPANESE} "YoluPainter を終了してから、再試行してください。"
LangString CLOSE_APP ${LANG_ENGLISH} "Close YoluPainter, then retry."
LangString DELETE_DATA ${LANG_JAPANESE} "設定・ブラシ・復旧のデータも削除しますか？"
LangString DELETE_DATA ${LANG_ENGLISH} "Also delete settings, brushes and recovery data?"

; exe のバージョン情報（アプリの exe と同じ製品名・版。コード署名の条件）。
VIProductVersion "${VERSION_NUMERIC}"
!macro VersionKeys LANG
  VIAddVersionKey /LANG=${LANG} "ProductName" "${PRODUCT}"
  VIAddVersionKey /LANG=${LANG} "ProductVersion" "${VERSION}"
  VIAddVersionKey /LANG=${LANG} "FileVersion" "${VERSION}"
  VIAddVersionKey /LANG=${LANG} "FileDescription" "${PRODUCT} Setup"
  VIAddVersionKey /LANG=${LANG} "CompanyName" "${PUBLISHER}"
  VIAddVersionKey /LANG=${LANG} "LegalCopyright" "Copyright (c) 2026 ${PUBLISHER}"
!macroend
!insertmacro VersionKeys ${LANG_JAPANESE}
!insertmacro VersionKeys ${LANG_ENGLISH}

; exe が使われている間は待つ（動いている exe は書き込みで開けない）。入れる・消すの前に呼ぶ。
!macro DefineWaitUnlock PREFIX
Function ${PREFIX}WaitUnlock
  StrCpy $R9 0
  ${Do}
    ClearErrors
    ${IfNot} ${FileExists} "$INSTDIR\${EXE}"
      ${Break}
    ${EndIf}
    FileOpen $R8 "$INSTDIR\${EXE}" a
    ${IfNot} ${Errors}
      FileClose $R8
      ${Break}
    ${EndIf}
    ${If} ${Silent}
      IntOp $R9 $R9 + 1
      ${If} $R9 >= ${WAIT_STEPS}
        SetErrorLevel 5
        Abort
      ${EndIf}
      Sleep 500
    ${Else}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(CLOSE_APP)" /SD IDCANCEL IDRETRY retry
      Abort
      retry:
    ${EndIf}
  ${Loop}
FunctionEnd
!macroend
!insertmacro DefineWaitUnlock ""
!insertmacro DefineWaitUnlock "un."

Section "$(NAME_CORE)" SecCore
  SectionIn RO
  Call WaitUnlock
  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${STAGE}\${EXE}"
  File "${STAGE}\LICENSE"
  File "${STAGE}\README.md"
  File "${STAGE}\THIRD_PARTY.md"
  File "${STAGE}\DEPENDENCIES.md"
  File "${STAGE}\THIRD_PARTY_LICENSES.txt"
  WriteUninstaller "$INSTDIR\${UNINSTALLER}"

  CreateShortCut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\${UNINSTALLER}"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\${UNINSTALLER}" /S'
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${HOMEPAGE}"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  SectionGetSize ${SecCore} $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "$(NAME_ASSOC)" SecAssoc
  WriteRegStr HKCU "${EXT_KEY}" "" "${PROGID}"
  WriteRegStr HKCU "${PROGID_KEY}" "" "YoluPainter Project"
  WriteRegStr HKCU "${PROGID_KEY}\DefaultIcon" "" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${PROGID_KEY}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  ; 関連付けが変わったことをエクスプローラーに知らせる（SHCNE_ASSOCCHANGED）。
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

; セクションの番号（SecAssoc）を使うので、セクションの後ろに置く。
Function .onInit
  ${If} ${Silent}
    ; 無音の関連付けは、指定が無ければ今の状態のまま（更新で、利用者の選んだ状態を変えない）。
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/ASSOC=" $R1
    ${If} ${Errors}
      ReadRegStr $R2 HKCU "${EXT_KEY}" ""
      ${If} $R2 != "${PROGID}"
        !insertmacro UnselectSection ${SecAssoc}
      ${EndIf}
    ${ElseIf} $R1 == "0"
      !insertmacro UnselectSection ${SecAssoc}
    ${EndIf}
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RUN" $R1
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

; アプリが入れたファイルだけを消す（入れ先の中を丸ごとは消さない）。入れ先は、空になったときだけ消える。
Section "Uninstall"
  Call un.WaitUnlock
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\THIRD_PARTY.md"
  Delete "$INSTDIR\DEPENDENCIES.md"
  Delete "$INSTDIR\THIRD_PARTY_LICENSES.txt"
  Delete "$INSTDIR\${UNINSTALLER}"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"

  DeleteRegKey HKCU "${UNINST_KEY}"
  ; 関連付けは、自分が付けたものだけを外す（ほかのアプリに替えられていたら触らない）。
  ReadRegStr $R2 HKCU "${EXT_KEY}" ""
  ${If} $R2 == "${PROGID}"
    DeleteRegKey HKCU "${EXT_KEY}"
  ${EndIf}
  DeleteRegKey HKCU "${PROGID_KEY}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  ; 更新で落としたインストーラーの置き場（利用者のデータではない）
  RMDir /r "$LOCALAPPDATA\${PRODUCT}\updates"
  RMDir "$LOCALAPPDATA\${PRODUCT}"

  ; 利用者の設定・ブラシ・復旧のデータは、消すかを聞く（無音では /DELETEDATA のときだけ消す）。
  ${un.GetParameters} $R0
  ClearErrors
  ${un.GetOptions} $R0 "/DELETEDATA" $R1
  ${If} ${Errors}
    ${IfNot} ${Silent}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(DELETE_DATA)" IDNO keep_data
      Goto delete_data
    ${EndIf}
    Goto keep_data
  ${EndIf}
  delete_data:
  RMDir /r "$APPDATA\${PRODUCT}"
  RMDir /r "$LOCALAPPDATA\${PRODUCT}"
  keep_data:
SectionEnd
