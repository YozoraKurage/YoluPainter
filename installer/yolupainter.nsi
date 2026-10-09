; YoluPainter の Windows インストーラー（NSIS。利用者ごとのインストールで、管理者権限は要らない）。
; ビルドは `cargo xtask installer --target x86_64-pc-windows-msvc`。xtask が -D で次を渡す。
;   VERSION          SemVer の文字列（プレリリースの識別子つき。exe のバージョン情報の ProductVersion と同じ値）
;   VERSION_NUMERIC  a.b.c.0（バージョン情報の数字の欄）
;   STAGE            入れるファイル（exe・コマンドラインの exe（yolupainter-cli.exe）・LICENSE・README.md・README.en.md・THIRD_PARTY.md・DEPENDENCIES.md・THIRD_PARTY_LICENSES.txt と、docs\ の文書）を集めたフォルダ
;   OUTFILE          書き出すインストーラー
;   ICON             アイコン（crates/yolu-app/assets/logo/yolupainter.ico。exe と同じロゴ）
;
; コマンドライン
;   インストーラー  /S            無音。画面を出さない
;                   /ASSOC=1|0    .ylp の関連付けを付ける・付けない（無音のとき。省略は今の状態のまま、初めてなら付けない）
;                   /RUN          入れ終わったらアプリを起こす（アプリの更新が使う）。待ちの上限で何も変えずに終わるときも、今入っているアプリを起こし直す
;                   /D=PATH       入れ先（最後に置く。省略は前の入れ先、初めては %LOCALAPPDATA%\Programs\YoluPainter）
;   アンインストーラー  /S           無音
;                       /DELETEDATA  アプリが作り直せるデータ（設定・ウィンドウの配置・復旧・クラッシュの記録・サムネイルのキャッシュ）も消す（無音のとき。省略は残す）。
;                                    利用者が作った物（個人のライブラリ・ブラシ・サブツール・グラデーション・カラーセット・表示のプリセット・ポーズのプリセット）は、どちらでも消さない
;
; 実行中のアプリは終了させない。exe が使われている間（アプリと、MCP の接続などが使っている yolupainter-cli.exe のどちらでも）は待つ
; （無音は 60 秒まで。超えたら何も変えずに終わり、終了コード 5。/RUN が付いていれば今入っているアプリの exe を起こし直す）。
; 文書は DocFiles の一覧から $INSTDIR\docs・$INSTDIR\docs\en に入れ、入れた名前を docs\.installed に記録する。更新は、前の版の記録にある文書を先に消すので、
; 前の版にだけあった文書が残らない。アンインストールは一覧の文書と記録を消し、フォルダは空のときだけ消す（利用者が docs に置いたファイルは消さない）。

; インストーラーの外側は 32 ビット（Unicode）。公式の Windows 版 NSIS には 64 ビットの外側の部品が無いため。入れるアプリは 64 ビット（x86_64）。
; 書くのは利用者の領域（HKCU・LocalAppData）だけで、32 ビットの外側でも WOW64 の向け替えを受けない。
Target x86-unicode
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
!define HOMEPAGE "https://github.com/YozoraKurage/YoluPainter"
!define EXE "yolupainter.exe"
; コマンドラインと、標準入出力の MCP の中継（アプリと同じ入れ先・同じ版。PATH は変えない）
!define CLI_EXE "yolupainter-cli.exe"
!define UNINSTALLER "uninstall.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"
!define EXT_KEY "Software\Classes\.ylp"
!define PROGID "YoluPainter.Project"
!define PROGID_KEY "Software\Classes\${PROGID}"
; 入れた文書の名前の記録（次の更新が、前の版にだけあった文書を消すのに読む）。
!define DOC_LIST "$INSTDIR\docs\.installed"

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
LangString CLOSE_APP ${LANG_JAPANESE} "YoluPainter（と、yolupainter-cli を使っているアプリ）を終了してから、再試行してください。"
LangString CLOSE_APP ${LANG_ENGLISH} "Close YoluPainter (and any app using yolupainter-cli), then retry."
LangString DELETE_DATA ${LANG_JAPANESE} "設定と復旧のデータも削除しますか？"
LangString DELETE_DATA ${LANG_ENGLISH} "Also delete settings and recovery data?"

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

; /RUN が付いていれば、今入っている exe を起こす（入れ終わったとき、と、待ちの上限で何も変えずに終わるとき）。exe が無ければ何もしない。
; 更新のアプリは、インストーラーを起こしたあと自分を閉じる。別のウィンドウが exe を使い続けていてインストーラーが諦めても、利用者をウィンドウの無い状態に置かない。
Function RunIfRequested
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RUN" $R1
  ${IfNot} ${Errors}
  ${AndIf} ${FileExists} "$INSTDIR\${EXE}"
    SetOutPath "$INSTDIR"
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

