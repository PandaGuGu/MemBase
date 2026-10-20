# 变更记录

版本号遵循「功能 + 格式变更才进位」：补丁版只修 bug，次版本号动了快照格式、命令参数或默认行为时进位。

## v0.5.1（2026-10-20）

- 修复 `collect --self-destruct` 删不掉程序副本：`cmd /c` 的参数转义让 `del` 收到坏路径，
  改用 `/S /C` + `raw_arg` 原样传命令行；任务已删、exe 却残留的问题不再出现
- 补齐快照里空着的字段（列不变，只填值）：
  - `UserName`：`OpenProcessToken` + `GetTokenInformation(TokenUser)` + `LookupAccountSidW`
  - `CpuSeconds`：`GetProcessTimes` 的内核 + 用户时间
  - `PagedSystemMB`：`QuotaPagedPoolUsage`；`VirtualMB`：`PagefileUsage`（提交大小）
  - `system.json` 的 `OsCaption` / `OsVersion` / `OsBuild`：读
    `CurrentVersion` 注册表，`CurrentBuildNumber` + `UBR` 拼成完整 build
- `PrivateMB` 改用 `PrivateUsage`（`PROCESS_MEMORY_COUNTERS_EX` 的私有提交）。
  之前的 `PagefileUsage` 包含共享页，数值偏大；同一台机器上老基准与新快照的
  `PrivateMB` 会有几个到几十 MB 的差异，属于口径修正，不是内存真的变了

## v0.5.0

- **界面重做**：左侧分页导航（进程 / 基准 / 保护名单 / 开机采集 / 设置），
  进程页改任务管理器式表格（名称 / 类型 / 分级 / 内存 / 保护 / 恢复，内存列热力着色、行悬停）
- **外观可切**：设置页新增浅色 / 深色开关，选择写进 `%APPDATA%\MemBase\theme.txt`，下次启动沿用
- **先礼后兵**：有可见窗口的进程先收 `WM_CLOSE`（等同点 ✕），宽限期（默认 3 秒）后才强杀；
  新增 `--grace-wait` / `--force` / `--no-force`
- **身份而不是 PID**：快照记录 `(PID, 创建时间)`，动手前在同一句柄上复核，PID 复用不会误伤
- **关闭编排只有一份**：`crates/win/src/close.rs`，界面与命令行共用；多组共用一个宽限期
- **结果分类上报**：礼貌关闭 / 强制 / 已退出 / 跳过 / 仍未退出，JSON 字段同步扩展
  （`pids` 之外新增 `members`，每个实例带 `pid` + 创建时间，调用方可据此判断实例是否还在）
- **收尾中的进程不再误报**：进程已经关窗、正在退出时 `TerminateProcess` 会返回
  `ERROR_ACCESS_DENIED`，旧版把它当成「关不掉」；现在以退出码 `STILL_ACTIVE` 判定存活，
  如实记账为「礼貌关闭」
- **`kill` 也走同一套编排**：`kill <名|PID>` 默认先礼后兵，要旧行为得显式 `--force`
- **CI**：Linux 跑纯逻辑测试，Windows 跑全量测试 + 构建 + PE 子系统校验
  （见 [TESTING.md](TESTING.md)）

格式与接口变化：

| 位置 | 变化 | 兼容性 |
|---|---|---|
| 快照 CSV | **没有变化**（仍是 `StartTime` 这个可读时间） | 创建时间的整数形式只活在内存模型和 JSON 输出里，身份复核只在执行那一刻需要，不需要落盘 |
| `plan` / `list --json` | 新增 `members[]` | 旧的 `pids[]` 保留，字段只增不减 |
| `trim` / `kill` 的 `--json` | 新增 `requested` / `closed` / `forced` / `exited` / `skipped` / `remaining` | 旧的 `ok` / `failed` 保留 |

## v0.4.0

- **开机采集进了界面**：秒数可调（5–600，默认 60），按钮直接注册 / 取消计划任务；
  任务逻辑统一到 `crates/win/src/task.rs`，界面与命令行共用一份
- **采集耗时可见**：`snapshot_now()` 内部计时，写进 `system.json` 的 `CollectMs` 与 `summary.md`，
  界面显示「采集约 N 毫秒」
- **勾选语义理顺**：右侧「恢复」决定这一轮处理谁，左侧「保护」永久免处理；
  批量操作合并为两个随状态翻转的按钮
- **保护名单修 bug**：通配条目（`进程:MuMu*`）此前永远匹配不上；界面保存会抹掉名单里此刻没在跑的条目
- **分发安全**：注册任务前做 PE 子系统校验，拒绝把界面版当作命令行版拷进数据目录

## v0.3.x

- 0.3.0 — 首个 Rust 版：分层架构、三档分级、dry-run 护栏、基准打包、JSON 输出
- 0.3.1 / 0.3.2 — 品牌图标与版本资源修正
