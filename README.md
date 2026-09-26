# Tig — safe Rust migration in progress

基于 [jonas/tig](https://github.com/jonas/tig)，上游基线为
[`7d841c9`](https://github.com/jonas/tig/commit/7d841c9302456b8c8f0629a559344137dc4faa4f)。
保留 Jonas Fonseca 与其他贡献者的 Git 历史、版权和 GPL-2.0-or-later 许可。
本仓库是独立迁移项目，不是上游官方 Rust 版本。迁移使用
[code-migration-kit-with-codex](https://github.com/naikokodayo/code-migration-kit-with-codex)。
Rust 迁移与兼容性验证由 [Codex](https://github.com/apps/chatgpt-codex-connector)
协助完成，并保留原项目所有历史贡献者的署名。

**当前为可运行的 Rust 迁移检查点，完整行为兼容尚未通过验收。不要将其作为完整 Tig 替代品。**
原始 C 源码仍保留作为行为基准；Rust 可执行程序不调用 C Tig，也不链接其实现。

## 运行 Rust 版本

需要 Rust 1.81 或以上、Git，以及 macOS/Linux 终端：

```sh
cargo build --locked --release
./target/release/tig
./target/release/tig status
./target/release/tig blame -- path/to/file
```

本地/locale 日期还需要系统 GNU/BSD `date`。非本地自定义 `%s` 需要 64 位 macOS 或 GNU/Linux，以及 PATH 中使用宿主 libc 的 64 位系统 Perl（核心 POSIX 模块，无 CPAN 依赖）；工具不可用时明确报错。此路径启动子进程，性能门仍未关闭。

`make` 仍构建原版 C Tig；`cargo` 构建 Rust。原始说明见 [README.adoc](README.adoc)。

已实现并检查的部分包括 v2 提交图、Git 数据解析、基本提交/差异/状态/目录浏览、
搜索、整文件及文本补丁逐块/逐行暂存与取消暂存、窗口调整与终端恢复。
配置能够解析，但**不是所有配置项、绑定或命令都已生效**。
分屏、部分块选择、复杂重命名/二进制补丁、完整脚本/终端行为等仍待迁移。

完整边界、证据与复现命令见 [MIGRATION.md](MIGRATION.md)。
[Benchmark](migration/BENCHMARK.zh-CN.md) 仅比较已经验证输出相同的提交图组件，
不能解读为完整应用的性能结论。
