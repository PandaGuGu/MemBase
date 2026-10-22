# 路线图 / 待办

只记「已经想清楚、按收益排过序」的项。每条都带实测依据和验收方式，避免空话。

数据来自 2026-10-21 的三份基准（用户已确认对应关系）：

| 代号 | 文件 | 性质 |
|---|---|---|
| A | `baseline_20261020_214745.mbz` | 10-20 开机 60s，SYSTEM 身份（UserName/CommandLine/OS 字段整列为空） |
| B | `baseline_20261021_213008.mbz` | 10-21 开机 9h22m 后未提权手采（内存大量读不到，**不可用于内存对比**） |
| C | `baseline_20261021_224225.mbz` | 10-21 开机 30s，已提权、Build 有值（第一份完全可用的基准） |
| D | `baseline_20261022_073128.mbz` | 10-22 开机 300s，双采样（T+60s / T+300s），1.2.0 的实测依据 |

## 已落地

| 能力 | 说明 |
|---|---|
| 采集口径 | 内存读不到写空不写 0；`system.json` 记 `Elevated` / `Build`；不可比项单独标注 |
| `svc disable/enable` | 真正改服务启动类型（`ChangeServiceConfigW`），不再只是 `stop` |
| `slim` | 以基准为输入列出可砍的自启服务，动作为 `disable-service` / `uninstall-appx`，按宿主去重、幂等 |
| `restart` | 局部重启：先礼后兵关掉 → 按快照 `Path` 拉起（或识别自愈）→ 等 3 秒**实测**工作集 |
| `feature` | 已知功能开关表（只写 HKCU）：`list / off / on`，证据进程按父进程链归属 |
| `startup` | 启动项扫描：Run（HKCU/HKLM/32 位）、启动文件夹（IShellLink 解 .lnk）、非 Microsoft 计划任务；禁用只写 `StartupApproved` / `schtasks`，可逆 |
| 双采样 | `install --at 30 --also 300`：一次开机采两点；`compare --label` 选采样点；`transient` 列短命项 |
| 界面 | 新增「瘦身」「启动项」两页；「进程」页加局部重启；「开机采集」页加双采样开关；`ACTIONS` 表 30 → 37 条 |

装机效果：A 已用 3710 MB → C 3449 MB（−261 MB），进程 −8、线程 −190、句柄 −4552、服务 −4。

## 1.2.0 已落地（2026-10-22，来自基准 `baseline_20261022_073128.mbz` 的实测）

