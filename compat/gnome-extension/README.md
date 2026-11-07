# Sidera GNOME 原生扩展

此扩展与当前 `custom` 分支的 Rust 版配合，为 GNOME Wayland 提供原生覆盖层。批注在 GNOME Shell 的 Clutter actor 中显示；不创建 X11 窗口，不依赖 XWayland 的输入区域刷新。已接入 WPS 放映时使用加载项 API，未接入时使用 Shell 虚拟键盘，不请求 RemoteDesktop / 共享桌面授权。

当前已完成编译、语法、历史安装核对及隔离模块测试；**新版 GNOME 的完整桌面交互仍待用户验收**。元数据列出 GNOME 46–50，接口兼容按这些版本的上游源码处理；不能把版本声明或模拟测试当作逐版本实测通过。最新检查范围见 [复查报告](../../docs/audit/REVIEW_2026-10-24.md)。

扩展版本 2 修复 GNOME 50 移除 `Meta.is_wayland_compositor()` 后的加载错误；46–49 保留原生会话检查。升级已加载或已报错的扩展后，需要注销并重新登录才能加载新模块。

扩展版本 3 修复 GNOME 50 创建覆盖层时拒绝已移除的 `affectsInputRegion` 参数的问题；46–49 仍传入该参数，所有版本均由 actor 的 `reactive` 属性控制原生 Wayland 点击。创建失败会清理已添加的 actor，并将具体错误回传给应用。`ACTIVE` 和桥 socket 存在仅代表扩展已加载，不能替代应用启动与操作验收。详细证据见 [启动修复记录](../../docs/audit/GNOME_STARTUP_2026-10-24.md)。

扩展版本 4 根据真实鼠标移动事件携带的按键状态，清除已经松开却残留的按下记录，并取消对应客户端拖动/点击；不根据闲置时间猜测松手，不触发补偿点击。Rust 同时限制只有左键能开始工具栏拖动，正在拖动时其他按下不能覆盖其归属。详见 [拖动释放记录](../../docs/audit/DRAG_RELEASE_2026-10-24.md)。

扩展版本 7 修复原位输入的字符转换：GJS 的 `get_key_unicode()` 返回单字符字符串，旧逻辑错误地把数字字符再次当作编码转换，导致 `0`、小数点和 HEX 字母丢失，其余数字变成控制字符。现在保留字符本身，兼容数值码点返回值，并为缺少 Unicode 值的小键盘数字和小数点补充映射。界面继续使用原位输入，不增加输入框或弹窗。

