#!/usr/bin/env python3
"""NSIS のインストーラー（installer/yolupainter.nsi）を、Wine で無音のまま端から端まで通して確かめる。

本物の yolupainter.exe の代わりに、起動した印を残すだけの小さな exe（mingw でこの場で作る）を入れる。
確かめること: 無音のインストール（入れ先・ショートカット・アンインストールの登録）、.ylp の関連付けの
有無と更新での引き継ぎ、上書きの更新、/RUN での起こし直し、exe を書き込みで開けない間は待ち、上限を超えたら
何も変えずに終了コード 5 で終わること（待ちの上限は試験用に短くしたインストーラーで、書き込めない exe を使って確かめる）、
アンインストール（入れたファイルだけを消す・利用者のデータは残す・/DELETEDATA で消す）。
実際に動いている exe を待つ動きは、Wine が動いている exe の上書きを断るときだけ確かめられる（上書きできる Wine では注意を
出して通る。Windows の実機で確かめる）。画面を出す側（ページの並び・チェック・終了の確かめ）は確かめない。
時間は monotonic で測る（WSL2 では壁時計が数秒戻ることがある）。

必要: makensis・wine・x86_64-w64-mingw32-gcc。生成物と専用の Wine 環境は target/test-installer/ だけに置く。
使い方: python3 tools/test-installer.py [--keep]（--keep は終わっても target/test-installer/ を消さない）
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
WORK = ROOT / 'target/test-installer'
PRODUCT_KEY = r'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\YoluPainter'
FAKE_APP = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
/* 起動した印（<exe>.ran に引数を 1 行）を残す。`hold N` なら N 秒動いたままにする（exe が使われている状態を作る）。 */
int main(int argc, char **argv) {
    char path[MAX_PATH + 8];
    GetModuleFileNameA(NULL, path, MAX_PATH);
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


def expand(variable):
    out = wine('cmd', '/c', f'echo %{variable}%').stdout.strip()
    return to_unix(out)


def build_installer(version, numeric, name, wait_steps=None):
    stage = WORK / f'stage-{version}'
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    source = WORK / 'fake-app.c'
    source.write_text(FAKE_APP)
    subprocess.run(['x86_64-w64-mingw32-gcc', '-O1', '-o', stage / 'yolupainter.exe', source], check=True)
    # 版ごとに exe の中身を変える（上書きされたかを見分ける）。
    with open(stage / 'yolupainter.exe', 'ab') as exe:
        exe.write(f'build {version}'.encode())
    shutil.copy(ROOT / 'LICENSE', stage / 'LICENSE')
    for file in ['README.md', 'THIRD_PARTY.md', 'DEPENDENCIES.md', 'THIRD_PARTY_LICENSES.txt']:
        (stage / file).write_text(f'{file} {version}\n')
    output = WORK / name
    command = ['makensis', '-INPUTCHARSET', 'UTF8', '-WX', '-V2', f'-DVERSION={version}',
               f'-DVERSION_NUMERIC={numeric}', f'-DSTAGE={stage}', f'-DOUTFILE={output}']
    command.append(f'-DICON={ROOT / "crates/yolu-app/assets/logo/yolupainter.ico"}')
    if wait_steps is not None:
        command.append(f'-DWAIT_STEPS={wait_steps}')
    subprocess.run([*command, ROOT / 'installer/yolupainter.nsi'], check=True, cwd=ROOT)
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
    wine('wineboot', '-u', timeout=300)
    appdata = expand('APPDATA')

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
    for name in ['yolupainter.exe', 'LICENSE', 'README.md', 'THIRD_PARTY.md', 'DEPENDENCIES.md',
                 'THIRD_PARTY_LICENSES.txt', 'uninstall.exe']:
        check((install / name).is_file(), f'入る: {name}')
    check(shortcut() is not None, 'スタートメニューのショートカットができる')
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
    code, _ = run_silent(v2, '/RUN')
    check(code == 0, f'更新の無音のインストールが終わる（終了コード {code}）')
    check(exe.read_bytes() != before and exe.read_bytes().endswith(b'build 0.2.0-rc.1'), 'exe が新しい版に置き換わる')
    check(registry(PRODUCT_KEY, 'DisplayVersion') == '0.2.0-rc.1', '登録の版が新しくなる')
    check(registry(PRODUCT_KEY, 'InstallLocation') == location, '更新で入れ先が変わらない')
    check(wait_for(ran), '/RUN で入れ終わったあとにアプリが起きる')
    check(registry(r'HKCU\Software\Classes\.ylp') is None, '更新で、無かった関連付けを勝手に付けない')

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
    snapshot = {name: (install / name).read_bytes() for name in ['yolupainter.exe', 'README.md', 'uninstall.exe']}
    version_before = registry(PRODUCT_KEY, 'DisplayVersion')
    ran = Path(str(exe) + '.ran')
    ran.unlink(missing_ok=True)
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
          '上限を超えたら、exe・README・アンインストーラーを何も変えない')
    check(registry(PRODUCT_KEY, 'DisplayVersion') == version_before, '上限を超えたら、登録の版を変えない')
    check(not ran.exists(), '上限を超えたら、/RUN でもアプリを起こさない')
    code, _ = run_silent(v3)
    check(code == 0 and exe.read_bytes().endswith(b'build 0.3.0'), '開けるようになれば、同じインストーラーで入る')
    code, _ = run_silent(v1)
    check(code == 0 and exe.read_bytes().endswith(b'build 0.1.0'), '元の版へ戻して、以降の試験へ進む')

    # 5. アンインストール: 入れたファイルだけを消し、利用者のデータは残す
    data = appdata / 'YoluPainter'
    data.mkdir(exist_ok=True)
    (data / 'settings.conf').write_text('language=ja\n')
    (install / 'my-notes.txt').write_text('利用者のファイル')
    code, gone = uninstall(install)
    check(code == 0 and gone, f'無音のアンインストールが終わり、アンインストーラー自身も消える（終了コード {code}）')
    for name in ['yolupainter.exe', 'LICENSE', 'README.md', 'THIRD_PARTY.md', 'DEPENDENCIES.md',
                 'THIRD_PARTY_LICENSES.txt', 'uninstall.exe']:
        check(not (install / name).exists(), f'消える: {name}')
    check((install / 'my-notes.txt').exists(), '入れていないファイルは消さない（入れ先も残る）')
    check(shortcut() is None, 'ショートカットが消える')
    check(registry(PRODUCT_KEY, 'DisplayName') is None, '登録が消える')
    check(registry(r'HKCU\Software\Classes\.ylp') is None, '自分が付けた関連付けが外れる')
    check((data / 'settings.conf').exists(), '無音のアンインストールは利用者の設定を残す')

    # 6. /DELETEDATA で、設定などのデータも消える。他のアプリに替えられた関連付けには触らない
    (install / 'my-notes.txt').unlink()
    Path(str(exe) + '.ran').unlink(missing_ok=True)  # 試験用の exe が残した印（入れたファイルではないので、アンインストールは消さない）
    code, _ = run_silent(v1, '/ASSOC=1')
    wine('reg', 'add', r'HKCU\Software\Classes\.ylp', '/ve', '/d', 'OtherApp.File', '/f')
    code, gone = uninstall(install, '/DELETEDATA')
    check(code == 0 and gone, '/DELETEDATA の無音のアンインストールが終わる')
    check(not data.exists(), '/DELETEDATA で設定のフォルダが消える')
    check(registry(r'HKCU\Software\Classes\.ylp') == 'OtherApp.File', '他のアプリに替えられた関連付けには触らない')
    check(wait_gone(install), '入れ先が空になれば消える')


if __name__ == '__main__':
    sys.exit(main())
