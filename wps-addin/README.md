# Sidera WPS 加载项

加载项把 WPS 的放映位置和状态回传给批注程序，使批注只在真换页时切换，页内动画不切缓存。内容由 Sidera 的 `http://127.0.0.1:16666/` 提供；默认命令和状态都走同一条 WebSocket，SSE 只作为回退。

Sidera 默认启用完整模式：加载项上报页码、跳页能力和放映状态，并通过 `/ws` WebSocket 双向通道接收翻页、跳页、开始/结束放映命令；WebSocket 不可用或 5 秒内无法建立时，自动回退到 `/events` SSE + `/push`。新加载项不再使用 `/poll` 长轮询，服务端保留它供旧版兼容。

## 部署与使用

关闭 WPS，以普通用户运行一次登记：

```sh
bash install.sh
# 安装 DEB 后：
sidera-wps-addin-install
```

先启动 Sidera，再打开 WPS 演示。出现“批注联动”标签表示加载项已加载；“状态”按钮会询问桥的真实健康响应，连接失败不会再显示在线。修改或更新脚本后需重新打开 WPS。

安装器需要 Python 3，只合并本项目节点，迁移旧名称，保留其他登记、属性和注释；修改前备份、同目录原子替换，异常 XML/符号链接/写入失败不会覆盖原文件。Rust、C++ 自动登记和手动命令共用该脚本，通过文件锁协调本项目并发写入。锁不能约束外部修改器，因此仍建议退出 WPS 后登记。

## 按钮与页码状态

未接入 WPS 时，桌面支持直接按键模拟就继续使用普通样式的上下键和 Esc；联动状态、可执行的放映动作和翻页能力分别判断。

| 状态 | 当前 Rust 版行为 |
| --- | --- |
| 默认模式且联动在线 | 显示“开始放映”，点击注入 Shift+F5；放映中显示“结束放映”，点击注入 Esc |
| 联动在线且支持“当页开始” | 绿色“开始放映”，通过 WPS API 从当前页开始 |
| 联动放映中 | API 上下翻页、“结束放映”；API 失败不补发一次键盘 |
| 未联动/断线，桌面可直接模拟按键 | 普通样式的上下键模拟键盘翻页，放映按钮显示普通样式的 Esc；页码区域隐藏，不发起共享桌面授权 |
| 非白板模式下翻页暂不可用 | 联动放映暂不能执行命令，或当前需模拟按键但桌面不具备该能力时，只有上下键使用当前主题的提示色；Esc 没有这一提示色 |
| 联动在线且页码有效、跳页可用 | 显示当前页/总数；点击页码打开拨轮，预选后确认跳转 |
| 有页码但跳页暂不可用或不支持 | 显示已知页码，点击不打开拨轮 |
| 页码未知/未联动 | 整块页码隐藏；仅缺一项时显示该项为“—” |
| 白板 | 本地 10 页，外部状态继续跟踪；切回普通模式清除白板笔迹并恢复透明批注页 |

Esc、绿色“开始放映”、红色系“结束放映”之间的过渡表示当前可执行动作；上下翻页的提示色表示暂不可用，具体配色随浅色／深色主题变化。不能把绿色开始按钮或翻页提示色当作通用的 WPS 连接指示灯。

C++ 旧版共用此加载项与新桥，但界面仍以 NEXT/PREV/EXIT 为主，没有同步 Rust 的开始按钮、页码拨轮和 GNOME 原生能力。

## 状态与命令

保留兼容的 GET 协议：

- `/hello`：健康问候，不标记放映已联动。
- `/push?m=EVENT ...`：事件和状态上报。
- `/ws`：默认 WebSocket 双向通道；下发 `MODE`、`STATE` 和 `NEXT/PREV/START/EXIT/GOTO`，接收 `EVENT`、`COMMAND_OK/FAILED`、`WS` 诊断。
- `/events`：WebSocket 不可用时的 SSE 回退；连接时先下发 `MODE default/full`，之后推送命令、心跳和每 5 秒一次 `STATE`。
- `/push`：SSE 回退时的状态/ACK 上报；WebSocket 模式不使用。
- `/poll`、`/config`：仅保留给旧版兼容和 SSE 失败后的诊断，新加载项不使用长轮询。
- 静态白名单：manifest.xml、index.html、main.js、js/bridge.js、ribbon.xml；未知资源返回 404。

