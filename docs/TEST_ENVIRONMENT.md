# 测试环境与验收计划

日期：2026-09-30；分支 `custom`；基线 `0642103`。本文件分别记录**已经使用的环境**与**后续需要建立的环境**。后文命令是配置/验证说明，并不表示本轮已安装全部依赖或跑过全部测试。

## 1. 初次评审环境与后续修复验证

| 项目 | 实际值 |
| --- | --- |
| 宿主 | Windows / PowerShell，项目位于 NAS UNC 共享 |
| Linux | WSL2，Ubuntu 26.04.1 LTS，amd64 |
| 内核 | `6.18.33.2-microsoft-standard-WSL2` |
| C++ | GCC 15.2.0；Qt5Core/Widgets/Network 5.15.18 |
| Python / 隔离 | Python 3.14.4；bubblewrap 0.11.1 |
| JavaScript | Windows Node.js 24.19.0 |
| X11测试辅助 | Xvfb/xvfb-run 已安装，本轮未进行图形交互测试 |
| 缺失 | Cargo/Rust、Wayland开发文件/wayland-scanner/xkbcommon开发文件、arm64环境、真实WPS/触摸测试场景 |
| 初次评审副本 | WSL 本地 `/var/tmp/sidera-audit-4193fe8f`；初次评审未改生产源 |

初次评审已完成：

- C++ X11 编译通过；Qt 旧API弃用警告存在；Wayland 未编入。
- 原始独立 C++ WpsBridge 的 HTTP/回调/启停探针运行；用 bubblewrap 隔离家目录和 loopback 网络。
- 原 Python 接口套件在独立桥上报告5/5，但缺 manifest 的情况下也5/5，此结果不能作为资源验证通过。
- Node 检查两个生产 JS 文件语法通过；VM 探针记录重复监听、状态漏报、不重试和离线仍显示在线。
- 10 个 XML 可解析；10 个 Shell 脚本在 LF 规范化副本中语法通过。
- C++ 本轮 ELF 最高 GLIBC 符号2.34，并含 Qt5Network 与 Qt_5.15依赖；不是旧系统发行产物。

初次评审没有执行：Rust build/test/clippy、完整 Wayland 构建、真实桌面的绘图/穿透/截图、WPS联动实机测试、触控、arm64、DEB构建/安装、性能压测。后续验证补充如下，其余项目仍不提供通过结论。

### S14 架构保护回归（2026-09-30）

- 在同一 WSL Ubuntu 26.04 amd64 环境运行 `python3 rust/tests/test_build_deb.py`，5 组、17 个场景全部通过；旧脚本的 11 个拒绝场景失败，证明确实覆盖原缺陷。
- 测试在临时目录运行实际打包脚本与 dpkg-deb，覆盖容器/native release、amd64/arm64/aarch64 别名、EXEC/DYN、错误架构、非 ELF/截断头、32 位、大端、目标文件缺失、旧包与暂存目录保留；正确包解包后核对 control、二进制和执行权限。
- 依赖 Linux、Python 3、bash、binutils、dpkg-deb 和常规 coreutils；无需 Rust 工具链、root 或安装软件包。测试已加入现有 Rust CI job，CI 触发范围仍保持原状。
- ELF 夹具仅携带测试架构元数据。通过结论限于打包保护；真实程序启动、arm64 原生执行、最低 ABI 和安装后图标均未验证。

## 2. 后续环境分层

| 层 | 建议环境 | 用途 | 不能替代的验收 |
| --- | --- | --- | --- |
| E0：逻辑/协议 | 固定Linux amd64镜像；Rust、GCC、Qt、Python、Node | core、图像、手势、HTTP、JS、XML、配置测试 | 真实输入和合成器行为 |
| E1：X11冒烟 | E0 + Xvfb；另有含窗口管理器/合成器的X11 VM | 启动、退出、注入、输入区域、截图基线 | Xvfb单独不能证明透明叠加和真触摸 |
| E2：Wayland | 可公告layer-shell的真实合成器；建议至少Sway与实际教室合成器 | buffer、resize、fractional scale、seat、Portal | 嵌套/无头环境不能证明外设延迟 |
| E3：最低系统/包 | README目标旧系统、Qt5.12/glibc基线或真实国产发行版 | ABI、安装、依赖、自启动、升级 | 现代Ubuntu编译成功不证明这里可运行 |
| E4：原生arm64 | 实际arm64 Linux机器与触屏 | 架构、输入、内存、WPS | QEMU仅用于编译/接口冒烟 |
| E5：课堂验收 | 真实教学电脑、投影/双屏、课件和WPS | 完整授课、长时间运行和恢复 | 无WPS模拟不能证明加载项兼容 |