`get_key_unicode()` 的返回类型为 [`gunichar`](https://gnome.pages.gitlab.gnome.org/mutter/clutter/method.Event.get_key_unicode.html)。本次在虚拟机中仅查询同类型的 `GLib.utf8_get_char('1')`，结果为字符串 `"1"`，未操作桌面或安装更新。字符转换的 5 项隔离回归检查通过；可在仓库根目录使用 `node --test compat/gnome-extension/tests/keyboard.test.mjs` 运行。实际界面输入仍需应用更新后确认。

## 安装与切换

扩展版本 6 在鼠标按住输入控件时使用 Clutter stage grab，持续接收工具栏外及屏幕顶部的移动和松开。正常释放、真实按键状态恢复、其他 Shell 抓取打断、锁屏、连接断开和扩展停用均释放抓取；不使用等待超时来判断松手，不改变键盘焦点或桌面的模态计数。扩展本身不限制坐标，屏幕边界由 Rust 的工具栏布局与拖动共同处理。依据：[Stage.grab](https://mutter.gnome.org/clutter/method.Stage.grab.html)、[Mutter 50.1 grab 生命周期](https://github.com/GNOME/mutter/blob/50.1/clutter/clutter/clutter-stage.c)、[Mutter 46 grab 撤销状态](https://github.com/GNOME/mutter/blob/46.0/clutter/clutter/clutter-grab.c)。隔离模块检查没有覆盖这项抓取交互，实际桌面操作仍由用户验收。

安装完整 Rust DEB 后，在登录桌面的普通用户终端执行：

```sh
sidera-gnome-install
```

源码目录也可运行 `python3 compat/gnome-extension/install.py`。安装器复制扩展到用户目录、编译设置 schema、备份旧扩展，并在保留其他扩展的前提下登记启用。不会注销、重启 Shell 或运行 Sidera。首次安装或更新已加载的扩展后，保存内容并注销、重新登录。

- 应用菜单 **Sidera（GNOME 原生）**：明确选择扩展；扩展未加载时报告启动失败。
- 应用菜单 **Sidera（兼容模式）**：明确使用 X11/XWayland。
- 普通 **Sidera**：GNOME Wayland 中优先使用已加载的扩展；没有扩展时按原有规则尝试 layer-shell / XWayland。
- 切换前先退出当前 Sidera。两个入口共用设置、WPS 和单实例锁；重复启动不会创建第二层覆盖窗口。
- `sidera-gnome-install --disable` 禁用扩展，原生覆盖层随之释放。随后可打开兼容模式。

`Ctrl+Shift+D` 在画笔/光标间切换。`Ctrl+Shift+Escape` 紧急断开原生客户端并立即释放全部点击区域；此时进程退出，未保存批注不持久化。日常退出仍使用 Sidera 设置里的退出按钮。

## 实现与边界

| 功能 | 路径与限制 |
| --- | --- |
| 绘图与穿透 | Rust 软件绘制 256px 脏块，Shell 更新可见 actor；透明输入 actor 独立，按下期间保留到释放，切模式不重映射显示层 |
| 多屏 | 每个逻辑屏独立画布、页缓存；共享工具、白板页码和单个 WPS 桥；最多 16 屏，暂存最近 2 个断开屏幕 |
| 缩放/分辨率 | 按相对位置重采样笔迹；返回旧页时适配当前尺寸；显示几何改变会清空基于旧物理像素的撤销记录 |
| 触摸 | Begin/Update/End/Cancel；GNOME 不提供本桥所需的接触直径，不能保证基于物理面积的掌擦，保留多指识别 |
| 截图 | 用户点击后调用 Shell Screenshot，PNG 写到图片目录，并由 Shell 持有原生剪贴板；不申请 RemoteDesktop |
| 锁屏/概览 | 覆盖层隐藏并取消当前输入；锁屏期间不接受绘图、截图或按键，解锁后恢复现有画布 |
| 普通翻页键/右键 | 仅白名单 Up/Down/Escape 与右键；右键在物理释放、输入区更新后转发；WPS 联动开启时始终优先 API |
| 故障恢复 | 连接关闭、扩展停用或 5 秒没有客户端消息时移除全部覆盖 actor；不尝试自动重启桌面 |

其他桌面的原生路径仍为 layer-shell。KDE 等桌面可用的虚拟键盘、虚拟指针、截图 Portal、剪贴板 data-control 各有差异，缺少协议不自动提权或请求远程桌面授权。全局切换也可由桌面快捷键绑定 `sidera --toggle`；独立 Sway 测试桌面已附此绑定。

## 本地协议 v1

地址 `$XDG_RUNTIME_DIR/sidera-gnome/bridge.sock`。目录 `0700`、socket `0600`；两端检查同一 UID。单客户端，不使用 HTTP、动态代码或任意 Shell 命令。客户端读写、输入队列、截图并发和帧尺寸均有上限。

每包：4 字节大端长度（包括类型字节）、1 字节类型、负载。控制消息类型 1 为 UTF-8 JSON，最多 64 KiB；类型 2 为像素块，单包最多 1 MiB。像素块头为 8 个大端 u32：`generation, output index, full width, full height, x, y, width, height`；随后为预乘 RGBA8888。块起点按 256 对齐，边界块按剩余尺寸截断。

扩展先发送 `hello(protocol=1, generation, monitors, capabilities)`。Rust 每屏发送 `begin`、像素块、`end`，再提交 `input` 点击矩形；扩展只在完整帧结束时更新图像。显示配置改变会增加 generation；两端丢弃旧版本的输入/渲染命令。`ping/state` 维持连接并更新当前全屏输出。剩余控制仅为白名单截图、按键及右键。输入事件包含逻辑屏 ID 和局部逻辑坐标。

## 用户验收

1. 画线 → 点击箭头 → 操作桌面/WPS；确认不闪、笔迹保留、按下和松开都能传递。
2. WPS 待放映时绿色“开始放映”，放映后“结束放映”；未接入时为“Esc”并用模拟按键翻页。缺少直接输入能力时上下键变橙；不弹共享桌面授权。
3. 截图后粘贴到 WPS 和图片应用，确认包含原生 Wayland 窗口。
4. 分数缩放、改变分辨率、返回有笔迹的旧页；多屏拖动工具栏、拔插投影仪后恢复。
5. 长按 3 秒、双指/手掌、设备移除；锁屏/解锁与概览后恢复穿透。
6. 最后再验证紧急快捷键和扩展停用；未保存批注会随程序退出丢失。

启动错误保存在 `~/.local/state/sidera/startup-error.log`。Shell 扩展日志可用 `journalctl --user -b -g Sidera` 查看。系统诊断显示实际后端及按键、右键、快捷键能力。

参考：[GNOME session modes](https://gjs.guide/extensions/topics/session-modes.html)、[GNOME 46 Screenshot 实现](https://github.com/GNOME/gnome-shell/blob/gnome-46/js/ui/screenshot.js)、[GNOME 48 Screenshot 实现](https://github.com/GNOME/gnome-shell/blob/gnome-48/js/ui/screenshot.js)、[Shell Screenshot API](https://gnome.pages.gitlab.gnome.org/gnome-shell/shell/method.Screenshot.screenshot.html)。
