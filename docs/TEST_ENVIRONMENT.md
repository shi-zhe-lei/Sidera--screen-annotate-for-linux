# 测试环境与验收计划

更新：2026-10-25；分支 `custom`。当前用户授权执行隔离代码/协议检查，真实 VMware 桌面、WPS 和触屏由用户操作。这里的命令说明不等于每台机器已经通过，实际结果见 [旧问题复查](audit/REVIEW_2026-10-24.md) 和 [主题检查](DARK_MODE.md)。

## 环境与支持边界

| 环境 | 已知状态 | 结论范围 |
| --- | --- | --- |
| 本地 WSL Ubuntu 26.04 amd64 | Rust/Cargo 1.93.1、GCC 15.2、Qt 5.15.18、Python 3.14、bubblewrap、Xvfb、Sway 1.11/wlroots 0.19.2 | 编译、接口及私有图形协议检查；不等于真实桌面体验 |
| Windows / NAS 工作区 | PowerShell、Node 24.19；custom 分支 | 编辑、Node 模拟测试；Linux 构建使用本地文件系统副本 |
| VMware Ubuntu 26.04.1 amd64 | GNOME 50.1 Wayland、glibc 2.43；已覆盖安装含小屏幕设置修复、主题功能且保留白板底色的 custom.35，扩展文件版本 6 | 37 个安装文件校验通过、原有配置保留；2026-10-25 用户确认下列三组功能在此虚拟机上没有问题 |
| GitHub runner + Ubuntu 20.04 容器 amd64 / ARM64 | 3.2-Electro-stable~beta1 发布构建基线；Rust 1.93.1 | 发布工作流分别编译、检查主题与私有协议，再验证包内程序；实际结果见 Release 的 BUILDINFO |
| 学校银河麒麟 V10 SP1 | Phytium D2000/8 E8C、ARM64、UKUI 原生 X11、glibc 2.31、Qt 5.12；报告已装上游 2.6 | U 盘另备 custom.35 低 ABI 构建；QEMU + glibc 2.31 下 107 项普通 Rust、16 项启动检查通过，不是学校硬件验收；有图形窗口不代表 Wayland |
| Ubuntu 24.04 / GNOME 46–49 / KDE / 原生 ARM64 | 缺少完整当前版本验收 | 不预填为已支持或已通过 |

历史 VM 的 XWayland 穿透/不闪和 custom.4 WPS 联动曾得到用户确认；后续功能及本轮修改不能继承为“全部通过”。当前发行记录见 [custom.35](releases/2026-10-25-custom35.md)，历史 [custom.29](releases/2026-10-24-custom29.md) 另行保留。