amd64基准可以采用统一的 Ubuntu 24.04 开发VM，保持镜像和依赖版本可追溯；旧系统与实际教室发行版另外测试。具体发行版/合成器/WPS版本尚未提供，不预填“已支持”。

Wayland场景按公告协议区分：有layer-shell且有虚拟键盘；有layer-shell但无虚拟键盘/uinput不可用；缺layer-shell；有/无Portal、XWayland和剪贴板管理器。每种路径都应检查真实反馈，不能把缺协议场景标成正常可用。

## 3. Linux 开发环境配置示例

在专用测试VM/用户中运行；先在Linux本地文件系统检出custom，避免UNC路径转换和Windows CRLF问题。现有项目修改仍在custom，测试副本只用于编译/运行。

Debian/Ubuntu示例依赖（发行版包名可能随版本变化，失败时查该系统仓库）：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config qtbase5-dev \
  libx11-dev libxcb1-dev libxext-dev libxtst-dev \
  libwayland-dev libwayland-bin libxkbcommon-dev \
  python3 nodejs xvfb xauth bubblewrap
```

另准备可被当前字体解析器加载、覆盖中文的字体，记录实际文件名；不能仅因系统装了字体包就认定中文界面可用。Portal、目标桌面的截图后端和WPS在图形测试机上配置，按发行版提供的包和桌面设置验收。

Rust：使用已有rustup，安装候选toolchain和fmt/clippy，首轮验证后固定版本，避免长期使用未固定stable：

```bash
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustc -Vv
cargo -V
```

这里没有指定未经测试的MSRV；锁定依赖含 `image 0.25.10`，应以真实依赖的编译要求和验证结果确定最小版本，再新增 `rust-toolchain.toml` 和 `rust-version`。

记录环境：

```bash
uname -a
cat /etc/os-release
g++ --version
pkg-config --modversion Qt5Core Qt5Network wayland-client xkbcommon
wayland-scanner --version
python3 --version
node --version
git rev-parse HEAD
git status --short
```

构建rootfs还需要独立记录来源、版本、校验和、bwrap/qemu/binfmt和目标Rust工具链；当前仓库没有提供自动初始化步骤，不能把执行已有脚本当成环境已经完整。

## 4. 当前可运行检查

### 本轮诊断探针

从仓库根运行：

```bash
node --check wps-addin/main.js
node --check wps-addin/js/bridge.js
node docs/audit/check_addin.cjs
python3 docs/audit/check_cpp_bridge.py
```

最后一条要求Linux、GCC、Qt5Core/Network、pkg-config和bubblewrap；会临时编译原始桥，在独立网络及映射的测试家目录里运行。它不要求先启动Sidera或WPS，不会使用用户当前会话的16666端口。详情及预期诊断输出见 [audit/README.md](audit/README.md)。

这些探针为**现状记录**：退出0表示探针顺利完成，不表示已修复缺陷。重构时应把观察结果改成正式回归断言，异常时非零退出。

### C++ 编译

```bash
cd cpp
make clean
make -j4
```

安装Wayland依赖后，检查编译命令确实出现 `-DSIDERA_HAVE_WAYLAND`，并链接 wayland-client/xkbcommon；仅存在annotate_amd64文件不证明Wayland分支编译。改后端配置必须clean，避免旧对象文件混入。

可在测试副本中开启检查构建：

```bash
make clean
make -j4 CXXFLAGS='-std=c++17 -O0 -g -fPIC -Wall -Wextra'
```

ASan/UBSan需要编译和最终链接都启用；当前Makefile链接行不使用CXXFLAGS，所以仅把sanitizer放入CXXFLAGS是不完整的。阶段0/1应先支持独立LDFLAGS/测试构建目标，再记录通过结果。

### Rust 构建和测试（工具链配置后）

```bash
cd rust
cargo fmt --all -- --check
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

