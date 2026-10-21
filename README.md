# MemBase

**开机内存基准线 + 桌面图标布局，一个纯 Rust 程序。**

Windows 开机久了，内存被谁吃了、桌面图标被谁弄乱了？MemBase 把这两件事放进同一个窗口：
左侧切页，右侧查看和处理。界面用 egui 绘制，没有 .NET / Python / 浏览器运行时依赖。

桌面布局页的预览就是桌面的真实样子：真壁纸、真图标、真任务栏。

![进程页](docs/screenshots/processes.png)

## 功能

| 功能 | 说明 |
|---|---|
| 开机内存基准线 | 开机第 N 秒由 SYSTEM 计划任务采集快照；之后随时对比，列出基准之外的进程与服务 |
| 桌面图标布局 | 保存 / 对比 / 恢复图标位置，带真实预览、差异条和当前位置叠加 |
| 息屏 | 全屏纯黑，ESC / 空格 / 回车 / 任意键退出；时长精确到秒；默认不动显示器电源 |

## 界面

左侧导航八页：

| 页面 | 内容 |
|---|---|
| 进程 | 线外清单（名称 / 类型 / 分级 / 内存 / 保护 / 恢复），内存列按用量热力着色 |
| 基准 | 快照时间、采集于开机第 N 秒、已用内存，导出 / 导入 `.mbz` |
| 桌面布局 | 布局清单、真实预览、差异条、保存 / 恢复 / 删除 / 修复缓存 |
| 保护名单 | `protect.txt` 的全部规则，可单条移除或整份清空 |
| 开机采集 | 开机第 N 秒采集，SYSTEM 权限、无窗口，采完自动销毁任务 |
| 工具 | 息屏（可配热键和时长）+ 显示器与路径信息 |
| 命令行 | 30 个界面动作与等价命令，逐条可复制 |
| 设置 | 数据目录、布局目录、运行身份、浅色 / 深色外观 |

### 桌面布局

![桌面布局页](docs/screenshots/layouts.png)

- 真实预览：当前壁纸打底，逐个取系统图标，按保存时的坐标摆放，连任务栏一起画
- 悬停看名字：图标缩小后认不出，鼠标停上去显示名称
- 叠加当前位置：蓝色虚线标出这次恢复会移动的图标现在在哪
- 差异条：`会动 N · 已消失 N · 新增 N · shell 接口 可用/不可用 · 布局缓存 N 字节`
- 恢复优先走 shell 官方接口（能持久化），不可用时退回改 explorer 内存，并提示「重启可能被覆盖」
- 修复缓存只在「每次重启固定错乱同一批图标」时使用，先备份注册表值再删除

![浅色主题](docs/screenshots/layouts-light.png)

### 工具 · 息屏

![工具页](docs/screenshots/tools.png)

- 默认只盖全屏纯黑窗口，不动显示器电源：这是保证「一定退得出来」的做法
- 退出走系统级监听：后台常挂键盘钩子并轮询按键 / 光标，窗口没有焦点或被隐藏也能退出
- 前 3 秒在屏幕中央显示很暗的「按 ESC 退出 · 剩余时间」，之后彻底全黑
- 可选「同时关闭显示器」：更省电，但部分机器关得掉、叫不醒，所以默认关闭

![息屏黑屏层](docs/screenshots/blackout.png)

## 命令行

同一个程序两种身份：双击 `deskkit-ui.exe` 开界面，带参数就是命令行；`deskkit.exe` 是纯控制台版，给脚本和计划任务用。

```bash
# 基准线
deskkit status
deskkit list
deskkit trim --execute --yes
deskkit baseline capture
deskkit install --at 60

# 桌面布局
deskkit layout list
deskkit layout save 工作
deskkit layout diff 工作 --positions
deskkit layout restore 工作 --execute --yes
deskkit layout repair --execute --yes

# 息屏
deskkit screen off --seconds 90 --exit-key space
deskkit screen duration 0:05:30
deskkit screen status
```

- 所有命令支持 `--json`；`deskkit commands --json` 会导出完整的「界面动作 → 命令」表
- 危险动作默认只报计划：`trim` / `layout restore` / `layout repair` / `layout delete` 需要 `--execute`，删除类还要 `--yes`
- 退出码：`0` 成功 / `1` 参数 / `2` 没有基准或布局 / `3` 读写 / `4` 动作失败 / `5` 被拒绝

![命令行页](docs/screenshots/cli.png)

确认弹窗里也会直接写出这次操作的等价命令：

![确认弹窗](docs/screenshots/confirm.png)

## 下载与运行

从 [Releases](https://github.com/PandaGuGu/MemBase/releases/latest) 下载最新压缩包，解压后运行 `deskkit-ui.exe`。

- 开机采集需要管理员权限；安装后注册 SYSTEM 计划任务，采完自动删除任务和程序副本
- 首次使用先在「基准」页采集或导入一份基准，再回「进程」页对比
- 数据目录保持原位，旧版布局、主题、基准线都能直接读取

| 数据 | 位置 |
|---|---|
| 桌面布局 | `%APPDATA%\DesktopIconPos\layouts\*.json` |
| 主题 / 数据目录配置 | `%APPDATA%\MemBase\theme.txt`、`%APPDATA%\MemBase\config.json` |
| 内存基准线 | `C:\ProgramData\MemBaseline\run_*\T+60s\` |

## 构建与测试

需要 Windows 10/11 + Rust 1.75 以上。

```bash
cargo build --workspace --release
# target\release\deskkit-ui.exe   界面（带参数即命令行）
# target\release\deskkit.exe      控制台版

cargo test -p deskkit-core    # 42 个纯逻辑测试
cargo test -p deskkit-win     # 11 个平台层测试（1 个需真实桌面会话的用例默认忽略）
python tools/verify.py        # 99 项端到端验收
```

`tools/verify.py` 会真跑每个命令和每一页：写盘指向临时目录，危险动作只验护栏，改过的配置全部还原。
最新结果见 [docs/verify-1.0.0.md](docs/verify-1.0.0.md)。

## 已知限制

- 图标按显示名匹配：改名或删除的文件不会恢复，新加的图标保持原位
- 跨进程读写 explorer 内存，杀软可能报毒，加白名单即可
- shell 接口不可用时恢复只改内存，重启可能被系统覆盖
- 在别的分辨率或缩放下保存的布局，坐标可能被网格吸附
- 动态壁纸的预览取到的是当前帧

## License

[MIT](LICENSE)