; exe が使われている間は待つ（動いている exe は書き込みで開けない）。入れる・消すの前に呼ぶ。
; アプリの exe と、コマンドラインの exe（MCP のクライアントが動かし続けていることがある）の両方を見る。
; ON_TIMEOUT は、待ちの上限を超えて終わる直前にする命令（入れるときだけ、/RUN があれば今の exe を起こす）。
!macro CheckUnlocked EXENAME
  ClearErrors
  ${If} ${FileExists} "$INSTDIR\${EXENAME}"
    FileOpen $R8 "$INSTDIR\${EXENAME}" a
    ${If} ${Errors}
      StrCpy $R6 1
    ${Else}
      FileClose $R8
    ${EndIf}
  ${EndIf}
!macroend
!macro DefineWaitUnlock PREFIX ON_TIMEOUT
Function ${PREFIX}WaitUnlock
  StrCpy $R9 0
  ${Do}
    StrCpy $R6 0
    !insertmacro CheckUnlocked "${EXE}"
    !insertmacro CheckUnlocked "${CLI_EXE}"
    ${If} $R6 == 0
      ${Break}
    ${EndIf}
    ${If} ${Silent}
      IntOp $R9 $R9 + 1
      ${If} $R9 >= ${WAIT_STEPS}
        ${ON_TIMEOUT}
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
!insertmacro DefineWaitUnlock "" "Call RunIfRequested"
!insertmacro DefineWaitUnlock "un." "Nop"

; 入れる文書（STAGE\docs\…）の一覧。入れる・消す・記録の 3 つがこの 1 つの一覧を使う。
; xtask の BUNDLED_DOCS と同じ名前の集まりで、xtask の試験が突き合わせる（文書を足したら、両方に足す）。
!macro DocFiles ACTION
  !insertmacro ${ACTION} "docs" "GUIDE.md"
  !insertmacro ${ACTION} "docs" "GUIDE_START.md"
  !insertmacro ${ACTION} "docs" "GUIDE_PAINT.md"
  !insertmacro ${ACTION} "docs" "GUIDE_SELECT.md"
  !insertmacro ${ACTION} "docs" "GUIDE_LAYERS.md"
  !insertmacro ${ACTION} "docs" "GUIDE_SETTINGS.md"
  !insertmacro ${ACTION} "docs" "GUIDE_KEYS.md"
  !insertmacro ${ACTION} "docs" "CLI.md"
  !insertmacro ${ACTION} "docs" "MCP.md"
  !insertmacro ${ACTION} "docs" "UNITY.md"
  !insertmacro ${ACTION} "docs" "INSTALL.md"
  !insertmacro ${ACTION} "docs" "BUILDING.md"
  !insertmacro ${ACTION} "docs" "PSD.md"
  !insertmacro ${ACTION} "docs" "BRUSH.md"
  !insertmacro ${ACTION} "docs" "BRUSH_IMPORT.md"
  !insertmacro ${ACTION} "docs" "SUBTOOLS.md"
  !insertmacro ${ACTION} "docs" "GRADIENT_MAP.md"
  !insertmacro ${ACTION} "docs" "PREVIEW.md"
  !insertmacro ${ACTION} "docs" "RECOVERY.md"
  !insertmacro ${ACTION} "docs" "SAVE_FOR_DISTRIBUTION.md"
  !insertmacro ${ACTION} "docs" "YLP_FORMAT.md"
  !insertmacro ${ACTION} "docs" "YLP_DECISIONS.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_START.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_PAINT.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_SELECT.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_LAYERS.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_SETTINGS.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_KEYS.md"
  !insertmacro ${ACTION} "docs\en" "CLI.md"
  !insertmacro ${ACTION} "docs\en" "MCP.md"
  !insertmacro ${ACTION} "docs\en" "UNITY.md"
  !insertmacro ${ACTION} "docs\en" "INSTALL.md"
  !insertmacro ${ACTION} "docs\en" "BUILDING.md"
  !insertmacro ${ACTION} "docs" "LIVELINK.md"
  !insertmacro ${ACTION} "docs\en" "LIVELINK.md"
  !insertmacro ${ACTION} "docs" "GUIDE_FILL.md"
  !insertmacro ${ACTION} "docs" "GUIDE_PATHS.md"
  !insertmacro ${ACTION} "docs" "GUIDE_3D.md"
  !insertmacro ${ACTION} "docs" "GUIDE_FILES.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_FILL.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_PATHS.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_3D.md"
  !insertmacro ${ACTION} "docs\en" "GUIDE_FILES.md"
!macroend
!macro InstallDoc DIR NAME
  SetOutPath "$INSTDIR\${DIR}"
  File "${STAGE}\${DIR}\${NAME}"
!macroend
!macro ListDoc DIR NAME
  FileWrite $R0 "${DIR}\${NAME}$\r$\n"
!macroend
!macro DeleteDoc DIR NAME
  Delete "$INSTDIR\${DIR}\${NAME}"
!macroend