目前桥测试会写真实登记和使用固定端口；在完成S13前仅在专用测试用户/隔离家目录中运行，不能直接在日常WPS用户上把cargo test当无副作用检查。Cargo的测试运行规则见 [Cargo官方文档](https://doc.rust-lang.org/cargo/commands/cargo-test.html)。严格clippy门禁是拟新增要求，当前CI没有 `-D warnings`。

X11冒烟（Rust编译后，在隔离测试用户中）：

```bash
env -u WAYLAND_DISPLAY XDG_SESSION_TYPE=x11 WPS_API_DEBUG=0 \
  SIDERA_SMOKE=1 SIDERA_NO_SPLASH=1 \
  xvfb-run -a -s '-screen 0 1920x1080x24' ./target/release/sidera
```

仅验证初始化/一帧渲染/退出。C++当前没有同等退出式smoke参数，应在阶段0补受控启动/停止测试，不建议用无限挂起的GUI进程替代冒烟测试。

### 现有 Python 端点测试的使用限制

`cpp/tests/test_bridge_api.py` 默认要求已运行服务，还会push SlideShowBegin，可能清空该实例笔迹；只能对测试实例运行。`--mock` 只启动替代服务，不是在验证Sidera本身；缺manifest时会误通过，修复前结果只用于辅助诊断。

## 5. 功能与故障用例矩阵

| 组 | 用例 | 必须观察/断言 | 关联问题 |
| --- | --- | --- | --- |
| G01绘图 | 点、直线、曲线、快速长线，10色/3档，边缘 | 可见连贯、透明正确、实际交互渲染路径 | S01/S02/S25 |
| G02撤回 | 1/12/13次；擦除/手掌；跨128px瓦片 | 恢复前后像素相等，步骤语义明确 | S02/S05 |
| G03穿透 | 光标/笔/橡皮/白板；左右侧栏/弹窗/设置 | 只允许对应输入区域，不吞底层输入 | S18/S21 |
| G04触摸 | 单指、双指、手背、shape晚到、静止3秒、Cancel | 角色锁存/恢复，无重复鼠标笔迹、无卡死 | S20 |
| G05白板 | 1→10→1、上页、背景、进出 | 白板与课件隔离、上限提示一致 | S05/S08 |
| G06课件页 | 页内多动画、上一页、直接跳页、第5页开始 | 真换页才保存/加载；动画不清墨迹 | S06/S07 |
| G07容量 | 21/50页有墨迹、空页、返回旧页、两文稿 | 预算、保留规则、身份隔离正确 | S05 |
| G08重连 | 断3秒、同页恢复、桥重启、WPS重启 | 完整快照，旧页正确保存，无旧命令执行 | S07/S10/S12 |
| G09白板外部 | 白板中换页、End、新Begin，再退出 | 恢复最新外部会话，不清白板 | S08 |
| G10生命周期 | 重复Begin/End/JS onLoad、End同页再开始 | 幂等、监听数量稳定 | S07/S09/S10 |
| G11显示 | 1080p/4K，100/125/150/200%，旋转/热插拔 | canvas/缓存/撤回/输入区几何一致 | S01—S04/S19/S21 |
| G12双屏 | 一屏放映/另一屏操作、混合DPI、主屏切换 | 覆盖和坐标按声明支持，不误清页 | S06/S19 |
| G13截图 | X11/Portal、取消、超时、连点、空格URI | PNG大小/颜色正确，任务结束，反馈准确 | S16/S17 |
| G14剪贴板 | 无管理器、有管理器、纯Wayland/XWayland | 保存后5秒粘贴到WPS/图像软件成功 | S16 |
| G15注入/热键 | Ctrl+Shift+D与锁定键、NEXT/PREV/Esc/右键 | 成功反馈真实；不可用时不假称成功 | S18/S19 |
| G16配置 | 损坏、NaN、越界、只读、启动/保存 | 默认值安全、原子写、失败可见 | S24 |
| G17登记 | 其他加载项/紧凑XML/旧名称/损坏/无权限 | 只改变本项目节点，不损坏原文件 | S11 |
| G18接口 | 错方法/Host/Origin、缺资源、慢/长请求、乱序 | 错误码、大小边界、会话校验、正常请求不阻塞 | S12/S13 |
| G19启停 | 桥100次、应用并发启动、socket残留 | 单实例可靠、timer/线程/fd无持续增长 | S15/S24 |
| G20包 | 安装/升级/两版替换/卸载、自启、udev | 依赖、架构、desktop、权限与文档一致 | S14/S22/S23 |
| G21长课 | 2小时连续授课/8小时压力、时间调整 | 无崩溃/失墨，资源稳定，超时不受校时影响 | S25 |
| G22图标 | 实际包全新安装/升级/卸载重装；菜单、收藏、适用的任务栏和程序内部 | 图标可见且正确，入口可启动，不依赖源码目录、环境变量或手工刷新缓存 | S28 |

E0运行状态与图像断言，E1/E2运行图形子集；E4/E5完成外设/WPS用例。所有宣称支持的运行路径至少覆盖G01—G22中适用项，未支持项需明确说明，不能记为通过。

### G22 安装后图标专项

使用目标 Linux 桌面上的专用测试用户或可还原虚拟机，覆盖 C++/Rust、amd64/arm64 各自声明支持的发布组合。WSL 中能编译或解包，不代表应用菜单和任务栏显示已通过。

1. 记录发行版、桌面/合成器、X11/Wayland 会话、包版本与校验和，以及图标缺失的具体位置；保留故障截图。
2. 检查实际 DEB 的 desktop 与 SVG/PNG 文件清单、权限及安装路径，验证 desktop 字段和图像可解码；核对文件内容，不能只检查路径存在。
3. 在源码目录以外、未设置 `SIDERA_ICON` 的干净用户环境中，从应用菜单启动，检查菜单、搜索结果、收藏入口、设置和启动画面。任务栏仅验证本来应显示的窗口，并检查分组关联；透明覆盖层没有任务栏项不直接判失败。
4. 重复覆盖升级与卸载重装，再次检查入口和图标。核对桌面自身的刷新时机及软件包触发器，不能靠测试人员手工复制资源或刷新缓存后才记为通过。
5. 保存安装/升级记录与结果截图。静态包检查可进入 CI；桌面显示验收需要实际图形会话，未执行的组合明确记为待验证。

## 6. 测试夹具与证据

准备并版本化最小课件：

- 3页，第一页多个页内动画，第二页无动画，第三页测试回退。
- 30/50页，便于每页画唯一编号，检查缓存超过20页后的行为。
- 两份课件包含相同页码，检查文稿身份隔离。
- 从非首页开始、直接跳页、白板中结束/重新开始场景。

建立共享协议fixture：输入事件序列、预期会话/页号、save/load/clear动作、命令与确认。加入断线、重复、乱序、心跳、白板开关和显示变化；两种实现跑相同fixture。

图像测试保存操作前后PNG/像素哈希，撤回采用像素精确比较；字体/UI渲染按固定字体和尺寸对照，避免字体环境差异导致误判。交互测试记录短视频/截图、日志、软件版本及设备信息，不能只写“手工通过”。

## 7. 性能与稳定性目标

以下是待校准的初始验收目标，**不是本轮测量值或当前性能承诺**。低配置教室机应先跑基线，再共同确定最终预算。

| 指标 | 初始目标/方法 |
| --- | --- |
| 空闲CPU | 指定参考机下平均低于单核3%；分别测窗口扫描/无WPS/有WPS |
| 本地笔迹延迟 | 从输入到画面，P95≤33ms（60Hz）；分别测1080p与4K |
| WPS命令 | 从按钮到加载项执行，记录P50/P95；先以P95≤500ms为目标 |
| 内存 | 记录空闲/21页/50页/12次撤回/白板的峰值RSS；预算由参考机确定，超预算行为可解释 |
| 资源泄漏 | 100次桥启停、截图、模式切换后，线程/fd/timer不持续增长 |
| 稳定性 | 2小时课堂流程、8小时压力无崩溃；笔迹/页缓存正确；错误可恢复 |

可用 `/usr/bin/time -v`、`pidstat -r -u`、`/proc/<pid>/fd` 和记录的帧/事件时间辅助测量。不要把整屏32bit内存的计算值写成实测RSS，也不要用QEMU延迟代替原生arm64性能。

## 8. CI 与发布门禁建议

| 触发/层级 | 拟要求 |
| --- | --- |
| custom/main push与PR | fmt、Rust test/clippy、C++两种后端编译、JS回归、XML/脚本、协议fixture |
| 构建产物 | ELF架构、动态依赖、最大GLIBC/GLIBCXX/Qt符号、静态资源、包清单 |
| 定期/发布候选 | arm64原生测试、最低系统包安装、Wayland图形/热插拔、长时间压力 |
| 教室发布 | 实际WPS版本和触控型号，授课用例矩阵，已知限制与回退包 |

CI缓存按工具链/架构/lockfile/backend配置区分；Wayland条件未满足必须显式失败或分为X11构建任务，不能悄悄退化而声称全平台通过。

发布包的动态依赖从产物生成，架构/ABI与control一致。包安装时在专用VM执行，不在日常教室账户上运行会清空会话的探针。当前C++和Rust共用sidera包名，测试两版互相替换时须记录版本排序和残留文件行为。

## 9. 实机测试记录模板

```text
测试日期/人员：
Git分支/commit/工作区是否干净：
实现/版本/包SHA256：
发行版/内核/架构：
桌面/合成器/会话类型/公告协议：
CPU/RAM/输出分辨率/刷新率/DPI：
触控型号/固件/XInput或libinput信息：
WPS完整版本/加载项状态：
Portal/剪贴板管理器/uinput状态：
课件fixture与用例ID：
实际行为/预期行为/结果：PASS / FAIL / 未执行 / 不支持
截图/视频/日志路径：
CPU/RSS/延迟/线程/fd：
关联问题ID/复现步骤：
```

“不支持”须在产品说明中体现；“未执行”不能转换成“通过”。
