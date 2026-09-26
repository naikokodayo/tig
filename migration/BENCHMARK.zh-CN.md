# C 与 Rust benchmark：已验证的提交图组件

**这是迁移中的组件测量，不是完成迁移后的全应用 benchmark。** 全部 Tig 行为尚未通过兼容验收，不能用这些数字宣称整个 Rust Tig 更快。

对照仓库：[jonas/tig](https://github.com/jonas/tig) 与 [naikokodayo/tig](https://github.com/naikokodayo/tig)。迁移工具包是流程和文档项目，本次没有为它虚构运行时性能指标。

## 同机测量结果（2026-09-27 第二检查点）

单位为毫秒，越低越好。每个工作负载先验证输出逐字节相同，再计时。

| Workload | C median (ms) | Rust median (ms) | Time reduction | C p95 (ms) | Rust p95 (ms) |
|---|---:|---:|---:|---:|---:|
| tig_history | 13.996 | 13.025 | 6.9% | 14.882 | 14.745 |
| linear_10000 | 11.045 | 8.603 | 22.1% | 13.264 | 15.679 |
| diamonds_1000 | 6.980 | 5.543 | 20.6% | 7.139 | 6.659 |

Unstripped helper size: C 442,088 bytes; Rust 482,824 bytes (+9.2%).

`tig_history` 为固定版本的上游 Tig 完整历史；`linear_10000` 为一万次线性提交；`diamonds_1000` 为一千个菱形合并（3,001 次提交）。二进制大小受编译器运行时、链接方式和符号策略影响，不是内存占用。

## 方法与限制

- macOS 26.6 / arm64；Apple Clang 21.0.0，Rust 1.81.0。
- C 使用 Makefile 的 `-O2`；Rust 为 release、默认 `opt-level=3`、thin LTO、单 codegen unit。比较记录的优化构建，不是相同优化器。
- 同机、相同输入与 UTF-8 输出模式；每个程序预热 3 次，正式各测 16 次，C/Rust 与 Rust/C 交替顺序。
- 所有正式输出再次比对；计时包含进程启动、标准输入/输出传输及退出，排除生成 Git 日志的时间。
- p95 使用 16 个样本的 nearest-rank（这里为最大值），不是大量独立运行估出的稳定尾延迟。
- 本轮计时在原版测试和编译完成后执行；机器并非专用基准设备，未锁定 CPU 频率。差异不代表统计显著性或跨机器保证。
- 尚未测量完整 UI 首屏、滚动、所有视图、大仓库内存、Git 子进程峰值 RSS 或全量冷构建成本。
- 组件正确性另有 2,042 次 C/Rust 逐字节比较；本轮 v2 benchmark 不覆盖 curses 属性、graph v1 或 GH490 主视图样例。另有 v1 的 4,084 次字形和元数据差分通过，未混入 v2 性能数字。

## 证据与复现

- [原始计时、哈希及二进制信息](evidence/benchmark-second.json)
- [图形差分完整记录](evidence/graph-differential-second.json)
- [原版 C 测试基线](evidence/c-baseline.json)
- [真实 PTY 检查](evidence/terminal-smoke.json)

```sh
cargo build --locked --release
python3 migration/benchmark.py \
  --c-binary /path/to/original/test/tools/test-graph \
  --rust-binary target/release/test-graph \
  --repo /path/to/original/tig \
  --revision 7d841c9302456b8c8f0629a559344137dc4faa4f \
  --c-flags '填写实际 C 构建命令与编译选项'
```

完整迁移后的 benchmark 必须等 [MIGRATION.md](../MIGRATION.md) 所列兼容门通过后再做；本报告不关闭该门。

首次检查点的历史测量仍保留于 `evidence/benchmark.json`；当前表格来自第二次测量。
