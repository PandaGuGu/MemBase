# -*- coding: utf-8 -*-
"""监听脚本：把「谁按了什么键、窗口什么状态、进程还在不在」记成时间线。

用来查「息屏退不出来」：跑起来之后照常操作（点息屏、按 ESC），
脚本会把每一次按键（系统级，含被别的程序吞掉的判定线索）和窗口状态变化写进日志，
最后把日志发回来即可。不改动系统、不联网、不驻留，Ctrl+C 就退出。

用法：
    py -3 tools/watch_input.py            # 日志写到 %TEMP%\\deskkit_watch_<时间>.log
    py -3 tools/watch_input.py out.log    # 指定日志文件
"""
import ctypes
import ctypes.wintypes as wt
import os
import sys
import threading
import time

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
psapi = ctypes.windll.psapi

try:
    user32.SetProcessDPIAware()  # 坐标一律按物理像素记，免得 125% 缩放下对不上
except Exception:
    pass

WH_KEYBOARD_LL = 13
WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP = 0x100, 0x101, 0x104, 0x105
GWL_EXSTYLE, WS_EX_TOPMOST = -20, 0x8
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000

# 关心的程序（窗口标题里的关键字 -> 友好名）
TARGETS = {
    '内存基准线 + 桌面布局': 'MemBase',
    '息屏助手': '息屏助手',
}
VK_NAMES = {0x1B: 'Esc', 0x0D: 'Enter', 0x20: 'Space', 0x09: 'Tab',
            0x5B: 'Win', 0x5C: 'Win', 0x10: 'Shift', 0x11: 'Ctrl', 0x12: 'Alt'}

log_lock = threading.Lock()
log_fp = None
start = time.monotonic()
hook_cb = None  # 钩子回调对象要一直留着，否则会被回收、钩子失效


def log(msg):
    with log_lock:
        line = '%8.3fs  %s' % (time.monotonic() - start, msg)
        log_fp.write(line + '\n')
        log_fp.flush()
        print(line, flush=True)


def vk_name(vk):
    return VK_NAMES.get(vk) or '0x%02X' % vk


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [('vkCode', wt.DWORD), ('scanCode', wt.DWORD), ('flags', wt.DWORD),
                ('time', wt.DWORD), ('dwExtraInfo', ctypes.c_void_p)]


HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_ssize_t, ctypes.c_int, wt.WPARAM, wt.LPARAM)

# 句柄类返回值得按指针宽度声明，否则 64 位下会被截断、钩子挂不上
user32.SetWindowsHookExW.restype = wt.HHOOK
user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wt.HINSTANCE, wt.DWORD]
user32.CallNextHookEx.restype = ctypes.c_ssize_t
user32.CallNextHookEx.argtypes = [wt.HHOOK, ctypes.c_int, wt.WPARAM, wt.LPARAM]
kernel32.GetModuleHandleW.restype = wt.HINSTANCE
kernel32.GetModuleHandleW.argtypes = [wt.LPCWSTR]


def proc_name(hwnd):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    h = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid.value)
    if not h:
        return 'pid=%d' % pid.value
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        if kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return os.path.basename(buf.value)
        return 'pid=%d' % pid.value
    finally:
        kernel32.CloseHandle(h)


def win_title(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 2)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


def describe(hwnd):
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return '可见=%d 置顶=%d 最小化=%d 尺寸=%dx%d' % (
        bool(user32.IsWindowVisible(hwnd)),
        bool(user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST),
        bool(user32.IsIconic(hwnd)),
        r.right - r.left, r.bottom - r.top)


def foreground():
    fg = user32.GetForegroundWindow()
    if not fg:
        return '(无)'
    return '%s / %s' % (proc_name(fg), win_title(fg) or '(无标题)')


def on_key(n_code, w_param, l_param):
    if n_code >= 0 and w_param in (WM_KEYDOWN, WM_SYSKEYDOWN):
        info = ctypes.cast(l_param, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
        log('[键] %-6s 按下   前台=%s' % (vk_name(info.vkCode), foreground()))
    elif n_code >= 0 and w_param in (WM_KEYUP, WM_SYSKEYUP):
        info = ctypes.cast(l_param, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
        log('[键] %-6s 抬起' % vk_name(info.vkCode))
    return user32.CallNextHookEx(None, n_code, w_param, l_param)


def hook_thread():
    global hook_cb
    hook_cb = HOOKPROC(on_key)  # 必须留住引用，否则回调会被回收
    hook = user32.SetWindowsHookExW(WH_KEYBOARD_LL, hook_cb, kernel32.GetModuleHandleW(None), 0)
    if hook:
        log('[启动] 键盘钩子已挂上（%d）' % hook)
    else:
        log('[启动] 键盘钩子没挂上，错误码 = %d（按键可能记不到）' % ctypes.get_last_error())
    msg = wt.MSG()
    while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
        user32.TranslateMessage(ctypes.byref(msg))
        user32.DispatchMessageW(ctypes.byref(msg))


def sample_loop():
    last = {}
    seen = set()
    while True:
        found = set()
        rows = []

        def cb(hwnd, _):
            if not user32.IsWindowVisible(hwnd):
                return True
            title = win_title(hwnd)
            for key, name in TARGETS.items():
                if key in title:
                    found.add(name)
                    rows.append((name, describe(hwnd)))
            return True

        enum_cb = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)(cb)
        user32.EnumWindows(enum_cb, 0)
        for name, desc in rows:
            if last.get(name) != desc:
                last[name] = desc
                log('[窗] %-10s %s  前台=%s' % (name, desc, foreground()))
                seen.add(name)
        for name in sorted(seen - found):
            log('[窗] %s 窗口已不存在（进程可能退了）' % name)
            seen.discard(name)
            last.pop(name, None)
        time.sleep(0.25)


def main():
    global log_fp
    path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.environ.get('TEMP', '.'), 'deskkit_watch_%s.log' % time.strftime('%m%d_%H%M%S'))
    log_fp = open(path, 'w', encoding='utf-8')
    log('[启动] 日志 = %s' % path)
    log('[启动] 现在请照常操作：点 MemBase 的「息屏」→ 按 ESC；再拿「息屏助手」对比一次')
    log('[启动] 结束时按 Ctrl+C')
    threading.Thread(target=hook_thread, daemon=True).start()
    try:
        sample_loop()
    except KeyboardInterrupt:
        log('[结束] 已停止')
    finally:
        log_fp.close()
        print('日志已保存：%s' % path)


if __name__ == '__main__':
    main()