`pos` 是放映位置，用于批注分页；`page` 是 Slide.SlideIndex，`total` 是 Slides.Count，不用显示页码替代放映导航位置。自定义放映的两种编号可能不同。旧加载项缺少新字段时保持基本翻页，不伪造总页数。

`ready` 表示支持当前页开始；START 调用 `CommandBars.ExecuteMso("SlideShowFromCurrent")`，存在 GetEnabledMso 时先核对。不可用时不退回从头 Run()。GOTO 带预选页与文稿标记 `deck`，执行前再次核对，避免旧拨轮跳转到新课件。

新脚本在 push/poll 上增加 `sid` 与递增 `seq`，同一页面重载脚本继续使用原流；服务端丢弃该流倒序/重复请求。API 对象被替换时重新注册监听，旧对象回调停止发布。未携带序号的旧客户端仍兼容，但没有这些顺序保证。

## 重试与边界

WebSocket 模式状态通过同一条连接上报；SSE 回退状态收到成功 HTTP 和 OK 才确认，失败/超时后重试最新快照。服务端命令队列最多 8 条、有效期 1.5 秒，断线、结束放映或白板切换清待执行命令。

HTTP 请求头最多 8 KiB、请求行最多 4 KiB，有读取时限、Host/Origin 和方法校验；只监听回环地址。**sid/seq 和回环地址不是身份认证**，多加载项竞争、完整放映会话及命令 ID/执行确认仍见 [S12](../docs/ISSUES.md#s12--http-会话与资源边界不足)。不要转发端口到外网。

## Windows 版登记

Windows 版由 Sidera 自己完成登记，不依赖 Python、bash 或 PowerShell：

- 登记位置：`%APPDATA%\kingsoft\wps\jsaddons\publish.xml`
- Sidera 启动时（WPS 联动开启）自动合并 `sidera-bridge` 条目；首次运行或加载项内容升级后写入。
- 设置中的“重新登记 WPS 加载项”会再次合并；关闭 WPS 后重新打开演示生效。
- 合并使用 Rust + quick-xml，保留其他加载项，清理旧名称 `screen-annotate-bridge` 和旧 URL 条目，修改前备份。
- ZIP 包内的 `wps-addin/` 提供 manifest、ribbon、JS 和 `publish.xml`；HTTP 桥从 `127.0.0.1:16666` 提供静态文件。
- 如果 WPS 没有加载，请确认所用版本支持 JS 加载项（WPS 2019 及以上），并在 WPS 设置中允许加载项；Windows 版不修改 `oem.ini`。

## 文件与卸载

加载项文件为 manifest.xml、ribbon.xml、main.js、js/bridge.js 和 index.html；publish.xml 是登记模板。登记位置为 `~/.local/share/Kingsoft/wps/jsaddons/publish.xml`。卸载时先关闭 WPS，只删除 sidera-bridge 条目，保留其他项目。

普通 DEB 不修改 WPS OEM 开关；仅在已明确诊断需要时使用仓库的 enable-jsapi.py，并保留其备份。

## 验证

在仓库根运行：

```sh
node --test wps-addin/tests/bridge.test.cjs wps-addin/tests/state-sync.test.cjs
python3 wps-addin/tests/test_install.py
python3 cpp/tests/test_bridge_api.py
python3 tools/test_rust.py wps::
```

Node 使用可控 Application、网络和定时器，不启动真实 WPS；安装器和桥检查在临时用户目录/私有网络执行。缺依赖或隔离失败即报错。最新结果见 [复查报告](../docs/audit/REVIEW_2026-10-24.md)。

历史 custom.4 在当前 VMware 上的基本联动曾得到用户“成功了”的确认，见 [历史记录](../docs/audit/WPS_INTEGRATION_2026-10-24.md)。本轮对请求边界和 API 生命周期的修改仍需实际 WPS 验收，不能把模拟通过写成所有 WPS 版本兼容。
