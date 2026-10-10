#!/usr/bin/env python3
"""実アプリの通信を見るツール（network-watch.py）の確かめ。strace のログの読み（宛先・プロセスの親子）と、落とす条件・許す条件の判定を、
作り物のログで確かめる。アプリは起動しない（strace がある環境では、読みが効くことの較正だけを本物の strace で通す）。"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time
import unittest
from unittest import mock
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('network_watch', ROOT / 'tools/network-watch.py')
nw = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nw)

APP = 1000
T0 = 1791601860.0


def line(pid, offset, text):
    return f'{pid} {T0 + offset:.6f} {text}'


def inet(ip, port):
    return f'{{sa_family=AF_INET, sin_port=htons({port}), sin_addr=inet_addr("{ip}")}}'


def inet6(ip, port):
    return f'{{sa_family=AF_INET6, sin6_port=htons({port}), sin6_flowinfo=htonl(0), inet_pton(AF_INET6, "{ip}", &sin6_addr), sin6_scope_id=0}}'


def connect(pid, offset, addr, ret='0'):
    return line(pid, offset, f'connect(5, {addr}, 16) = {ret}')


def entries_of(*lines):
    return nw.parse_log('\n'.join(lines) + '\n')


def scenes(*items):
    """items: (名前, 開始の秒, curl を許すか)。"""
    out = []
    for name, start, allow in items:
        out.append(nw.Scene(name, T0 + start, allow))
    return out


def violations(spec_, scene_list, lines):
    found, rows = nw.evaluate(spec_, scene_list, entries_of(*lines))
    return found, rows


def plain(listen_port=None, expect_curl=False):
    return nw.Spec('t', '試験', listen_port=listen_port, expect_curl=expect_curl)


class Parse(unittest.TestCase):
    def test_destinations_of_real_strace_lines(self):
        entries = entries_of(
            line(APP, 0.1, 'connect(4, {sa_family=AF_UNIX, sun_path=@"/tmp/.X11-unix/X102"}, 22) = 0'),
            line(APP, 0.2, 'bind(11, {sa_family=AF_INET, sin_port=htons(29347), sin_addr=inet_addr("127.0.0.1")}, 16) = 0'),
            line(APP, 0.3, 'listen(11, 128) = 0'),
            line(APP, 0.4, 'connect(3, {sa_family=AF_INET6, sin6_port=htons(6102), sin6_flowinfo=htonl(0), '
                           'inet_pton(AF_INET6, "::1", &sin6_addr), sin6_scope_id=0}, 28) = -1 ECONNREFUSED (Connection refused)'),
            line(APP, 0.5, 'bind(3, {sa_family=AF_NETLINK, nl_pid=0, nl_groups=00000000}, 12) = 0'),
            line(APP, 0.6, 'sendmsg(9, {msg_name=NULL, msg_namelen=0, msg_iov=[{iov_base="l\\0\\v", iov_len=48}], msg_iovlen=1}, 0) = 48'),
            line(APP, 0.7, 'sendto(3, [{nlmsg_len=20, nlmsg_type=RTM_GETADDR}, {ifa_family=AF_UNSPEC, ...}], 20, 0, '
                           '{sa_family=AF_NETLINK, nl_pid=0, nl_groups=00000000}, 12) = 20'),
            line(APP, 0.8, 'sendmsg(7, {msg_name=' + inet('93.184.216.34', 443) + ', msg_namelen=16, msg_iov=[{iov_base="x", iov_len=1}]}, 0) = 1'),
            line(APP, 0.9, 'connect(3, {sa_family=AF_UNSPEC, sa_data="\\0\\0"}, 16) = 0'),
        )
        got = [(e.call, [(d.kind, d.text) for d in e.dests]) for e in entries]
        self.assertEqual(got, [
            ('connect', [('unix', '/tmp/.X11-unix/X102')]),
            ('bind', [('loopback', '127.0.0.1:29347')]),
            ('listen', []),
            ('connect', [('loopback', '[::1]:6102')]),
            ('bind', [('netlink', 'netlink')]),
            ('sendto', [('netlink', 'netlink')]),
            ('sendmsg', [('external', '93.184.216.34:443')]),
        ], '宛先の無い sendmsg（X の画面）と AF_UNSPEC は捨てる')

    def test_kinds_of_addresses(self):
        def kind(addr):
            e = entries_of(connect(APP, 0, addr))
            return e[0].dests[0].kind
        for ip, expected in [
            ('127.0.0.1', 'loopback'), ('127.1.2.3', 'loopback'), ('8.8.8.8', 'external'), ('10.0.0.5', 'external'),
            ('192.168.1.1', 'external'), ('169.254.169.254', 'external'), ('0.0.0.0', 'external'),
        ]:
            self.assertEqual(kind(inet(ip, 80)), expected, ip)
        for ip, expected in [
            ('::1', 'loopback'), ('::ffff:127.0.0.1', 'loopback'), ('::ffff:8.8.8.8', 'external'),
            ('2606:4700::1111', 'external'), ('fe80::1', 'external'), ('::', 'external'),
        ]:
            self.assertEqual(kind(inet6(ip, 80)), expected, ip)

    def test_name_resolution_is_not_hidden_by_a_loopback_resolver(self):
        for addr in (inet('127.0.0.53', 53), inet('192.168.65.7', 53), inet('8.8.8.8', 853), inet6('::1', 53), inet('224.0.0.251', 5353)):
            self.assertEqual(entries_of(connect(APP, 0, addr))[0].dests[0].kind, 'dns', addr)
        e = entries_of(line(APP, 0, 'connect(3, {sa_family=AF_UNIX, sun_path="/run/systemd/resolve/io.systemd.Resolve"}, 110) = 0'))
        self.assertEqual(e[0].dests[0].kind, 'dns')
        e = entries_of(line(APP, 0, 'connect(3, {sa_family=AF_UNIX, sun_path="/var/run/avahi-daemon/socket"}, 110) = 0'))
        self.assertEqual(e[0].dests[0].kind, 'dns')
        # nscd のソケットは、名前の引きとは限らない（passwd などにも使う）
        e = entries_of(line(APP, 0, 'connect(9, {sa_family=AF_UNIX, sun_path="/var/run/nscd/socket"}, 110) = -1 ENOENT (No such file or directory)'))
        self.assertEqual(e[0].dests[0].kind, 'unix')

    def test_pid_prefix_forms_and_unfinished_calls(self):
        entries = entries_of(
            f'[pid  2000] {T0 + 1:.6f} connect(5, {inet("93.184.216.34", 80)}, 16 <unfinished ...>',
            f'[pid  2000] {T0 + 1.1:.6f} <... connect resumed>) = -1 ENETUNREACH (Network is unreachable)',
            f'{T0 + 2:.6f} +++ exited with 0 +++',
        )
        self.assertEqual([(e.pid, e.call, e.dests[0].kind) for e in entries], [(2000, 'connect', 'external')],
                         '試みは 1 回だけ数える（再開の行は数えない）')

    def test_sendmmsg_and_accept_destinations_on_the_resumed_line_are_read(self):
        entries = entries_of(
            f'[pid  2000] {T0 + 1:.6f} sendmmsg(7, <unfinished ...>',
            f'[pid  2001] {T0 + 1.05:.6f} write(1, "x", 1) = 1',
            f'[pid  2000] {T0 + 1.1:.6f} <... sendmmsg resumed>[{{msg_hdr={{msg_name={inet("93.184.216.34", 9)}, msg_namelen=16, '
            f'msg_iov=[{{iov_base="x", iov_len=1}}], msg_iovlen=1, msg_controllen=0, msg_flags=0}}, msg_len=1}}], 1, 0) = 1',
            f'[pid  2002] {T0 + 2:.6f} accept4(5, <unfinished ...>',
            f'[pid  2002] {T0 + 2.1:.6f} <... accept4 resumed>{inet("203.0.113.9", 51000)}, [16], SOCK_CLOEXEC) = 8',
        )
        got = [(e.call, e.dests[0].kind, e.dests[0].text) for e in entries]
        self.assertEqual(got, [('sendmmsg', 'external', '93.184.216.34:9'), ('accept4', 'external', '203.0.113.9:51000')])

    def test_a_destination_on_both_lines_is_counted_once(self):
        entries = entries_of(
            f'[pid  2000] {T0 + 1:.6f} sendmmsg(7, [{{msg_hdr={{msg_name={inet("93.184.216.34", 9)}, msg_namelen=16}}}}], 1, 0 <unfinished ...>',
            f'[pid  2000] {T0 + 1.1:.6f} <... sendmmsg resumed>) = 1',
        )
        self.assertEqual(len(entries), 1)
        entries = entries_of(
            f'[pid  2000] {T0 + 1:.6f} sendmsg(7, {{msg_name={inet("93.184.216.34", 9)}, msg_namelen=16}}, 0 <unfinished ...>',
            f'[pid  2000] {T0 + 1.1:.6f} <... sendmsg resumed>{{msg_name={inet("93.184.216.34", 9)}}}, 0) = 1',
        )
        self.assertEqual(len(entries), 1)

    def test_a_multi_message_sendmmsg_reports_every_destination(self):
        entries = entries_of(line(APP, 1, 'sendmmsg(7, [{msg_hdr={msg_name=' + inet('8.8.8.8', 53) + ', msg_namelen=16}, msg_len=1}, '
                                      '{msg_hdr={msg_name=' + inet('1.1.1.1', 53) + ', msg_namelen=16}, msg_len=1}], 2, 0) = 2'))
        self.assertEqual([d.text for d in entries[0].dests], ['8.8.8.8:53', '1.1.1.1:53'])

    def test_exec_and_clone_results(self):
        entries = entries_of(
            line(1, 0, 'execve("/usr/local/sbin/curl", ["curl"], 0x7ffd /* 3 vars */) = -1 ENOENT (No such file or directory)'),
            line(1, 0.1, 'execve("/usr/bin/curl", ["curl", "-q"], 0x7ffd /* 3 vars */) = 0'),
            line(1, 0.2, 'clone(child_stack=NULL, flags=CLONE_CHILD_CLEARTID|CLONE_CHILD_SETTID|SIGCHLD, child_tidptr=0x7f) = 77'),
            line(1, 0.3, 'clone3({flags=CLONE_VM|CLONE_THREAD, child_tid=0x7f}, 88) = 78'),
            line(1, 0.4, 'vfork() = -1 EAGAIN (Resource temporarily unavailable)'),
            f'[pid 1] {T0 + 0.5:.6f} clone3({{flags=CLONE_VM}}, 88 <unfinished ...>',
            f'[pid 1] {T0 + 0.6:.6f} <... clone3 resumed>) = 79',
        )
        self.assertEqual([(e.exec_path, e.child) for e in entries], [
            ('/usr/bin/curl', None), (None, 77), (None, 78), (None, 79)])


class Attribution(unittest.TestCase):
    def test_threads_and_children_inherit_the_executable(self):
        entries = entries_of(
            line(APP, 0, 'execve("/opt/yolupainter", ["yolupainter"], 0x7f /* 9 vars */) = 0'),
            line(APP, 1, 'clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 2000'),
            line(2000, 1.1, 'execve("/usr/bin/curl", ["curl"], 0x7f /* 9 vars */) = 0'),
            line(2000, 1.2, 'clone3({flags=CLONE_VM|CLONE_THREAD}, 88) = 2001'),
            line(2001, 1.3, 'connect(5, ' + inet('1.2.3.4', 443) + ', 16) = 0'),
            line(APP, 2, 'clone3({flags=CLONE_VM|CLONE_THREAD}, 88) = 3000'),
        )
        procs = nw.Procs(entries)
        self.assertEqual(procs.exe(2001, T0 + 1.3), '/usr/bin/curl')
        self.assertEqual(procs.exe(2000, T0 + 1.05), '/opt/yolupainter', 'execve の前は親のまま')
        self.assertEqual(procs.exe(3000, T0 + 3), '/opt/yolupainter')
        self.assertEqual(procs.exe(9, T0), '?')


class Judge(unittest.TestCase):
    def test_a_quiet_session_passes(self):
        found, rows = violations(plain(), scenes(('起動', 0, False)), [
            connect(APP, 1, '{sa_family=AF_UNIX, sun_path=@"/tmp/.X11-unix/X1"}'),
            connect(APP, 2, inet('127.0.0.1', 6000)),
        ])
        self.assertEqual(found, [])
        self.assertEqual((rows['起動']['unix'], rows['起動']['loopback'], rows['起動']['external']), (1, 1, 0))

    def test_any_external_attempt_by_the_app_fails_even_when_refused(self):
        for addr in (inet('93.184.216.34', 443), inet6('2606:4700::1111', 443), inet('10.0.0.5', 80)):
            found, _ = violations(plain(), scenes(('描く', 0, False)), [connect(APP, 1, addr, '-1 ECONNREFUSED (Connection refused)')])
            self.assertEqual(len(found), 1, addr)
            self.assertIn('描く', found[0])
            self.assertIn('connect', found[0])

    def test_sendto_and_sendmsg_with_an_address_fail_too(self):
        found, _ = violations(plain(), scenes(('保存', 0, False)), [
            line(APP, 1, 'sendto(3, "x", 1, 0, ' + inet('93.184.216.34', 9) + ', 16) = 1'),
            line(APP, 2, 'sendmsg(3, {msg_name=' + inet('93.184.216.34', 9) + ', msg_namelen=16, msg_iov=[]}, 0) = 1'),
        ])
        self.assertEqual(len(found), 2)

    def test_name_resolution_fails_even_through_a_loopback_stub(self):
        found, _ = violations(plain(), scenes(('起動', 0, False)), [
            line(APP, 1, 'connect(3, ' + inet('127.0.0.53', 53) + ', 16) = 0')])
        self.assertEqual(len(found), 1)
        self.assertIn('名前の引き', found[0])

    UPDATE_URL = 'https://github.com/YozoraKurage/YoluPainter/releases/latest/download/updater-v1.json'

    ARGV = ('["curl", "-q", "--silent", "--show-error", "--fail", "--location", "--max-redirs", "5", "--connect-timeout", "10", '
            '"--max-time", "30", "--user-agent", "YoluPainter/0.6.0", "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2", '
            '"--output", "-", "--", "{url}"]')

    def curl_lines(self, extra=(), url=UPDATE_URL, argv=None):
        argv = (argv or self.ARGV).format(url=url)
        return [
            line(APP, 0.5, 'execve("/opt/yolupainter", ["yolupainter"], 0x7f /* 9 vars */) = 0'),
            line(APP, 1, 'clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 2000'),
            line(2000, 1.1, f'execve("/usr/bin/curl", {argv}, 0x7f /* 9 vars */) = 0'),
            line(2000, 1.2, 'clone3({flags=CLONE_VM|CLONE_THREAD}, 88) = 2001'),
            line(2001, 1.3, 'connect(5, ' + inet('192.168.65.7', 53) + ', 16) = 0'),
            line(2000, 1.4, 'connect(6, ' + inet6('2606:50c0:8000::154', 443) + ', 28) = -1 ENETUNREACH (Network is unreachable)'),
            *extra,
        ]

    def test_curl_may_reach_out_only_in_the_update_scene(self):
        allowed = scenes(('起動', 0, False), ('更新の確認', 1, True), ('終了', 2, False))
        found, rows = violations(plain(expect_curl=True), allowed, self.curl_lines())
        self.assertEqual(found, [])
        self.assertEqual(rows['更新の確認']['external'], 2)
        self.assertEqual(rows['更新の確認']['curl'], 1)
        self.assertEqual(rows['更新の確認']['commands'], ['curl'])
        # 同じログでも、更新の確認の場面でなければ、curl の起動も外への試みも落とす
        found, _ = violations(plain(), scenes(('起動', 0, False)), self.curl_lines())
        self.assertEqual(len(found), 3, found)
        self.assertTrue(any('curl が起動された' in x for x in found))

    def test_curl_in_the_update_scene_may_fetch_only_the_update_urls(self):
        for url in ('https://example.com/upload', 'http://github.com/YozoraKurage/YoluPainter/x', 'https://github.com.evil.example/'):
            found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), self.curl_lines(url=url))
            self.assertEqual(len(found), 1, url)
            self.assertIn('許す形でない', found[0])
        # URL が読めない（引数が切れた）ときも通さない
        lines = self.curl_lines()
        lines[2] = line(2000, 1.1, 'execve("/usr/bin/curl", ["curl", "-q"], 0x7f /* 9 vars */) = 0')
        found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), lines)
        self.assertEqual(len(found), 1)
        beta = 'https://github.com/YozoraKurage/YoluPainter/releases/download/updater-beta/updater-v1.json'
        self.assertEqual(violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), self.curl_lines(url=beta))[0], [])

    def test_an_exec_split_in_two_lines_keeps_its_arguments(self):
        # vfork の子の execve は、引数のある行と `resumed` の行に分かれる。引数の確かめには引数が要る
        argv = self.ARGV.format(url=self.UPDATE_URL)
        lines = self.curl_lines()
        lines[2] = (f'2000 {T0 + 1.1:.6f} execve("/usr/bin/curl", {argv}, 0x7f /* 9 vars */ <unfinished ...>\n'
                    f'2000 {T0 + 1.2:.6f} <... execve resumed>) = 0')
        found, rows = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), lines)
        self.assertEqual(found, [])
        lines[2] = lines[2].replace(self.UPDATE_URL, 'https://192.0.2.1/')
        found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), lines)
        self.assertEqual(len(found), 1)
        self.assertIn('許す形でない', found[0])

    def test_curl_arguments_beyond_the_pinned_ones_are_refused(self):
        base = ['curl', '-q', '--silent', '--show-error', '--fail', '--location', '--max-redirs', '5', '--connect-timeout', '10',
                '--max-time', '30', '--user-agent', 'YoluPainter/0.6.0', '--proto', '=https', '--proto-redir', '=https',
                '--tlsv1.2', '--output', '-', '--', self.UPDATE_URL]
        self.assertIsNone(nw.check_curl_argv(base))
        at = base.index('--')
        extras = [['--data', 'id=1'], ['-d', 'x'], ['-H', 'X-Id: 1'], ['-T', 'file'], ['-b', 'a=b'], ['-K', 'cfg'], ['--referer', 'x'],
                  ['--cookie', 'a=b'], ['--url=https://example.com/'], ['--url', 'https://example.com/'], ['-F', 'a=@f'], ['-x', 'proxy'],
                  ['--proto', '=https'], ['-A', 'other']]
        for extra in extras:
            problem = nw.check_curl_argv(base[:at] + extra + base[at:])
            self.assertIsNotNone(problem, extra)
        # 値の形・必須・URL の形
        self.assertIn('値', nw.check_curl_argv([a if a != 'YoluPainter/0.6.0' else 'x y' for a in base]))
        self.assertIn('必要', nw.check_curl_argv([a for a in base if a != '-q']))
        self.assertIn('必要', nw.check_curl_argv(base[:base.index('--proto')] + base[base.index('--proto') + 2:]))
        self.assertIn('`--` の後', nw.check_curl_argv(base + ['https://github.com/YozoraKurage/YoluPainter/y']))
        self.assertIn('`--` が無い', nw.check_curl_argv(base[:at]))
        for url in (self.UPDATE_URL + '?id=1', self.UPDATE_URL + '#x', 'https://user@github.com/YozoraKurage/YoluPainter/x',
                    'https://github.com/OtherOwner/YoluPainter/x', 'http://github.com/YozoraKurage/YoluPainter/x',
                    'https://github.com/YozoraKurage/YoluPainter/a b'):
            self.assertIsNotNone(nw.check_curl_argv(base[:-1] + [url]), url)
        self.assertIsNotNone(nw.check_curl_argv(base, truncated=True))
        self.assertIsNotNone(nw.check_curl_argv(['wget'] + base[1:]))

    def test_the_update_curl_fails_the_run_when_it_carries_an_identifier(self):
        extra = ('["curl", "-q", "--proto", "=https", "--proto-redir", "=https", "-H", "X-Install-Id: 1234", "--", "{url}"]')
        found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), self.curl_lines(argv=extra))
        self.assertEqual(len(found), 1)
        self.assertIn('-H', found[0])

    def test_exec_argv_reads_the_list_and_notices_truncation(self):
        items, truncated = nw.exec_argv('execve("/usr/bin/curl", ["curl", "-q", "--", "https://github.com/Yo"...], 0x7f /* 9 vars */) = 0')
        self.assertEqual(items, ['curl', '-q', '--', 'https://github.com/Yo'])
        self.assertTrue(truncated)
        items, truncated = nw.exec_argv('execve("/usr/bin/curl", ["curl", "-q"], 0x7f /* 9 vars */) = 0')
        self.assertEqual((items, truncated), (['curl', '-q'], False))
        self.assertEqual(nw.exec_argv('connect(3, {})'), ([], True))

    def test_the_app_itself_may_not_reach_out_in_the_update_scene(self):
        extra = [line(APP, 1.5, 'connect(7, ' + inet('93.184.216.34', 443) + ', 16) = 0')]
        found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), self.curl_lines(extra))
        self.assertEqual(len(found), 1)
        self.assertIn(f'プロセス {APP}', found[0])

    def test_the_update_session_needs_curl_to_have_run(self):
        found, _ = violations(plain(expect_curl=True), scenes(('更新の確認', 0, True)), [
            connect(APP, 1, inet('127.0.0.1', 5))])
        self.assertEqual(len(found), 1)
        self.assertIn('curl の起動が見えない', found[0])

    def test_the_listener_must_be_exactly_the_loopback_port(self):
        good = [line(APP, 1, 'bind(11, ' + inet('127.0.0.1', 29347) + ', 16) = 0'), line(APP, 1.1, 'listen(11, 128) = 0')]
        self.assertEqual(violations(plain(29347), scenes(('起動', 0, False)), good)[0], [])
        for bad in (inet('0.0.0.0', 29347), inet('127.0.0.1', 1234), inet('192.168.1.5', 29347), inet6('::', 29347)):
            found, _ = violations(plain(29347), scenes(('起動', 0, False)), [
                line(APP, 1, 'bind(11, ' + bad + ', 16) = 0'), line(APP, 1.1, 'listen(11, 128) = 0')] + good)
            self.assertTrue(any('bind' in x for x in found), bad)

    def test_no_listener_when_the_setting_is_off(self):
        found, _ = violations(plain(None), scenes(('起動', 0, False)), [
            line(APP, 1, 'bind(11, ' + inet('127.0.0.1', 29347) + ', 16) = 0')])
        self.assertEqual(len(found), 1)
        self.assertIn('受け口を入れていない', found[0])

    def test_a_session_that_should_listen_but_never_does_is_reported(self):
        found, _ = violations(plain(29347), scenes(('起動', 0, False)), [connect(APP, 1, inet('127.0.0.1', 29347))])
        self.assertEqual(len(found), 1)
        self.assertIn('空振り', found[0])

    def test_the_crash_dialog_means_the_run_is_not_trustworthy(self):
        found, _ = violations(plain(), scenes(('終了', 0, False)), [
            line(APP, 1, 'execve("/opt/yolupainter", ["yolupainter"], 0x7f /* 9 vars */) = 0'),
            line(APP, 2, 'clone(child_stack=NULL, flags=SIGCHLD) = 5'),
            line(5, 2.1, 'execve("/usr/bin/zenity", ["zenity", "--question"], 0x7f /* 9 vars */) = 0')])
        self.assertEqual(len(found), 1)
        self.assertIn('落ちています', found[0])

    def test_events_before_the_first_scene_are_still_judged(self):
        found, _ = violations(plain(), scenes(('起動', 5, False)), [connect(APP, 1, inet('93.184.216.34', 443))])
        self.assertEqual(len(found), 1)
        self.assertIn('場面の前', found[0])

    def test_the_table_shows_scenes_and_destinations(self):
        scene_list = scenes(('起動', 0, False), ('3D のモデルと Live Link の受け渡し', 1, False))
        _, rows = violations(plain(), scene_list, [
            connect(APP, 0.5, '{sa_family=AF_UNIX, sun_path=@"/x"}'),
            connect(APP, 1.5, inet('127.0.0.1', 29347)), connect(APP, 1.6, inet('127.0.0.1', 29347))])
        text = nw.render_table(plain(), scene_list, rows, 12.3)
        self.assertIn('3D のモデルと Live Link の受け渡し', text)
        self.assertIn('connect 127.0.0.1:29347 ×2', text)
        self.assertIn('12 秒', text)


class Guard(unittest.TestCase):
    class Fake:
        failure = None
        killed = False

        def kill(self):
            self.killed = True

    def test_a_scene_that_cannot_finish_is_recorded_and_the_app_is_killed(self):
        fake = self.Fake()
        with nw.guarded(fake):
            raise nw.Fail('Live Link の返事が来ない')
        self.assertEqual(fake.failure, 'Live Link の返事が来ない')
        self.assertTrue(fake.killed)

    def test_programming_errors_are_not_swallowed(self):
        fake = self.Fake()
        with self.assertRaises(KeyError):
            with nw.guarded(fake):
                raise KeyError('x')
        self.assertTrue(fake.killed)
        self.assertIsNone(fake.failure)


class Skipped(unittest.TestCase):
    def test_scenes_that_could_not_run_are_listed_for_a_warning(self):
        class Fake:
            spec = plain()
            scenes = scenes(('起動', 0, False), ('描く（xdotool の筆）', 1, False))
        Fake.spec.name = 'main'
        Fake.scenes[1].skipped = True
        Fake.scenes[1].note = '筆が届かず、描けなかった（飛ばした）'
        self.assertEqual(nw.skipped_scenes(Fake), ['main / 描く（xdotool の筆）: 筆が届かず、描けなかった（飛ばした）'])


class Pieces(unittest.TestCase):
    def test_png_is_a_valid_png(self):
        data = nw.png_bytes(4)
        self.assertTrue(data.startswith(b'\x89PNG\r\n\x1a\n'))
        self.assertEqual(data[12:16], b'IHDR')
        self.assertEqual(int.from_bytes(data[16:20], 'big'), 4)

    def test_the_live_link_request_has_the_parts_the_app_reads(self):
        req = nw.arm_request('id-1', Path('/tmp/a b/arm.fbx'), Path('/tmp/skin.png'), Path('/tmp/out'))
        self.assertEqual((req['format'], req['kind'], req['id']), (1, 'open', 'id-1'))
        self.assertEqual(req['models'][0]['fbx'], '/tmp/a b/arm.fbx')
        self.assertEqual(req['target']['key'], 'GlobalObjectId_V1-2-id-1')
        for renderer in req['renderers']:
            self.assertIn(renderer['model'], [m['id'] for m in req['models']])
        json.dumps(req)

    def test_the_model_fixture_is_present(self):
        text = (ROOT / 'tools/network-watch/arm.fbx').read_text(encoding='utf-8')
        self.assertTrue(text.startswith('; FBX 7.4.0 project file'))
        self.assertIn('ArmMesh', text)

    def test_missing_tools_stop_the_run_with_status_2(self):
        with tempfile.TemporaryDirectory() as empty, mock.patch.dict(os.environ, {'DISPLAY': ':1'}), \
                contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(nw.main(['--bin-dir', empty]), 2)
        self.assertIn('必要な物が無い', err.getvalue())

    def test_a_display_is_required(self):
        with tempfile.TemporaryDirectory() as empty:
            for name in ('yolupainter', 'yolupainter-cli'):
                Path(empty, name).write_text('')
            env = {k: v for k, v in os.environ.items() if k != 'DISPLAY'}
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(nw.shutil, 'which', return_value='/usr/bin/strace'), \
                    contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(nw.main(['--bin-dir', empty]), 2)
            self.assertIn('DISPLAY', err.getvalue())

    def test_unknown_sessions_are_refused(self):
        with tempfile.TemporaryDirectory() as empty, mock.patch.dict(os.environ, {'DISPLAY': ':1'}), \
                mock.patch.object(nw.shutil, 'which', return_value='/usr/bin/strace'), contextlib.redirect_stderr(io.StringIO()) as err:
            for name in ('yolupainter', 'yolupainter-cli'):
                Path(empty, name).write_text('')
            self.assertEqual(nw.main(['--bin-dir', empty, '--only', 'nothing']), 2)
        self.assertIn('知らない起動', err.getvalue())


@unittest.skipUnless(shutil.which('strace') and sys.platform == 'linux', 'strace が無い')
class RealStrace(unittest.TestCase):
    """本物の strace で、読みが外への試みを捉えること（較正）と、静かなプログラムは落とさないことを確かめる。"""

    def run_under(self, code):
        tools = nw.Tools(ROOT / 'target/debug')
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / 'x.strace'
            result = nw.subprocess.run(tools.strace_command(log) + [sys.executable, '-c', code], capture_output=True, timeout=60)
            if not log.exists() or b'Operation not permitted' in result.stderr:
                self.skipTest('ptrace が許されていない')
            return nw.parse_log(log.read_text(errors='replace')), tmp

    def test_calibration_passes_with_a_real_strace(self):
        tools = nw.Tools(ROOT / 'target/debug')
        with tempfile.TemporaryDirectory() as tmp:
            run = nw.Run(tools, tmp, tmp, time.monotonic() + 120)
            try:
                nw.calibrate(run)
            except nw.Fail as error:
                if 'ptrace' in str(error):
                    self.skipTest(str(error))
                raise

    def test_a_quiet_program_has_no_violations(self):
        entries, _ = self.run_under('import socket\nl = socket.socket()\nl.bind(("127.0.0.1", 0))\nl.listen(1)\n'
                                    'c = socket.create_connection(l.getsockname())\n')
        found, _ = nw.evaluate(plain(), [nw.Scene('静か', 0.0)], entries)
        self.assertTrue(all('bind' in x for x in found), found)
        self.assertFalse(any('外' in x for x in found), found)


if __name__ == '__main__':
    unittest.main()
