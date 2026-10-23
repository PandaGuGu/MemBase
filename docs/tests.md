# MemBase 测试集

三层结构，成本从低到高。**发版前三层都要过**；只跑第一层就发版是不允许的。

```
第一层 单元测试      cargo test --workspace        毫秒级，随代码走
第二层 端到端验收    python tools/verify.py        1～2 分钟，打 dist/MemBase.exe
第三层 专项实验      手工，动真桌面                 每次改布局相关代码必跑
```

## 0. 基线与环境

| 项 | 值 |
|---|---|
| 系统 | Windows 11 x64（具体 build 看 `MemBase.exe status --json` 的 `Build`） |
| 显示 | 1920×1080 @125%（120dpi），单屏 |
| 桌面规模 | 82 个图标 |
| 基准布局 | `10-22 16_46` |
| 当前基线 | `cargo test --workspace` 全绿（core 73 / win 17 + 1 ignored）；`verify.py` **129/129** |

数字会随版本变化，改动后请同步更新本表和 `tools/verify.py` 里的 `expect_actions`。

---

## 1. 第一层：单元测试

```bash
cargo test --workspace
```

| crate | 数量 | 覆盖什么 |
|---|---|---|
| `membase-core` | 73 | 内存分级与判据、CSV 读写、snapshot diff、bundle 打包、布局模型（`Layout` / `DisplayInfo::same_hardware`）、保护名单通配匹配 |
| `membase-win` | 17（+1 ignored） | `IconLayouts` 二进制编解码、显示器指纹枚举、计划任务 XML、路径与转义 |

- 零平台依赖的部分在容器 / CI 里也能跑；`membase-win` 的用例只要 Windows 就行，**不需要桌面会话**。
- 那 1 个 `#[ignore]` 是真桌面用例，手动跑：

```bash
cargo test -p membase-win -- --ignored
```

判定：`0 failed`。有 ignored 是正常的，不算失败。

---

## 2. 第二层：端到端验收

```bash
python tools/verify.py          # 全量，含界面截图
python tools/verify.py --fast   # 跳过截图，改界面外的改动可以用这个
```

打的是 `dist/MemBase.exe`（单文件：不带参数开界面，带参数走命令行）。共 **129 项，11 组**：

| 分组 | 项数 | 关注点 |
|---|---|---|
| 产物 | 3 | 只有一个 exe、版本号与 `Cargo.toml` 一致、体积合理 |
| 内存侧 | 26 | `collect` / `diff` / `trim` / `feature` / `startup` / `selfheal` 每个动词 + 危险动作的护栏 |
| 瘦身与启动项 | 20 | `slim` 计划、服务真禁用、`StartupApproved` 写法、还原 |
| 配置与外观 | 5 | 配置读写、主题切换、还原 |
| 桌面布局 | 14 | 保存 / 差异 / 恢复 / 演练、缓存修复护栏、坐标自洽 |
| 开机采集 | 4 | 计划任务安装 / 查询 / 卸载 |
| 一致性 | 5 | `ACTIONS` 表 ↔ 界面按钮 ↔ CLI 动词三方对齐 |
| MCP | 4 | MCP 入口可用 |
| 息屏 | 30 | 时长 / 热键 / 关屏开关 + 黑屏层置顶 + ESC 退出（含最小化场景） |
| 环境复原 | 4 | 跑完检查：配置、缓存、注册表都回到原样 |
| 界面 | 14 | 11 页截图互不相同 + 关键控件在位 |

脚本写死的三条原则（见 `tools/verify.py` 文件头）：

1. **只做可逆操作**：写盘一律写到临时目录；改配置、主题、保护名单的最后都还原。
2. **危险动作只验护栏**：不加 `--execute` 必须是演练；加了 `--execute` 但不加 `--yes` 必须被拒。
3. **失败红着写进报告**，不粉饰。

报告落在 `docs/verify-<版本>.md`，每项都记命令、期望、实际、结论。

判定：**129/129**，其中「环境复原」组必须全过 —— 有残留就是 bug，不是"小问题"。

---

## 3. 第三层：专项实验（手工）

脚本替不了的部分：要动真桌面、要看刷新后的实际表现。**改布局相关代码必跑 3.1 和 3.3。**

### 3.1 按 F5 会不会被打回

> 必须用**真实输入**。`PostMessage(WM_KEYDOWN, VK_F5)` 在桌面没有焦点时什么都不做，
> 会得到假的「0 差异」——2026-10-23 在这上面栽过一次，误判成"修好了"。

