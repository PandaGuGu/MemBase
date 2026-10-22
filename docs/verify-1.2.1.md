# MemBase 1.2.1 验收报告

由 `tools/verify.py` 自动生成，2026-10-22 09:05:36

**共 107 项，通过 107 项，失败 0 项。**

> 本次用 `--fast` 跑（跳过 10 页界面截图与黑屏层那两项，避免抢占前台），
> 覆盖全部命令行动词、产物断言、配置/布局/计划任务往返与环境复原。
> 含界面截图的全量 124 项见 [docs/verify-1.2.0.md](verify-1.2.0.md)；
> 1.2.1 相对 1.2.0 只改了品牌串、分发打包与文档，界面渲染逻辑未动。

验收原则：写盘一律写到临时目录；危险动作只验护栏（演练 / 拒绝）；改过的配置、主题、保护名单、计划任务全部还原。


## 产物

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| MemBase.exe | `C:\Users\Administrator\Desktop\项目集合\MemBase\dist\MemBase.exe` | 存在 | 7313920 字节 | 通过 |  |
| 只有一个 exe | `dist/*.exe` | 仅 MemBase.exe | 仅 MemBase.exe | 通过 |  |
| 版本号 | `MemBase.exe version` | 0 / MemBase 1.2.1 | 0 / MemBase 1.2.1 | 通过 |  |

