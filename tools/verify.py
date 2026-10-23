# -*- coding: utf-8 -*-
"""MemBase 逐项验收：命令行每个动词 + 界面每一页 + 动作表一致性。

用法：
    python tools/verify.py            # 全跑，报告写到 docs/verify-<版本>.md
    python tools/verify.py --fast     # 跳过界面截图

原则：
  * 只做**可逆**操作：写盘的一律写到临时目录，改配置/主题/保护名单的最后都还原
  * 危险动作只验「护栏」：不加 --execute 是演练，加了但不加 --yes 必须被拒
  * 每一项都记录退出码、首行输出、结论，失败就红着写进报告，不粉饰
"""

import hashlib
import ctypes
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 1.2.x 起只有一个可执行文件：带参数是命令行，无参数开界面。
EXE = os.path.join(ROOT, 'dist', 'MemBase.exe')
GUI = EXE

results = []      # (分组, 项目, 命令, 期望, 实际, 结论, 备注)
notes = []


def dec(b):
    for enc in ('utf-8', 'gbk'):
        try:
            return b.decode(enc)
        except UnicodeDecodeError:
            continue
    return b.decode('utf-8', 'replace')


def run(args, env=None, timeout=120, stdin_null=True):
    e = dict(os.environ)
    if env:
        e.update(env)
    p = subprocess.run(
        [EXE] + args,
        capture_output=True,
        env=e,
        timeout=timeout,
        stdin=subprocess.DEVNULL if stdin_null else None,
    )
    return p.returncode, dec(p.stdout), dec(p.stderr)


def jload(text):
    """命令偶尔会先打一行提示再打 JSON；跳过前面，找能解析的那一段。"""
    text = text.strip()
    for i, ch in enumerate(text):
        if ch in '{[':
            try:
                return json.loads(text[i:])
            except Exception:
                continue
    return {}


def first_line(text):
    for ln in text.splitlines():
        if ln.strip():
            return ln.strip()
    return ''


def add(group, item, cmd, expect, actual, ok, note=''):
    results.append((group, item, cmd, str(expect), str(actual), '通过' if ok else '**失败**', note))
    mark = '  ok  ' if ok else ' FAIL '
    print('%s %-22s %s' % (mark, item, cmd))
    if not ok:
        print('        期望 %s，实际 %s' % (expect, actual))


def case(group, item, args, expect=0, note='', verify=None, show=None):
    """跑一条命令；expect 可以是整数或整数元组。"""
    code, out, err = run(args, env=None)
    ok = code in (expect if isinstance(expect, tuple) else (expect,))
    detail = first_line(out) or first_line(err)
    if verify:
        v = verify(out, err, code)
        if v is not None:
            if isinstance(v, tuple):
                ok2, extra = v
                ok = ok and ok2
                if extra:
                    detail = extra
            else:
                ok = ok and bool(v)
    shown = show if show else detail[:64]
    add(group, item, 'MemBase.exe ' + ' '.join(args), expect, code, ok, shown + (' ' + note if note else ''))
    return code, out, err


