# 贡献指南

感谢你帮助 Sidera 改善 Linux 课堂批注体验。本仓库是[上游 Sidera](https://github.com/offduckling4825/Sidera--screen-annotate-for-linux) 的定制分支；原作者与本分支的归属见 [NOTICE.md](NOTICE.md)。

## 在哪里贡献

- 当前定制开发以 `custom` 为基线，Pull Request 请面向 `custom`。
- `rust/` 是本分支当前主要维护的实现；不要把 Rust 新行为直接描述为 C++ 版也已实现。
- 普通错误、兼容性和体验建议请提交 [Issue](https://github.com/shi-zhe-lei/Sidera--screen-annotate-for-linux/issues)。
- 涉及漏洞、权限越界或敏感数据的问题，按 [SECURITY.md](SECURITY.md) 私密报告。

## 有用的问题反馈

说明实际行为、期望行为和最短重现步骤，并附上：

1. Sidera 包版本 / 提交号，以及标准、GNOME 原生或兼容入口。
2. 发行版、处理器架构、X11 / Wayland 会话、桌面及版本。
3. 鼠标或触摸、缩放比例、多屏情况；涉及 WPS 时再提供 WPS 版本与联动状态。
4. 已脱敏的截图和相关日志。不要上传学生资料、完整课件、密码、访问令牌或个人路径。

`dpkg-query -W sidera`、`dpkg --print-architecture`、`getconf GNU_LIBC_VERSION` 可以帮助区分包版本、架构和系统库。桌面入口或设置中的系统诊断也有参考价值。

## 开发与构建

在 Linux 上使用 Git、Rust / Cargo 和 C 编译工具。`rust-toolchain.toml` 固定 Rust 1.93.1；它是当前构建版本，不是已验证的最低 Rust 版本。

```sh
git clone --branch custom https://github.com/shi-zhe-lei/Sidera--screen-annotate-for-linux.git
cd Sidera--screen-annotate-for-linux/rust
cargo build --release --locked
cargo check --tests --examples --locked
```

第二条 Cargo 命令只检查包括测试和预览在内的代码能否编译，**不会执行测试**。启动应用或测试输入前，使用独立环境并保存工作；项目协作中用户指定的验证范围优先。

### 按改动选择验证

| 范围 | 可用检查 | 说明 |
| --- | --- | --- |
| Rust 源码 | `cargo check --tests --examples --locked` | 编译检查，不证明行为正确 |
| Rust 行为 | `python3 tools/test_rust.py` | 仓库根运行，隔离 HOME/网络/运行目录；另用 --x11 / --wayland 执行协议检查 |
| C++ 行为 | `python3 cpp/tests/test_bridge_api.py`、`python3 cpp/tests/test_core_state.py` | 真实桥及私有 Sway；不连接现有应用 |
| WPS JavaScript | `node --check wps-addin/js/bridge.js` | 在仓库根执行，仅语法检查 |
| GNOME JavaScript | `node --input-type=module --check < compat/gnome-extension/extension.js` | 在仓库根执行；不验证 Shell API |
| 静态界面 | `cargo run --locked --example render_ui -- /tmp/sidera-preview /path/to/CJK-font.ttc` | 在 rust/ 执行；调用绘制函数，不运行桌面后端 |
| 行为回归与桌面验收 | [测试环境](docs/TEST_ENVIRONMENT.md) | 使用文档中的隔离条件；在获得对应环境操作授权后执行 |

提交时写清哪些检查已执行、哪些仅编译、哪些还需要手工验收。不要把“源码里有测试”写成“测试通过”。

### 打包与系统兼容性

`rust/build_deb.sh amd64` 或 `arm64` 会先检查 ELF 架构，并按 `target/container-<arch>/<triple>/release/sidera` → `target/container-<arch>/release/sidera` → `target/release/sidera` 的顺序选择二进制。交叉编译产物位于 triple 子目录（如 `container-aarch64/aarch64-unknown-linux-gnu/release/`），漏掉这一层会误取早期构建残留的旧文件。打包前核对实际被选中的文件，避免取到旧二进制。

当前三个打包入口由 `tools/deb_dependencies.py` 校验 ELF，并用 dpkg-shlibdeps 生成实际依赖。二进制本身无效（架构不符、没有动态依赖）时停止并保留已有包；主机缺少 dpkg 库清单（Arch/Fedora 等）时退回固定依赖并打印警告，该依赖未经实际二进制校验，正式发布仍应在目标架构的 Debian/Ubuntu 上重新打包。通过 `SIDERA_VERSION` 指定包版本；不能手工降低 libc 声明来伪装兼容。已发布 custom.29 的历史元数据与重建流程见[发布说明](docs/releases/2026-10-24-custom29.md)。

容器脚本需要仓库外准备的 `amd64-sysroot/` 或 `aarch64-sysroot/`、匹配 Rust 工具链、bubblewrap；ARM64 还需要对应的 QEMU / binfmt 配置。这些材料不包含在 Git 仓库。更低系统库构建和 ARM64 均需独立验证后才能发布。

为已有 Release 补充 ARM64 时，也可使用[原生 ARM64 发布构建流程](docs/releases/ARM64.md)：应用源码固定到原 tag，在独立 ARM64 构建机编译和检查，不需要本地交叉编译环境。

## 提交约定

- 一个改动围绕一个明确问题，解释用户可见变化以及原因。
- 保持课堂界面的布局与触摸习惯；涉及视觉时附静态预览，并说明是否经过真实桌面验证。
- 保留上游署名与许可证，新增内容遵循本项目授权；第三方代码注明来源及许可。
- 不提交编译产物、系统 rootfs、个人配置、未经脱敏的日志或密钥。
- 修改协议、设置格式、GNOME 扩展或 WPS 加载项时，说明旧版本如何升级与恢复。

PR 描述建议包含“问题与变化”“验证”“限制 / 升级步骤”三部分。请尊重参与者，以具体复现、截图或代码说明分歧。

## 发布核对

发布者应确认分支及提交、包内主程序来源、架构与系统库要求、扩展和加载项版本、源代码入口、许可证与 SHA256。验证未完成时发布为 **pre-release**，明确范围，不写成稳定版。CI 已配置 `custom` / `main` 的 push 和 PR；实际执行结果仍应逐次核对，本地检查不代表远程 CI 已通过。

新版本使用[双架构发布工作流](.github/workflows/release.yml)：推送 custom 版本标签或手动指定标签后，分别在原生 amd64 / ARM64 Ubuntu 22.04 上构建并运行隔离检查。工作流仅生成 artifact；核对两种架构的同源提交、测试结果及下载哈希后，再发布安装包、源码依赖归档与完整校验清单。不要只更新文件名或移动已发布标签。
