#!/usr/bin/env python3
"""実際のアプリ（yolupainter）を strace の下で動かし、外へ出る通信が無いことを確かめる（Linux だけ。標準ライブラリだけ）。

アプリが外へ通信するのは、利用者が選んだ更新の確認だけ（README・INSTALL の約束）。ソースと依存を読む見張り（`cargo xtask netguard`）の
ほかに、本物のアプリが実際に出す接続を見る。xvfb の中で、アプリを `strace -f` の下で起動し、CLI（yolupainter-cli）で操作して、
`connect`・`bind`・`listen`・`sendto`・`sendmsg`・`sendmmsg`・`execve` を読む。

- 通すのは `connect` などの「試み」で、届いたか・拒まれたかは見ない（CI から GitHub に届かなくても同じ判定）。
- PC の中（AF_UNIX の画面や D-Bus・AF_NETLINK）と 127.0.0.1・::1 のほかへの接続・送信・待ち受けが 1 つでもあれば落とす。
  名前の引き（宛先のポートが 53・853・5353。127.0.0.53 のようなループバックの中継も含む。systemd-resolved の UNIX ソケットも）も外への通信として落とす。
- 更新の確認を「する」にした起動では、外への接続が `curl` のプロセス（子のスレッドも）からだけであることを確かめる。ほかの起動では curl も起動してはならない。
- 受け口（外からの操作）は、入れた起動では 127.0.0.1 のその番号だけで待ち、入れない起動では何も待たない。
- 始めに、strace の読みが効いていることを、外への接続・送信をする小さなプログラムで確かめる（効いていなければ、アプリの結果は信用できないので止める）。

通す場面（起動ごと）:
  main    起動・編集（レイヤーと効果）・描く（xdotool の筆）・保存・書き出し（PNG・テンプレート・PSD）・3D のモデルと Live Link の受け渡し・保存・終了
  reopen  保存した文書（3D のモデル入り）を起動の引数で開き直す・見本・終了
  idle    受け口を入れない既定の設定（更新の確認は未選択）で、文書を開いて放っておく・終了
  update  更新の確認を「する」にした起動（curl の子だけが外へ試みる）
描く操作は CLI・MCP の命令に無いので、xdotool で筆を動かします（無ければ飛ばす）。ウィンドウの閉じる操作を、キー（Ctrl+Q）で行います。

終わりに、場面ごとの接続の数と宛先の表を出す。

    xvfb-run -a -s "-screen 0 1920x1080x24" python3 tools/network-watch.py --log-dir /tmp/network-logs
    tools/network-watch.py --only update            # 起動を絞る（main・reopen・idle・update）

先にアプリを作っておく（`target/debug/yolupainter` と `yolupainter-cli`。`--bin-dir` で場所を変えられる）。更新の確認は、更新用の公開鍵を組み込んだビルドでだけ動くので
（鍵の無いビルドは更新の項目も問いも出さない）、使い捨ての公開鍵（秘密鍵は捨てた。配る物の鍵ではない）を渡して作る。鍵が無いと update の起動は「curl の起動が見えない」で落ちる:

    YOLUPAINTER_UPDATE_PUBLIC_KEY=6acc8faaa5f8beb6897bc7936ab6b930853dc4e563e4dc5f49a76f156ab16201 cargo build -p yolu-app -p yolu-cli

鍵を変えると yolu-update と yolu-app を作り直す（`.github/workflows/ci.yml` の通信の試験は、ほかのジョブの作った物を汚さないよう別のジョブで作る）。
"""
import argparse
import contextlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib

ROOT = Path(__file__).resolve().parents[1]

# strace が読む呼び出し。X の画面とのやり取り（sendmsg）も入るが、宛先があるものだけを後で選ぶ。--seccomp-bpf で、読まない呼び出しは止めない。
TRACE = ('connect,bind,listen,sendto,sendmsg,sendmmsg,socket,accept,accept4,'
         'clone,clone3,fork,vfork,execve,execveat')
DNS_PORTS = {53, 853, 5353}
ADDRESS_CALLS = ('connect', 'bind', 'sendto', 'sendmsg', 'sendmmsg', 'accept', 'accept4', 'listen')
# 更新の確認の curl が取りに行ってよい URL の頭（yolu-update の RELEASE_BASE・更新情報の URL）。
UPDATE_URL_PREFIX = 'https://github.com/YozoraKurage/YoluPainter/'
CRASH_DIALOGS = {'zenity', 'kdialog', 'xmessage'}
# 起動・待ち受け・応答の待ちの上限（秒）
START_TIMEOUT = 90
STEP_TIMEOUT = 90

LINE = re.compile(r'^(?:\[pid\s+)?(\d+)\]?\s+(\d+\.\d+)\s+(.*)$')
CALL = re.compile(r'^(\w+)\(')
RESUMED = re.compile(r'^<\.\.\. (\w+) resumed>')
SOCKADDR = re.compile(r'\{sa_family=(AF_\w+)(?:,\s*([^}]*))?\}')
# 名前の引きを受ける UNIX ソケット（systemd-resolved の varlink・D-Bus、avahi の mDNS）。nscd のソケット（/var/run/nscd/socket）は、
# hosts のほか passwd・group の問い合わせにも glibc が最初に開く（アプリの起動でも出る）ので、名前の引きとは数えない。
RESOLVER_SOCKETS = ('systemd/resolve', 'resolved', 'avahi')


class Fail(Exception):
    """場面を最後まで通せなかった（空振りを、通ったことにしない）。"""


# ───────── strace のログの読み ─────────

class Dest:
    """1 つの宛先。kind: unix・netlink（PC の中）、loopback、dns、external、other。"""
    __slots__ = ('family', 'kind', 'text', 'port', 'ip')

    def __init__(self, family, kind, text, port=None, ip=None):
        self.family, self.kind, self.text, self.port, self.ip = family, kind, text, port, ip

    def __repr__(self):
        return f'Dest({self.kind} {self.text})'