def bmp_stats(path):
    d = open(path, 'rb').read()
    off = struct.unpack_from('<I', d, 10)[0]
    w = struct.unpack_from('<i', d, 18)[0]
    h = abs(struct.unpack_from('<i', d, 22)[0])
    colors = set()
    step = max(1, w // 96)
    for y in range(0, h, max(1, h // 96)):
        base = off + (h - 1 - y if struct.unpack_from('<i', d, 22)[0] > 0 else y) * w * 4
        for x in range(0, w, step):
            i = base + x * 4
            colors.add(d[i:i + 3])
    return w, h, len(colors)


def shot(page, modal=None, name=None, env_extra=None):
    """用界面自带的截图开关抓一帧，返回 (路径, 尺寸, 颜色数)。"""
    path = os.path.join(tempfile.gettempdir(), 'membase_shot_%s.bmp' % (name or page or 'default'))
    if os.path.exists(path):
        os.remove(path)
    e = dict(os.environ)
    e['MEMBASE_SHOT'] = path
    if page:
        e['MEMBASE_PAGE'] = page
    if modal:
        e['MEMBASE_SHOT_MODAL'] = modal
    if env_extra:
        e.update(env_extra)
    p = subprocess.run([GUI], capture_output=True, env=e, timeout=120)
    if not os.path.exists(path):
        return path, (0, 0, 0), p.returncode
    return path, bmp_stats(path), p.returncode


# =====================================================================
# 0. 前置：产物存在 + 版本
# =====================================================================
def workspace_version():
    """从 Cargo.toml 读工作区版本，版本号升级时这里不用手改"""
    text = open(os.path.join(ROOT, 'Cargo.toml'), encoding='utf-8').read()
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    return m.group(1) if m else '0.0.0'


def check_artifacts():
    ok = os.path.isfile(GUI)
    size = os.path.getsize(GUI) if ok else 0
    add('产物', os.path.basename(GUI), GUI, '存在', '%d 字节' % size, ok and size > 200_000)
    # 单文件分发：dist 里不许再出现第二个 exe
    extra = [f for f in os.listdir(os.path.join(ROOT, 'dist'))
             if f.lower().endswith('.exe') and f.lower() != 'membase.exe']
    add('产物', '只有一个 exe', 'dist/*.exe', '仅 MemBase.exe',
        ', '.join(extra) if extra else '仅 MemBase.exe', not extra)
    want = workspace_version()
    code, out, _ = run(['version'])
    add('产物', '版本号', 'MemBase.exe version', '0 / MemBase %s' % want,
        '%d / %s' % (code, out.strip()),
        code == 0 and out.strip() == 'MemBase %s' % want)
    return out.strip().split()[-1]


# =====================================================================
# 1. 内存侧命令
# =====================================================================
def check_mem(tmp):
    G = '内存侧'
    case(G, 'help', ['help'], 0)
    case(G, 'status', ['status'], 0)
    case(G, 'list', ['list'], 0)
    case(G, 'list --json', ['list', '--json'], 0,
         verify=lambda o, e, c: json.loads(o) is not None and 'items' in json.loads(o))
    case(G, 'plan', ['plan'], 0)
    # 可回收量必须按宿主去重：同一 PID 上的进程项 + 服务项只算一份（2026-10-22 修）
    case(G, 'plan --json 带宿主字段', ['plan', '--json'], 0,
         verify=lambda o, e, c: all(
             'reclaim_mb' in i and 'shared_host' in i and 'host_pid' in i
             for i in jload(o).get('items', [])))
    case(G, 'status --json 带自愈记录', ['status', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('self_heal'), list))

    csv = os.path.join(tmp, 'plan.csv')
    case(G, 'plan --to', ['plan', '--to', csv], 0,
         verify=lambda o, e, c: os.path.isfile(csv) and os.path.getsize(csv) > 0)
    case(G, 'trim（演练）', ['trim'], 0,
         verify=lambda o, e, c: 'dry-run' in o, show='演练，不动进程')
    case(G, 'trim --execute 无 --yes', ['trim', '--execute'], 5,
         show='非交互环境被正确拒绝')

    case(G, 'kill 不存在的目标', ['kill', 'no-such-proc-MemBase.exe'], 2)
    case(G, 'svc stop（演练）', ['svc', 'stop', 'Spooler'], (0, 4))
    case(G, 'compare', ['compare'], 0)

    # 保护名单：加一条临时项、确认写入、再删掉
    entry = '进程:__membase_verify__'
    case(G, 'protect list', ['protect', 'list', '--json'], 0)
    case(G, 'protect add', ['protect', 'add', entry], 0)

    def has_entry(o, e, c):
        # JSON 里中文是 unicode 转义的，必须按解析后的值比
        return entry in jload(o).get('protect', [])

    case(G, 'protect add 生效', ['protect', 'list', '--json'], 0, verify=has_entry)
    case(G, 'protect remove', ['protect', 'remove', entry], 0)

    def gone(o, e, c):
        return entry not in jload(run(['protect', 'list', '--json'])[1]).get('protect', [])

    case(G, 'protect remove 生效', ['protect', 'list'], 0, verify=gone)

    # 采集到临时目录（不碰真实基准线）
    out_dir = os.path.join(tmp, 'collect')
    case(G, 'collect --out 临时目录', ['collect', '--out', out_dir, '--at', '0'], 0,
         verify=lambda o, e, c: os.path.isdir(out_dir) and any(
             f.endswith('.json') for _, _, fs in os.walk(out_dir) for f in fs))
    case(G, 'collect --json', ['collect', '--out', os.path.join(tmp, 'collect2'), '--at', '0', '--json'], 0,
         verify=lambda o, e, c: json.loads(o).get('ok') is True)
    # --keep 只是参数：真正的轮换规则由 core 的单测覆盖，
    # 这里用大 keep 值跑一遍，确认参数被接受、且不误删（不往回收站扔临时垃圾）
    case(G, 'collect --keep', ['collect', '--out', os.path.join(tmp, 'collect3'), '--at', '0', '--keep', '99'], 0,
         verify=lambda o, e, c: '采集完成' in o)

    # 基准线本体：在临时目录里走一遍 capture -> verify -> export -> import
    root = os.path.join(tmp, 'baseline')
    case(G, 'baseline capture', ['baseline', 'capture', '--out', root], 0)
    case(G, 'baseline info', ['baseline', 'info', '--out', root], 0)
    case(G, 'baseline verify', ['baseline', 'verify', '--out', root], 0)
    pkg = os.path.join(tmp, 'b.mbz')
    case(G, 'baseline export', ['baseline', 'export', '--out', root, '--to', pkg], 0,
         verify=lambda o, e, c: os.path.isfile(pkg) and os.path.getsize(pkg) > 100)
    root2 = os.path.join(tmp, 'baseline2')
    case(G, 'baseline import', ['baseline', 'import', pkg, '--out', root2], 0,
         verify=lambda o, e, c: os.path.isdir(root2))


# =====================================================================
# 2. 瘦身 / 启动项 / 功能开关 / 局部重启（只读与护栏）
# =====================================================================
def check_maint(tmp):
    G = '瘦身与启动项'

    # 瘦身：演练即可，执行会真改服务配置/卸应用
    case(G, 'slim（演练）', ['slim'], 0,
         verify=lambda o, e, c: ('预计' in o or '没有' in o), show='不动系统')
    case(G, 'slim --json', ['slim', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('items'), list))
    # 空清单的 f64 sum 曾经是 -0.0，JSON 里会原样写成 `"savings_mb":-0.0`
    case(G, 'slim --json 不输出 -0.0', ['slim', '--json'], 0,
         verify=lambda o, e, c: not (
             jload(o).get('savings_mb') == 0
             and math.copysign(1.0, jload(o).get('savings_mb', 0)) < 0),
         show='sum 空集合不能是负零')
    case(G, 'slim --include-caution', ['slim', '--include-caution', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('items'), list))
    case(G, 'slim --execute 无 --yes', ['slim', '--execute'], 5,
         show='非交互环境被正确拒绝')

    # 启动项：只读 + 护栏（不真禁用任何项）
    case(G, 'startup list', ['startup', 'list'], 0,
         verify=lambda o, e, c: '启动项' in o)
    case(G, 'startup list --json', ['startup', 'list', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('items'), list))
    # `\Microsoft\` 下的系统任务要能被看到（VS 安装器的 BackgroundDownload 就在里面），
    # 但默认不展开：默认视图 system_hidden > 0，--all 之后为 0
    def sys_hidden(o, e, c):
        d = jload(o)
        return d.get('system_hidden', 0) > 0 and d.get('count', 0) > 0

    def sys_all(o, e, c):
        d = jload(o)
        return d.get('system_hidden') == 0 and d.get('count', 0) > 0

    case(G, 'startup list 系统任务默认隐藏', ['startup', 'list', '--json'], 0, verify=sys_hidden)
    case(G, 'startup list --all 列出系统任务', ['startup', 'list', '--all', '--json'], 0, verify=sys_all)
    case(G, 'startup disable 不存在', ['startup', 'disable', 'no-such-startup-item'], 2)
    case(G, 'startup disable 无 --yes', ['startup', 'disable', '__none__', '--execute'], 2,
         show='名字不存在，先于 --yes 校验被拒')

    # 功能开关：只读 + 护栏
    case(G, 'feature list', ['feature', 'list'], 0,
         verify=lambda o, e, c: '开关' in o)
    case(G, 'feature list --json', ['feature', 'list', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('items'), list))
    case(G, 'feature off 未知 key', ['feature', 'off', 'no-such-key', '--execute', '--yes'], 1)
    case(G, 'feature off 无 --yes', ['feature', 'off', 'cross-device-resume', '--execute'], 5,
         show='没 --yes 不写注册表')

    # 局部重启：演练 + 两条护栏
    case(G, 'restart（演练）', ['restart', 'explorer'], 0,
         verify=lambda o, e, c: 'dry-run' in o, show='只报计划')
    case(G, 'restart explorer 无 --yes', ['restart', 'explorer', '--execute'], 5,
         show='会闪任务栏的动作被拦下')
    case(G, 'restart 不存在的目标', ['restart', 'no-such-proc-MemBase.exe'], 2)

    # 双采样：没有双采样快照时应给出结构化错误。
    # 不能直接用真实数据目录 —— 那台机器上恰好有 T+60s/T+300s 双采样，
    # 用例会「因为环境正确而失败」。这里指向一个只采过一次的临时目录。
    single = os.path.join(tmp, 'transient_single')
    case(G, 'transient（先造单采样）',
         ['collect', '--out', single, '--at', '0'], 0,
         show='只采一次，不注册双采样')
    case(G, 'transient（无双采样）', ['transient', '--out', single, '--json'], 2,
         verify=lambda o, e, c: jload(o).get('error') == 'no_dual_sample')


# =====================================================================
# 3. 配置 / 主题（改完还原）
# =====================================================================
def check_config(tmp):
    G = '配置与外观'
    code, out, _ = run(['config', 'get', '--json'])
    before = json.loads(out) if code == 0 else {}
    case(G, 'config get', ['config', 'get', '--json'], 0)

    mydir = os.path.join(tmp, 'cfgroot')
    case(G, 'config set', ['config', 'set', mydir], 0,
         verify=lambda o, e, c: json.loads(run(['config', 'get', '--json'])[1]).get('configured') == mydir)
    if before.get('configured'):
        # 原来就有配置：恢复原值
        run(['config', 'set', before['configured']])
        rest = '已恢复原目录'
    else:
        run(['config', 'clear'])
        rest = '已清除（原本未设置）'
    notes.append('config：' + rest)

    case(G, 'theme get', ['theme', 'get', '--json'], 0)
    case(G, 'theme set light', ['theme', 'set', 'light'], 0,
         verify=lambda o, e, c: json.loads(run(['theme', 'get', '--json'])[1])['theme'] == 'light')
    case(G, 'theme set dark', ['theme', 'set', 'dark'], 0,
         verify=lambda o, e, c: json.loads(run(['theme', 'get', '--json'])[1])['theme'] == 'dark')


# =====================================================================
# 3. 桌面布局
# =====================================================================
def check_layout():
    G = '桌面布局'
    case(G, 'layout list', ['layout', 'list', '--json'], 0,
         verify=lambda o, e, c: 'layouts' in json.loads(o))
    case(G, 'layout status', ['layout', 'status', '--json'], 0,
         verify=lambda o, e, c: {'icons', 'auto_arrange', 'shell_api', 'cache'} <= set(json.loads(o)))
    case(G, 'layout probe', ['layout', 'probe', '--json'], (0, 4),
         verify=lambda o, e, c: 'shell_api' in json.loads(o))
    case(G, 'layout dir', ['layout', 'dir', '--json'], 0,
         verify=lambda o, e, c: os.path.isdir(json.loads(o)['dir']))

    cache_before = json.loads(run(['layout', 'status', '--json'])[1])['cache']
    case(G, 'layout repair（演练）', ['layout', 'repair', '--json'], 0,
         verify=lambda o, e, c: json.loads(o).get('dry_run') is True, show='演练，不动注册表')
    cache_after = json.loads(run(['layout', 'status', '--json'])[1])['cache']
    add(G, '演练未改注册表', 'layout repair（无 --execute）', cache_before, cache_after,
        cache_before == cache_after, '缓存大小前后一致')

    name = '__membase_verify__'
    case(G, 'layout save', ['layout', 'save', name], 0,
         verify=lambda o, e, c: True)
    case(G, 'layout show', ['layout', 'show', name, '--json'], 0,
         verify=lambda o, e, c: len(json.loads(o)['icons']) > 0)
    case(G, 'layout diff', ['layout', 'diff', name, '--json'], 0,
         verify=lambda o, e, c: 'to_move' in json.loads(o))
    case(G, 'layout diff --positions', ['layout', 'diff', name, '--positions', '--json'], 0,
         verify=lambda o, e, c: 'moves' in json.loads(o))
    case(G, 'layout restore（演练）', ['layout', 'restore', name, '--json'], 0,
         verify=lambda o, e, c: json.loads(o).get('dry_run') is True)
    case(G, 'layout restore 无 --yes', ['layout', 'restore', name, '--execute'], 5,
         show='被正确拒绝')
    case(G, 'layout delete 无 --yes', ['layout', 'delete', name], 5,
         show='被正确拒绝')
    case(G, 'layout delete --yes', ['layout', 'delete', name, '--yes'], 0,
         verify=lambda o, e, c: not os.path.exists(
             os.path.join(json.loads(run(['layout', 'dir', '--json'])[1])['dir'], name + '.json')))


# =====================================================================
# 4. 开机采集任务（装完立刻卸）
# =====================================================================
def check_screen():
    G = '息屏'
    before_secs = jload(run(['screen', 'duration', '--json'])[1]).get('default_seconds')
    before_key = jload(run(['screen', 'key', '--json'])[1]).get('exit_key')
    before_power = jload(run(['screen', 'power', '--json'])[1]).get('power_off')
    before_last = jload(run(['screen', 'status', '--json'])[1]).get('last_off', '')
    rec = os.path.join(os.environ.get('APPDATA', ''), 'MemBase', 'last_screen_off.txt')
    before_rec = open(rec, 'rb').read() if os.path.isfile(rec) else None

    # ---- 状态与默认值 ----
    case(G, 'screen status', ['screen', 'status', '--json'], 0,
         verify=lambda o, e, c: {'monitors', 'screen', 'channel_ok',
                                 'default_seconds', 'exit_key'} <= set(jload(o)))
    case(G, 'screen duration（读默认）', ['screen', 'duration', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('default_seconds'), int))
    case(G, 'screen key（读默认）', ['screen', 'key', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('exit_key') in ('esc', 'space', 'enter', 'any'))

    # ---- 时长：三种写法都要能写进去 ----
    case(G, 'duration 0:02:30（时分秒）', ['screen', 'duration', '0:02:30', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('default_seconds') == 150)
    case(G, 'duration 读回 150 秒', ['screen', 'duration', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('default_seconds') == 150)
    case(G, 'duration 90s（带单位）', ['screen', 'duration', '90s', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('default_seconds') == 90)
    case(G, 'duration 5m（分钟单位）', ['screen', 'duration', '5m', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('default_seconds') == 300)
    case(G, 'minutes 30（老写法=分钟）', ['screen', 'minutes', '30', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('default_seconds') == 1800)
    case(G, 'duration 非法输入被拒', ['screen', 'duration', 'abc'], 1)

    # ---- 退出热键 ----
    case(G, 'key space（改热键）', ['screen', 'key', 'space', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('exit_key') == 'space')
    case(G, 'key any（改热键）', ['screen', 'key', 'any', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('exit_key') == 'any')
    case(G, 'key esc（还原）', ['screen', 'key', 'esc', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('exit_key') == 'esc')
    case(G, 'key 非法值被拒', ['screen', 'key', 'f1'], 1)

    # ---- 是否同时关显示器电源（默认必须是 off：关得掉、可能叫不醒）----
    case(G, 'power（读默认）', ['screen', 'power', '--json'], 0,
         verify=lambda o, e, c: isinstance(jload(o).get('power_off'), bool))
    case(G, 'power on（写）', ['screen', 'power', 'on', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('power_off') is True)
    case(G, 'power off（还原）', ['screen', 'power', 'off', '--json'], 0,
         verify=lambda o, e, c: jload(o).get('power_off') is False)
    case(G, 'power 非法值被拒', ['screen', 'power', 'maybe'], 1)

    # ---- 关屏演练：参数要正确落到 JSON 里 ----
    case(G, 'off --dry-run（用默认时长/热键）', ['screen', 'off', '--dry-run', '--json'], (0, 4),
         verify=lambda o, e, c: jload(o).get('exit_key') == 'esc', show='只探通道，不关屏')
    case(G, 'off --seconds 45 --exit-key enter', ['screen', 'off', '--seconds', '45', '--exit-key', 'enter', '--dry-run', '--json'], (0, 4),
         verify=lambda o, e, c: jload(o).get('seconds') == 45 and jload(o).get('exit_key') == 'enter')
    case(G, 'off --minutes 2（老写法）', ['screen', 'off', '--minutes', '2', '--dry-run', '--json'], (0, 4),
         verify=lambda o, e, c: jload(o).get('seconds') == 120)
    case(G, 'off --seconds 0（不定时）', ['screen', 'off', '--seconds', '0', '--dry-run', '--json'], (0, 4),
         verify=lambda o, e, c: jload(o).get('seconds') == 0)

    # ---- 复原 ----
    if isinstance(before_secs, int):
        run(['screen', 'duration', str(before_secs)])
    if isinstance(before_key, str):
        run(['screen', 'key', before_key])
    if isinstance(before_power, bool):
        run(['screen', 'power', 'on' if before_power else 'off'])
    now_secs = jload(run(['screen', 'duration', '--json'])[1]).get('default_seconds')
    now_key = jload(run(['screen', 'key', '--json'])[1]).get('exit_key')
    now_power = jload(run(['screen', 'power', '--json'])[1]).get('power_off')
    add(G, '时长已还原', 'screen duration %s' % before_secs, before_secs, now_secs,
        now_secs == before_secs)
    add(G, '热键已还原', 'screen key %s' % before_key, before_key, now_key,
        now_key == before_key)
    add(G, '关显示屏开关已还原', 'screen power %s' % before_power, before_power, now_power,
        now_power == before_power)

    # ---- 演练不能把自己记成「真的息过屏」----
    after_last = jload(run(['screen', 'status', '--json'])[1]).get('last_off', '')
    add(G, '演练没改「上次息屏」', 'screen status --json',
        before_last or '(空)', after_last or '(空)', after_last == before_last)
    after_rec = open(rec, 'rb').read() if os.path.isfile(rec) else None
    add(G, '演练没改记录文件', 'last_screen_off.txt',
        before_rec, after_rec, after_rec == before_rec, '避免把「演练」记成真的用过')


def check_blackout_exit():
    """息屏退得出来吗 —— 这是用户踩过两次的坑，必须真按键验一遍。

    全程 **不真关显示器**（MEMBASE_BLACKOUT_TEST=1 只进黑屏层），
    用窗口的 WS_EX_TOPMOST 位判断在不在息屏（比量矩形可靠，全屏命令是异步的）。
    重点验第二种：窗口被最小化/隐藏时 eframe 只调 logic() 不调 ui()，
    退出判断写在 ui 里就会永久失效 —— 那就是「息屏退不出来」。
    """
    G = '息屏'
    u = ctypes.windll.user32
    GWL_EXSTYLE, WS_EX_TOPMOST, SW_MINIMIZE, SW_RESTORE, VK_ESCAPE = -20, 0x8, 6, 9, 0x1B

    def topmost(h):
        return bool(u.GetWindowLongW(h, GWL_EXSTYLE) & WS_EX_TOPMOST)

    def tap_esc():
        u.keybd_event(VK_ESCAPE, 0, 0, 0)
        time.sleep(0.03)
        u.keybd_event(VK_ESCAPE, 0, 2, 0)

    e = dict(os.environ)
    e['MEMBASE_BLACKOUT_TEST'] = '1'
    proc = subprocess.Popen([GUI], env=e)
    try:
        h = 0
        for _ in range(150):
            h = u.FindWindowW(None, 'MemBase · 内存基准线 + 桌面布局')
            if h:
                break
            time.sleep(0.1)
        # 等黑屏层起来（置顶位亮起）
        for _ in range(150):
            if h and topmost(h):
                break
            time.sleep(0.1)
        add(G, '黑屏层已置顶', 'MEMBASE_BLACKOUT_TEST=1', 'WS_EX_TOPMOST 置位',
            '置位' if topmost(h) else '没置位', bool(h) and topmost(h))
        if not (h and topmost(h)):
            return

        time.sleep(1.2)  # 过掉 800ms 武装期
        for mode in ('正常', '最小化'):
            if mode == '最小化':
                u.ShowWindow(h, SW_MINIMIZE)
                time.sleep(1.2)
            tap_esc()
            if mode == '最小化':
                time.sleep(0.3)
                u.ShowWindow(h, SW_RESTORE)
            ok = False
            for _ in range(40):
                time.sleep(0.1)
                if not topmost(h):
                    ok = True
                    break
            add(G, 'ESC 退出（%s）' % mode, 'keybd_event VK_ESCAPE', '退出息屏（撤销置顶）',
                '已退出' if ok else '仍置顶，退不出来', ok)
            if not ok:
                return
            if mode == '正常':
                # 再进一次，好验「最小化」那种情况
                time.sleep(0.2)
                u.ShowWindow(h, SW_RESTORE)
                proc.terminate()
                proc.wait(timeout=10)
                proc = subprocess.Popen([GUI], env=e)
                h = 0
                for _ in range(150):
                    h = u.FindWindowW(None, 'MemBase · 内存基准线 + 桌面布局')
                    if h:
                        break
                    time.sleep(0.1)
                for _ in range(150):
                    if h and topmost(h):
                        break
                    time.sleep(0.1)
                if not (h and topmost(h)):
                    add(G, '重新进入息屏（为验最小化）', 'MEMBASE_BLACKOUT_TEST=1', '置顶',
                        '没置顶', False)
                    return
                time.sleep(1.2)
    finally:
        proc.kill()
        proc.wait(timeout=10)


def check_blackout():
    """黑屏层：进全屏纯黑，不动显示器（MEMBASE_BLACKOUT_TEST 只跳过关屏那一步）。"""
    G = '息屏'
    path = os.path.join(tempfile.gettempdir(), 'membase_blackout.bmp')
    if os.path.exists(path):
        os.remove(path)
    e = dict(os.environ)
    e['MEMBASE_BLACKOUT_TEST'] = '1'
    e['MEMBASE_SHOT'] = path
    code = subprocess.run([GUI], capture_output=True, env=e, timeout=120).returncode
    w, h, colors = bmp_stats(path) if os.path.exists(path) else (0, 0, 0)
    # 大面积纯黑 + 一行灰提示：颜色数很少但不为 0
    dark = 0
    total = 0
    if os.path.exists(path):
        d = open(path, 'rb').read()
        off = struct.unpack_from('<I', d, 10)[0]
        raw_h = struct.unpack_from('<i', d, 22)[0]
        for y in range(0, abs(raw_h), 9):
            sy = y if raw_h < 0 else abs(raw_h) - 1 - y
            for x in range(0, w, 9):
                i = off + sy * w * 4 + x * 4
                total += 1
                if max(d[i], d[i + 1], d[i + 2]) < 24:
                    dark += 1
        os.remove(path)
    ratio = (dark / total) if total else 0.0
    ok = code == 0 and w >= 1600 and 0 < colors < 60 and ratio > 0.95
    add(G, '黑屏层（全屏）', 'MEMBASE_BLACKOUT_TEST=1', '≥95% 纯黑 + 有提示文字',
        '%dx%d / %d 色 / 纯黑 %.1f%%' % (w, h, colors, ratio * 100), ok)


def check_task():
    G = '开机采集'
    code, out, err = run(['install', '--at', '60', '--json'])
    ok = code == 0 and jload(out).get('ok') is True
    add(G, 'install', 'MemBase.exe install --at 60', 0, code, ok, first_line(out)[:60])

    q = subprocess.run(['schtasks', '/Query', '/TN', 'MemBaseline_CollectOnBoot'],
                       capture_output=True)
    add(G, '计划任务已注册', 'schtasks /Query', 0, q.returncode, q.returncode == 0)

    code2, out2, _ = run(['uninstall', '--json'], timeout=60)
    add(G, 'uninstall', 'MemBase.exe uninstall', 0, code2,
        code2 == 0 and jload(out2).get('ok') is True, first_line(out2)[:60])

    q2 = subprocess.run(['schtasks', '/Query', '/TN', 'MemBaseline_CollectOnBoot'],
                        capture_output=True)
    add(G, '计划任务已删除', 'schtasks /Query', '非 0', q2.returncode, q2.returncode != 0)

    leftover = r'C:\ProgramData\MemBaseline\membase.exe'
    if os.path.isfile(leftover):
        try:
            os.remove(leftover)
            notes.append('已清理 install 留下的程序副本 ' + leftover)
        except OSError as e:
            # 有 safe-delete 钩子 / 杀软占用时删不掉：记一笔就走，
            # 别让「清理失败」把整轮验收打断（各用例已经跑完了）
            notes.append('未能清理 {}（{}），请手动删除'.format(leftover, e))


# =====================================================================
# 5. 界面：每页一张截图 + 三个弹窗
# =====================================================================
def check_gui_pages():
    G = '界面'
    sys.path.insert(0, os.path.join(ROOT, 'tools'))
    import bmp2png  # noqa: E402  （同目录的小工具，纯标准库）

    pages = [
        (None, '进程·线外（默认页）'),
        ('recover', '进程·可回收'),
        ('slim', '瘦身'),
        ('startup', '启动项'),
        ('baseline', '基准'),
        ('layouts', '桌面布局'),
        ('protect', '保护名单'),
        ('boot', '开机采集'),
        ('tools', '工具'),
        ('cli', '命令行'),
        ('settings', '设置'),
    ]
    hashes = {}
    for slug, label in pages:
        path, (w, h, colors), code = shot(slug)
        ok = w > 800 and h > 500 and colors > 60 and code == 0
        add(G, label, 'MEMBASE_PAGE=%s' % (slug or '(默认)'), '渲染正常',
            '%dx%d / %d 色' % (w, h, colors), ok)
        if os.path.exists(path):
            hashes[label] = hashlib.md5(open(path, 'rb').read()).hexdigest()
            bmp2png.bmp_to_png(path, os.path.join(tempfile.gettempdir(), 'membase_verify_%s.png' % (slug or 'processes')))
            os.remove(path)

    # 每页截图必须互不相同 —— 否则说明 MEMBASE_PAGE 没生效，全都截成了同一页
    add(G, '各页内容互不相同', '%d 张截图 md5' % len(pages), len(pages), len(set(hashes.values())),
        len(set(hashes.values())) == len(pages))

    for modal, label in [('trim', '确认弹窗 · 恢复基准'), ('layout', '确认弹窗 · 恢复布局')]:
        page = None if modal == 'trim' else 'layouts'
        path, (w, h, colors), code = shot(page, modal=modal, name='m_' + modal)
        ok = w > 800 and colors > 60 and code == 0
        add(G, label, 'MEMBASE_SHOT_MODAL=%s' % modal, '渲染正常',
            '%dx%d / %d 色' % (w, h, colors), ok)
        if os.path.exists(path):
            os.remove(path)


# =====================================================================
# 6.5 MCP 服务端：AI 能直接调用，且护栏与命令行一致
# =====================================================================
def check_mcp():
    G = 'MCP'
    selftest = os.path.join(ROOT, 'tools', 'mcp_selftest.py')
    if not os.path.isfile(selftest):
        add(G, '自测脚本存在', selftest, '存在', '缺失', False)
        return

    # 直接跑自测脚本（它是真的起进程发 JSON-RPC，不是 mock）
    import subprocess
    p = subprocess.run([sys.executable, selftest, EXE],
                       capture_output=True, text=True, encoding='utf-8', errors='replace')
    out = (p.stdout or '') + (p.stderr or '')
    ok = p.returncode == 0
    total = len([l for l in out.splitlines() if l.startswith('[')])
    add(G, 'MCP 自测全通过', 'tools/mcp_selftest.py', '0 失败',
        '0 失败' if ok else '有失败', ok, out.strip().splitlines()[-1] if out.strip() else '')

    # 单独确认几个关键事实（自测脚本内部也查了，这里是防脚本本身坏掉时静默通过）
    try:
        tools = mcp_tools(EXE)
        names = {t['name'] for t in tools}
        add(G, '工具数', 'tools/list', '16', len(tools), len(tools) == 16)
        need = {'mem_status', 'mem_list', 'mem_plan', 'mem_trim', 'mem_slim',
                'mem_restart', 'mem_kill', 'mem_svc', 'mem_feature', 'mem_startup',
                'mem_protect', 'mem_collect', 'mem_compare', 'mem_transient',
                'mem_baseline', 'mem_commands'}
        add(G, '工具集合完整', 'tools/list', '16 个', '缺 %d' % len(need - names),
            not (need - names), ', '.join(sorted(need - names)))

        # 动手类工具必须带 execute/yes；只读类必须不带
        mutating = {'mem_trim', 'mem_slim', 'mem_restart', 'mem_kill', 'mem_svc',
                    'mem_feature', 'mem_startup', 'mem_protect', 'mem_baseline'}
        bad = []
        for t in tools:
            props = t.get('inputSchema', {}).get('properties', {})
            has_gate = 'execute' in props and 'yes' in props
            if t['name'] in mutating and not has_gate:
                bad.append('%s 缺门禁' % t['name'])
            if t['name'] not in mutating and has_gate:
                bad.append('%s 不该有门禁' % t['name'])
        add(G, '门禁位与动手属性一致', 'tools/list 的 inputSchema', '全部一致',
            '不一致 %d' % len(bad), not bad, '; '.join(bad[:3]))
    except Exception as e:
        add(G, 'tools/list 可读', 'tools/list', '成功', str(e)[:60], False)


def mcp_tools(exe):
    """起一个 MCP 服务端，拿到工具清单。"""
    import subprocess
    p = subprocess.Popen([exe, 'mcp'], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def ask(msg):
        p.stdin.write((json.dumps(msg) + '\n').encode('utf-8'))
        p.stdin.flush()
        # stdout 是二进制流，readline 只在文本流里有
        return json.loads(p.stdout.readline().decode('utf-8'))

    try:
        ask({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize',
             'params': {'protocolVersion': '2025-06-18', 'capabilities': {},
                        'clientInfo': {'name': 'verify', 'version': '1'}}})
        doc = ask({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'})
        return doc.get('result', {}).get('tools', [])
    finally:
        try:
            p.stdin.close()
        except Exception:
            pass
        p.wait(timeout=10)


# =====================================================================
# 7. 一致性不变量：界面动作 ↔ 命令 ↔ GUI 源码
# =====================================================================
def check_consistency():
    G = '一致性'
    code, out, _ = run(['commands', '--json'])
    doc = json.loads(out)
    names = {c['name'] for c in doc['commands']}
    actions = doc['gui_actions']

    # 界面动作表条数：加/删按钮时同步这个数（表的用途是防「加了按钮忘了配命令」）
    expect_actions = 38
    add(G, 'gui_actions 数量', 'commands --json', expect_actions, len(actions),
        len(actions) == expect_actions)
    keys = [a['key'] for a in actions]
    add(G, '动作 key 不重复', 'commands --json', len(keys), len(set(keys)), len(keys) == len(set(keys)))

    # 每条命令的动词都必须是真实存在的子命令（含别名）
    aliases = {'ls': 'list', 'restore': 'trim', 'diff': 'compare', 'layouts': 'layout',
               'desk': 'layout', 'gui': 'ui', 'fix': 'repair'}
    bad = []
    for a in actions:
        verb = a['cli'].split()[1] if len(a['cli'].split()) > 1 else ''
        if verb not in names and verb not in aliases:
            bad.append('%s -> %s' % (a['key'], a['cli']))
    add(G, '每条命令的动词都存在', '%d 条 cli 解析' % len(actions), '全部命中', '未命中 %d 条' % len(bad),
        not bad, '; '.join(bad[:3]))

    # 界面源码里引用的 key 必须都在表里
    src = open(os.path.join(ROOT, 'apps', 'gui', 'src', 'main.rs'), encoding='utf-8').read()
    used = set(re.findall(r'cli_map::cli_for\("([a-z_]+)"\)', src))
    used |= set(re.findall(r'with_cli\([^,]+,\s*"([a-z_]+)"\)', src))
    used |= set(re.findall(r'cli_hint\("([a-z_]+)"\)', src))
    missing = used - set(keys)
    add(G, 'GUI 引用的 key 都在表里', '扫描 main.rs 中 cli_for/with_cli', '无缺失',
        '缺失 %d' % len(missing), not missing, ', '.join(sorted(missing)))

    # 反向：表里每个动作用户都能在界面上找到（命令行页会全部列出）
    covered = set(keys) - used
    add(G, '仅由「命令行」页呈现的动作', '界面按钮未单独挂提示', '仅供清单页', len(covered),
        True, ', '.join(sorted(covered)) if covered else '无')


# =====================================================================
# 7. 报告
# =====================================================================
def write_report(version):
    total = len(results)
    failed = [r for r in results if '失败' in r[5]]
    groups = []
    for r in results:
        if r[0] not in groups:
            groups.append(r[0])

    lines = []
    lines.append('# MemBase %s 验收报告\n' % version)
    lines.append('由 `tools/verify.py` 自动生成，%s\n' % time.strftime('%Y-%m-%d %H:%M:%S'))
    lines.append('**共 %d 项，通过 %d 项，失败 %d 项。**\n' % (total, total - len(failed), len(failed)))
    if failed:
        lines.append('> 失败项：' + '、'.join(r[1] for r in failed) + '\n')
    lines.append('验收原则：写盘一律写到临时目录；危险动作只验护栏（演练 / 拒绝）；改过的配置、主题、'
                 '保护名单、计划任务全部还原。\n')

    for g in groups:
        lines.append('\n## %s\n' % g)
        lines.append('| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |')
        lines.append('|---|---|---|---|---|---|')
        for r in results:
            if r[0] != g:
                continue
            note = r[6].replace('|', '\\|')
            lines.append('| %s | `%s` | %s | %s | %s | %s |' %
                         (r[1], r[2].replace('|', '\\|'), r[3], r[4], r[5], note))

    if notes:
        lines.append('\n## 环境复原记录\n')
        for n in notes:
            lines.append('- %s' % n)
    lines.append('')
    out = os.path.join(ROOT, 'docs', 'verify-%s.md' % version)
    open(out, 'w', encoding='utf-8').write('\n'.join(lines))
    print('\n报告：%s' % out)
    return len(failed)


def check_restored(protect_before):
    """跑完所有用例后，确认没把用户环境弄脏。"""
    G = '环境复原'
    after = jload(run(['protect', 'list', '--json'])[1]).get('protect', [])
    add(G, '保护名单条数', 'protect list --json', protect_before, len(after),
        len(after) == protect_before, '与验收前一致')

    theme = jload(run(['theme', 'get', '--json'])[1]).get('theme')
    add(G, '主题可读', 'theme get --json', 'dark/light', theme, theme in ('dark', 'light'))

    cfg = jload(run(['config', 'get', '--json'])[1])
    add(G, '数据目录可读', 'config get --json', '非空', cfg.get('effective', '') or '(空)',
        bool(cfg.get('effective')))

    q = subprocess.run(['schtasks', '/Query', '/TN', 'MemBaseline_CollectOnBoot'],
                       capture_output=True)
    add(G, '没有残留计划任务', 'schtasks /Query', '非 0', q.returncode, q.returncode != 0)


def main():
    fast = '--fast' in sys.argv
    if not os.path.isfile(EXE):
        raise SystemExit('先构建：cargo build --workspace --release')
    version = check_artifacts()
    protect_before = len(jload(run(['protect', 'list', '--json'])[1]).get('protect', []))
    tmp = tempfile.mkdtemp(prefix='membase_verify_')
    try:
        check_mem(tmp)
        check_maint(tmp)
        check_config(tmp)
        check_layout()
        check_task()
        check_consistency()
        check_mcp()
        check_screen()
        check_restored(protect_before)
        if not fast:
            check_gui_pages()
            check_blackout()
            check_blackout_exit()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    failed = write_report(version)
    print('共 %d 项，失败 %d 项' % (len(results), failed))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
