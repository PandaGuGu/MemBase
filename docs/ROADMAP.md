# 路线图 / 待办

只记「已经想清楚、按收益排过序」的项。每条都带实测依据和验收方式，避免空话。

数据来自 2026-10-21 的两份基准：A（10-20 开机 60s）与 C（10-21 开机 30s，口径干净：已提权、Build 有值）。

## 已落地

| 能力 | 说明 |
|---|---|
| 采集口径 | 内存读不到写空不写 0；`system.json` 记 `Elevated` / `Build`；不可比项单独标注 |
| `svc disable/enable` | 真正改服务启动类型（`ChangeServiceConfigW`），不再只是 `stop` |
| `slim` | 以基准为输入列出可砍的自启服务，动作为 `disable-service` / `uninstall-appx`，按宿主去重、幂等 |

效果：A 已用 3710 MB → C 3449 MB（−261 MB），进程 −8、线程 −190、句柄 −4552、服务 −4。

---

## P0 · 功能开关（feature-off）

**依据**：C 里 `msedgewebview2 ×6 = 369.4 MB`，父进程全是 `SearchHost.exe` —— 是 Win11 的「搜索亮点」，不是 Edge、不是 PC Manager。这是目前剩下最大的一笔，而软件现在只能提示用户手动去设置里关。

**做法**

- `TrimAction` 增加 `FeatureOff`，配一张**已知开关表**（不开放任意注册表编辑）：
  - 搜索亮点：`HKCU\Software\Microsoft\Windows\CurrentVersion\SearchSettings\IsDynamicSearchBoxEnabled = 0`
  - 跨设备继续：设置项对应的用户级开关
  - 小组件：随 `Microsoft.WidgetsPlatformRuntime` 卸载一并处理
- 只写 `HKCU`，不碰 `HKLM` 策略；每条都带「怎么撤销」
- 生效通常要重启 `SearchHost` / `explorer`，执行后自动做一次并校验

**验收**：执行后 webview2 实例从 6 降到 ≤1；`slim` 不再列出该项；撤销后能恢复。

## P0 · 局部重启（restart）

**依据**：explorer 在跑 9.5 小时后是 667 MB，重启后回到 226 MB —— **−440 MB，不用重启整机**。可回收量的主体在进程工作集里，粒度可以是单个进程。

**做法**

- 复用 `close.rs` 的「先礼后兵」：发关闭请求 → 3 秒 → 强制
- 进程：用快照里的 `Path` 重新拉起；服务宿主（svchost）走 `sc stop` / `sc start` 而不是 kill
- 执行后等 3 秒，再读一次工作集，报告**实际回收量**（不报告理论值）

**边界**：只处理有明确 `Path` 且能自行重启的进程；`explorer` 需要 `--yes`。

**验收**：`restart explorer --execute --yes` 后 explorer 工作集 ≤ 基准 ×1.2。

## P1 · 启动项扫描（startup list）

**依据**：目前完全没覆盖。A 时期常驻的 MuMu 三件套、`crashpad_handler ×3` 都属这一类；现在只能等它们出现在快照里才被看见。

**来源**：`HKCU/HKLM ...\Run`、启动文件夹、非 Microsoft 的计划任务、启动批准记录。

**输出**：来源 / 名称 / 命令行 / 是否在基准里 / 建议动作。只读优先，禁用要 `--execute`。

**验收**：`startup list --json` 能把 MuMu 那几条列出来并标出属于哪一类启动项。

## P1 · 双采样（T+30s / T+300s）

**依据**：现在只有单点，分不清「起了又退的短命启动项」和「真常驻」。C 里 `TabTip` 没起、A 起了 77 MB，就是典型噪音。

**做法**：一次开机采两次 → `run_<stamp>\T+30s\` 与 `run_<stamp>\T+300s\`；
`find_latest_baseline()` 要能按 label 取；只在前者出现的判为短命项。

**验收**：`compare --label T+300s` 能用第二次采样做基准，短命项单独成组。

## P2 · 界面接 slim

**依据**：GUI 列表只显示「基准里没有的」线外项，`slim`（基准内部的瘦身）还只有命令行。

**做法**：新页「瘦身」，直接复用 `core::build_trim_plan()`，勾选 + 确认弹窗；弹窗必须写明**下次开机才反映到基准**。

**验收**：`commands --json` 导出的「界面动作 → 命令」表里，每个新按钮都有等价命令。

---

## 明确不做

| 项 | 原因 |
|---|---|
| `EmptyWorkingSet` / 清 standby list | 只是把页搬到页面文件、把「已用」刷好看，访问即回涨，还更慢 |
| 硬编码改 Defender / Windows Update | 会被系统自愈，且不安全；禁用第三方杀软才算数，那是另一类产品 |
| 按 `HostWorkingSetMB` 累加服务内存 | 同一 svchost 宿主上 N 个服务共享一份内存，相加是假账 |

## 已知上限

16 GB 机器上系统自身底线约 3.3 GB（C 实测 3449 MB）。扣掉 shell（explorer / dwm / TextInputHost / StartMenu / SearchHost）和 svchost 706 MB 之后，**"关东西"还能挖的空间不足 500 MB**。再往下要靠「控制增长」（功能开关、启动项）和「局部回收」（restart），不是靠继续卸载。
