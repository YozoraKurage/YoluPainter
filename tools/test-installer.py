#!/usr/bin/env python3
"""NSIS のインストーラー（installer/yolupainter.nsi）を、Wine で無音のまま端から端まで通して確かめる。

本物の yolupainter.exe・yolupainter-cli.exe の代わりに、起動した印を残すだけの小さな exe（mingw でこの場で作る。2 つとも同じ物）を入れる。
確かめること: 無音のインストール（入れ先・ショートカット・アンインストールの登録）、.ylp の関連付けの
有無と更新での引き継ぎ、上書きの更新、/RUN での起こし直し、exe を書き込みで開けない間は待ち、上限を超えたら
何も変えずに終了コード 5 で終わること（アプリの exe でも、MCP のクライアントが動かし続けることがあるコマンドラインの exe でも。
待ちの上限は試験用に短くしたインストーラーで、書き込めない exe を使って確かめる。
上限を超えたときは、/RUN が付いていれば今入っている exe を起こし直し、付いていなければ起こさない）、
アンインストール（入れたファイルだけを消す・利用者のデータは残す・/DELETEDATA では、アプリが作り直せるデータ（設定・ウィンドウの配置・復旧・
クラッシュの記録・サムネイルのキャッシュ・落とした更新）だけを消し、利用者が作った物（個人のライブラリ・ブラシ・サブツール・
グラデーション・カラーセット・表示のプリセット・ポーズのプリセット）と、知らないファイルは残す）。
文書（docs\ と docs\en\）は、入れる・上書きで新しい版の中身になる・前の版にだけあった文書が更新で消える・利用者が docs\ に
置いたファイルと、記録が書き換えられていても入れ先の外には触れない・アンインストールで空になったフォルダだけが消える、を確かめる。
ショートカットの作業フォルダと、/RUN で起こしたアプリの作業フォルダが入れ先であること（文書を入れたあとで入れ先へ戻す SetOutPath の確かめ）も読む。
実際に動いている exe を待つ動きは、Wine が動いている exe の上書きを断るときだけ確かめられる（上書きできる Wine では注意を
出して通る。Windows の実機で確かめる）。画面を出す側（ページの並び・チェック・終了の確かめ）は確かめない。
時間は monotonic で測る（WSL2 では壁時計が数秒戻ることがある）。

必要: makensis・wine（32 ビットの外側を動かすので wine32 も）・x86_64-w64-mingw32-gcc。生成物と専用の Wine 環境は target/test-installer/ だけに置く。
使い方: python3 tools/test-installer.py [--keep]（--keep は終わっても target/test-installer/ を消さない）
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
WORK = ROOT / 'target/test-installer'
PRODUCT_KEY = r'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\YoluPainter'
SCRIPT = ROOT / 'installer/yolupainter.nsi'
# 入れる実行ファイル（アプリと、コマンドライン・MCP サーバー）
EXES = ['yolupainter.exe', 'yolupainter-cli.exe']
ROOT_FILES = ['LICENSE', 'README.md', 'README.en.md', 'THIRD_PARTY.md', 'DEPENDENCIES.md', 'THIRD_PARTY_LICENSES.txt']
# アプリが %APPDATA%\YoluPainter（設定のフォルダ）と %LOCALAPPDATA%\YoluPainter に作る物。どれがどちらかは docs/INSTALL.md の表と同じ。
# 作り直せる物（/DELETEDATA で消える）
REBUILDABLE_ROAMING = ['settings.conf', 'recovery.conf', 'update.conf', 'layout.json', 'places.conf', 'settings.4242.pending', 'layout.json.4242.pending',
                       '.settings.conf.4242-0.pending~', '.layout.json.4242-1.pending~', '.recovery.conf.4242-2.pending~', '.update.conf.4242-3.pending~',
                       '.places.conf.4242-4.pending~',
                       'recovery/session-a/generation-1/data.bin', 'recovery/session.lock', 'logs/crash-1.log', 'logs/session-1.log']
REBUILDABLE_LOCAL = ['thumbnails/ab/cd.png', 'LiveLink/link.sock']
# 利用者が作った物と、知らないファイル（どちらの答えでも消えない）
USER_MADE = ['Library/picture.png', 'Library/sub/material.ylsmart', 'brushes/mine.ylbrush', 'subtools/mine.ylsubtool',
             'gradients/mine.ylgradients', 'hide_presets/mine.ylhide', 'pose_presets/mine.ylpose', 'colorsets/mine.ylcolors']
UNKNOWN = ['future-folder/thing.bin', 'notes.txt']
STAGED_UPDATE = 'updates/yolupainter-0.9.0-x86_64-pc-windows-msvc-setup.exe'
FAKE_APP = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
/* 起動した印（<exe>.ran に引数を 1 行）と、そのときの作業フォルダ（<exe>.cwd）を残す。
   `hold N` なら N 秒動いたままにする（exe が使われている状態を作る）。 */
int main(int argc, char **argv) {
    char path[MAX_PATH + 8];
    char cwd[MAX_PATH];
    GetModuleFileNameA(NULL, path, MAX_PATH);
    if (GetCurrentDirectoryA(MAX_PATH, cwd)) {
        char cwd_path[MAX_PATH + 8];
        strcpy(cwd_path, path);
        strcat(cwd_path, ".cwd");
        FILE *c = fopen(cwd_path, "w");
        if (c) {
            fprintf(c, "%s\n", cwd);
            fclose(c);
        }
    }
    strcat(path, ".ran");
    FILE *f = fopen(path, "a");
    if (f) {
        for (int i = 1; i < argc; i++) fprintf(f, "%s%s", i > 1 ? " " : "", argv[i]);
        fprintf(f, "\n");
        fclose(f);
    }
    if (argc > 2 && strcmp(argv[1], "hold") == 0) Sleep(atoi(argv[2]) * 1000);
    return 0;
}
'''
failures = []


def check(condition, message):
    print(('成功: ' if condition else '失敗: ') + message, flush=True)
    if not condition:
        failures.append(message)


def environment():
    env = os.environ.copy()
    env.update(WINEPREFIX=str(WORK / 'prefix'), WINEARCH='win64', WINEDEBUG='-all',
               WINEDLLOVERRIDES='mscoree,mshtml=')
    env.pop('DISPLAY', None)
    return env


def wine(*args, wait=True, timeout=180):
    command = ['wine', *map(str, args)]
    if not wait:
        return subprocess.Popen(command, env=environment(), stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, cwd=WORK)
    return subprocess.run(command, env=environment(), capture_output=True, text=True,
                          cwd=WORK, timeout=timeout)


def to_unix(windows_path):
    out = subprocess.run(['winepath', '-u', windows_path], env=environment(),
                         capture_output=True, text=True, check=True).stdout.strip()
    return Path(out)


def registry(key, name=None):
    """値を返す（無ければ None）。名前が None なら既定の値。"""
    args = ['reg', 'query', key] + (['/v', name] if name else ['/ve'])
    result = wine(*args)
    if result.returncode != 0:
        return None
    for line in result.stdout.splitlines():
        match = re.match(r'^\s*(.*?)\s+(REG_\w+)\s*(.*)$', line)
        if match:
            return match.group(3).strip()
    return None


def shortcut_working_dir(path):
    """.lnk（MS-SHLLINK）の作業フォルダ（StringData の WorkingDir）を返す。無ければ None。"""
    data = Path(path).read_bytes()
    assert data[:4] == b'L\x00\x00\x00', '.lnk のヘッダーではない'
    flags = struct.unpack_from('<I', data, 0x14)[0]
    unicode_strings = bool(flags & 0x80)
    offset = 0x4C
    if flags & 0x01:  # HasLinkTargetIDList
        offset += 2 + struct.unpack_from('<H', data, offset)[0]
    if flags & 0x02:  # HasLinkInfo
        offset += struct.unpack_from('<I', data, offset)[0]
    # StringData は NAME・RELATIVE_PATH・WORKING_DIR・ARGUMENTS・ICON_LOCATION の順（フラグが立っているものだけ）。
    for bit in (0x04, 0x08, 0x10):
        if not flags & bit:
            continue
        count = struct.unpack_from('<H', data, offset)[0]
        offset += 2
        size = count * 2 if unicode_strings else count
        # Wine は文字列の終わりの NUL まで数えて書くので、取り除く。
        text = data[offset:offset + size].decode('utf-16-le' if unicode_strings else 'latin-1', errors='replace').rstrip('\x00')
        offset += size
        if bit == 0x10:
            return text
    return None


def expand(variable):
    out = wine('cmd', '/c', f'echo %{variable}%').stdout.strip()
    return to_unix(out)


def script_docs():
    """スクリプトの DocFiles の一覧（入れ先からの相対。例: docs\\en\\GUIDE.md）。
    一覧が xtask の配布物の一覧と同じことは xtask の試験が確かめる。ここでは、一覧のとおりに入って・消えるかを確かめる。"""
    text = SCRIPT.read_text(encoding='utf-8-sig')
    return [f'{folder}\\{name}' for folder, name in re.findall(r'!insertmacro \$\{ACTION\} "([^"]+)" "([^"]+)"', text)]


def build_installer(version, numeric, name, wait_steps=None, extra_docs=()):
    """extra_docs は、スクリプトの一覧に足す文書（入れ先からの相対。前の版にだけあった文書を作るための、試験用のスクリプトの写しを使う）。"""
    stage = WORK / f'stage-{version}'
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    source = WORK / 'fake-app.c'
    source.write_text(FAKE_APP)
    subprocess.run(['x86_64-w64-mingw32-gcc', '-O1', '-o', stage / 'yolupainter.exe', source], check=True)
    shutil.copy(stage / 'yolupainter.exe', stage / 'yolupainter-cli.exe')
    # 版ごとに exe の中身を変える（上書きされたかを見分ける）。
    for exe_name in EXES:
        with open(stage / exe_name, 'ab') as exe:
            exe.write(f'build {version}'.encode())
    shutil.copy(ROOT / 'LICENSE', stage / 'LICENSE')
    for file in ROOT_FILES[1:]:
        (stage / file).write_text(f'{file} {version}\n')
    script = SCRIPT
    if extra_docs:
        lines = []
        for doc in extra_docs:
            folder, file = doc.rsplit('\\', 1)
            lines.append(f'  !insertmacro ${{ACTION}} "{folder}" "{file}"\n')
        patched = SCRIPT.read_text(encoding='utf-8-sig')
        marker = '!macroend\n!macro InstallDoc'
        assert marker in patched
        patched = patched.replace(marker, ''.join(lines) + marker, 1)
        script = WORK / f'{name}.nsi'
        script.write_text(patched, encoding='utf-8-sig')
    for doc in [*script_docs(), *extra_docs]:
        path = stage.joinpath(*doc.split('\\'))
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f'{doc} {version}\n')
    output = WORK / name
    command = ['makensis', '-INPUTCHARSET', 'UTF8', '-WX', '-V2', f'-DVERSION={version}',
               f'-DVERSION_NUMERIC={numeric}', f'-DSTAGE={stage}', f'-DOUTFILE={output}']
    command.append(f'-DICON={ROOT / "crates/yolu-app/assets/logo/yolupainter.ico"}')
    if wait_steps is not None:
        command.append(f'-DWAIT_STEPS={wait_steps}')
    subprocess.run([*command, script], check=True, cwd=ROOT)
    return output


def run_silent(setup, *args, timeout=180):
    started = time.monotonic()
    result = wine(setup, '/S', *args, timeout=timeout)
    return result.returncode, time.monotonic() - started


def wait_for(path, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if path.exists():
            return True
        time.sleep(0.25)
    return path.exists()


def wait_gone(path, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not path.exists():
            return True
        time.sleep(0.25)
    return not path.exists()


def lay_out_data(roaming, local, user_made=True):
    """アプリが作る物（作り直せる物・利用者が作った物・知らないファイル・落とした更新）を、中身が自分の名前のファイルで置く。"""
    names = {roaming: REBUILDABLE_ROAMING + ([*USER_MADE, *UNKNOWN] if user_made else []),
             local: [*REBUILDABLE_LOCAL, STAGED_UPDATE]}
    for base, files in names.items():
        for name in files:
            path = base / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)


def all_exist(base, names):
    return all((base / name).is_file() for name in names)


def uninstall(install, *args):
    """実際と同じ流れ（アンインストーラーは自分を一時の場所へ写して動き、元を消す）で消し、終わるのを待つ。"""
    code, _ = run_silent(install / 'uninstall.exe', *args)
    return code, wait_gone(install / 'uninstall.exe')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--keep', action='store_true')
    args = parser.parse_args()
    for tool in ['makensis', 'wine', 'winepath', 'x86_64-w64-mingw32-gcc']:
        if not shutil.which(tool):
            parser.error(f'{tool} が見つかりません')
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    try:
        run(args)
    finally:
        subprocess.run(['wineserver', '-k'], env=environment())
        if not args.keep:
            shutil.rmtree(WORK, ignore_errors=True)
    print(f'失敗 {len(failures)} 件' if failures else '全部成功')
    return 1 if failures else 0


def run(args):
    v1 = build_installer('0.1.0', '0.1.0.0', 'setup-0.1.0.exe')
    v2 = build_installer('0.2.0-rc.1', '0.2.0.0', 'setup-0.2.0.exe')
    # 待ちの上限を 3 秒（0.5 秒 × 6 回）に縮めた版（通常は 60 秒）。待ちの上限の試験だけが使う。
    v3 = build_installer('0.3.0', '0.3.0.0', 'setup-0.3.0-short-wait.exe', wait_steps=6)
    # 前の版にだけあった文書（今のスクリプトの一覧に無い 2 つ）を入れる版。更新で消えることの試験が使う。
    retired = [r'docs\RETIRED.md', r'docs\en\RETIRED.md']
    vold = build_installer('0.0.9', '0.0.9.0', 'setup-0.0.9-retired-docs.exe', extra_docs=retired)
    docs = script_docs()
    wine('wineboot', '-u', timeout=300)
    appdata = expand('APPDATA')
    localdata = expand('LOCALAPPDATA')

    def shortcut():
        # スタートメニューの場所は Windows と Wine で違うので、ユーザーの領域の中から探す
        users = WORK / 'prefix/drive_c/users'
        return next(iter(users.rglob('YoluPainter.lnk')), None)

    # 1. 初めての無音のインストール: 入れ先は既定、関連付けは付けない
    code, _ = run_silent(v1)
    check(code == 0, f'初めての無音のインストールが終わる（終了コード {code}）')
    location = registry(PRODUCT_KEY, 'InstallLocation')
    check(bool(location) and location.lower().endswith(r'\programs\yolupainter'),
          f'既定の入れ先は %LOCALAPPDATA%\\Programs\\YoluPainter（{location}）')
    install = to_unix(location) if location else WORK / 'missing'
    for name in [*EXES, *ROOT_FILES, 'uninstall.exe']:
        check((install / name).is_file(), f'入る: {name}')
    missing = [doc for doc in docs if not (install / Path(*doc.split('\\'))).is_file()]
    check(not missing and len(docs) >= 20, f'文書が一覧のとおり入る（{len(docs)} 件。無いもの {missing}）')
    check(r'docs\CLI.md' in docs and r'docs\en\MCP.md' in docs, 'コマンドラインと MCP の文書も入る一覧にある')
    check((install / 'docs/en/GUIDE.md').read_text() == 'docs\\en\\GUIDE.md 0.1.0\n', '入った文書の中身は、その版の段のもの')
    recorded = (install / 'docs/.installed').read_text().split()
    check(recorded == docs, '入れた文書の名前の記録（docs\\.installed）が、一覧のとおりにある')
    check(shortcut() is not None, 'スタートメニューのショートカットができる')
    # 文書を入れると出力先（$OUTDIR）が docs\en に移るので、SetOutPath で入れ先へ戻していないと、ショートカットの作業フォルダが docs\en になる。
    working = shortcut_working_dir(shortcut()) if shortcut() else None
    check(bool(working) and bool(location) and working.rstrip('\\').lower() == location.lower(),
          f'ショートカットの作業フォルダは入れ先（{working}）')
    check(registry(PRODUCT_KEY, 'DisplayName') == 'YoluPainter', 'アンインストールの登録: 製品名')
    check(registry(PRODUCT_KEY, 'DisplayVersion') == '0.1.0', 'アンインストールの登録: 版')
    check(bool(registry(PRODUCT_KEY, 'Publisher')), 'アンインストールの登録: 作者')
    check('uninstall.exe' in (registry(PRODUCT_KEY, 'QuietUninstallString') or ''), '無音のアンインストールの口がある')
    check(registry(r'HKCU\Software\Classes\.ylp') is None, '無音で省略すると、初めてなら関連付けを付けない')

    # 2. 更新（無音・関連付けの指定なし・/RUN）: 上書き・関連付けなしのまま・起こし直す
    exe = install / 'yolupainter.exe'
    before = exe.read_bytes()
    ran = Path(str(exe) + '.ran')
    ran.unlink(missing_ok=True)
    cwd_record = Path(str(exe) + '.cwd')
    cwd_record.unlink(missing_ok=True)
    code, _ = run_silent(v2, '/RUN')
    check(code == 0, f'更新の無音のインストールが終わる（終了コード {code}）')
    check(exe.read_bytes() != before and exe.read_bytes().endswith(b'build 0.2.0-rc.1'), 'exe が新しい版に置き換わる')
    check((install / 'yolupainter-cli.exe').read_bytes().endswith(b'build 0.2.0-rc.1'), 'コマンドラインの exe も、同じ版に置き換わる')
    check(registry(PRODUCT_KEY, 'DisplayVersion') == '0.2.0-rc.1', '登録の版が新しくなる')
    check(registry(PRODUCT_KEY, 'InstallLocation') == location, '更新で入れ先が変わらない')
    check(wait_for(ran), '/RUN で入れ終わったあとにアプリが起きる')
    started_in = cwd_record.read_text().strip() if cwd_record.exists() else None
    check(bool(started_in) and bool(location) and started_in.rstrip('\\').lower() == location.lower(),
          f'/RUN で起きたアプリの作業フォルダは入れ先（{started_in}）')
    check(registry(r'HKCU\Software\Classes\.ylp') is None, '更新で、無かった関連付けを勝手に付けない')
    check((install / 'docs/GUIDE.md').read_text() == 'docs\\GUIDE.md 0.2.0-rc.1\n'
          and (install / 'README.en.md').read_text() == 'README.en.md 0.2.0-rc.1\n', '更新で、文書も新しい版の中身に置き換わる')

    # 2b. 前の版にだけあった文書は、更新で消える。利用者が docs\ に置いたファイルと、記録が書き換えられていても入れ先の外は、消えない
    code, _ = run_silent(vold)
    check(code == 0 and all((install / Path(*doc.split('\\'))).is_file() for doc in retired),
          '前の版にだけあった文書（試験用）を入れる')
    mine = [install / 'docs/my-notes.md', install / 'docs/en/my-notes.txt', install / 'keep.md', install / 'outside.md']
    for path in mine:
        path.write_text('利用者のファイル')
    with open(install / 'docs/.installed', 'a') as record:
        # 書き換えられた記録: 入れ先の外へ出る名前・docs\ の外の名前・.md でない名前は消さない
        record.write('docs\\..\\outside.md\r\nkeep.md\r\ndocs\\en\\my-notes.txt\r\n')
    code, _ = run_silent(v2)
    check(code == 0, f'更新の無音のインストールが終わる（終了コード {code}）')
    check(not any((install / Path(*doc.split('\\'))).exists() for doc in retired), '前の版にだけあった文書は、更新で消える')
    check(all(path.exists() for path in mine), '記録に無い利用者のファイルと、記録が書き換えられても入れ先の外・docs\\ の外・.md でないものは消さない')
    check(all((install / Path(*doc.split('\\'))).is_file() for doc in docs), '今の版の文書は全部ある')
    check((install / 'docs/.installed').read_text().split() == docs, '記録は今の版の一覧に書き直される')
    for path in mine:
        path.unlink()

    # 3. 関連付けを付ける（/ASSOC=1）と、次の更新（指定なし）でも保たれる。/ASSOC=0 で外れはしない
    code, _ = run_silent(v1, '/ASSOC=1')
    check(code == 0, '/ASSOC=1 の無音のインストールが終わる')
    check(registry(r'HKCU\Software\Classes\.ylp') == 'YoluPainter.Project', '.ylp が YoluPainter.Project に結び付く')
    command = registry(r'HKCU\Software\Classes\YoluPainter.Project\shell\open\command') or ''
    check('yolupainter.exe' in command and '%1' in command, f'開く命令に exe と引数が入る（{command}）')
    code, _ = run_silent(v2)
    check(code == 0 and registry(r'HKCU\Software\Classes\.ylp') == 'YoluPainter.Project',
          '更新（指定なし）が、付けてある関連付けを保つ')
    code, _ = run_silent(v2, '/ASSOC=0')
    check(code == 0 and registry(r'HKCU\Software\Classes\.ylp') == 'YoluPainter.Project',
          '/ASSOC=0 は付けないだけで、付けてあるものを外さない')

    # 4. 動いている exe があっても、終わるまで待って入れ終わる。Wine が動いている exe の上書きを断るときだけ、待ったことを確かめる
    #    （上書きできる Wine では待たないので注意を出すだけ。上限は次の 4b で、書き込めない exe を使って確かめる）
    holder = wine(exe, 'hold', 8, wait=False)
    time.sleep(2)
    code, seconds = run_silent(v1)
    holder.wait()
    check(code == 0, f'動いているアプリがあっても、入れ終わる（終了コード {code}）')
    if seconds >= 4:
        check(True, f'動いている間は待った（{seconds:.1f} 秒）')
    else:
        print(f'注意: この Wine は動いている exe を上書きできるので、待つ動きは確かめられない（{seconds:.1f} 秒。Windows の実機で確かめる）')
    check(exe.read_bytes().endswith(b'build 0.1.0'), '入れ終わると置き換わっている')

    # 4b. 待ちの上限: exe を書き込みで開けない間は待ち、上限を超えたら何も変えずに終了コード 5 で終わる。
    #     開けない状態は読み取り専用で作る（Windows の「使用中」と同じく、書き込みで開くのが失敗する）。上限は試験用に 3 秒。
    #     開けるようになれば、同じインストーラーで入る。
    cli = install / 'yolupainter-cli.exe'
    snapshot = {name: (install / name).read_bytes() for name in
                [*EXES, 'README.md', 'docs/GUIDE.md', 'docs/en/GUIDE.md', 'docs/CLI.md', 'docs/.installed', 'uninstall.exe']}
    version_before = registry(PRODUCT_KEY, 'DisplayVersion')
    ran = Path(str(exe) + '.ran')
    ran.unlink(missing_ok=True)
    cwd_record = Path(str(exe) + '.cwd')
    cwd_record.unlink(missing_ok=True)
    exe.chmod(0o444)
    try:
        locked = not os.access(exe, os.W_OK)
        check(locked, '試験の前提: exe を書き込みで開けない状態を作れる（root では作れない）')
        code, seconds = run_silent(v3, '/RUN')
    finally:
        exe.chmod(0o644)
    check(code == 5, f'待ちの上限を超えたら終了コード 5（終了コード {code}）')
    check(2.0 <= seconds < 30, f'上限（試験用に 3 秒）まで待って終わる（{seconds:.1f} 秒）')
    check(all((install / name).read_bytes() == data for name, data in snapshot.items()),
          '上限を超えたら、exe・README・文書とその記録・アンインストーラーを何も変えない')
    check(registry(PRODUCT_KEY, 'DisplayVersion') == version_before, '上限を超えたら、登録の版を変えない')
    # /RUN が付いていれば、利用者をウィンドウの無い状態に置かないよう、今入っている（変わっていない）exe を起こし直す。
    check(wait_for(ran), '上限を超えても、/RUN が付いていれば今入っている exe を起こし直す')
    check(exe.read_bytes().endswith(b'build 0.1.0'), '起こし直すのは、置き換わっていない今の版')
    started_in = cwd_record.read_text().strip() if cwd_record.exists() else None
    check(bool(started_in) and bool(location) and started_in.rstrip('\\').lower() == location.lower(),
          f'上限を超えて起こし直したアプリの作業フォルダは入れ先（{started_in}）')
    # /RUN が付いていなければ、上限を超えても何も起こさない。
    ran.unlink(missing_ok=True)
    exe.chmod(0o444)
    try:
        code, _ = run_silent(v3)
    finally:
        exe.chmod(0o644)
    check(code == 5, f'/RUN なしでも、待ちの上限を超えたら終了コード 5（終了コード {code}）')
    time.sleep(2)
    check(not ran.exists(), '/RUN が付いていなければ、上限を超えてもアプリを起こさない')
    code, _ = run_silent(v3)
    check(code == 0 and exe.read_bytes().endswith(b'build 0.3.0'), '開けるようになれば、同じインストーラーで入る')
    code, _ = run_silent(v1)
    check(code == 0 and exe.read_bytes().endswith(b'build 0.1.0'), '元の版へ戻して、以降の試験へ進む')

    # 4c. コマンドラインの exe（MCP のクライアントが動かし続けていることがある）が使われているときも、アプリの exe は空いていても、同じく待って
    #     上限で何も変えずに終了コード 5 で終わる（片方だけ置き換わった半端な状態にしない）。
    snapshot = {name: (install / name).read_bytes() for name in
                [*EXES, 'README.md', 'docs/GUIDE.md', 'docs/en/GUIDE.md', 'docs/CLI.md', 'docs/.installed', 'uninstall.exe']}
    cli.chmod(0o444)
    try:
        check(not os.access(cli, os.W_OK), '試験の前提: コマンドラインの exe だけを書き込みで開けない状態を作れる')
        code, seconds = run_silent(v3)
    finally:
        cli.chmod(0o644)
    check(code == 5, f'コマンドラインの exe が使われていても、待ちの上限を超えたら終了コード 5（終了コード {code}）')
    check(2.0 <= seconds < 30, f'上限（試験用に 3 秒）まで待って終わる（{seconds:.1f} 秒）')
    check(all((install / name).read_bytes() == data for name, data in snapshot.items()),
          'アプリの exe が空いていても、何も置き換えない（アプリとコマンドラインの版が食い違わない）')
    code, _ = run_silent(v3)
    check(code == 0 and cli.read_bytes().endswith(b'build 0.3.0') and exe.read_bytes().endswith(b'build 0.3.0'),
          '開けるようになれば、2 つとも同じ版で入る')
    code, _ = run_silent(v1)
    check(code == 0 and cli.read_bytes().endswith(b'build 0.1.0'), '元の版へ戻して、以降の試験へ進む')

    # 5. アンインストール: 入れたファイルだけを消し、利用者のデータは残す
    data = appdata / 'YoluPainter'
    local = localdata / 'YoluPainter'
    lay_out_data(data, local)
    (install / 'my-notes.txt').write_text('利用者のファイル')
    (install / 'docs/en/mine.txt').write_text('利用者のファイル')
    code, gone = uninstall(install)
    check(code == 0 and gone, f'無音のアンインストールが終わり、アンインストーラー自身も消える（終了コード {code}）')
    for name in [*EXES, *ROOT_FILES, 'uninstall.exe']:
        check(not (install / name).exists(), f'消える: {name}')
    left = [doc for doc in docs if (install / Path(*doc.split('\\'))).exists()]
    check(not left and not (install / 'docs/.installed').exists(), f'入れた文書と記録が全部消える（残り {left}）')
    check((install / 'docs/en/mine.txt').exists(), '利用者が docs\\ に置いたファイルは消さない（docs のフォルダも残る）')
    check((install / 'my-notes.txt').exists(), '入れていないファイルは消さない（入れ先も残る）')
    check(shortcut() is None, 'ショートカットが消える')
    check(registry(PRODUCT_KEY, 'DisplayName') is None, '登録が消える')
    check(registry(r'HKCU\Software\Classes\.ylp') is None, '自分が付けた関連付けが外れる')
    check(all_exist(data, [*REBUILDABLE_ROAMING, *USER_MADE, *UNKNOWN]) and all_exist(local, REBUILDABLE_LOCAL),
          '無音のアンインストールは、設定・復旧・利用者が作った物・知らないファイルを全部残す')
    check(not (local / STAGED_UPDATE).exists() and not (local / 'updates').exists(),
          '落とした更新だけは、データを残す答えでも消える（利用者のデータではない）')

    # 6. /DELETEDATA で、設定などのデータも消える。他のアプリに替えられた関連付けには触らない
    (install / 'my-notes.txt').unlink()
    shutil.rmtree(install / 'docs')  # 利用者のファイルだけが残っていたフォルダ。消えると、この後の「入れ先が空になれば消える」が文書のフォルダも確かめる
    Path(str(exe) + '.cwd').unlink(missing_ok=True)
    Path(str(exe) + '.ran').unlink(missing_ok=True)  # 試験用の exe が残した印（入れたファイルではないので、アンインストールは消さない）
    code, _ = run_silent(v1, '/ASSOC=1')
    wine('reg', 'add', r'HKCU\Software\Classes\.ylp', '/ve', '/d', 'OtherApp.File', '/f')
    code, gone = uninstall(install, '/DELETEDATA')
    check(code == 0 and gone, '/DELETEDATA の無音のアンインストールが終わる')
    gone_roaming = [name for name in REBUILDABLE_ROAMING if (data / name).exists()]
    check(not gone_roaming, f'/DELETEDATA で、作り直せる設定・ウィンドウの配置・復旧・クラッシュの記録・一時ファイルが消える（残り {gone_roaming}）')
    for folder in ['recovery', 'logs']:
        check(not (data / folder).exists(), f'/DELETEDATA で {folder} のフォルダごと消える')
    check(not any((local / name).exists() for name in REBUILDABLE_LOCAL) and not (local / 'thumbnails').exists()
          and not (local / 'LiveLink').exists(), '/DELETEDATA で、サムネイルのキャッシュと Live Link の置き場が消える')
    check(not local.exists(), '/DELETEDATA で、%LOCALAPPDATA%\\YoluPainter は空になって消える')
    # 利用者が作った物と知らないファイルは、/DELETEDATA でも消えない（持ち主のいるフォルダも残る）。
    missing = [name for name in [*USER_MADE, *UNKNOWN] if not (data / name).is_file()]
    check(not missing, f'/DELETEDATA でも、個人のライブラリ・ブラシ・サブツール・グラデーション・カラーセット・表示のプリセット・ポーズのプリセットと、知らないファイルは残る（無い物 {missing}）')
    check((data / 'brushes/mine.ylbrush').read_text() == 'brushes/mine.ylbrush', '残った利用者の物の中身が変わらない')
    check(registry(r'HKCU\Software\Classes\.ylp') == 'OtherApp.File', '他のアプリに替えられた関連付けには触らない')
    check(wait_gone(install), '入れ先が空になれば消える')

    # 6b. 作り直せる物しか無ければ、/DELETEDATA でフォルダごと消える（空のフォルダを残さない）
    shutil.rmtree(data)
    lay_out_data(data, local, user_made=False)
    code, _ = run_silent(v1)
    code, gone = uninstall(install, '/DELETEDATA')
    check(code == 0 and gone, '作り直せる物だけのとき、/DELETEDATA の無音のアンインストールが終わる')
    check(not data.exists() and not local.exists(), '作り直せる物しか無ければ、設定のフォルダも %LOCALAPPDATA%\\YoluPainter も消える')


if __name__ == '__main__':
    sys.exit(main())
