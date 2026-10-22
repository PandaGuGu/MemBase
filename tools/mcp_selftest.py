"""MCP 客户端实测：真的按 JSON-RPC 协议跟 MemBase 对话。

不是 mock —— 真的起进程、发 initialize、tools/list、tools/call，
把返回的 JSON 解析出来看。
"""
import json
import subprocess
import sys

EXE = sys.argv[1] if len(sys.argv) > 1 else r"C:\Users\Administrator\Desktop\项目集合\MemBase\target\release\membase-cli.exe"


class McpClient:
    def __init__(self, exe):
        self.p = subprocess.Popen(
            [exe, "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
        )
        self.id = 0

    def call(self, method, params=None):
        self.id += 1
        msg = {"jsonrpc": "2.0", "id": self.id, "method": method}
        if params is not None:
            msg["params"] = params
        line = json.dumps(msg, ensure_ascii=False) + "\n"
        self.p.stdin.write(line.encode("utf-8"))
        self.p.stdin.flush()
        out = self.p.stdout.readline()
        if not out:
            err = self.p.stderr.read().decode("utf-8", "replace")
            raise RuntimeError(f"服务端无响应。stderr: {err}")
        return json.loads(out.decode("utf-8"))

    def notify(self, method, params=None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        self.p.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode("utf-8"))
        self.p.stdin.flush()

    def close(self):
        try:
            self.p.stdin.close()
        except Exception:
            pass
        self.p.wait(timeout=10)


fails = []


def check(label, cond, detail=""):
    mark = "OK  " if cond else "FAIL"
    if not cond:
        fails.append(label)
    print(f"[{mark}] {label}" + (f"  {detail}" if detail else ""))


c = McpClient(EXE)

# 1. initialize
r = c.call("initialize", {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "membase-selftest", "version": "1.0"},
})
check("initialize 返回 protocolVersion", r.get("result", {}).get("protocolVersion") == "2025-06-18",
      r.get("result", {}).get("protocolVersion", "?"))
check("initialize 声明 tools 能力", "tools" in r.get("result", {}).get("capabilities", {}))
check("initialize 带 serverInfo", r.get("result", {}).get("serverInfo", {}).get("name") == "membase")

# 2. 通知（不该有响应）
c.notify("notifications/initialized")
r2 = c.call("ping", {})
check("ping 可用（通知后仍能通信）", r2.get("result") is not None)

# 3. tools/list
r3 = c.call("tools/list")
tools = r3.get("result", {}).get("tools", [])
check("tools/list 有工具", len(tools) >= 10, f"{len(tools)} 个")
names = [t["name"] for t in tools]
check("含 mem_status", "mem_status" in names)
check("含 mem_trim", "mem_trim" in names)
for t in tools:
    if "inputSchema" not in t or "type" not in t["inputSchema"]:
        check(f"{t['name']} 有 inputSchema", False)
        break
else:
    check("每个工具都有 inputSchema", True)

# 只读工具不该有 execute/yes；动手工具必须有
for t in tools:
    props = t["inputSchema"].get("properties", {})
    if t["name"] in ("mem_trim", "mem_slim", "mem_restart"):
        if "execute" not in props or "yes" not in props:
            check(f"{t['name']} 有 execute/yes 门禁", False)
            break
else:
    check("动手工具有 execute/yes 门禁", True)

# 4. tools/call —— 只读工具
r4 = c.call("tools/call", {"name": "mem_status", "arguments": {}})
res = r4.get("result", {})
check("mem_status 调用成功", not res.get("isError"), res.get("content", [{}])[0].get("text", "")[:120])
try:
    doc = json.loads(res["content"][0]["text"])
    check("mem_status 返回的是合法 JSON", isinstance(doc, dict))
    # status 的 JSON 结构：baseline / live / outliers / is_admin / self_heal
    check("mem_status 含 baseline 与 live", "baseline" in doc and "live" in doc,
          ",".join(doc.keys()))
except Exception as e:
    check("mem_status 返回的是合法 JSON", False, str(e))

# 5. 未知工具
r5 = c.call("tools/call", {"name": "no_such_tool", "arguments": {}})
check("未知工具报 isError", r5.get("result", {}).get("isError") is True)

# 6. 缺参数
r6 = c.call("tools/call", {"name": "mem_svc", "arguments": {"action": "stop"}})
check("缺必填参数报 isError", r6.get("result", {}).get("isError") is True,
      r6.get("result", {}).get("content", [{}])[0].get("text", "")[:80])

# 7. 护栏：execute=true 但 yes=false 必须被拒
r7 = c.call("tools/call", {"name": "mem_trim", "arguments": {"execute": True, "yes": False}})
check("execute 缺 yes 被拒", r7.get("result", {}).get("isError") is True,
      r7.get("result", {}).get("content", [{}])[0].get("text", "")[:100])

# 8. 默认（都不给）应当是 dry-run
r8 = c.call("tools/call", {"name": "mem_trim", "arguments": {}})
res8 = r8.get("result", {})
if res8.get("isError"):
    # 没有基准快照时报错也算合理（退出码 2），不能是「执行了」
    check("mem_trim 默认不执行", "退出码" in res8["content"][0]["text"],
          res8["content"][0]["text"][:100])
else:
    txt = res8["content"][0]["text"]
    try:
        d = json.loads(txt)
        check("mem_trim 默认是 dry_run", d.get("mode") == "dry_run", d.get("mode", "?"))
    except Exception:
        check("mem_trim 默认是 dry_run", False, txt[:100])

# 9. mem_commands 自描述
r9 = c.call("tools/call", {"name": "mem_commands", "arguments": {}})
res9 = r9.get("result", {})
if not res9.get("isError"):
    try:
        d9 = json.loads(res9["content"][0]["text"])
        check("mem_commands 自描述可用", "commands" in d9)
        names9 = {x["name"] for x in d9["commands"]}
        check("自描述里有 mcp 子命令", "mcp" in names9)
    except Exception as e:
        check("mem_commands 自描述可用", False, str(e))
else:
    check("mem_commands 自描述可用", False, res9["content"][0]["text"][:100])

c.close()

print()
if fails:
    print(f"失败 {len(fails)} 项: {fails}")
    sys.exit(1)
print("全部通过")
