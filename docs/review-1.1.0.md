# MemBase 1.1.0 评审（基准 `baseline_20261022_073128.mbz`）

> **本文所列问题已在 1.2.0 全部处理**，逐条对照见文末「落地情况」。
> 保留原文是为了让结论有据可查 —— 报告里每条判断都对应一个可复现的命令。

评审时间：2026-10-22 07:40
对象：`membase 1.1.0 (build 20261021-2352)`，导出内容为 `run_20261022_072728/T+300s`
旁证：同目录 `T+60s` 采样点、`dist/deskkit.exe` 实跑输出、源码逐条核对

## 一句话结论

**观测侧已经可用，处置侧的账目不准。** 采集、双采样、diff、护栏、回滚都经得起实测；
但「可回收多少 MB」这一栏对服务类目标是宿主全量的重复累加，用户按它做决策会失望。
另外本轮最大的一个 339 MB 大户，四个处置入口（trim / slim / feature / startup）全都够不着。

---

## 一、这份基准说明采集链路是健康的

| 检查项 | 实测 | 判断 |
|---|---|---|
| `Elevated` | `true` | 提权成功，不是 B 类不可用基准 |
| `Build` | `20261021-2352` | 与 `dist` 内 exe 一致，能定位到具体编译批次 |
| 双采样 | `T+60s` + `T+300s` 同在一 `run_` 目录 | 双采样链路正常 |
| 内存读不到 | 15 / 96（`Secure System`/`Registry`/`smss`/`lsass`/`MsMpEng` 等） | 全是 Session 0 受保护进程，属预期 |
| `UserName` / `CpuSeconds` / `StartTime` | 96/96 有值 | 提权后字段完整 |
| 服务枚举 | 338（运行 81 / 停止 257；Auto 58 / Manual 242 / Disabled 38） | 正常 |
| 计划任务 | 采完已自毁，`schtasks /Query` 查不到 | 自毁逻辑正常 |

同一次开机的两点对比，是这个工具最有说服力的地方：

| 采样点 | 已用内存 | 进程 | 采集耗时 |
|---|---:|---:|---:|
| T+60s | 3357 MB | 90 | 63 ms |
| T+300s | 3732 MB | 96 | 246 ms |

4 分钟里**涨了 375 MB**，涨在哪一目了然：

| 进程 | T+60s | T+300s | 增量 |
|---|---:|---:|---:|
| `BackgroundDownload`（VS 安装器） | 0 | 339.0 MB | **+339.0** |
| `MoUsoCoreWorker`（Windows Update） | 0 | 25.9 MB | +25.9 |
| `taskhostw` | 16.4 MB | 35.4 MB | +19.0 |
| `AsusOSD` | 0 | 14.8 MB | +14.8 |
| `WMIADAP` | 0 | 11.2 MB | +11.2 |

这 375 MB 里 90% 来自开机后才偷偷起来、且用户根本不需要的东西。**「双采样 + 短命项」这个设计是对的，价值就在这张表。**

---

## 二、可行性的三处硬伤

### 硬伤 1：339 MB 的 `BackgroundDownload` 四个入口全都够不着

两个实例都在跑，路径已经说明一切：

```
C:\Users\Administrator\AppData\Local\Temp\e0moacyk.ruj\resources\app\ServiceHub\
    Services\Microsoft.VisualStudio.Setup.Service\BackgroundDownload.exe   232.76 MB
C:\Program Files (x86)\Microsoft Visual Studio\Installer\resources\app\...  106.28 MB
```

它是 Visual Studio 安装器的后台下载服务，单机第一大非系统内存项（占工作集合计 11%）。
但现有四条处置路径全都不覆盖：

