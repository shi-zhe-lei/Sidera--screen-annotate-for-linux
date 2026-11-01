# Sidera · Windows 版工作区

Windows 10 x64 起步。Windows 平台代码、构建脚本、验收 VM 和打包放在这里；Sidera 的核心逻辑与 Linux 版共享 `rust/`，不复制第二份。`cpp/`（Qt5 legacy）不受影响。

## 当前状态

- [x] Linux 宿主交叉编译工具链（Rust 1.93.1 + mingw-w64）
- [x] Win32 覆盖层：透明/置顶/局部点击穿透 + 共享 UI（Ctrl+Shift+Q 退出）
- [x] 共享核心拆分：`rust` 同时产出 `sidera-core` 库，Windows 直接依赖
- [x] Linux 134 项测试通过；Windows 核心 109 项测试在 Wine 下通过
- [x] `Win32Plat` 实现 `Backend`，接入 `ui::render_region` 与共享 `App`
- [x] 鼠标、WM_POINTER 触摸、热键、截图、单实例、系统主题、字体
- [x] 已通过 Wine 验证 Windows 字体与共享 UI 渲染
- [x] 托盘菜单（设置/切换/白板/显示/退出）与注册表自启动
- [x] WPS Windows 加载项注册（原生 XML 合并）
- [ ] Windows 进程扫描
- [ ] 多显示器 / 混合 DPI、真实触摸设备验收
- [ ] Win10 VM / 真机验收与安装包签名

## 环境

宿主：Linux x86_64（当前 CachyOS），已安装：

- `mingw-w64-gcc` 16.2.0（`sudo pacman -S --needed mingw-w64-gcc`）
- 项目内 Rust 1.93.1：`../.toolchains/rustup`
- target：`x86_64-pc-windows-gnu`

检查：

```sh
./check-env.sh
```

重新安装工具链：

```sh
./setup-toolchain.sh
```

## 调试渲染

在 Wine/Windows 下可导出共享 UI 的渲染结果，不需要真实桌面截图：

```sh
SIDERA_RENDER_PNG='Z:\tmp\sidera-ui.png' wine sidera.exe
SIDERA_DUMP_DIB='Z:\tmp\sidera-dib.png' wine sidera.exe   # 走 present/flush 后的 DIB
```

## 编译

```sh
./build.sh
```

产物：

```text
target/x86_64-pc-windows-gnu/release/sidera.exe
```

依赖 DLL 会打印在末尾。如果出现 `libgcc_s_seh-1.dll` / `libwinpthread-1.dll`，在 `Cargo.toml` 或 `.cargo/config.toml` 中加静态链接参数。

## Windows 10 验收

本机已有 `ghcr.io/dockur/windows:6.05`，直接启动 Win10 Pro：

```sh
./run-win10.sh
```

- 浏览器：<http://127.0.0.1:8006>
- RDP：`127.0.0.1:3389`，用户 `Docker`，密码 `admin`
- 本目录 `dist/` 映射为 Windows 的 `Z:` 盘（桌面 `Shared`）
- 编译后把 `sidera.exe` 放到 `dist/`，在 Windows 里运行 `Z:\sidera.exe`

> 首次启动会自动下载 Windows 10 ISO 并安装，需要较长时间。ISO 下载失败时，可手动下载后加 `-v /path/win10.iso:/custom.iso` 并去掉 `VERSION`。

## 打包

```sh
./package.sh
```

产物：`dist/sidera-windows-x64.zip`。

## 托盘与自启动

- 托盘图标使用 `Shell_NotifyIconW`，左键显示覆盖层，右键菜单：打开设置、切换批注/光标、白板、显示 Sidera、退出；双击打开设置。
- 自启动写入 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `Sidera` 值，设置面板的开关直接生效。
- 调试自启动：`SIDERA_AUTOSTART_TEST=1 wine sidera.exe`，结果写入 `%TEMP%\sidera-win-startup.log`。

## WPS 加载项注册

- 登记位置：`%APPDATA%\kingsoft\wps\jsaddons\publish.xml`（WPSJS 在 Windows 上的标准路径）。
- Sidera 启动时会检查加载项指纹；首次运行或升级后自动合并 `sidera-bridge` 条目。
- 设置里的“重新登记 WPS 加载项”会再次合并；关闭 WPS 后重新打开演示生效。
- 合并逻辑在 Rust/quick-xml 中完成，会保留其他加载项，清理旧名称和旧 URL 条目。
- 加载项本体随 ZIP 放在 `wps-addin/`，HTTP 桥从 `127.0.0.1:16666` 提供 manifest/ribbon/JS。

## Windows 覆盖层结构

- **视觉窗口**：分层、置顶、`WS_EX_TRANSPARENT`，始终点击穿透；用 `UpdateLayeredWindow` 逐像素上屏。
- **输入窗口**：另一个不可见的置顶分层窗口，用 `SetWindowRgn` 限定可点击区域。光标模式下区域是工具栏/弹窗矩形；画笔/白板模式下区域是整屏。
- 这样旧笔迹不会挡住桌面点击，切入画笔模式后又能整屏接收鼠标/触摸。
- 每 500ms 重新置顶一次，避免 WPS 放映窗口把覆盖层压下去。

## 与共享核心的关系

Windows 版不再复制一份 Sidera 逻辑，而是依赖 `rust/` 产出的 `sidera-core`：

```toml
sidera-core = { package = "sidera", path = "../rust" }
```

共享内容包括 `app`（状态/分页缓存）、`paint`、`ui`、`gesture`、`wps`、`page_cache`、`backend::Backend` trait 和事件路由。Windows 侧只需要实现 `Backend` 和平台模块。

## 目录

```text
windows/
├── Cargo.toml
├── src/                 # Windows 版源码（Win32 后端）
├── assets/              # 图标、manifest、安装器资源
├── build.sh             # 交叉编译
├── setup-toolchain.sh   # 安装项目内 Rust + Windows target
├── check-env.sh         # 环境检查
├── run-win10.sh         # 启动 Win10 Pro 验收 VM
└── package.sh           # 打包
```

## 移植路线

1. **Win32 覆盖层骨架**：分层窗口、Per-Monitor DPI、置顶、点击穿透、GDI/DIB 渲染。
2. **共享核心**：把 `rust/src` 中可移植的 `app`、`paint`、`ui`、`gesture`、`wps`、`page_cache` 抽成 core 库复用，避免复制 1.6 万行。
3. **输入**：鼠标、`WM_POINTER` 触摸、手掌触点尺寸、`SendInput` 翻页。
4. **WPS**：HTTP 桥不变；Windows JS 加载项注册用 PowerShell 脚本，在 Win10 VM 实测。
5. **平台功能**：托盘、单实例、热键、截图、剪贴板、主题、进程屏蔽、字体、持久化。
6. **打包**：manifest（PerMonitorV2、Win10 supportedOS）、图标、ZIP/NSIS。

## 注意

- 不要用 `pacman -S rustup`：会替换系统 `rust`。本项目工具链在 `../.toolchains/`，独立于系统。
- 容器不是必需的；Windows 运行时行为必须在 Win10 VM/真机验证。
- 当前 `src/main.rs` 只是 Win32 骨架，不是完整 Sidera。