class Entry:
    """ログの 1 行（読む呼び出しのもの）。"""
    __slots__ = ('pid', 'time', 'call', 'line', 'exec_path', 'child', 'dests', 'family')

    def __init__(self, pid, time_, call, line):
        self.pid, self.time, self.call, self.line = pid, time_, call, line
        self.exec_path = None
        self.child = None
        self.dests = []
        self.family = None


def parse_dest(family, body):
    body = body or ''
    if family == 'AF_UNIX':
        m = re.search(r'sun_path=@?"([^"]*)"', body)
        path = m.group(1) if m else '?'
        kind = 'dns' if any(s in path for s in RESOLVER_SOCKETS) else 'unix'
        return Dest(family, kind, path)
    if family == 'AF_NETLINK':
        return Dest(family, 'netlink', 'netlink')
    if family in ('AF_INET', 'AF_INET6'):
        if family == 'AF_INET':
            a = re.search(r'sin_addr=inet_addr\("([^"]+)"\)', body)
            p = re.search(r'sin_port=htons\((\d+)\)', body)
        else:
            a = re.search(r'inet_pton\(AF_INET6, "([^"]+)"', body)
            p = re.search(r'sin6_port=htons\((\d+)\)', body)
        if not a:
            return Dest(family, 'other', f'{family} ?')
        port = int(p.group(1)) if p else None
        try:
            ip = ipaddress.ip_address(a.group(1))
        except ValueError:
            return Dest(family, 'other', f'{a.group(1)}', port)
        if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
            loopback = ip.ipv4_mapped.is_loopback
        else:
            loopback = ip.is_loopback
        text = f'[{ip}]:{port}' if family == 'AF_INET6' else f'{ip}:{port}'
        if port in DNS_PORTS:
            kind = 'dns'
        elif loopback:
            kind = 'loopback'
        else:
            kind = 'external'
        return Dest(family, kind, text, port, ip)
    return Dest(family, 'other', family)


def parse_log(text):
    """strace のログ（-ttt つき）から、読む呼び出しの Entry を並べる。宛先の無い sendmsg（X の画面とのやり取り）は捨てる。"""
    entries = []
    pending_exec = {}
    had_dests = {}
    for raw in text.splitlines():
        m = LINE.match(raw)
        if not m:
            continue
        pid, when, rest = int(m.group(1)), float(m.group(2)), m.group(3)
        if rest.startswith(('+++', '---')):
            continue
        resumed = RESUMED.match(rest)
        call = resumed.group(1) if resumed else (CALL.match(rest).group(1) if CALL.match(rest) else None)
        if call is None:
            continue
        ret = rest.rsplit(' = ', 1)[1].split()[0] if ' = ' in rest else None
        entry = Entry(pid, when, call, raw)
        if call in ('execve', 'execveat'):
            if resumed:
                # 引数は始めの行にある（vfork の子の execve は 2 行に分かれる）。判定には 2 行を 1 つにして渡す
                path, first = pending_exec.pop(pid, (None, ''))
                entry.line = f'{first} {raw}'
            else:
                e = re.search(r'exec\w*\((?:[A-Z_]+, )?"([^"]*)"', rest)
                path = e.group(1) if e else None
                if '<unfinished' in rest:
                    pending_exec[pid] = (path, raw)
                    continue
            if path and ret == '0':
                entry.exec_path = path
            elif path and not resumed:
                # 起動に失敗した試み（PATH を順に探す途中）は無視する
                continue
            else:
                continue
        elif call in ('clone', 'clone3', 'fork', 'vfork'):
            if ret and ret.isdigit() and int(ret) > 0:
                entry.child = int(ret)
            else:
                continue
        elif call == 'socket':
            s = re.match(r'socket\((AF_\w+)', rest)
            entry.family = s.group(1) if s else None
        elif call in ADDRESS_CALLS:
            if call == 'listen' and resumed:
                continue
            dests = [parse_dest(family, body) for family, body in SOCKADDR.findall(rest) if family != 'AF_UNSPEC']
            # 宛先は始めの行に出るのが普通。スレッドが多いと sendmmsg・accept* の宛先は、再開の行（`<... sendmmsg resumed>`）にだけ出る。
            # 始めの行で読めていれば、再開の行は数えない（二重に数えない）
            if resumed:
                if had_dests.pop(pid, False):
                    continue
            elif '<unfinished' in rest:
                had_dests[pid] = bool(dests)
            entry.dests = dests
            if call != 'listen' and not dests:
                continue
        else:
            continue
        entries.append(entry)
    return entries


class Procs:
    """プロセスの親子と、実行したファイルの履歴（スレッドは親のプロセスの実行ファイルを受け継ぐ）。"""

    def __init__(self, entries):
        self.parent = {}
        self.execs = {}
        for e in entries:
            if e.child is not None:
                self.parent.setdefault(e.child, e.pid)
            if e.exec_path is not None:
                self.execs.setdefault(e.pid, []).append((e.time, e.exec_path))

    def exe(self, pid, when):
        seen = set()
        while pid is not None and pid not in seen:
            seen.add(pid)
            past = [path for t, path in self.execs.get(pid, []) if t <= when]
            if past:
                return past[-1]
            pid = self.parent.get(pid)
        return '?'


ARG = re.compile(r'"((?:[^"\\]|\\.)*)"(\.\.\.)?')
# 更新の確認の curl が持ってよい引数（update/http.rs の `curl_args`。単体試験 `curl_arguments_are_exactly_these` が全体を固定している）。
# 値の形が None のものは値を取らない。ここに無いオプション（`--data`・`-H`・`-b`・`-T`・`-K`・`--url`・`--referer`・`--cookie` など）は、
# 利用者を識別する情報や別の取り先を足せるので通さない。
CURL_OPTIONS = {
    '-q': None, '--silent': None, '--show-error': None, '--fail': None, '--location': None, '--tlsv1.2': None,
    '--max-redirs': r'\d{1,3}', '--connect-timeout': r'\d{1,4}', '--max-time': r'\d{1,5}',
    '--user-agent': r'YoluPainter/[0-9A-Za-z.+-]+',
    '--proto': '=https', '--proto-redir': '=https', '--output': '-',
}
CURL_REQUIRED = ('-q', '--proto', '--proto-redir')
CURL_URL = re.compile(r'https://[A-Za-z0-9._~/%+=-]+')