; 前の版が入れた文書を、記録にある名前で消す（今の版に無い文書を残さない）。消すのは docs\ の下の .md で、.. を含まない名前だけ
; （記録が書き換えられていても、入れ先の外には触れない）。利用者が docs\ に置いたファイルは、記録に無いので消えない。
Function RemoveOldDocs
  ClearErrors
  FileOpen $R0 "${DOC_LIST}" r
  ${If} ${Errors}
    Return
  ${EndIf}
  ${Do}
    ClearErrors
    FileRead $R0 $R1
    ${If} ${Errors}
      ${Break}
    ${EndIf}
    StrCpy $R2 $R1 1 -1
    ${If} $R2 == "$\n"
      StrCpy $R1 $R1 -1
    ${EndIf}
    StrCpy $R2 $R1 1 -1
    ${If} $R2 == "$\r"
      StrCpy $R1 $R1 -1
    ${EndIf}
    StrCpy $R2 $R1 5
    StrCpy $R3 $R1 3 -3
    ${If} $R2 == "docs\"
    ${AndIf} $R3 == ".md"
      StrLen $R4 $R1
      StrCpy $R5 0
      StrCpy $R6 1
      ${While} $R5 < $R4
        StrCpy $R7 $R1 2 $R5
        ${If} $R7 == ".."
          StrCpy $R6 0
          ${Break}
        ${EndIf}
        IntOp $R5 $R5 + 1
      ${EndWhile}
      ${If} $R6 == 1
        Delete "$INSTDIR\$R1"
      ${EndIf}
    ${EndIf}
  ${Loop}
  FileClose $R0
FunctionEnd

; 今の版の文書の名前を記録する（入れる前に書くので、途中で止まっても、次の更新が前の記録から消せる）。
Function WriteDocList
  CreateDirectory "$INSTDIR\docs"
  ClearErrors
  FileOpen $R0 "${DOC_LIST}" w
  ${IfNot} ${Errors}
    !insertmacro DocFiles ListDoc
    FileClose $R0
  ${EndIf}
FunctionEnd

Section "$(NAME_CORE)" SecCore
  SectionIn RO
  Call WaitUnlock
  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${STAGE}\${EXE}"
  File "${STAGE}\${CLI_EXE}"
  File "${STAGE}\LICENSE"
  File "${STAGE}\README.md"
  File "${STAGE}\README.en.md"
  File "${STAGE}\THIRD_PARTY.md"
  File "${STAGE}\DEPENDENCIES.md"
  File "${STAGE}\THIRD_PARTY_LICENSES.txt"
  Call RemoveOldDocs
  Call WriteDocList
  !insertmacro DocFiles InstallDoc
  ; ショートカットの作業フォルダ（$OUTDIR）を入れ先に戻す。
  SetOutPath "$INSTDIR"
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
  Call RunIfRequested
FunctionEnd

; アプリが入れたファイルだけを消す（入れ先の中を丸ごとは消さない）。入れ先は、空になったときだけ消える。
Section "Uninstall"
  Call un.WaitUnlock
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\${CLI_EXE}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\README.en.md"
  Delete "$INSTDIR\THIRD_PARTY.md"
  Delete "$INSTDIR\DEPENDENCIES.md"
  Delete "$INSTDIR\THIRD_PARTY_LICENSES.txt"
  !insertmacro DocFiles DeleteDoc
  Delete "${DOC_LIST}"
  RMDir "$INSTDIR\docs\en"
  RMDir "$INSTDIR\docs"
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

  ; アプリが作り直せるデータは、消すかを聞く（無音では /DELETEDATA のときだけ消す）。
  ; 消すのは名指しした物だけ。利用者が作った物（%APPDATA%\YoluPainter の Library・brushes・subtools・gradients・hide_presets・pose_presets・colorsets・actions）と、
  ; 後の版が足した知らないファイル・フォルダは、どちらの答えでも消さない（フォルダも、空になったときだけ消す）。
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
  ; %APPDATA%\YoluPainter: 設定・ウィンドウの配置・復旧・クラッシュの記録（と、書き込み途中で残った一時ファイル）
  Delete "$APPDATA\${PRODUCT}\settings.conf"
  Delete "$APPDATA\${PRODUCT}\recovery.conf"
  Delete "$APPDATA\${PRODUCT}\update.conf"
  Delete "$APPDATA\${PRODUCT}\layout.json"
  Delete "$APPDATA\${PRODUCT}\places.conf"
  ; 書き込み途中の一時ファイル: 今の版は .{名前}.{pid}-{番号}.pending~、前の版は {名前}.{pid}.pending
  Delete "$APPDATA\${PRODUCT}\.*.pending~"
  Delete "$APPDATA\${PRODUCT}\*.pending"
  RMDir /r "$APPDATA\${PRODUCT}\recovery"
  RMDir /r "$APPDATA\${PRODUCT}\logs"
  RMDir "$APPDATA\${PRODUCT}"
  ; %LOCALAPPDATA%\YoluPainter: 作り直せる写し（updates は上で消した）
  RMDir /r "$LOCALAPPDATA\${PRODUCT}\thumbnails"
  RMDir /r "$LOCALAPPDATA\${PRODUCT}\LiveLink"
  RMDir "$LOCALAPPDATA\${PRODUCT}"
  keep_data:
SectionEnd