做法：`SetProcessDPIAware()` → 点桌面空白处 → `keybd_event(VK_F5)`。

```bash
MemBase.exe layout restore "10-22 16_46" --execute --yes
# 按一次 F5
MemBase.exe layout diff "10-22 16_46"      # 期望：需要移动 0
```

双向 A/B（同一份布局、同一台机器、只改一位）：

```bash
MemBase.exe layout flags --no-enum-refresh off
MemBase.exe layout restore "10-22 16_46" --execute --yes   # → F5 后 63～66 项错位
MemBase.exe layout flags --no-enum-refresh on
MemBase.exe layout restore "10-22 16_46" --execute --yes   # → F5 后 0 差异
```

`layout status` 里那行 `视图标志 FFlags` 会直接写「刷新重排已关闭 / 未关闭」。

**正向对照**：先恢复成一份"打乱"的布局，再按 F5 —— 应该保持在打乱状态（与打乱布局 0 差异、
与正式布局有差异）。只测"恢复正式布局后 F5 没变"是不够的，那可能是"什么都没动"。

### 3.2 重启资源管理器

```bash
taskkill /f /im explorer.exe & start explorer.exe
# 等 20 秒
MemBase.exe layout diff "10-22 16_46"       # 预期 39 项错位（shell 按自己的记录铺）
MemBase.exe layout restore "10-22 16_46" --execute --yes
# 按两次 F5
MemBase.exe layout diff "10-22 16_46"       # 期望 0
```

强杀后 `IShellWindows` 要 4～20 秒才注册好桌面视图，太急会退回内存写入（位置对、开关没设，
F5 又乱）。`restore` 默认等 5.6 秒，可用 `--wait` 加长。

### 3.3 登录自动恢复

```bash
MemBase.exe layout autostart on "10-22 16_46"
```

真重启，登录后**等 1 分钟**，然后：

```bash
MemBase.exe layout diff "10-22 16_46"       # 0
MemBase.exe layout status                   # 刷新重排已关闭
```

再按 `%ProgramData%\MemBaseline\layout_restore.log` 看它干了什么：

```
2026-10-23 01:35:51Z  layout=10-22 16_46  moved=39  restored=82  failed=0  missing=0  shell=true  guard=true
```

- `moved=0` 且 `restored=0` → 桌面还没加载出来，那次是空操作（v1.3.0 起会返回非零并自动重试）
- `shell=false` → 没拿到 shell 接口，只改了内存，刷新会丢
- `guard=false` → 没关掉刷新重排，F5 会打回

### 3.4 显示器指纹

手工把布局 JSON 的 `display.virtual_screen` 改成别的分辨率，再 `restore`：
应打印「保存时 / 现在」两行警告，**但仍然继续执行**（不是拒绝）。

### 3.5 界面动作表

新增界面按钮时：

1. `apps/cli/src/layout.rs` 的 `ACTIONS` 加一条；
2. `tools/verify.py` 的 `expect_actions` 改数字；
3. 跑第二层，「一致性」组会验三方是否对齐。

---

## 4. 判定标准

| 层 | 通过条件 |
|---|---|
| 单元 | `0 failed`（ignored 不算失败） |
| 验收 | 129/129，且「环境复原」全过 |
| 专项 | 3.1 / 3.2 / 3.3 判据全中 |

任何一项**不能靠"再跑一次就过了"含糊过去** —— 偶发失败要么是环境没复原，要么是时序问题，
都得查清楚再发版。

---

## 5. 明确不覆盖

- **真实长时间内存漂移**：要跑几小时，交给用户日常采集（`collect` + `compare`），不进测试集
- **多显示器实机热插拔**：只验指纹比对逻辑，不验真实插拔
- **杀毒软件拦截**：Defender 会把工具判成 `Behavior:Win32/Persistence.A!ml`，见 README 的说明
- **非中文系统 / 非 125% 缩放**：只做过手工抽查，没有自动化覆盖

---

## 6. 回归清单

| 改了什么 | 必跑 |
|---|---|
| `crates/core` | 第一层 |
| CLI 动词 / 界面按钮 / `ACTIONS` | 第一层 + 第二层 |
| 桌面布局（`shellview` / `display` / `task`） | 三层全跑，重点 3.1、3.3 |
| 只是文档 / 文案 | 第二层的「产物」「一致性」两组 |