def exec_argv(line):
    """execve の行から、引数の並びと、読み切れなかったか（`...` で省かれた・長さで切られた）を返す。"""
    m = re.search(r'execve\("[^"]*", \[', line)
    if not m:
        return [], True
    items, pos, truncated = [], m.end(), False
    while True:
        e = ARG.match(line, pos)
        if not e:
            break
        items.append(e.group(1).replace('\\"', '"').replace('\\\\', '\\'))
        truncated = truncated or bool(e.group(2))
        pos = e.end()
        if line.startswith(', ', pos):
            pos += 2
        else:
            break
    if line.startswith('...', pos):
        truncated = True
    return items, truncated


def check_curl_argv(argv, truncated=False):
    """更新の確認の curl の引数が許す形か。許さないときは理由、許すときは None。
    決めたオプションだけ（同じオプションは 1 回）・必須の `-q`/`--proto =https`/`--proto-redir =https`・`--` の後に URL が 1 つだけで、
    その URL は YoluPainter の GitHub の https（`?`・`#`・利用者情報の `@` なし）。"""
    if truncated:
        return '引数が長くて読み切れない'
    if not argv or argv[0] != 'curl':
        return '起動した物が curl でない'
    seen, i = set(), 1
    while i < len(argv) and argv[i] != '--':
        option = argv[i]
        if option not in CURL_OPTIONS:
            return f'許していないオプション {option}'
        if option in seen:
            return f'オプション {option} が重なっている'
        seen.add(option)
        pattern = CURL_OPTIONS[option]
        if pattern is None:
            i += 1
            continue
        if i + 1 >= len(argv) or not re.fullmatch(pattern, argv[i + 1]):
            return f'{option} の値が許す形でない'
        i += 2
    missing = [o for o in CURL_REQUIRED if o not in seen]
    if missing:
        return '必要なオプションが無い: ' + ' '.join(missing)
    if i >= len(argv):
        return '`--` が無い'
    rest = argv[i + 1:]
    if len(rest) != 1:
        return f'`--` の後が URL 1 つでない（{len(rest)} 個）'
    url = rest[0]
    if not url.startswith(UPDATE_URL_PREFIX) or not CURL_URL.fullmatch(url):
        return f'取りに行く URL が許す形でない: {url[:80]}'
    return None


# ───────── 判定と表 ─────────

class Scene:
    def __init__(self, name, start, allow_curl=False):
        self.name, self.start, self.end, self.allow_curl = name, start, None, allow_curl
        self.note = ''
        # 場面が通らず飛ばした（通ったことにしない）。終わりに `::warning::` で出す
        self.skipped = False


class Spec:
    """起動ごとの期待。"""

    def __init__(self, name, title, listen_port=None, expect_curl=False):
        self.name, self.title, self.listen_port, self.expect_curl = name, title, listen_port, expect_curl


def scene_at(scenes, when):
    chosen = None
    for scene in scenes:
        if scene.start <= when:
            chosen = scene
    return chosen


def base(path):
    return os.path.basename(path)


def evaluate(spec, scenes, entries):
    """（落ちた理由の並び, 場面ごとの表の行）。scenes は開始の時刻の順。"""
    procs = Procs(entries)
    violations = []
    rows = {s.name: {'unix': 0, 'loopback': 0, 'external': 0, 'commands': [], 'targets': {}, 'curl': 0} for s in scenes}
    binds = 0
    listens = 0
    curl_runs = 0
    outside = {'unix': 0, 'loopback': 0, 'external': 0, 'commands': [], 'targets': {}, 'curl': 0}
    for e in sorted(entries, key=lambda e: e.time):
        scene = scene_at(scenes, e.time)
        row = rows[scene.name] if scene else outside
        where = scene.name if scene else '（場面の前）'
        exe = procs.exe(e.pid, e.time)
        is_curl = base(exe) == 'curl'
        allow_curl = bool(scene and scene.allow_curl)
        if e.exec_path is not None:
            name = base(e.exec_path)
            if name == 'curl':
                curl_runs += 1
                row['curl'] += 1
                if not allow_curl:
                    violations.append(f'{where}: curl が起動された（更新の確認を選んでいない場面）: {short(e.line)}')
                else:
                    problem = check_curl_argv(*exec_argv(e.line))
                    if problem:
                        violations.append(f'{where}: 更新の確認の curl の引数が許す形でない（{problem}）: {short(e.line)}')
            elif name in CRASH_DIALOGS:
                violations.append(f'{where}: 落ちたときのダイアログ（{name}）が起動された。アプリが落ちています: {short(e.line)}')
            if name not in row['commands'] and e.pid in procs.parent:
                row['commands'].append(name)
        if e.call == 'listen':
            listens += 1
        for d in e.dests:
            if d.kind in ('unix', 'netlink'):
                row['unix'] += 1
                continue
            key = f'{e.call} {d.text}'
            if d.kind == 'loopback':
                row['loopback'] += 1
                row['targets'][key] = row['targets'].get(key, 0) + 1
                if e.call == 'bind':
                    binds += 1
                    if spec.listen_port is None:
                        violations.append(f'{where}: 受け口を入れていないのに待ち受けようとした（bind {d.text}）: {short(e.line)}')
                    elif d.port != spec.listen_port or str(d.ip) not in ('127.0.0.1', '::1'):
                        violations.append(f'{where}: 待ち受けの場所が 127.0.0.1:{spec.listen_port} でない（bind {d.text}）: {short(e.line)}')
                continue
            # dns・external・other
            what = '名前の引き' if d.kind == 'dns' else '外への通信'
            row['external'] += 1
            key = f'{e.call} {d.text}（{what}）' + (' [curl]' if is_curl else '')
            row['targets'][key] = row['targets'].get(key, 0) + 1
            if is_curl and allow_curl:
                continue
            violations.append(f'{where}: {what}の試み（{e.call} {d.text}、プロセス {e.pid} {base(exe)}）: {short(e.line)}')
    if spec.listen_port is not None and (binds == 0 or listens == 0):
        violations.append(f'受け口（127.0.0.1:{spec.listen_port}）を待つ呼び出しが見えない（bind {binds}・listen {listens}）。見張りが空振りになっています')
    if spec.expect_curl and curl_runs == 0:
        violations.append('更新の確認を「する」にした起動で、curl の起動が見えない。更新の確認の場面が通っていません')
    return violations, rows