## 内存侧

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| help | `MemBase.exe help` | 0 | 0 | 通过 | MemBase.exe 1.2.1 —— 内存基准线 + 桌面图标布局（每个界面操作都有命令） |
| status | `MemBase.exe status` | 0 | 0 | 通过 | 基准快照 : C:\ProgramData\MemBaseline\run_20261022_082929\T+300s |
| list | `MemBase.exe list` | 0 | 0 | 通过 | 基准：C:\ProgramData\MemBaseline\run_20261022_082929\T+300s |
| list --json | `MemBase.exe list --json` | 0 | 0 | 通过 | {"baseline":{"dir":"C:\\ProgramData\\MemBaseline\\run_20261022_0 |
| plan | `MemBase.exe plan` | 0 | 0 | 通过 | 线外 14 项，可处理 9 项（预计回收约 757 MB） |
| plan --json 带宿主字段 | `MemBase.exe plan --json` | 0 | 0 | 通过 | {"mode":"plan","baseline":"C:\\ProgramData\\MemBaseline\\run_202 |
| status --json 带自愈记录 | `MemBase.exe status --json` | 0 | 0 | 通过 | {"baseline":{"dir":"C:\\ProgramData\\MemBaseline\\run_20261022_0 |
| plan --to | `MemBase.exe plan --to C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\plan.csv` | 0 | 0 | 通过 | 计划已写入 C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0 |
| trim（演练） | `MemBase.exe trim` | 0 | 0 | 通过 | 演练，不动进程 |
| trim --execute 无 --yes | `MemBase.exe trim --execute` | 5 | 5 | 通过 | 非交互环境被正确拒绝 |
| kill 不存在的目标 | `MemBase.exe kill no-such-proc-MemBase.exe` | 2 | 2 | 通过 | 没找到匹配的进程：no-such-proc-MemBase.exe |
| svc stop（演练） | `MemBase.exe svc stop Spooler` | (0, 4) | 0 | 通过 | [dry-run] 将停止服务 Spooler（加 --execute 才执行） |
| compare | `MemBase.exe compare` | 0 | 0 | 通过 | [..] 基准：C:\ProgramData\MemBaseline\run_20261022_082929\T+300s |
| protect list | `MemBase.exe protect list --json` | 0 | 0 | 通过 | {"file":"C:\\ProgramData\\MemBaseline\\protect.txt","count":0,"p |
| protect add | `MemBase.exe protect add 进程:__deskkit_verify__` | 0 | 0 | 通过 | 保护名单（1 项，C:\ProgramData\MemBaseline\protect.txt）： |
| protect add 生效 | `MemBase.exe protect list --json` | 0 | 0 | 通过 | {"file":"C:\\ProgramData\\MemBaseline\\protect.txt","count":1,"p |
| protect remove | `MemBase.exe protect remove 进程:__deskkit_verify__` | 0 | 0 | 通过 | 保护名单为空（C:\ProgramData\MemBaseline\protect.txt） |
| protect remove 生效 | `MemBase.exe protect list` | 0 | 0 | 通过 | 保护名单为空（C:\ProgramData\MemBaseline\protect.txt） |
| collect --out 临时目录 | `MemBase.exe collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\collect --at 0` | 0 | 0 | 通过 | [OK] 采集完成：128 个进程 / 338 个服务，开机第 2219 秒 |
| collect --json | `MemBase.exe collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\collect2 --at 0 --json` | 0 | 0 | 通过 | {"ok":true,"dir":"C:\\Users\\Administrator\\AppData\\Local\\Temp |
| collect --keep | `MemBase.exe collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\collect3 --at 0 --keep 99` | 0 | 0 | 通过 | [OK] 采集完成：128 个进程 / 338 个服务，开机第 2219 秒 |
| baseline capture | `MemBase.exe baseline capture --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\baseline` | 0 | 0 | 通过 | 已扫描当前状态并设为基准 |
| baseline info | `MemBase.exe baseline info --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\baseline` | 0 | 0 | 通过 | 当前基准 : C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_ |
| baseline verify | `MemBase.exe baseline verify --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\baseline` | 0 | 0 | 通过 | 当前基准 : C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_ |
| baseline export | `MemBase.exe baseline export --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\baseline --to C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\b.mbz` | 0 | 0 | 通过 | 已导出基准线 |
| baseline import | `MemBase.exe baseline import C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\b.mbz --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\baseline2` | 0 | 0 | 通过 | 已导入基准线（5 个文件，crc32 57acb8af） |

## 瘦身与启动项

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| slim（演练） | `MemBase.exe slim` | 0 | 0 | 通过 | 不动系统 |
| slim --json | `MemBase.exe slim --json` | 0 | 0 | 通过 | {"mode":"dry_run","baseline":"2026-10-22 08:33:29","count":0,"sa |
| slim --json 不输出 -0.0 | `MemBase.exe slim --json` | 0 | 0 | 通过 | sum 空集合不能是负零 |
| slim --include-caution | `MemBase.exe slim --include-caution --json` | 0 | 0 | 通过 | {"mode":"dry_run","baseline":"2026-10-22 08:33:29","count":23,"s |
| slim --execute 无 --yes | `MemBase.exe slim --execute` | 5 | 5 | 通过 | 非交互环境被正确拒绝 |
| startup list | `MemBase.exe startup list` | 0 | 0 | 通过 | 共 18 项启动项：启用 3 / 已禁用 15（来源：Run、启动文件夹、计划任务） |
| startup list --json | `MemBase.exe startup list --json` | 0 | 0 | 通过 | {"mode":"list","count":18,"system_hidden":100,"items":[{"name":" |
| startup list 系统任务默认隐藏 | `MemBase.exe startup list --json` | 0 | 0 | 通过 | {"mode":"list","count":18,"system_hidden":100,"items":[{"name":" |
| startup list --all 列出系统任务 | `MemBase.exe startup list --all --json` | 0 | 0 | 通过 | {"mode":"list","count":118,"system_hidden":0,"items":[{"name":"\ |
| startup disable 不存在 | `MemBase.exe startup disable no-such-startup-item` | 2 | 2 | 通过 | 没找到启动项：no-such-startup-item（用 startup list 看准确名字） |
| startup disable 无 --yes | `MemBase.exe startup disable __none__ --execute` | 2 | 2 | 通过 | 名字不存在，先于 --yes 校验被拒 |
| feature list | `MemBase.exe feature list` | 0 | 0 | 通过 | 功能开关（只写 HKCU，不碰组策略；改完下次开机 / 重启宿主才见效） |
| feature list --json | `MemBase.exe feature list --json` | 0 | 0 | 通过 | {"mode":"list","count":2,"items":[{"key":"search-highlights","la |
| feature off 未知 key | `MemBase.exe feature off no-such-key --execute --yes` | 1 | 1 | 通过 | 未知开关: no-such-key（可选 search-highlights, cross-device-resume） |
| feature off 无 --yes | `MemBase.exe feature off cross-device-resume --execute` | 5 | 5 | 通过 | 没 --yes 不写注册表 |
| restart（演练） | `MemBase.exe restart explorer` | 0 | 0 | 通过 | 只报计划 |
| restart explorer 无 --yes | `MemBase.exe restart explorer --execute` | 5 | 5 | 通过 | 会闪任务栏的动作被拦下 |
| restart 不存在的目标 | `MemBase.exe restart no-such-proc-MemBase.exe` | 2 | 2 | 通过 | 没找到进程或服务：no-such-proc-MemBase.exe |
| transient（先造单采样） | `MemBase.exe collect --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\transient_single --at 0` | 0 | 0 | 通过 | 只采一次，不注册双采样 |
| transient（无双采样） | `MemBase.exe transient --out C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\transient_single --json` | 2 | 2 | 通过 | {"ok":false,"error":"no_dual_sample","hint":"先注册双采样：MemBase.exe  |

## 配置与外观

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| config get | `MemBase.exe config get --json` | 0 | 0 | 通过 | {"ok":true,"config_file":"C:\\Users\\Administrator\\AppData\\Roa |
| config set | `MemBase.exe config set C:\Users\Administrator\AppData\Local\Temp\deskkit_verify_0efnd5wd\cfgroot` | 0 | 0 | 通过 | 已设置数据目录：C:\Users\Administrator\AppData\Local\Temp\deskkit_verify |
| theme get | `MemBase.exe theme get --json` | 0 | 0 | 通过 | {"ok":true,"theme":"dark","file":"C:\\Users\\Administrator\\AppD |
| theme set light | `MemBase.exe theme set light` | 0 | 0 | 通过 | 当前外观 : 浅色 |
| theme set dark | `MemBase.exe theme set dark` | 0 | 0 | 通过 | 当前外观 : 深色 |

## 桌面布局

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| layout list | `MemBase.exe layout list --json` | 0 | 0 | 通过 | {"dir":"C:\\Users\\Administrator\\AppData\\Roaming\\DesktopIconP |
| layout status | `MemBase.exe layout status --json` | 0 | 0 | 通过 | {"explorer_pid":1224,"icons":85,"auto_arrange":false,"screen":{" |
| layout probe | `MemBase.exe layout probe --json` | (0, 4) | 0 | 通过 | {"shell_api":true,"count":85,"items":[{"name":"GuGu Panda","x":1 |
| layout dir | `MemBase.exe layout dir --json` | 0 | 0 | 通过 | {"dir":"C:\\Users\\Administrator\\AppData\\Roaming\\DesktopIconP |
| layout repair（演练） | `MemBase.exe layout repair --json` | 0 | 0 | 通过 | 演练，不动注册表 |
| 演练未改注册表 | `layout repair（无 --execute）` | 4582 字节 | 4582 字节 | 通过 | 缓存大小前后一致 |
| layout save | `MemBase.exe layout save __deskkit_verify__` | 0 | 0 | 通过 | 已保存 85 个图标的位置 -> C:\Users\Administrator\AppData\Roaming\DesktopI |
| layout show | `MemBase.exe layout show __deskkit_verify__ --json` | 0 | 0 | 通过 | {"name":"__deskkit_verify__","created":"2026-10-22 09:05:32","sc |
| layout diff | `MemBase.exe layout diff __deskkit_verify__ --json` | 0 | 0 | 通过 | {"layout":"__deskkit_verify__","total":85,"to_move":0,"missing": |
| layout diff --positions | `MemBase.exe layout diff __deskkit_verify__ --positions --json` | 0 | 0 | 通过 | {"layout":"__deskkit_verify__","total":85,"to_move":0,"missing": |
| layout restore（演练） | `MemBase.exe layout restore __deskkit_verify__ --json` | 0 | 0 | 通过 | {"ok":true,"dry_run":true,"layout":"__deskkit_verify__","total": |
| layout restore 无 --yes | `MemBase.exe layout restore __deskkit_verify__ --execute` | 5 | 5 | 通过 | 被正确拒绝 |
| layout delete 无 --yes | `MemBase.exe layout delete __deskkit_verify__` | 5 | 5 | 通过 | 被正确拒绝 |
| layout delete --yes | `MemBase.exe layout delete __deskkit_verify__ --yes` | 0 | 0 | 通过 | 已删除: __deskkit_verify__ |

## 开机采集

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| install | `MemBase.exe install --at 60` | 0 | 0 | 通过 | {"ok":true,"task":"MemBaseline_CollectOnBoot","exe":"C:\\Pro |
| 计划任务已注册 | `schtasks /Query` | 0 | 0 | 通过 |  |
| uninstall | `MemBase.exe uninstall` | 0 | 0 | 通过 | {"ok":true,"task":"MemBaseline_CollectOnBoot"} |
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
| screen status | `MemBase.exe screen status --json` | 0 | 0 | 通过 | {"monitors":1,"screen":{"w":1920,"h":1080},"dpi_scale":1.25,"cha |
| screen duration（读默认） | `MemBase.exe screen duration --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":15,"default_minutes":0,"text":"0:00 |
| screen key（读默认） | `MemBase.exe screen key --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"esc"} |
| duration 0:02:30（时分秒） | `MemBase.exe screen duration 0:02:30 --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":150,"default_minutes":2,"text":"0:0 |
| duration 读回 150 秒 | `MemBase.exe screen duration --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":150,"default_minutes":2,"text":"0:0 |
| duration 90s（带单位） | `MemBase.exe screen duration 90s --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":90,"default_minutes":1,"text":"0:01 |
| duration 5m（分钟单位） | `MemBase.exe screen duration 5m --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":300,"default_minutes":5,"text":"0:0 |
| minutes 30（老写法=分钟） | `MemBase.exe screen minutes 30 --json` | 0 | 0 | 通过 | {"ok":true,"default_seconds":1800,"default_minutes":30,"text":"0 |
| duration 非法输入被拒 | `MemBase.exe screen duration abc` | 1 | 1 | 通过 | 看不懂这个时长：abc（用法：1:05:09 / 90s / 5m / 1h / 330） |
| key space（改热键） | `MemBase.exe screen key space --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"space"} |
| key any（改热键） | `MemBase.exe screen key any --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"any"} |
| key esc（还原） | `MemBase.exe screen key esc --json` | 0 | 0 | 通过 | {"ok":true,"exit_key":"esc"} |
| key 非法值被拒 | `MemBase.exe screen key f1` | 1 | 1 | 通过 | 用法: MemBase.exe screen key esc\|space\|enter\|any |
| power（读默认） | `MemBase.exe screen power --json` | 0 | 0 | 通过 | {"ok":true,"power_off":false} |
| power on（写） | `MemBase.exe screen power on --json` | 0 | 0 | 通过 | {"ok":true,"power_off":true} |
| power off（还原） | `MemBase.exe screen power off --json` | 0 | 0 | 通过 | {"ok":true,"power_off":false} |
| power 非法值被拒 | `MemBase.exe screen power maybe` | 1 | 1 | 通过 | 用法: MemBase.exe screen power on\|off（默认 off：只盖全屏黑窗） |
| off --dry-run（用默认时长/热键） | `MemBase.exe screen off --dry-run --json` | (0, 4) | 0 | 通过 | 只探通道，不关屏 |
| off --seconds 45 --exit-key enter | `MemBase.exe screen off --seconds 45 --exit-key enter --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":45,"minutes":0,"delay":0,"ex |
| off --minutes 2（老写法） | `MemBase.exe screen off --minutes 2 --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":120,"minutes":2,"delay":0,"e |
| off --seconds 0（不定时） | `MemBase.exe screen off --seconds 0 --dry-run --json` | (0, 4) | 0 | 通过 | {"ok":true,"dry_run":true,"seconds":0,"minutes":0,"delay":0,"exi |
| 时长已还原 | `screen duration 15` | 15 | 15 | 通过 |  |
| 热键已还原 | `screen key esc` | esc | esc | 通过 |  |
| 关显示屏开关已还原 | `screen power False` | False | False | 通过 |  |
| 演练没改「上次息屏」 | `screen status --json` | 2026-10-21 16:53:46 | 2026-10-21 16:53:46 | 通过 |  |
| 演练没改记录文件 | `last_screen_off.txt` | b'2026-10-21 16:53:46' | b'2026-10-21 16:53:46' | 通过 | 避免把「演练」记成真的用过 |

## 环境复原

| 项目 | 命令 | 期望 | 实际 | 结论 | 备注 |
|---|---|---|---|---|---|
| 保护名单条数 | `protect list --json` | 0 | 0 | 通过 | 与验收前一致 |
| 主题可读 | `theme get --json` | dark/light | dark | 通过 |  |
| 数据目录可读 | `config get --json` | 非空 | C:\ProgramData\MemBaseline | 通过 |  |
| 没有残留计划任务 | `schtasks /Query` | 非 0 | 1 | 通过 |  |

## 环境复原记录

- config：已清除（原本未设置）
- 已清理 install 留下的程序副本 C:\ProgramData\MemBaseline\membase.exe
