# 测试标准

这份文档说明**什么必须测、在哪一层测、什么测不了**，以及合并 / 发版前要过哪些闸门。
命令只列最常用的，完整参数用 `membase commands` 查。

## 一、分层原则

测试跟着架构走，不跟着功能走。三层各自的职责是固定的：

| 层 | crate | 允许依赖 | 必须覆盖 | 禁止出现 |
|---|---|---|---|---|
| 纯逻辑 | `membase-core` | 只允许标准库 | 分级、CSV、diff、线外计划、保护名单匹配、基准打包、配置 | 任何平台 API、任何 `unsafe` |
| 平台 | `membase-win` | Win32 | 只测「同一台机器上能确定性复现」的部分：PE 子系统校验、任务 XML 解析、关闭报告记账 | 依赖真实桌面的断言混进普通 `#[test]` |
| 前端 | `apps/cli` / `apps/gui` | 两个下层 crate | 不写单测，靠 CI 的冒烟命令 + 人工验收 | 在界面里复制一份判定逻辑 |

一条硬规矩：**判定逻辑只有 `core::build_plan()` 一份**。如果某个测试需要「在 core 里造一个 Win32 进程」才能通过，说明设计错了，不是测试错了。

## 二、判定标准（什么时候算通过）

一次改动要算合格，下面五条全过，缺一条就是没做完：

1. `cargo test --workspace` 全绿，且**不新增 `ignored`**（真桌面测试除外，见下）；
2. `cargo check --workspace` **零警告**——警告当错误看，不靠 `#[allow]` 压；
3. `unsafe` 不出现在 `membase-core` / `apps/*`（workspace 级 `unsafe_code = "forbid"` 自动拦）；
4. 新增的**可判定行为**必须配单测：能写成 `assert_eq!` 的就不许留给人眼；
5. 动了快照格式、JSON 字段、命令参数，同一提交里改 `docs/CHANGELOG.md`。

反向标准也说清楚：**跑通一次 GUI 不算测试**。界面只在「人工验收清单」里出现，不进自动化。

## 三、三层怎么跑

### 1. 纯逻辑（任何平台）

```bash
cargo test -p membase-core     # 34 个，秒级
cargo build -p membase-cli     # 平台层在非 Windows 上是空 crate，CLI 仍应能编译
```

这层是回归的主力。包含两个「历史上真踩过」的用例，改动相关代码时特别注意别删：

- `裸名字和问号也认` / `通配保护能命中且不串类型` —— 2026-10 保护名单通配 bug 的守门人，
  当年 `进程:MuMu*` 永远匹配不上，靠 `whitelist.contains(key)` 是查不出来的；
- `identity_matches_rejects_recycled_pid` —— PID 被复用时必须判「不是同一个进程」，这是误伤的唯一防线。

### 2. 平台层（只在 Windows）

```powershell
cargo test -p membase-win      # 9 个 + 1 个 ignored
```

能自动测的是这些：PE 子系统读取（防止把界面版当 CLI 拷进数据目录）、任务 XML 转义与开机秒数解析、
关闭报告的记账分类（`done()` / `remaining` / `skipped` 的口径）。

### 3. 真桌面（手动，`--ignored`）

```powershell
cargo test -p membase-win -- --ignored --nocapture
```

只有一条：`能向窗口应用发出关闭请求`。它会真的弹一个字符映射表窗口，确认「能找到可见窗口并把
`WM_CLOSE` 投递进去」。需要交互式桌面会话，**在 CI 或远程无会话环境里必然失败**，所以默认 ignored。

跑完的输出长这样才算对：

```
charmap: 礼貌关闭 1
test result: ok. 1 passed; ...
```

如果出现 `仍未退出 1（TerminateProcess 失败，错误码 5）`，说明存活判定退回了「句柄打得开就是活着」——
那正是 v0.5.0 修掉的 bug，进程在收尾时 `TerminateProcess` 会返回 `ERROR_ACCESS_DENIED`。

## 四、CI 闸门

`.github/workflows/ci.yml` 两条线，都是合并的必要条件：

| 任务 | 平台 | 跑什么 | 拦的是哪类事故 |
|---|---|---|---|
| `core（Linux）` | ubuntu | `cargo test -p membase-core`、`cargo build -p membase-cli`、`commands --json` / `version` 冒烟 | 判定逻辑被平台细节污染；CLI 在非 Windows 上编不过 |
| `workspace（Windows）` | windows | `cargo test --workspace`、`--release` 构建、两个 exe 的 PE 子系统（CLI=3 / GUI=2） | 测试红着合进去；把窗口版注册成开机任务导致每次开机弹窗 |

PE 子系统那条是**用 `od` 直接读二进制头**做的，不走 `cargo` 的任何元数据：
`e_lfanew(0x3C) → PE\0\0(4) → COFF 头(20) → OptionalHeader.Subsystem`。
看着土，但它验的是真正会被 `task::install()` 检查的那个字节。

## 五、发版前人工验收清单

自动化管不到的部分，逐条打勾再发：

- [ ] `membase collect --out <临时目录>` 能出快照，`summary.md` 里的采集耗时是合理值（本机约 40–60 ms）
- [ ] `membase list` 的线外项和肉眼看到的开机后新增程序对得上
- [ ] `membase trim` 不加 `--execute` 只出计划，项数与 `list` 一致
- [ ] 界面：拖动开机秒数 → 注册任务 → `membase.exe install` 之外的路径都正常，任务里是 **60/自定义秒**而不是硬编码
- [ ] 界面：勾「保护」后该项从建议列表消失，`protect.txt` 里多了一行且顺序被重排
- [ ] 界面：勾「恢复」→ 确认弹窗里的项数与右侧勾选数一致（不是左侧保护数）
- [ ] 关一个**有窗口**的程序，确认它是「礼貌关闭」而不是「强制」；关一个后台常驻进程，确认是「强制」
- [ ] 卸载任务：`membase uninstall` 后计划任务列表里不再有 `MemBase*`
- [ ] 数据目录只剩契约内的四类内容（`membase.exe` / `protect.txt` / `run_*` / `exports`）

## 六、明确不测的东西（及其原因）

写下来是为了避免下次有人「顺手补一个」，结果把 CI 变成看运气的：

| 不测 | 原因 |
|---|---|
| 真实进程被终止后的内存回收 | 依赖系统缓存与调度，同一台机器两次结果都不同 |
| 计划任务真的在开机第 N 秒触发 | 需要重启 + SYSTEM 权限，在 CI 里无法复现；只能人工验一次，看 `run_*` 目录的时间戳 |
| 服务启停 | 有依赖关系、有延迟，环境之间差异大；靠 `svc stop` 的人工验证 |
| 托盘程序 / UWP 的窗口判定 | 窗口归属不稳定（托盘图标不等于顶层可见窗口），属于已知限制而非缺陷 |
| PID 复用 | 系统行为，无法稳定制造；用 `identity_matches` 的纯函数测试代替，真机上只能观测 |

## 七、改测试本身的原则

- 断言失败信息要写「为什么」，不是「expected X got Y」——真机出问题时那条信息就是全部线索；
- 不为通过而放宽断言。`assert!(x >= 0)` 这种等于没测；
- 涉及时间的断言给足余量，或者换成「状态最终一致」的轮询，不写 `sleep(3)` 然后赌；
- 需要真实桌面 / 管理员 / 重启的用例，一律 `#[ignore]` 并写清触发条件，不许偷偷混进默认套件。