custom.35 的双架构原生发布构建各通过 136 项隔离检查（107 Rust + 11 X11 + 2 Wayland + 16 启动），[发布工作流](https://github.com/shi-zhe-lei/Sidera--screen-annotate-for-linux/actions/runs/40124628783)和[常规 CI](https://github.com/shi-zhe-lei/Sidera--screen-annotate-for-linux/actions/runs/40124628700)均通过。学校离线安装器另有 31 项隔离检查及完整安装／自启接管／关闭／恢复流程检查；U 盘当前套件 51 个文件完成校验，并保留上一套资料。虚拟机仅覆盖安装与核对文件，旧进程未被结束。

## 小屏幕设置回归

custom.35 新增完整设置可达性、点击区域无重叠、页码边界与原生像素绘制检查。覆盖 320×240 至 1920×1080 的八组逻辑分辨率，两种橡皮触发方式、深浅主题，以及 100% / 150% / 200% 缩放。底部完成按钮始终可见；小屏幕通过上下按钮或滚轮换页。用户的虚拟机操作反馈见下节；学校实体触屏仍需单独验收。

## 2026-10-25 用户虚拟机验收

在 custom.35 部署后的正式版准备讨论中，用户选中以下三组测试内容并明确反馈：“在虚拟机上已经没有问题”。据此记录为用户人工验收通过；没有由开发工具重复操作 VMware 或 WPS。

| 用户确认的范围 | 结果 |
| --- | --- |
| 画笔、橡皮、撤回、点击穿透、工具栏拖拽 | 通过（用户反馈） |
| WPS 当页开始、翻页、跳页、结束，以及断线后恢复 | 通过（用户反馈） |
| 白板内容保留、小屏幕设置、深浅主题，以及所列触摸操作 | 通过（用户反馈，虚拟机范围） |

结论覆盖这台虚拟机此次实际使用的配置和启动入口；触摸反馈未附设备型号，不能外推为学校实体触屏、所有输入设备或全部桌面后端通过。没有把本次确认扩大为其他 GNOME 版本、Ubuntu 24.04、麒麟 ARM64 或多屏热插拔已通过。

正式版准备仍待：学校机器的离线安装、升级、自启动和原版恢复；实际课堂硬件验收；两小时课堂与八小时压力检查及资源记录。剩余开放问题按发布支持范围复核。本次反馈不改变预发布标记，也不将尚未执行的场景记为通过。

## 准备独立开发环境

以下为 Debian/Ubuntu 测试机依赖示例，使用该发行版自己的包源。在 Linux 本地文件系统构建，避免直接将 UNC 检出交给 Bash。

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config dpkg-dev binutils \
  qtbase5-dev libx11-dev libxcb1-dev libxext-dev libxtst-dev \
  libwayland-dev libwayland-bin libxkbcommon-dev \
  python3 bubblewrap xvfb xauth sway dbus-x11 fonts-noto-cjk
```

另外准备 Node.js 24 与 rustup。进入 rust/ 后，`rust-toolchain.toml` 选择 1.93.1；首次按配置安装工具链，随后 `cargo fetch --locked` 准备依赖。aarch64 交叉构建还需 `rustup target add --toolchain 1.93.1 aarch64-unknown-linux-gnu`。此版本是当前工具链，不是已验证的最低 Rust 版本。

字体回归单独绘制中文，不能用“英文有像素”代替中文覆盖。系统需有可被 ab_glyph 读取的中文字体，例如 Noto Sans CJK。

隔离工具使用只读系统、临时 HOME/XDG 目录、私有网络/PID 与运行目录；Wayland 检查另启动自己的 D-Bus、headless Sway 和软件渲染，不连接用户图形会话。缺依赖或隔离失败直接报错，不降级到真实桌面。

GitHub Actions 固定使用 Ubuntu 24.04，避免 `ubuntu-latest` 镜像迁移改变基线。三个任务在编译前运行 `.github/scripts/prepare-isolation.sh`：先检测网络/PID 隔离，需要时仅在临时 runner 内加载 `/usr/bin/bwrap` 的 AppArmor 配置，再验证嵌套隔离与私有回环连接。不关闭全局 AppArmor 限制，不以 root 运行测试，也不跳过失败用例。

该准备脚本拒绝在普通机器和自托管 runner 上执行。本地若出现 `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`，应由环境管理员按发行版规则检查应用配置；不要删除测试的 `--unshare-net`。背景见 [Ubuntu 24.04 用户命名空间限制](https://github.com/ubuntu/ubuntu-release-notes/blob/main/docs/24.04/index.md#unprivileged-user-namespace-restrictions) 和 [AppArmor 文档](https://documentation.ubuntu.com/security/security-features/privilege-restriction/apparmor/)。

## 可重复执行的代码检查

从仓库根目录执行。首次编译需要下载的依赖应提前获取；真正运行用例时网络隔离。

```sh
node --test wps-addin/tests/bridge.test.cjs \
  wps-addin/tests/state-sync.test.cjs compat/gnome-extension/tests/chrome.test.mjs
python3 wps-addin/tests/test_install.py
python3 rust/tests/test_build_deb.py
python3 cpp/tests/test_bridge_api.py
python3 tools/test_rust.py
python3 tools/test_rust.py --x11
python3 tools/test_rust.py --wayland
python3 cpp/tests/test_core_state.py
python3 cpp/tests/test_core_state.py --sanitize
python3 tools/check_markdown.py
```

| 入口 | 检查内容 | 限制 |
| --- | --- | --- |
| Node 测试 | 实际 JS、可控 Application/API/请求/定时器；GNOME 模块适配 | 不启动真实 WPS 或 Shell |
| test_install.py | 真实登记脚本、XML 合并、权限/异常/备份/失败保留 | 临时家目录、私有网络与独立 `/usr/share`；无需预装 Sidera |
| test_build_deb.py | 本机真实 ELF 打包、架构/格式拒绝、旧产物保留 | 不安装包，不证明 ARM64 运行 |
| test_bridge_api.py / test_bridge_lifecycle.py | 同一真实 C++ 桥的 8 项检查，包含 100 次启停 | 两入口等效，不重复计数；不再连接现有 16666 |
| test_rust.py | 构建 locked 测试后在隔离环境执行普通用例 | 需要图形协议的 13 项在这一步明确跳过 |
| --x11 | 私有 Xvfb 的 11 项输入区域/触摸协议检查 | 不验证真实触摸设备 |
| --wayland | 私有 Sway 的 2 项 wl_shm 存储检查 | release 时序由夹具控制 |
| test_core_state.py | C++ 45 页落盘、白板、busy 存储、尺寸/缩放、取消 | 真实私有 compositor；无用户输入注入 |
| --sanitize | 同上，启用 ASan/UBSan | 关闭 Qt 进程全局分配的 leak 检查，不代表长期零泄漏 |
| check_markdown.py | 当前源文档的相对链接、图片及内部锚点 | 不验证远程 URL 或外部服务可用性 |

## 构建与启动诊断

```sh
cd rust
cargo check --tests --examples --locked
cargo build --release --locked
cargo clippy --all-targets --locked
cd ..
python3 tools/run_isolated.py --timeout 180 -- \
  python3 rust/tests/test_startup.py rust/target/release/sidera
make -C cpp -j4 WITH_WAYLAND=0
make -C cpp -j4 WITH_WAYLAND=1
```

`cargo check` 只编译，不执行用例。Clippy 仍有风格和结构建议，不使用“零警告”口径。C++ 后端开关或编译参数变化会刷新配置依赖并重编对象；显式要求 Wayland 但缺依赖时必须失败。

启动套件使用自己的 Xvfb、协议夹具和临时用户目录；外层隔离使 WSLg 的显示 socket 也不可访问。指定 `CARGO_TARGET_DIR` 时，把示例路径替换为实际构建产物。

本轮只运行上述协议/状态检查，没有运行会驱动完整应用交互的 `test_wayland_native.py` 或 `test_mutter_passthrough.py`。它们保留供后续获得对应操作授权时使用；历史结果见 [兼容记录](audit/COMPATIBILITY_2026-10-24.md)。

## 打包与构建输入

`rust/build_deb.sh amd64|arm64` 按 `target/container-<arch>/<triple>/release/sidera` → `target/container-<arch>/release/sidera` → `target/release/sidera` 的顺序取二进制。`build_aarch64.sh` 的交叉产物在 `target/container-aarch64/aarch64-unknown-linux-gnu/release/`，必须先查这一层；否则会命中早期构建残留的 `target/container-aarch64/release/sidera`，得到“日期新、内容旧”的包。务必确认被选中的是本次构建，尤其使用独立 CARGO_TARGET_DIR 时需将待打包文件放到脚本约定目录。

三个 DEB 脚本均调用 `tools/deb_dependencies.py`，用 readelf 检查 ELF，再由 dpkg-shlibdeps 计算实际动态依赖，另加登记需要的 Python 3。二进制无效时停止打包；主机缺少 dpkg 库清单（Arch/Fedora 等）时退回固定依赖并打印警告，不能当作已验证的依赖声明。通过 `SIDERA_VERSION` 指定本地包版本，失败不得删除旧包，成功后原子替换输出。

包的最低运行基线由**构建环境的 glibc** 决定，不是由打包脚本决定。发布流程在 `ubuntu:20.04` 容器内编译（`ubuntu-22.04` runner 只负责测试），`tools/release_metadata.py` 再从实际 ELF 读出最高 `GLIBC_x.y`，超过 `MAX_GLIBC`（默认 `2.31`）直接失败。不要把 22.04 / 24.04 主机上的原生编译产物当作发布包。

```sh
cd rust
SIDERA_VERSION=3.2-Electro-stable+local1 bash build_deb.sh amd64
dpkg-deb -f ../sidera_3.2-Electro-stable+local1_amd64.deb Architecture Depends
```

外部 `amd64-sysroot/`、`aarch64-sysroot/` 不在仓库。现有容器脚本不负责创建它们；缺少来源、版本和校验和时不能声称可复现。ARM64 现在由 amd64 容器交叉编译：需要 x86_64 1.93.1 工具链的 `aarch64-unknown-linux-gnu` 标准库，并用 `aarch64-sysroot` 提供目标 crt/libc；不再需要 QEMU/binfmt 或 aarch64 宿主工具链。低 ABI 仍以实际链接的目标库为准，并需到目标机验收，不能仅降低 control 中的 libc 要求。

已有 Release 的 [ARM64 补充构建](releases/ARM64.md) 使用可选 Ubuntu 22.04 / 24.04 的原生 ARM64 runner，分别记录应用 tag 与打包工具提交，并运行隔离启动检查。每次结果以其构建日志和 `BUILDINFO-arm64.md` 为准，不能继承 amd64 的桌面验收结论。

## 人工验收矩阵

所有真实桌面操作先保存用户文档。当前由用户执行，不自动注销、重启或关闭应用。

| 组 | 场景 | 通过标准 |
| --- | --- | --- |
| G01 / G02 | 画笔、橡皮、撤回、不同 DPI、边缘 | 笔迹连贯；撤回恢复对应像素 |
| 主题 | 三档即时切换、重启、系统偏好变化、多显示器、白板白色／绿板 | 所有界面和动画一致，字色清晰，白板底色、内容与页码不变；不出现共享桌面授权 |
| G03 | 画线→光标→操作底层窗口；工具面板开关 | 穿透正常、无闪烁和残片 |
| G04 | 单指、多指、大触点、静止按住、Cancel | 无假松手、重复鼠标笔迹或拖动残留 |
| G05 / G09 | 白板进出、多页；白板中外部换页/结束/新课件 | 白板保留，退出恢复最新透明页 |
| G06 / G10 | 第 N 页开始、页内动画、重复事件、拨轮跳页 | 真换页才切笔迹，不从头误放映 |
| G07 | 50 个有笔迹页、空页、反复往返 | 超过 20 页不丢；清空页不复活 |
| G08 | 桥/WPS 重启、离线后同页或不同页恢复 | 无过期命令、错误清空或串页 |
| G11 / G12 | 1080p/4K、混合 DPI、旋转、双屏、热插拔 | 画布/缓存/输入区一致，符合该后端支持范围 |
| G13 / G14 | 截图、取消、超时、粘贴到 WPS/图片应用 | PNG 正确；粘贴符合实际剪贴板能力 |
| G15 | 快捷键、翻页、Esc、右键与不可用状态 | 不弹共享桌面授权；失败反馈与能力一致 |
| G16 / G17 | 配置、恢复默认、只读目录、多个加载项 | 保存失败可观察，不损坏其他登记 |
| G18 / G19 | 错误/慢 HTTP、乱序、并发启动与残留 socket | 边界明确，不污染正常会话 |
| G20 | 安装/升级/两版替换/卸载、自启和权限 | 包与架构/依赖一致，无残留误引用 |
| G21 | 两小时课堂、八小时压力、系统校时 | 无崩溃/丢页；CPU、RSS、延迟有记录 |
| G22 | 应用菜单、收藏、任务栏和内置图标 | 正确显示并能启动，见下节 |

课堂验收另覆盖：绘图组默认收起、画笔展开、空页光标收回、有笔迹页展开、白板清除不收回、整条手动收起保持不弹出；工具栏贴顶/贴底、离开控件仍拖动、释放即停止。

### G22 安装后图标专项

使用实际安装包及可还原的目标桌面，记录版本、架构、包 SHA256 和缺失位置。检查包内 SVG/PNG、desktop 引用、文件权限及资源能否解码，再在源码目录外、不设 SIDERA_ICON 的用户会话中检查菜单、搜索、收藏、设置和启动页。

覆盖全新安装、升级、卸载重装。没有任务栏项的透明覆盖层记为不适用；不能靠手工复制资源或刷新缓存后才记为安装通过。Rust 图标路径已随内容变化，C++ 与 ARM64 的对应桌面显示结果仍需单独记录。

## 性能目标与结果记录

以下是待目标机器校准的目标，不是当前测量结果：空闲 CPU 平均低于单核 3%，60Hz 本地笔迹 P95 ≤ 33ms，WPS 命令 P95 ≤ 500ms。记录 1080p/4K、21/50 页、12 次撤回、白板和双屏的峰值 RSS，以及 2 小时课堂/8 小时压力表现。

```text
日期/测试人：
分支/提交/未提交改动：
实现/版本/包 SHA256：
发行版/内核/架构/桌面/会话：
屏幕/缩放/触控型号：
WPS/加载项/Portal/剪贴板/uinput：
用例 ID、预期、实际：
PASS / FAIL / 未执行 / 不支持：
证据与 CPU/RSS/延迟：
```

CI 已配置 custom/main 的 push/PR 检查；逐次结果见 [Build & Verify](https://github.com/shi-zhe-lei/Sidera--screen-annotate-for-linux/actions/workflows/ci.yml)。`171f100` 的三个任务曾因 runner 的 AppArmor 策略在启动隔离环境时失败，不能计为远程测试通过；后续提交以其自身运行结果为准。双架构发布工作流补充原生 amd64 / ARM64 Ubuntu 22.04 的编译和隔离检查，但真实安装与最低系统矩阵仍需目标机器和适用场景的证据；“未执行”不能改写成“通过”。