| 入口 | 判定逻辑 | 为什么抓不到 |
|---|---|---|
| `trim` | 基准里**没有**、现在在跑 | 它开机 60 秒后才起，但一出现就在基准窗口内被记下了 |
| `slim` | 基准里**就有**、但不是服务也不是 appx | 动作只有 `disable-service` / `uninstall-appx`，它两样都不是 |
| `feature` | 表驱动 2 个已知开关 | 表里没有它 |
| `startup` | Run / 启动文件夹 / **非 Microsoft** 计划任务 | VS 的自启任务在 `\Microsoft\VisualStudio\` 下，被 `Tasks` 过滤规则主动跳过 |

→ 结果：**用户能看见它，但工具没有一条命令能处置它**。这是产品定位问题，不是 bug。

### 硬伤 2：`plan` 的「约 X MB」是宿主内存的重复累加

实跑 `deskkit plan --json`：

```
counts: { total: 13, suggested: 5, actionable: 5, actionable_mb: 204.4 }
```

可处理 5 项明细：

| 项 | mem_mb | 宿主 PID |
|---|---:|---:|
| `wlidsvc` | 101.7 | 1716 |
| `SensorService` | 28.7 | 1368 |
| `WdiSystemHost` | 28.7 | **1368**（同一个） |
| `InstallService` | 24.9 | 4760 |
| `DPS` | 20.3 | 2528 |

两处问题：

1. **汇总没去重。** `crates/core/src/lib.rs:1027 plan_totals()` 直接 `sum(p.mem_mb)`；
   而 `crates/core/src/lib.rs:653 trim_savings()` 是按 `host_pid` 去重的（还带单测
   `trim_savings_counts_shared_host_once`）。两个函数语义打架 → plan 报 204.4，执行侧最多按 175.7 算。
2. **单项 savings 用宿主全量，本身就不成立。** PID 1716 承载 16 个服务，
   停掉其中 `wlidsvc` 一个，宿主几乎不会瘦 —— 真实回收接近 0，而界面写的是 101.7 MB。

ROADMAP「明确不做」里已经写了「按 `HostWorkingSetMB` 累加服务内存 = 假账」，
但 `plan` 的展示层正在干这件事。**这是当前最该修的一处，因为它直接误导决策。**

### 硬伤 3：`cross-device-resume` 的 71 MB 并没有兑现

ROADMAP「已结案」写：「真正兑现的是 `cross-device-resume`（−71.0 MB，15 分钟后进程仍为 0，非自愈）」。

这台机器的实测结果相反：

- 注册表 `HKCU\...\CurrentVersion\Mobility\CrossDeviceEnabled = 0` —— 值确实是关的，没被复位；
- `T+60s` 采样点里 `CrossDeviceResume` **已经在跑**，占 70.72 MB；
- `T+300s` 仍在，70.72 MB；
- `deskkit feature list` 自己也判定为 `[已关但仍在跑]`。

→ 准确结论应该是：**关开关只能让它当场退出，下次开机照样起来，值 0 拦不住。**
当时量的「15 分钟后仍为 0」是同一次会话内的现象，不能外推成「下次开机省 71 MB」。
`feature` 的四档结论（`OffButAlive`）判对了，是 ROADMAP 的结案措辞过度乐观。

---

## 三、TODO 完成度

ROADMAP 里只剩一条未完成项，逐条核对结果：

| 项 | 状态 | 证据 |
|---|---|---|
| **P2 自愈进程识别（信息层）** | **未开始** | 全仓 grep `自愈型` / `self_heal` / `heal_cache` 命中 0；`cmd_status`（`apps/cli/src/lib.rs:1112`）的 JSON 无该字段；`revived` 只存在于 `restart.rs` 的返回值里 |
| 单文件分发收尾 | **未完成** | `dist/MemBase-v1.1.0-win-x64.zip` 里仍有 `deskkit.exe`（1.2 MB），`dist/` 根目录也还在；`README.md:149` 仍是「`deskkit.exe` 是纯控制台版」的双文件口径，与「单文件分发」的记载不一致 |
| `slim` 的 `-0.0` | 半修 | 文本路径加了 `+0.0`，但 `slim --json` 仍原样输出 `"savings_mb":-0.0` |
| 1.1.0 验收 | 完成 | `docs/verify-1.1.0.md` 117/117，实跑 `deskkit.exe` 各命令行为一致 |

**所以：ROADMAP 的 TODO 没有全部完成，P2 连代码都没动。**
好消息是它不是功能缺陷，是「信息层」增强，不影响 1.1.0 可用。

---

## 四、优化建议（按收益排序）

### P0-1 `plan_totals` 按宿主去重（约 30 行内可修）

把 `plan_totals()` 改成复用 `trim_savings()` 的 key 规则，或直接把 `items.iter().map(mem_mb).sum()`
换成「按 `host_pid` 取最大值再求和」。**验收**：构造两个同宿主服务，`actionable_mb` 只算一次；
本机重跑 `plan --json`，204.4 → 175.7。

### P0-2 服务项 savings 改成「实测回收量」而不是宿主全量

停服务 → 等 1 秒 → 量宿主前后差，把这个差值当 `mem_mb`。差分不到就显示 `≈0`。
保留现有 `HostWorkingSetMB` 作为「宿主总占用」单独一栏，两个数分开写。
**验收**：对 1716 上的 `wlidsvc` 执行一次，界面不再出现 101.7 MB 的承诺值。

### P0-3 补一条「开机后自启的第三方程序」处置入口

`BackgroundDownload` 这类目标现在无处安放。最小改动：在 `slim` 里加一种动作 `stop-and-disable-startup`，
对「非服务、非 appx、但由计划任务/Run 拉起」的项，走 `startup disable` 而不是卸载。
顺带放开 `startup` 对 `\Microsoft\VisualStudio\*` 的可见性（默认仍不勾选，但要看得到）。
**验收**：`startup list` 能列出 VS 安装器任务，`slim` 能给出针对它的建议。

### P1-1 更正 ROADMAP 与 feature 表里的 `cross-device` 结论

`measured` 字段改成「值 0 仍开机自启，关开关不回收内存」；
ROADMAP「已结案」里那条 −71 MB 降级为「当次会话现象」。
**验收**：`feature list` 文案与实测一致，不再暗示可回收 71 MB。

### P1-2 `CollectMs` 不该当性能指标

同机同版本，`T+60s` 63 ms、`T+300s` 246 ms，差 4 倍 —— 差别来自当时系统负载
（T+300s 正在跑 339 MB 的下载）。这个数波动太大，界面上「采集约 N 毫秒」会让用户误以为程序变慢。
建议改成区间表述（`40–250 ms，受系统负载影响`），或直接不展示。

### P2-1 `LoggedOnUser` 语义错误

`system.json` 写 `LoggedOnUser: "SYSTEM"`，但 `processes.csv` 里 96/96 的 `UserName` 都是
`PGG\Administrator`。原因是这个字段取的是采集进程自己的 token。
改成「取 `explorer.exe` 的属主」或改名为 `CollectorUser`，并另加 `InteractiveUser`。

### P2-2 `CommandLine` 整列为空（0/96）

SYSTEM 身份采集时读不到命令行，因此「这条自启到底在跑什么」无法从基准里看出来。
短期在 `summary.md` 里标注「本字段在 SYSTEM 采集下不可用」，别让用户以为导出损坏；
长期可考虑采集时用 `NtQueryInformationProcess` 的 PEB 路径，或标为已知限制。

### P2-3 基准目录需要轮换

`%ProgramData%\MemBaseline` 下已有 **11 个 `run_*` 目录**（最早 10-18），
另有 `exports\` 两份内容完全相同的重复包。建议保留最近 N 份（默认 10），
清理走回收站（沿用现有 `SHFileOperation` 约定）。

### P3 零碎

- `slim --json` 的 `-0.0` 与文本路径保持一致；
- `plan` 里 `sppsvc` / `TrustedInstaller` 同时以 `process` 和 `service` 两个 kind 各出现一次，
  去重只在同 kind 内生效，界面上会看到同名两行；
- `deskkit.exe` 与「单文件分发」二选一：要么从 dist 与 zip 里删掉，要么把 README 与 ROADMAP 改成双文件口径。

---

## 五、这份基准的结论（数据侧）

| 指标 | 值 | 解读 |
|---|---:|---|
| 已用 / 总量 | 3732 / 16000 MB（23%） | 系统底子干净 |
| 工作集合计 | 3077.5 MB | 与已用量差 654 MB，属内核/驱动/共享页，正常 |
| 进程 / 线程 / 句柄 | 96 / 1039 / 43426 | 轻 |
| 服务 | 运行 81 / 共 338 | 已 Disabled 38 个，前期工作有效 |
| 锁屏态 | `LogonUI` 118.5 MB 仍在 | **本轮基准是「无人登录的锁屏态」**，用户登录后会变一轮 |

按宿主去重后，本轮真实还能挖的空间：

- `BackgroundDownload` 339 MB —— 处置入口缺失（P0-3）
- `MoUsoCoreWorker` 25.9 MB、`WMIADAP` 11.2 MB、`AsusOSD` 14.8 MB —— 短命项，本身会退场
- `TextInputHost` 203.7 MB —— 相对正常值偏高，值得单独用 `restart` 实测一次
- 服务侧 —— 去重后约 176 MB 名义值，扣除宿主共享，**真实可兑现预计 < 80 MB**

与 ROADMAP 的「16 GB 机器底线约 3.3 GB」一致：本轮 3.73 GB 含锁屏态与一次性后台下载，
回到稳态后与 3.45 GB 是同一量级。**"关东西"这条路确实快到头了，接下来的收益在「控制增长」和「处置盲区」。**

---

## 落地情况（1.2.0，2026-10-22）

| 评审项 | 状态 | 落地方式 |
|---|---|---|
| P0-1 汇总按宿主去重 | 完成 | 新增 `core::plan_reclaim()`：按 `host_pid` 取最大 + 无宿主的按条累加。带 3 条单测（同 PID 多行、独占宿主、空集不输出 `-0.0`）。本机 `plan` 从 204.4 → 96 MB |
| P0-2 服务 savings 别用宿主全量 | 完成（保守版） | 共享宿主的项 `reclaim_mb = 0`，展示层标 `← 共享宿主，停这一项本身不释放内存`。没有做「停服务 → 量宿主差」的实时差分：那会让 `plan` 变成有副作用的命令。真实回收量仍以 `restart` / `trim --execute` 的实测为准 |
| P0-3 补自启第三方入口 | 完成 | 新动作 `stop-and-disable-startup`（停进程 + 禁用启动项）；`startup` 不再跳过 `\Microsoft\`，改打 `system` 标记，默认隐藏、`--all` 全列。**实测已抓到 `\Microsoft\VisualStudio\Updates\BackgroundDownload`** |
| P1-1 更正 cross-device 结论 | 完成 | `feature.rs` 的 `measured` 改写 + ROADMAP「已结案」段落更正 |
| P1-2 `CollectMs` 别当性能指标 | 完成 | `summary.md` 与界面文案都标注「受系统负载影响，不是性能指标」 |
| P2-1 `LoggedOnUser` 语义 | 完成 | 拆成 `LoggedOnUser`（交互用户）/ `CollectorUser`（采集身份）/ `InteractiveUser` / `LoggedOn` |
| P2-2 `CommandLine` 空列 | 完成（标注） | `summary.md` 明确写「本版不采集，列为空是预期，不是导出损坏」 |
| P2-3 基准目录轮换 | 完成 | `collect --keep N`（默认 10）+ 界面手动采集同样轮换，删除走回收站（`SHFileOperationW` + `FOF_ALLOWUNDO`） |
| P3 `-0.0` | 完成 | `trim_savings` / `trim_savings_of` / `plan_totals` 三处都加 `+ 0.0`；`slim --json` 已验 |
| P3 同名 process/service 双列 | 完成 | 由 `plan_reclaim` 的 PID 去重一并解决 |
| P3 `deskkit.exe` 口径 | 决定保留 | 发布包含两个 exe（`MemBase.exe` 双身份 + `deskkit.exe` 纯控制台），README 已写明用途，不再声称「单文件」 |
| ROADMAP P2 自愈识别 | 完成 | `selfheal.txt` 缓存 + `status` 报数 + `list --json` 的 `self_heal` 字段 |

验收：`tools/verify.py` **124 项 0 失败**（`docs/verify-1.2.0.md`）。