def short(line, limit=240):
    line = line.strip()
    return line if len(line) <= limit else line[:limit] + '…'


def skipped_scenes(session):
    """飛ばした場面（xdotool が効かない・問いを閉じられない）の説明。"""
    return [f'{session.spec.name} / {scene.name}: {scene.note}' for scene in session.scenes if scene.skipped]


def display_width(text):
    return sum(2 if ord(c) > 255 else 1 for c in text)


def render_table(spec, scenes, rows, seconds):
    out = [f'■ {spec.name}: {spec.title}（{seconds:.0f} 秒）']
    width = max([display_width(s.name) for s in scenes] + [display_width('場面')])
    out.append(f'  {"場面"}{" " * (width - display_width("場面"))}  PC の中  127.0.0.1/::1  外・名前の引き  curl 起動  外部コマンド')
    for s in scenes:
        r = rows[s.name]
        out.append(f'  {s.name}{" " * (width - display_width(s.name))}  {r["unix"]:>6}  {r["loopback"]:>13}  {r["external"]:>14}  {r["curl"]:>8}  '
                   f'{", ".join(r["commands"]) or "-"}' + (f'  ※{s.note}' if s.note else ''))
        for target, count in sorted(r['targets'].items()):
            out.append(f'      {target} ×{count}')
    return '\n'.join(out)


# ───────── 実行の部品 ─────────

def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def png_bytes(size=4, rgba=(200, 100, 50, 255)):
    raw = b''.join(b'\x00' + bytes(rgba) * size for _ in range(size))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def slash(path):
    return str(path).replace('\\', '/')


def arm_request(rid, fbx, texture, export_dir):
    """Live Link の頼み（Unity が書くのと同じ形。腕のメッシュ 2 つのマテリアルと帽子。lilToon の値つき）。"""
    return {
        'format': 1, 'kind': 'open', 'id': rid,
        'bridge': {'version': '0.5.0', 'unity': '2022.3.22f1'},
        'project': {'root': '/work/Project', 'name': 'Project'},
        'target': {'key': f'GlobalObjectId_V1-2-{rid}', 'name': 'Arm', 'export_dir': slash(export_dir)},
        'models': [{'id': 0, 'fbx': slash(fbx), 'guid': '0123456789abcdef0123456789abcdef',
                    'import': {'global_scale': 1.0, 'use_file_scale': True, 'bake_axis_conversion': False, 'import_blend_shapes': True}}],
        'renderers': [
            {'path': 'ArmMesh', 'model': 0, 'node': 'ArmMesh', 'enabled': True, 'skinned': True,
             'blend_shapes': {'Thick': 50.0}, 'materials': [0, 1]},
            {'path': 'Armature/Upper/Lower/Hat', 'model': 0, 'node': 'Armature/Upper/Lower/Hat', 'enabled': True,
             'skinned': False, 'materials': [0]},
        ],
        'bones': [{'model': 0, 'node': 'Armature/Upper/Lower',
                   'local': {'t': [-1.0, 0.0, 0.0], 'r': [0.0, 0.0, 0.38268343, 0.9238795], 's': [1.0, 1.0, 1.0]}}],
        'materials': [
            {'key': 'guid:00000000000000000000000000000001/fileid:2100000', 'name': 'Skin',
             'shader': {'name': 'Hidden/lilToonCutout', 'guid': 'fedcba9876543210fedcba9876543210', 'version': '2.3.4',
                        'keywords': [], 'render_queue': 2450},
             'values': {'floats': {'_Cutoff': 0.25}, 'ints': {'_lilToonVersion': 45}, 'colors': {'_Color': [1.0, 1.0, 1.0, 1.0]}},
             'textures': [{'property': '_MainTex', 'path': slash(texture), 'guid': '00000000000000000000000000000001',
                           'srgb': True, 'normal_map': False}]},
            {'key': 'guid:00000000000000000000000000000002/fileid:2100000', 'name': 'Cloth', 'shader': {'name': 'Custom/lilToonLike'}},
        ],
        'refused': [{'path': 'Accessory', 'reason': 'mesh_not_from_fbx'}],
    }


class Tools:
    """使う外部のコマンド。"""

    def __init__(self, bin_dir):
        self.bin_dir = Path(bin_dir)
        self.app = self.bin_dir / 'yolupainter'
        self.cli = self.bin_dir / 'yolupainter-cli'
        self.strace = shutil.which('strace')
        self.xdotool = shutil.which('xdotool')
        self.seccomp = False
        if self.strace:
            probe = subprocess.run([self.strace, '-f', '--seccomp-bpf', '-o', os.devnull, 'true'],
                                   capture_output=True, text=True)
            self.seccomp = probe.returncode == 0

    def strace_command(self, log, append=False):
        cmd = [self.strace, '-f', '-ttt', '-s', '200', '-e', f'trace={TRACE}', '-o', str(log)]
        if self.seccomp:
            cmd.insert(2, '--seccomp-bpf')
        if append:
            cmd.append('-A')
        return cmd


