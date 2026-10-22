# MemBase 1.1.0 验收报告

由 `tools/verify.py` 自动生成，2026-10-21 23:53:39

**共 117 项，通过 117 项，失败 0 项。**

验收原则：写盘一律写到临时目录；危险动作只验护栏（演练 / 拒绝）；改过的配置、主题、保护名单、计划任务全部还原。


## 产物

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| deskkit.exe | `C:\Users\Administrator\Desktop\项目集合\MemBase\dist\deskkit.exe` | 存在 | 1201664 字节 | 通过 |  |
| MemBase.exe | `C:\Users\Administrator\Desktop\项目集合\MemBase\dist\MemBase.exe` | 存在 | 7239680 字节 | 通过 |  |
| 版本号 | `deskkit version` | 0 / deskkit 1.1.0 | 0 / deskkit 1.1.0 | 通过 |  |

## 内存侧

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| help | `deskkit help` | 0 | 0 | 通过 | deskkit 1.1.0 —— 内存基准线 + 桌面图标布局（每个界面操作都有命令） |
| status | `deskkit status` | 0 | 0 | 通过 | 基准快照 : C:\ProgramData\MemBaseline\run_20261021_224225\T+30s |
| list | `deskkit list` | 0 | 0 | 通过 | 基准：C:\ProgramData\MemBaseline\run_20261021_224225\T+30s |
| list --json | `deskkit list --json` | 0 | 0 | 通过 | {"baseline":{"dir":"C:\\ProgramData\\MemBaseline\\run_20261021_2 |
| plan | `deskkit plan` | 0 | 0 | 通过 | 线外 12 项，可处理 4 项（约 785 MB） |
| plan --to | `deskkit plan --to C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\plan.csv` | 0 | 0 | 通过 | 计划已写入 C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_n |
| trim（演练） | `deskkit trim` | 0 | 0 | 通过 | 演练，不动进程 |
| trim --execute 无 --yes | `deskkit trim --execute` | 5 | 5 | 通过 | 非交互环境被正确拒绝 |
| kill 不存在的目标 | `deskkit kill no-such-proc-deskkit` | 2 | 2 | 通过 | 没找到匹配的进程：no-such-proc-deskkit |
| svc stop（演练） | `deskkit svc stop Spooler` | (0, 4) | 0 | 通过 | [dry-run] 将停止服务 Spooler（加 --execute 才执行） |
| compare | `deskkit compare` | 0 | 0 | 通过 | [..] 基准：C:\ProgramData\MemBaseline\run_20261021_224225\T+30s |
| protect list | `deskkit protect list --json` | 0 | 0 | 通过 | {"file":"C:\\ProgramData\\MemBaseline\\protect.txt","count":0,"p |
| protect add | `deskkit protect add 进程:__deskkit_verify__` | 0 | 0 | 通过 | 保护名单（1 项，C:\ProgramData\MemBaseline\protect.txt）： |
| protect add 生效 | `deskkit protect list --json` | 0 | 0 | 通过 | {"file":"C:\\ProgramData\\MemBaseline\\protect.txt","count":1,"p |
| protect remove | `deskkit protect remove 进程:__deskkit_verify__` | 0 | 0 | 通过 | 保护名单为空（C:\ProgramData\MemBaseline\protect.txt） |
| protect remove 生效 | `deskkit protect list` | 0 | 0 | 通过 | 保护名单为空（C:\ProgramData\MemBaseline\protect.txt） |
| collect --out 临时目录 | `deskkit collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\collect --at 0` | 0 | 0 | 通过 | [OK] 采集完成：132 个进程 / 338 个服务，开机第 4258 秒 |
| collect --json | `deskkit collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\collect2 --at 0 --json` | 0 | 0 | 通过 | {"ok":true,"dir":"C:\\Users\\Administrator\\AppData\\Local\\Temp |
| baseline capture | `deskkit baseline capture --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\baseline` | 0 | 0 | 通过 | 已扫描当前状态并设为基准 |
| baseline info | `deskkit baseline info --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\baseline` | 0 | 0 | 通过 | 当前基准 : C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_ |
| baseline verify | `deskkit baseline verify --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\baseline` | 0 | 0 | 通过 | 当前基准 : C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_ |
| baseline export | `deskkit baseline export --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\baseline --to C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\b.mbz` | 0 | 0 | 通过 | 已导出基准线 |
| baseline import | `deskkit baseline import C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\b.mbz --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\baseline2` | 0 | 0 | 通过 | 已导入基准线（5 个文件，crc32 80a3855e） |

## 瘦身与启动项

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| slim（演练） | `deskkit slim` | 0 | 0 | 通过 | 不动系统 |
| slim --json | `deskkit slim --json` | 0 | 0 | 通过 | {"mode":"dry_run","baseline":"2026-10-21 22:42:25","count":0,"sa |
| slim --include-caution | `deskkit slim --include-caution --json` | 0 | 0 | 通过 | {"mode":"dry_run","baseline":"2026-10-21 22:42:25","count":22,"s |
| slim --execute 无 --yes | `deskkit slim --execute` | 5 | 5 | 通过 | 非交互环境被正确拒绝 |
| startup list | `deskkit startup list` | 0 | 0 | 通过 | 共 18 项启动项：启用 3 / 已禁用 15（来源：Run、启动文件夹、非 Microsoft 计划任务） |
| startup list --json | `deskkit startup list --json` | 0 | 0 | 通过 | {"mode":"list","count":18,"items":[{"name":"RtkAudUService","sou |
| startup disable 不存在 | `deskkit startup disable no-such-startup-item` | 2 | 2 | 通过 | 没找到启动项：no-such-startup-item（用 startup list 看准确名字） |
| startup disable 无 --yes | `deskkit startup disable __none__ --execute` | 2 | 2 | 通过 | 名字不存在，先于 --yes 校验被拒 |
| feature list | `deskkit feature list` | 0 | 0 | 通过 | 功能开关（只写 HKCU，不碰组策略；改完下次开机 / 重启宿主才见效） |
| feature list --json | `deskkit feature list --json` | 0 | 0 | 通过 | {"mode":"list","count":2,"items":[{"key":"search-highlights","la |
| feature off 未知 key | `deskkit feature off no-such-key --execute --yes` | 1 | 1 | 通过 | 未知开关: no-such-key（可选 search-highlights, cross-device-resume） |
| feature off 无 --yes | `deskkit feature off cross-device-resume --execute` | 5 | 5 | 通过 | 没 --yes 不写注册表 |
| restart（演练） | `deskkit restart explorer` | 0 | 0 | 通过 | 只报计划 |
| restart explorer 无 --yes | `deskkit restart explorer --execute` | 5 | 5 | 通过 | 会闪任务栏的动作被拦下 |
| restart 不存在的目标 | `deskkit restart no-such-proc-deskkit` | 2 | 2 | 通过 | 没找到进程或服务：no-such-proc-deskkit |
| transient（无双采样） | `deskkit transient --json` | 2 | 2 | 通过 | {"ok":false,"error":"no_dual_sample","hint":"先注册双采样：deskkit inst |

## 配置与外观

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| config get | `deskkit config get --json` | 0 | 0 | 通过 | {"ok":true,"config_file":"C:\\Users\\Administrator\\AppData\\Roa |
| config set | `deskkit config set C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_nufsl10j\cfgroot` | 0 | 0 | 通过 | 已设置数据目录：C:\Users\Administrator\AppData\Local\Temp\deskkit_verify |
| theme get | `deskkit theme get --json` | 0 | 0 | 通过 | {"ok":true,"theme":"dark","file":"C:\\Users\\Administrator\\AppD |
| theme set light | `deskkit theme set light` | 0 | 0 | 通过 | 当前外观 : 浅色 |
| theme set dark | `deskkit theme set dark` | 0 | 0 | 通过 | 当前外观 : 深色 |

## 桌面布局

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| layout list | `deskkit layout list --json` | 0 | 0 | 通过 | {"dir":"C:\\Users\\Administrator\\AppData\\Roaming\\DesktopIconP |
| layout status | `deskkit layout status --json` | 0 | 0 | 通过 | {"explorer_pid":3844,"icons":87,"auto_arrange":false,"screen":{" |
| layout probe | `deskkit layout probe --json` | (0, 4) | 0 | 通过 | {"shell_api":true,"count":87,"items":[{"name":"GuGu Panda","x":1 |
| layout dir | `deskkit layout dir --json` | 0 | 0 | 通过 | {"dir":"C:\\Users\\Administrator\\AppData\\Roaming\\DesktopIconP |
| layout repair（演练） | `deskkit layout repair --json` | 0 | 0 | 通过 | 演练，不动注册表 |
| 演练未改注册表 | `layout repair（无 --execute）` | 4742 字节 | 4742 字节 | 通过 | 缓存大小前后一致 |
| layout save | `deskkit layout save __deskkit_verify__` | 0 | 0 | 通过 | 已保存 87 个图标的位置 -> C:\Users\Administrator\AppData\Roaming\DesktopI |
| layout show | `deskkit layout show __deskkit_verify__ --json` | 0 | 0 | 通过 | {"name":"__deskkit_verify__","created":"2026-10-21 23:52:55","sc |
| layout diff | `deskkit layout diff __deskkit_verify__ --json` | 0 | 0 | 通过 | {"layout":"__deskkit_verify__","total":87,"to_move":0,"missing": |
| layout diff --positions | `deskkit layout diff __deskkit_verify__ --positions --json` | 0 | 0 | 通过 | {"layout":"__deskkit_verify__","total":87,"to_move":0,"missing": |
| layout restore（演练） | `deskkit layout restore __deskkit_verify__ --json` | 0 | 0 | 通过 | {"ok":true,"dry_run":true,"layout":"__deskkit_verify__","total": |
| layout restore 无 --yes | `deskkit layout restore __deskkit_verify__ --execute` | 5 | 5 | 通过 | 被正确拒绝 |
| layout delete 无 --yes | `deskkit layout delete __deskkit_verify__` | 5 | 5 | 通过 | 被正确拒绝 |
| layout delete --yes | `deskkit layout delete __deskkit_verify__ --yes` | 0 | 0 | 通过 | 已删除: __deskkit_verify__ |

## 开机采集

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| install | `deskkit install --at 60` | 0 | 0 | 通过 | {"ok":true,"task":"MemBaseline_CollectOnBoot","exe":"C:\\Pro |
| 计划任务已注册 | `schtasks /Query` | 0 | 0 | 通过 |  |
| uninstall | `deskkit uninstall` | 0 | 0 | 通过 | {"ok":true,"task":"MemBaseline_CollectOnBoot"} |
| 计划任务已删除 | `schtasks /Query` | 非 0 | 1 | 通过 |  |

## 一致性

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| gui_actions 数量 | `commands --json` | 37 | 37 | 通过 |  |
| 动作 key 不重复 | `commands --json` | 37 | 37 | 通过 |  |
| 每条命令的动词都存在 | `37 条 cli 解析` | 全部命中 | 未命中 0 条 | 通过 |  |
| GUI 引用的 key 都在表里 | `扫描 main.rs 中 cli_for/with_cli` | 无缺失 | 缺失 0 | 通过 |  |
| 仅由「命令行」页呈现的动作 | `界面按钮未单独挂提示` | 仅供清单页 | 3 | 通过 | screen_key, screen_power, slim_caution |

## 息屏

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| screen status | `deskkit screen status --json` | 0 | 0 | 通过 | {"monitors":1,"screen":{"w":1920,"h":1080},"dpi_scale":1.25,"cha |
| screen duration（读默认） | `deskkit screen duration --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":15,"default_minutes":0,"text":"0:00 |
| screen key（读默认） | `deskkit screen key --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"esc"} |
| duration 0:02:30（时分秒） | `deskkit screen duration 0:02:30 --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":150,"default_minutes":2,"text":"0:0 |
| duration 读回 150 秒 | `deskkit screen duration --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":150,"default_minutes":2,"text":"0:0 |
| duration 90s（带单位） | `deskkit screen duration 90s --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":90,"default_minutes":1,"text":"0:01 |
| duration 5m（分钟单位） | `deskkit screen duration 5m --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":300,"default_minutes":5,"text":"0:0 |
| minutes 30（老写法=分钟） | `deskkit screen minutes 30 --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":1800,"default_minutes":30,"text":"0 |
| duration 非法输入被拒 | `deskkit screen duration abc` | 1 | 1 | 通过 | 看不懂这个时长：abc（用法：1:05:09 / 90s / 5m / 1h / 330） |
| key space（改热键） | `deskkit screen key space --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"space"} |
| key any（改热键） | `deskkit screen key any --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"any"} |
| key esc（还原） | `deskkit screen key esc --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"esc"} |
| key 非法值被拒 | `deskkit screen key f1` | 1 | 1 | 通过 | 用法: deskkit screen key esc\|space\|enter\|any |
| power（读默认） | `deskkit screen power --json` | 0 | 0 | 通过 | {"ok":true,"power_off":false} |
| power on（写） | `deskkit screen power on --json` | 0 | 0 | 通过 | {"ok":true,"power_off":true} |
| power off（还原） | `deskkit screen power off --json` | 0 | 0 | 通过 | {"ok":true,"power_off":false} |
| power 非法值被拒 | `deskkit screen power maybe` | 1 | 1 | 通过 | 用法: deskkit screen power on\|off（默认 off：只盖全屏黑窗） |
| off --dry-run（用默认时长/热键） | `deskkit screen off --dry-run --json` | (0, 4) | 0 | 通过 | 只探通道，不关屏 |
| off --seconds 45 --exit-key enter | `deskkit screen off --seconds 45 --exit-key enter --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":45,"minutes":0,"delay":0,"ex |
| off --minutes 2（老写法） | `deskkit screen off --minutes 2 --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":120,"minutes":2,"delay":0,"e |
| off --seconds 0（不定时） | `deskkit screen off --seconds 0 --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":0,"minutes":0,"delay":0,"exi |
| 时长已还原 | `screen duration 15` | 15 | 15 | 通过 |  |
| 热键已还原 | `screen key esc` | esc | esc | 通过 |  |
| 关显示屏开关已还原 | `screen power False` | False | False | 通过 |  |
| 演练没改「上次息屏」 | `screen status --json` | 2026-10-21 16:53:46 | 2026-10-21 16:53:46 | 通过 |  |
| 演练没改记录文件 | `last_screen_off.txt` | b'2026-10-21 16:53:46' | b'2026-10-21 16:53:46' | 通过 | 避免把「演练」记成真的用过 |
| 黑屏层（全屏） | `DESKKIT_BLACKOUT_TEST=1` | ≥95% 纯黑 + 有提示文字 | 1920x1080 / 6 色 / 纯黑 99.9% | 通过 |  |
| 黑屏层已置顶 | `DESKKIT_BLACKOUT_TEST=1` | WS_EX_TOPMOST 置位 | 置位 | 通过 |  |
| ESC 退出（正常） | `keybd_event VK_ESCAPE` | 退出息屏（撤销置顶） | 已退出 | 通过 |  |
| ESC 退出（最小化） | `keybd_event VK_ESCAPE` | 退出息屏（撤销置顶） | 已退出 | 通过 |  |

## 环境复原

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| 保护名单条数 | `protect list --json` | 0 | 0 | 通过 | 与验收前一致 |
| 主题可读 | `theme get --json` | dark/light | dark | 通过 |  |
| 数据目录可读 | `config get --json` | 非空 | C:\ProgramData\MemBaseline | 通过 |  |
| 没有残留计划任务 | `schtasks /Query` | 非 0 | 1 | 通过 |  |

## 界面

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| 进程（默认页） | `DESKKIT_PAGE=(默认)` | 渲染正常 | 1475x975 / 180 色 | 通过 |  |
| 瘦身 | `DESKKIT_PAGE=slim` | 渲染正常 | 1475x975 / 75 色 | 通过 |  |
| 启动项 | `DESKKIT_PAGE=startup` | 渲染正常 | 1475x975 / 149 色 | 通过 |  |
| 基准 | `DESKKIT_PAGE=baseline` | 渲染正常 | 1475x975 / 142 色 | 通过 |  |
| 桌面布局 | `DESKKIT_PAGE=layouts` | 渲染正常 | 1475x975 / 2035 色 | 通过 |  |
| 保护名单 | `DESKKIT_PAGE=protect` | 渲染正常 | 1475x975 / 92 色 | 通过 |  |
| 开机采集 | `DESKKIT_PAGE=boot` | 渲染正常 | 1475x975 / 97 色 | 通过 |  |
| 工具 | `DESKKIT_PAGE=tools` | 渲染正常 | 1475x975 / 135 色 | 通过 |  |
| 命令行 | `DESKKIT_PAGE=cli` | 渲染正常 | 1475x975 / 149 色 | 通过 |  |
| 设置 | `DESKKIT_PAGE=settings` | 渲染正常 | 1475x975 / 124 色 | 通过 |  |
| 各页内容互不相同 | `10 张截图 md5` | 10 | 10 | 通过 |  |
| 确认弹窗 · 恢复基准 | `DESKKIT_SHOT_MODAL=trim` | 渲染正常 | 1475x975 / 220 色 | 通过 |  |
| 确认弹窗 · 恢复布局 | `DESKKIT_SHOT_MODAL=layout` | 渲染正常 | 1475x975 / 1406 色 | 通过 |  |

## 环境复原记录

- config：已清除（原本未设置）
- 已清理 install 留下的程序副本 C:\ProgramData\MemBaseline\deskkit.exe