| 能力 | 说明 |
|---|---|
| 可回收量按宿主去重 | `plan_reclaim()` 按 `host_pid` 去重取最大；`PlanItem` 新增 `host_pid` / `shared_host` / `reclaim_mb`。本机实测 204.4 → 175.7 |
| 共享宿主标出来 | `TrimItem` 新增 `shared_host` / `host_service_count`，`plan` / `slim` 直接标「与 N 个服务共享宿主，停它本身不释放内存」 |
| `stop-and-disable-startup` | 新动作：停进程 + 禁用启动项。补的是「开机后才起、非服务非 appx」这一类 |
| 启动项收 `\Microsoft\` | `startup` 不再一刀切跳过系统任务，改为标记 `system`；`startup list` 默认隐藏、`--all` 全列。VS 安装器 `BackgroundDownload`（339 MB）从此可见 |
| 自愈进程识别（原 P2） | `restart` 判到 `revived` 时写 `selfheal.txt`；`status` 报「N 个自愈型」，`list --json` 带 `self_heal` 字段。**不做自动探测** |
| 基准目录轮换 | `collect --keep N`（默认 10）自动把更早的 `run_*` 移进回收站；界面手动采集同样轮换 |
| 采集口径修正 | `system.json` 区分 `LoggedOnUser`（交互用户）/ `CollectorUser`（采集身份）/ `LoggedOn`；`summary.md` 标注命令行未采集 |
| `-0.0` 收尾 | `plan_totals` / `trim_savings` / `slim --json` 全部修掉空集合的负零 |

**验收**：`python tools/verify.py` 全量 **124 项 0 失败**（`docs/verify-1.2.0.md`），
含 4 条新用例：`plan --json` 带宿主字段、`status --json` 带自愈记录、`slim --json` 不出 `-0.0`、
`startup list` 系统任务默认隐藏 / `--all` 全列。发布包 `dist/MemBase-v1.2.0-win-x64.zip`。

> ⚠ 已知误报：Defender 的行为监控会把本程序判成 `Behavior:Win32/Persistence.A!ml`
> （注册开机任务 + 改服务启动类型 + 写 `StartupApproved`，行为特征就是持久化）。
> 文件被隔离后连 `./MemBase.exe version` 都会 Permission denied。发布说明里必须写清这条。

## 1.2.1 收尾（2026-10-22）—— 双文件残留清零

评审报告里唯一没做完的「单文件分发收尾」，连同品牌口径一起处理：

| 项 | 说明 |
|---|---|
| 只发一个 exe | `build.bat` 只收 `MemBase.exe`，`dist/` 里的 `deskkit.exe`（1.27 MB）删除并加清理；`task::install` 复制的副本改名 `membase.exe` |
| 版本自述串 | `MemBase.exe version` 输出 `MemBase 1.2.1`（原来打的是 `deskkit 1.2.0`） |
| 命令示例统一 | 帮助、报错提示、`ACTIONS` 表 37 条 `cli` 字段、README 全部改成 `MemBase.exe <动词>` |
| README 口径 | 「命令行」章节改为单文件说明，并注明 PowerShell 用 `.\MemBase.exe`、Git Bash 用 `./MemBase.exe`；构建段的测试数 / 验收项数 / 验收文档链接更新到 1.2.x |
| 验收加断言 | `verify.py` 新增「`dist/` 里只有一个 exe」，产物检查从「两个文件都存在」改为「只有一个」 |

内部 crate 名（`membase-core` / `membase-win` / `membase-cli` / `membase-ui`）保持不变 —— 只出现在 `Cargo.toml` 与 `use` 里，不影响任何用户可见行为，改名会连带 116 处引用与 `-p` 参数，收益为零。

**验收**：`python tools/verify.py --fast` **107 项 0 失败**（`docs/verify-1.2.1.md`），
含新断言「`dist/` 里只有一个 exe」、版本断言改为 `MemBase 1.2.1`。
发布包 `dist/MemBase-v1.2.1-win-x64.zip`（`MemBase.exe` + `README.md` + `LICENSE`，3.3 MB）。
`docs/screenshots/cli.png` 已按新文案重抓（命令行页 37 条命令全部 `MemBase.exe <动词>`）。

---

## 2026-10-22 · 命名统一 + MCP 入口

上一节「crate 名保持不变」的结论**已被推翻** —— 当时只算「改名对用户零收益」，
没算「历史包袱」：仓库里 `deskkit` 这个名字散在 crate 名、二进制名、临时文件名、
CI 路径、图标文件名里，新人接手时分不清 `deskkit-cli`（crate）和 `deskkit.exe`（产物）
到底是什么关系。既然品牌已经统一到 MemBase，名字就该一次清干净。

### 改名（`deskkit` → `membase`）

| 对象 | 旧 | 新 |
|---|---|---|
| crate | `deskkit-core/win/cli/ui` | `membase-core/win/cli/ui` |
| 控制台二进制 | `deskkit.exe` | `membase-cli.exe` |
| 图标 | `assets/deskkit.ico` | `assets/membase.ico` |
| 环境变量 | `DESKKIT_PAGE` / `DESKKIT_SHOT` / `DESKKIT_BLACKOUT_TEST` | `MEMBASE_*` |
| 临时文件 | `deskkit_shot_*` / `deskkit_verify_*` / `__deskkit_verify__` | `membase_*` / `__membase_verify__` |

- **二进制不能叫 `membase.exe`**：Windows 文件名不区分大小写，会和数据目录里
  `task::install()` 复制的任务副本 `membase.exe` 撞名。控制台版叫 `membase-cli.exe`。
- 替换用「长模式优先」的成对替换（`deskkit-core` → `membase-core` 先于裸 `deskkit`），
  否则会像 1.2.1 那次一样误伤 crate 名。
- 历史归档文档（`docs/verify-1.0.0.md` / `1.1.0` / `1.2.0` / `review-1.1.0.md`）**不改** ——
  那是当时的事实记录，改了就是篡改历史。

### MCP 入口（`MemBase.exe mcp`）

同一个 exe 还能当 MCP server（stdio + JSON-RPC 2.0），把 CLI 那套动词原样暴露给 AI。

- **零新依赖**：只用已在编译的 `serde_json`，手写 JSON-RPC 的行分隔收发。
  官方 Rust SDK 会带进 tokio + async-trait，把 CLI 的编译量从 15 个第三方小库抬到几十个。
- **16 个工具**：`mem_status` / `mem_list` / `mem_plan` / `mem_trim` / `mem_slim` /
  `mem_restart` / `mem_kill` / `mem_svc` / `mem_feature` / `mem_startup` / `mem_protect` /
  `mem_collect` / `mem_compare` / `mem_transient` / `mem_baseline` / `mem_commands`。
- **护栏与命令行完全一致**：动手类工具的 `execute` 与 `yes` 必须**同时**为 true，
  只给 `execute` 直接拒绝；两者都 false（默认）时只输出预演结果。
  `Tier::Blocked` 的关键服务仍然被拒，保护名单仍然生效。
- **实现手法**：工具直接调用已有 CLI 动词（起子进程跑自己），不重写业务逻辑 ——
  也避免了某个动词调 `std::process::exit` 把 MCP 服务端带走。
- **协议版本**：支持 `2024-11-05` / `2025-03-26` / `2025-06-18` / `2025-11-25` / `2026-07-28`。
  2026-07-28 去掉了 `initialize` 握手（版本号改走每个请求的 `_meta`），
  但老客户端仍占多数，所以握手必须留着，同时支持新版的 `server/discover`。
- **实测**：`tools/mcp_selftest.py` 是真的起进程发 JSON-RPC（不是 mock），
  18 项断言全过；便携版（GUI 子系统）和控制台版都验过。
  管道捕获实测正常（13884 字节）—— GUI 子系统丢 stdout 只是 PowerShell 的怪癖，
  MCP 客户端用管道 spawn 不受影响。

**验收**：`python tools/verify.py --fast` **111 项 0 失败**（含 4 项 MCP 断言）。

---

## 已结案 · 局部重启（原 P0）

2026-10-21 真机实测（`deskkit restart`）：

| 目标 | 前 | 后 | 实测 | 说明 |
|---|---:|---:|---:|---|
| `Notepad` | 162.6 MB | 163.2 MB | **−0.6**（反而多占） | 走完整条链路，回收量按实测如实报负值 |
| `SearchHost` | 147.0 MB | 156.6 MB | **−9.6** | 关掉后 2 秒内被外壳拉起，识别为 `revived`，**不重复拉起** |

- 只处理有明确 `Path` 的进程；`svchost` / `dllhost` / `WmiPrvSE` / `RuntimeBroker` 拒绝，改走服务
- `explorer` / `SearchHost` 必须显式 `--yes`；服务走 `stop` + `start`，标注「共享宿主，变化不全是这个服务」
- **已验证的坑**：外壳托管的进程杀掉会自愈，这类目标不适合用重启回收

## 已结案 · 功能开关（原 P0，结论与立项时相反）

立项假设「6 个 `msedgewebview2` = 369 MB 是搜索亮点，关开关即可回收」被实测否掉：

1. `IsDynamicSearchBoxEnabled` **本来就是 0**，6 个 webview2 照旧常驻；
2. 终止 `SearchHost` 后 **2 秒内自愈**，重新拉起同样 6 个（147 → 156.6 MB）；
3. `SearchSettings` / `Search` 两棵子树**连管理员写入都返回「拒绝访问」**（`reg add` 同样失败）；
4. 父进程链全是 `SearchHost` —— 搜索界面本身在用 WebView2，不是 Edge、不是电脑管家。

→ 该项从 P0 移除，记入「明确不做」。当时以为真正兑现的是 `cross-device-resume`
（`−71.0 MB`，15 分钟后进程仍为 0）。

**2026-10-22 复核：这条结论不成立。** 注册表 `HKCU\...\CurrentVersion\Mobility\CrossDeviceEnabled`
确为 `0`（没被复位），但下一次开机 `T+60s` 采样里 `CrossDeviceResume` 就已经在跑、70.72 MB，
`T+300s` 仍在。→ 关开关只让它**当场退出**，不阻止自启；「15 分钟后仍为 0」只是同会话现象，
不能外推成「省 71 MB」。`feature list` 的四档判定（`OffButAlive`）是对的，错的是措辞。

## 已结案 · 启动项扫描（原 P1）

2026-10-21 真机扫描本机：**18 项（启用 3 / 已禁用 15）**，四类来源全部覆盖。

| 来源 | 本机样本 | 备注 |
|---|---|---|
| `HKLM\...\Run` | `RtkAudUService` | 禁用/启用往返实测通过（写 `StartupApproved`，字节 `0x03`/`0x02`） |
| 计划任务 | `\AsusSystemAnalysis_*`、`\GoogleSystem\...` | 任务 XML 用 `parse_task_xml` 抽 Command/Arguments/Enabled |
| 启动文件夹 | 本机为空 | `.lnk` 目标走 shell `IShellLinkW`，不自己抠二进制 |
| `HKCU\...\Run` | 本机为空 | — |

- 禁用**不删原值**：Run/文件夹走 `Explorer\StartupApproved\{Run,Run32,StartupFolder}`，任务走 `schtasks /change /disabled`，都能一条命令恢复
- 命令解析覆盖：引号路径、无引号带空格、`rundll32 <dll>,<entry>`、`%ProgramFiles%` 展开、`by user disabled` 占位串（Google Updater 会把 Command 改写成这句话）
- 每条标注「是否在基准里出现过」，把「装了但没跑起来」和「真常驻」分开

## 已结案 · 双采样（原 P1）

- `install --at 30 --also 300` → 任务参数实测为 `collect --out ... --at 30 --also 300 --self-destruct ...`
- 一次开机、一个 `run_*` 目录下写 `T+30s\` 与 `T+300s\`（实测用 `--at 1 --also 2` 在临时目录验过）
- `compare --label T+300s` 精确取第二个采样点；`status` 会打印「同一次开机的采样点」
- `transient` 对比最早/最晚两个采样点，列出**只在前者出现**的进程（本机实测两点间无差异，因为采集间隔太短；真实开机 T+30s→T+300s 才有意义）

---

## 已结案 · 界面接四个新能力（原 P2）

2026-10-21 落地，逐页截图验证：

- **瘦身页**：`core::build_trim_plan()` + `build_feature_items()`，勾选后确认弹窗列明「禁用服务 N / 卸载应用 N / 关开关 N，
  合计约 X MB」，并写明**下次开机才反映到基准**；「含系统组件」放开 Caution 档（默认不勾，则 Caution 项一条都不勾）
- **启动项页**：18 项实测渲染，行尾「禁用 / 启用」直接写批准记录，悬停显示完整命令行与来源位置
- **进程页**：勾选后出现「重启选中 N 项」，走 `restart`，确认弹窗写明会闪任务栏、回收量按实测报
- **开机采集页**：「双采样」勾选 + 第二次的秒数（`--also`）
- `commands --json` 的 `gui_actions` 30 → 37；`tools/verify.py` 的界面页从 8 张增到 10 张，动作数断言改为读取变量

**验收**：`python tools/verify.py` 全量 **117 项 0 失败**（含 10 页截图 md5 互不相同、确认弹窗样式、黑屏层与 ESC 退出）。

## 已结案 · `tools/verify.py` 补新命令（原 P2）

新增 14 个用例：`slim`（演练 / JSON / Caution / 缺 `--yes` 被拒）、`startup list`（文本 / JSON / 不存在的项）、
`feature list`（文本 / JSON / 未知 key / 缺 `--yes` 不写注册表）、`restart`（演练 / explorer 缺 `--yes` 被拒 / 不存在的目标）、
`transient`（无双采样时返回结构化错误）。

顺手修了两处脚本自身的毛病：版本号断言写死 `1.0.0`（改为读 `Cargo.toml`）、动作数断言写死 30（改为变量）。

---

## 已结案 · 自愈进程识别（原 P2）

2026-10-22 落地，走「信息层」路线，**不做自动探测**（探测本身就要杀进程，正是要避免的打地鼠）：

- `restart` 判到 `revived`（关掉后 800 ms 内被外壳拉起）时，把结论写进
  `%ProgramData%\MemBaseline\selfheal.txt`（`进程名<TAB>时间`，最多 64 条）
- `status` 报「自愈型：N 个（SearchHost / explorer …）」；`status --json` 带 `self_heal` 数组
- `list --json` / `plan --json` 的每一项带 `self_heal` 布尔
- 只读缓存文件，`status` 不会因此变慢（无任何探测动作）

**验收**：跑过一次 `restart SearchHost` 之后，`status` 能报出来；重复跑不会重复记账（同名覆盖）。

**2026-10-22 真机验证（补上原来缺的那一步）**：`MemBase.exe restart SearchHost --execute --yes` →
输出「回收 6.9 MB（自愈重生，1 个实例 / 160.8 MB → 1 个 / 153.9 MB）」，
`%ProgramData%\MemBaseline\selfheal.txt` 随即生成 `SearchHost	2026-10-22 09:03:54`，
`status` 打印「自愈型：1 个（SearchHost）」，`status --json` 的 `self_heal` 为 `["SearchHost"]`。
此前只有代码、没有真机记录 —— 这条现在才算闭环。

---

## 明确不做

| 项 | 原因 |
|---|---|
| 关「搜索亮点」换 369 MB | 已实测否掉：开关本就是 0、子树写不进去、宿主杀完自愈（2026-10-21） |
| `EmptyWorkingSet` / 清 standby list | 只是把页搬到页面文件、把「已用」刷好看，访问即回涨，还更慢 |
| 硬编码改 Defender / Windows Update | 会被系统自愈，且不安全；禁用第三方杀软才算数，那是另一类产品 |
| 按 `HostWorkingSetMB` 累加服务内存 | 同一 svchost 宿主上 N 个服务共享一份内存，相加是假账。**合计已按宿主去重（1.2.0）**；单项展示保留宿主全量，旁边标 `shared_host` |
| 对自愈型进程反复强杀 | 打地鼠：外壳会在 2 秒内重建，净收益为 0，还可能丢用户正在用的界面 |
| B 基准做内存对比 | 未提权采集，3/4 进程内存读不到，只能看名单 |

## 已知上限

16 GB 机器上系统自身底线约 3.3 GB。**2026-10-22 实测（开机 300 秒、已提权）**：

| 采样点 | 已用 | 进程 | 主要构成 |
|---|---:|---:|---|
| `T+60s` | 3357 MB | 90 | — |
| `T+300s` | 3732 MB | 96 | BackgroundDownload 339 / msedgewebview2 381 / explorer 227 / TextInputHost 204 |

4 分钟涨的 375 MB 里 97% 是开机后才起的东西（VS 安装器后台下载 339 + Windows Update 编排 26）。
扣掉 shell、svchost 与这类一次性后台任务，**稳态下"关东西"还能挖的空间不足 300 MB**：

- `msedgewebview2 ×6 ≈ 381 MB`：搜索界面自带，不可通过开关回收（已排除）
- `svchost` 32 个合计 660 MB：只能靠服务配置，且同宿主上的服务停一个不释放宿主内存
- `CrossDeviceResume` 70.7 MB：值已为 0 仍开机自启，**关开关回收不到**（1.2.0 复核）
- explorer / TextInputHost 等工作集随运行时长增长的项：靠 `restart` 局部回收，**按实测报数**

再往下要靠「控制增长」（启动项、双采样找短命项）和「局部回收」，不是靠继续卸载。