class Run:
    """全体の状態（作業のフォルダ・上限の時間）。"""

    def __init__(self, tools, work, log_dir, deadline):
        self.tools, self.work, self.log_dir, self.deadline = tools, Path(work), Path(log_dir), deadline

    def left(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise Fail('全体の上限の時間（--time-limit）を超えました')
        return remaining


class Session:
    def __init__(self, run, spec, *, ops, update_check=False, project=None):
        self.run, self.spec = run, spec
        self.tools = run.tools
        self.dir = run.work / spec.name
        self.port = free_port() if ops else None
        spec.listen_port = self.port
        self.update_check = update_check
        self.project = project
        self.scenes = []
        self.proc = None
        self.app_log = run.log_dir / f'{spec.name}.app.strace'
        self.cli_log = run.log_dir / f'{spec.name}.cli.strace'
        self.out_log = run.log_dir / f'{spec.name}.app.out'
        self.exit_mode = '-'
        self.failure = None
        self.env = None
        self.livelink = self.dir / 'livelink'

    # 準備と起動
    def prepare(self):
        for sub in ('config/YoluPainter', 'data', 'home', 'run', 'out'):
            (self.dir / sub).mkdir(parents=True, exist_ok=True)
        (self.dir / 'run').chmod(0o700)
        settings = ''
        if self.port is not None:
            settings += f'external_ops=on\nexternal_ops_port={self.port}\n'
        (self.dir / 'config/YoluPainter/settings.conf').write_text(settings, encoding='utf-8')
        if self.update_check:
            (self.dir / 'config/YoluPainter/update.conf').write_text('check_on_startup=on\n', encoding='utf-8')
        env = dict(os.environ)
        env.update({
            'HOME': str(self.dir / 'home'),
            'XDG_CONFIG_HOME': str(self.dir / 'config'),
            'XDG_DATA_HOME': str(self.dir / 'data'),
            'XDG_STATE_HOME': str(self.dir / 'state'),
            'XDG_CACHE_HOME': str(self.dir / 'cache'),
            'XDG_RUNTIME_DIR': str(self.dir / 'run'),
            'YOLUPAINTER_LIVELINK_DIR': str(self.livelink),
        })
        env.setdefault('WGPU_BACKEND', 'vulkan')
        for name in ('DBUS_SESSION_BUS_ADDRESS',):
            env.pop(name, None)
        self.env = env

    def launch(self):
        self.prepare()
        args = [str(self.project)] if self.project else []
        cmd = self.tools.strace_command(self.app_log) + [str(self.tools.app), *args]
        with self.out_log.open('wb') as out:
            self.proc = subprocess.Popen(cmd, cwd=self.dir, env=self.env, stdout=out, stderr=subprocess.STDOUT,
                                         start_new_session=True)

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def kill(self):
        if self.proc is not None and self.proc.poll() is None:
            with contextlib.suppress(ProcessLookupError):
                os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait()

    # CLI
    def cli(self, *args, ok=True):
        if self.port is None:
            raise Fail('受け口を入れていない起動で CLI は使えない')
        cmd = self.tools.strace_command(self.cli_log, append=True) + [str(self.tools.cli), '--port', str(self.port), *map(str, args)]
        r = subprocess.run(cmd, capture_output=True, text=True, env=self.env, cwd=self.dir, timeout=min(STEP_TIMEOUT, self.run.left()))
        if ok and r.returncode != 0:
            raise Fail(f'yolupainter-cli {" ".join(map(str, args))} が失敗（終了コード {r.returncode}）: {r.stdout.strip()[:300]} {r.stderr.strip()[:300]}')
        try:
            return json.loads(r.stdout) if r.stdout.strip() else {}
        except ValueError:
            return {}

    def wait_until(self, what, probe, timeout):
        end = min(time.monotonic() + timeout, self.run.deadline)
        while time.monotonic() < end:
            if not self.alive():
                raise Fail(f'{what} を待つ間にアプリが終わった（終了コード {self.proc.returncode}）。{self.out_log} を見る')
            value = probe()
            if value:
                return value
            time.sleep(0.4)
        raise Fail(f'{what} が {timeout} 秒の間に来ない')

    def wait_ready(self):
        if self.port is not None:
            def ping():
                cmd = self.tools.strace_command(self.cli_log, append=True) + [str(self.tools.cli), '--port', str(self.port), 'doc.info']
                r = subprocess.run(cmd, capture_output=True, text=True, env=self.env, cwd=self.dir, timeout=60)
                return r.returncode == 0
            self.wait_until('アプリの受け口', ping, START_TIMEOUT)
        elif self.tools.xdotool:
            self.wait_until('アプリのウィンドウ', lambda: self.window() is not None, START_TIMEOUT)
        else:
            time.sleep(8)

    # xdotool
    def xdotool(self, *args):
        r = subprocess.run([self.tools.xdotool, *map(str, args)], capture_output=True, text=True, env=self.env, timeout=30)
        return r.returncode, r.stdout

    def window(self):
        if not self.tools.xdotool:
            return None
        code, out = self.xdotool('search', '--onlyvisible', '--name', 'YoluPainter')
        ids = out.split()
        if code != 0 or not ids:
            return None
        return ids[0]

    def geometry(self):
        wid = self.window()
        if wid is None:
            return None
        code, out = self.xdotool('getwindowgeometry', '--shell', wid)
        values = dict(line.split('=', 1) for line in out.split() if '=' in line)
        try:
            return int(values['X']), int(values['Y']), int(values['WIDTH']), int(values['HEIGHT'])
        except (KeyError, ValueError):
            return None

    def draw(self):
        """キャンバスの上で筆を動かす（xdotool）。取り消しの段が 1 つ増えたら描けた。"""
        g = self.geometry()
        if g is None:
            return False
        x, y, w, h = g
        cx, cy = x + int(w * 0.56), y + int(h * 0.52)
        before = self.cli('history.info').get('undo_count', 0)
        self.xdotool('mousemove', cx, cy)
        time.sleep(0.4)
        self.xdotool('mousedown', 1)
        for step in range(1, 7):
            self.xdotool('mousemove', cx + step * 20, cy + step * 4)
            time.sleep(0.1)
        self.xdotool('mouseup', 1)
        time.sleep(1.0)
        after = self.cli('history.info').get('undo_count', 0)
        return after > before

    def press(self, key):
        """アプリのウィンドウの上へポインターを置き、入力先を決めてキーを押す。ウィンドウマネージャーが無い画面では、入力先を自分で決めないとキーが届かない。"""
        g = self.geometry()
        if g is None:
            return False
        x, y, w, h = g
        self.xdotool('mousemove', x + int(w * 0.56), y + int(h * 0.52))
        self.xdotool('windowfocus', self.window())
        time.sleep(0.3)
        self.xdotool('key', key)
        return True

    def quit(self, timeout=20):
        """Ctrl+Q で終える。効かなければ殺す。"""
        pressed = self.press('ctrl+q')
        if pressed:
            end = time.monotonic() + timeout
            while time.monotonic() < end and self.alive():
                time.sleep(0.3)
        if self.alive():
            self.exit_mode = 'キーで閉じず、殺した' if pressed else 'xdotool が無く、殺した'
            self.kill()
        else:
            self.exit_mode = f'キー（Ctrl+Q）で終了（終了コード {self.proc.returncode}）'

    @contextlib.contextmanager
    def scene(self, name, allow_curl=False):
        scene = Scene(name, time.time(), allow_curl)
        self.scenes.append(scene)
        try:
            yield scene
        finally:
            scene.end = time.time()

    def entries(self):
        entries = []
        for log in (self.app_log, self.cli_log):
            if log.exists():
                entries.extend(parse_log(log.read_text(encoding='utf-8', errors='replace')))
        return entries


# ───────── 場面 ─────────

@contextlib.contextmanager
def guarded(session):
    """場面が途中で落ちたら、残ったアプリを殺して、理由を session.failure に残す（そこまでの通信は判定に使う。次の起動へ持ち越さない）。"""
    try:
        yield session
    except (Fail, subprocess.TimeoutExpired, OSError) as error:
        session.failure = str(error)
        session.kill()
    except BaseException:
        session.kill()
        raise


def scene_start(session, name='起動'):
    with session.scene(name):
        session.launch()
        session.wait_ready()


def session_main(run):
    spec = Spec('main', '既定の設定に受け口だけ入れて、編集・描く・保存・書き出し・3D のモデルと Live Link')
    s = Session(run, spec, ops=True)
    work = s.dir / 'files'
    work.mkdir(parents=True, exist_ok=True)
    with guarded(s):
        scene_start(s)
        with s.scene('更新の確認の問い（Esc で「いいえ」）') as scene:
            # 初めての起動は問いを出す（聞くまで通信しない）。Esc は「いいえ」（通信しない）。問いが開いたままだと、後の筆が届かない
            if s.press('Escape'):
                time.sleep(1.0)
                saved = s.dir / 'config/YoluPainter/update.conf'
                answered = saved.exists() and 'check_on_startup=off' in saved.read_text(encoding='utf-8')
                scene.note = '「いいえ」を保存した' if answered else '問いに答えられなかった（問いのないビルドなら正常）'
                scene.skipped = not answered
            else:
                scene.note = 'xdotool が無く、飛ばした'
                scene.skipped = True
        with s.scene('編集（レイヤー・効果）'):
            s.cli('layer.add', '--kind', 'fill', '--name', 'Wash', '--fill.Color', '#336699')
            s.cli('effect.add', '--layer', 'Wash', '--kind', 'blur', '--values.radius', '4')
            s.cli('layer.set', '--layer', 'Wash', '--opacity', '0.5', '--blend-mode', 'Multiply')
            s.cli('preview', '--max-edge', '128', '--out', work / 'preview.png')
        with s.scene('描く（xdotool の筆）') as scene:
            if not s.tools.xdotool:
                scene.note = 'xdotool が無く、飛ばした'
                scene.skipped = True
            elif s.draw():
                scene.note = '筆のストロークが取り消しの段に入った'
            else:
                scene.note = '筆が届かず、描けなかった（飛ばした）'
                scene.skipped = True
        with s.scene('保存（.ylp）'):
            s.cli('save_as', '--path', work / 'doc.ylp')
        with s.scene('書き出し（PNG・テンプレート・PSD）'):
            s.cli('export.channels', '--dir', work / 'out', '--channels', 'Color')
            s.cli('export.textures', '--dir', work / 'tex', '--template', 'liltoon')
            s.cli('export.psd', '--path', work / 'out/doc.psd', '--channel', 'Color')
        with s.scene('3D のモデルと Live Link の受け渡し'):
            live_link(s, work)
        with s.scene('保存（モデル入り）'):
            s.cli('save_as', '--path', work / 'link.ylp')
        with s.scene('終了'):
            s.quit()
    return s


def live_link(s, work):
    fbx = work / 'arm.fbx'
    shutil.copyfile(ROOT / 'tools/network-watch/arm.fbx', fbx)
    texture = work / 'skin.png'
    texture.write_bytes(png_bytes())
    export_dir = work / 'link-export'
    export_dir.mkdir(exist_ok=True)
    rid = 'netwatch-1'
    s.wait_until('Live Link の受け付け（presence.json）', lambda: (s.livelink / 'presence.json').exists(), STEP_TIMEOUT)
    inbox = s.livelink / 'inbox'
    outbox = s.livelink / 'outbox'
    tmp = inbox / f'{rid}.json.tmp'
    tmp.write_text(json.dumps(arm_request(rid, fbx, texture, export_dir)), encoding='utf-8')
    os.replace(tmp, inbox / f'{rid}.json')

    def reply():
        files = sorted(outbox.glob(f'{rid}-*.json')) if outbox.exists() else []
        return files[0] if files else None
    path = s.wait_until('Live Link の返事（outbox）', reply, STEP_TIMEOUT)
    answer = json.loads(path.read_text(encoding='utf-8'))
    if answer.get('kind') != 'opened':
        raise Fail(f'Live Link の返事が opened でない: {json.dumps(answer, ensure_ascii=False)[:400]}')
    path.unlink()
    time.sleep(2.0)
    info = s.cli('doc.info')
    if not info.get('sets'):
        raise Fail('Live Link で開いたあとの文書にテクスチャセットが無い')


def session_reopen(run, project):
    spec = Spec('reopen', '保存した文書（3D のモデル入り）を起動の引数で開き直す')
    s = Session(run, spec, ops=True, project=project)
    with guarded(s):
        scene_start(s, '起動（開き直し）')
        with s.scene('文書の確認・見本'):
            info = s.cli('doc.info')
            if project and Path(info.get('path', '')).name != Path(project).name:
                raise Fail(f'開き直した文書の道が違う: {info.get("path")}')
            s.cli('preview', '--max-edge', '128', '--out', s.dir / 'out/preview.png')
            time.sleep(2.0)
        with s.scene('終了'):
            s.quit()
    return s


def session_idle(run, project):
    spec = Spec('idle', '受け口を入れない既定の設定（更新の確認は未選択）で、文書を開いて放っておく')
    s = Session(run, spec, ops=False, project=project)
    with guarded(s):
        scene_start(s, '起動（受け口なし）')
        with s.scene('放っておく'):
            time.sleep(5)
        with s.scene('終了'):
            s.quit()
    return s


def session_update(run, project):
    spec = Spec('update', '更新の確認を「する」にした起動（curl の子だけが外へ試みる）', expect_curl=True)
    s = Session(run, spec, ops=False, update_check=True, project=project)
    with guarded(s):
        with s.scene('起動と更新の確認', allow_curl=True):
            s.launch()

            def curl_seen():
                if not s.app_log.exists():
                    return False
                return any(e.exec_path and base(e.exec_path) == 'curl' for e in parse_log(s.app_log.read_text(errors='replace')))
            try:
                s.wait_until('curl の起動（更新の確認）', curl_seen, 60)
            except Fail as error:
                raise Fail(f'{error}。更新の確認は、公開鍵を組み込んだビルド（YOLUPAINTER_UPDATE_PUBLIC_KEY）でだけ動く') from error
            # curl が試みを終えるまで待つ（届かなくても、名前の引きや接続の試みは短い間に出る）
            time.sleep(8)
        with s.scene('終了', allow_curl=True):
            s.quit()
    return s


# ───────── 較正（strace の読みが効いているか） ─────────

CALIBRATION = '''
import ctypes, socket, threading
a = socket.socket()
a.settimeout(0.2)
a.connect_ex(("192.0.2.1", 9))          # connect: 外（TEST-NET-1。届かない）
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
u.sendto(b"x", ("192.0.2.1", 9))        # sendto: 外
d = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
d.sendto(b"x", ("127.0.0.1", 53))       # sendto: ループバックの名前の引き
l = socket.socket()
l.bind(("127.0.0.1", 0))
l.listen(1)
c = socket.create_connection(l.getsockname())   # connect: ループバック
try:
    socket.getaddrinfo("example.invalid", 443)  # 本物の名前の引き（リゾルバーへの connect と送信）
except OSError:
    pass

# sendmmsg: 宛先つきの 1 通。別のスレッドが accept で待つ間に送る（スレッドが多いと宛先は再開の行に出る）
threading.Thread(target=lambda: l.accept(), daemon=True).start()
class Iovec(ctypes.Structure):
    _fields_ = [("base", ctypes.c_void_p), ("len", ctypes.c_size_t)]
class Msghdr(ctypes.Structure):
    _fields_ = [("name", ctypes.c_void_p), ("namelen", ctypes.c_uint), ("iov", ctypes.POINTER(Iovec)),
                ("iovlen", ctypes.c_size_t), ("control", ctypes.c_void_p), ("controllen", ctypes.c_size_t), ("flags", ctypes.c_int)]
class Mmsghdr(ctypes.Structure):
    _fields_ = [("hdr", Msghdr), ("len", ctypes.c_uint)]
class SockaddrIn(ctypes.Structure):
    _fields_ = [("family", ctypes.c_ushort), ("port", ctypes.c_ushort), ("addr", ctypes.c_ubyte * 4), ("zero", ctypes.c_ubyte * 8)]
sin = SockaddrIn(socket.AF_INET, socket.htons(9), (ctypes.c_ubyte * 4)(192, 0, 2, 1))
data = ctypes.create_string_buffer(b"x")
iov = Iovec(ctypes.cast(data, ctypes.c_void_p), 1)
msg = Mmsghdr(Msghdr(ctypes.cast(ctypes.byref(sin), ctypes.c_void_p), ctypes.sizeof(sin), ctypes.pointer(iov), 1, None, 0, 0), 0)
m = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
ctypes.CDLL(None, use_errno=True).sendmmsg(m.fileno(), ctypes.byref(msg), 1, 0)
'''


def calibrate(run):
    """外への接続・送信・名前の引きを 1 つずつ試みる小さなプログラムを strace で読み、見張りが捉えることを確かめる。"""
    log = run.log_dir / 'calibration.strace'
    cmd = run.tools.strace_command(log) + [sys.executable, '-c', CALIBRATION]
    subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    if not log.exists():
        raise Fail('strace がログを書かなかった（ptrace が許されていない環境かもしれない）')
    entries = parse_log(log.read_text(encoding='utf-8', errors='replace'))
    kinds = {}
    for e in entries:
        for d in e.dests:
            kinds.setdefault(d.kind, []).append(f'{e.call} {d.text}')
    problems = []
    if not any(e.exec_path for e in entries):
        problems.append('execve が読めない')
    if sum('192.0.2.1' in t for t in kinds.get('external', [])) < 3:
        problems.append('外への connect・sendto・sendmmsg（192.0.2.1）を捉えていない')
    if not any(t.startswith('sendmmsg') and '192.0.2.1' in t for t in kinds.get('external', [])):
        problems.append('宛先つきの sendmmsg を捉えていない')
    if not any(t.startswith('sendto') and ':53' in t for t in kinds.get('dns', [])):
        problems.append('ループバックの 53 番への送信を名前の引きとして捉えていない')
    if sum(not t.startswith('sendto') for t in kinds.get('dns', [])) < 1:
        problems.append('本物の名前の引き（getaddrinfo）を捉えていない')
    if not any(t.startswith('connect') for t in kinds.get('loopback', [])) or not any(t.startswith('bind') for t in kinds.get('loopback', [])):
        problems.append('ループバックの bind・connect を捉えていない')
    if problems:
        raise Fail('見張りの較正が通らない（アプリの結果は信用できない）: ' + '、'.join(problems) + f'。{log} を見る')
    spec = Spec('calibration', 'strace の読みが外への試みを捉えること（対照）')
    violations, _ = evaluate(spec, [Scene('対照', 0.0)], entries)
    if len(violations) < 5:
        raise Fail(f'較正の外への試みを判定が落としていない（{len(violations)} 件）')


# ───────── 入り口 ─────────

SESSIONS = ('main', 'reopen', 'idle', 'update')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'), help='yolupainter と yolupainter-cli がある場所')
    parser.add_argument('--log-dir', help='strace のログと表の置き場（既定は一時のフォルダ）')
    parser.add_argument('--work-dir', help='アプリの設定・書き出しの置き場（既定は一時のフォルダ。終わると消す）')
    parser.add_argument('--only', help='起動を絞る（コンマ区切り: ' + '・'.join(SESSIONS) + '）。reopen・idle・update は main の保存を使うので、絞ると文書なしで起動する')
    parser.add_argument('--time-limit', type=int, default=420, help='全体の上限（秒）')
    args = parser.parse_args(argv)
    if sys.platform != 'linux':
        print('network-watch は Linux だけ（strace を使う）', file=sys.stderr)
        return 2
    tools = Tools(args.bin_dir)
    missing = [name for name, found in (('strace', tools.strace), ('yolupainter', tools.app.exists()), ('yolupainter-cli', tools.cli.exists())) if not found]
    if missing:
        print('必要な物が無い: ' + '、'.join(missing) + '（strace は apt install strace、アプリは cargo build -p yolu-app -p yolu-cli）', file=sys.stderr)
        return 2
    if not os.environ.get('DISPLAY'):
        print('DISPLAY が無い。xvfb-run -a -s "-screen 0 1920x1080x24" python3 tools/network-watch.py … で回す', file=sys.stderr)
        return 2
    only = [n for n in (args.only.split(',') if args.only else SESSIONS) if n]
    unknown = [n for n in only if n not in SESSIONS]
    if unknown:
        print('知らない起動: ' + '、'.join(unknown), file=sys.stderr)
        return 2
    log_dir = Path(args.log_dir) if args.log_dir else Path(tempfile.mkdtemp(prefix='network-watch-logs-'))
    log_dir.mkdir(parents=True, exist_ok=True)
    for old in log_dir.glob('*.strace'):
        old.unlink()
    work_cleanup = args.work_dir is None
    work = Path(args.work_dir) if args.work_dir else Path(tempfile.mkdtemp(prefix='network-watch-work-'))
    work.mkdir(parents=True, exist_ok=True)
    run = Run(tools, work, log_dir, time.monotonic() + args.time_limit)
    print(f'ログ: {log_dir}')
    started = time.monotonic()
    sessions = []
    failures = []
    try:
        calibrate(run)
        print('較正: strace の読みが、外への connect・sendto・名前の引き・ループバックを捉えた')
        project = None
        for name in only:
            began = time.monotonic()
            if name == 'reopen' and project is None and 'main' in only:
                failures.append('reopen: 開き直す文書（main の保存）が無いので、起動しなかった')
                continue
            if name == 'main':
                s = session_main(run)
                link = s.dir / 'files/link.ylp'
                project = link if link.exists() else None
            elif name == 'reopen':
                s = session_reopen(run, project)
            elif name == 'idle':
                s = session_idle(run, project)
            else:
                s = session_update(run, project)
            if s.failure:
                failures.append(f'{name}: {s.failure}')
                print(f'× {name}: {s.failure}', file=sys.stderr)
            sessions.append((s, time.monotonic() - began))
    finally:
        for s, _ in sessions:
            s.kill()
    violations = []
    skipped = []
    print()
    for s, seconds in sessions:
        v, rows = evaluate(s.spec, s.scenes, s.entries())
        print(render_table(s.spec, s.scenes, rows, seconds))
        print(f'  終了: {s.exit_mode}')
        violations.extend(f'{s.spec.name} / {x}' for x in v)
        skipped.extend(skipped_scenes(s))
    total = time.monotonic() - started
    print(f'\n全体 {total:.0f} 秒（起動 {len(sessions)} 回）')
    if work_cleanup:
        shutil.rmtree(work, ignore_errors=True)
    # 飛ばした場面は、通ったことにせず目に付く形で残す（xdotool が効かないと、描く・キー操作の場面が空になる）
    for text in skipped:
        print(f'::warning::通信の見張りで飛ばした場面: {text}', file=sys.stderr)
    if violations or failures:
        for text in failures:
            print(f'::error::通れなかった場面: {text}', file=sys.stderr)
        for text in violations:
            print(f'::error::外への通信の試み・想定外: {text}', file=sys.stderr)
        print('\n許す通信は「更新の確認の curl」と「127.0.0.1 の受け口」だけです。新しい通信なら README・INSTALL の約束と見張りの許す一覧'
              '（crates/xtask/src/netguard.rs）を先に見直します。', file=sys.stderr)
        return 1
    print('外への通信の試みは 1 つもありませんでした（PC の中と 127.0.0.1・::1、更新の確認の curl だけ）。')
    return 0


if __name__ == '__main__':
    sys.exit(main())
