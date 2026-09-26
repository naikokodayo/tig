# Chrono 日期兼容接入证据

核实日期：2026-09-27。范围是日期切片新增依赖与 C 日期行为，非全仓安全审计。

## 决定

采用 `chrono = { version = "=0.4.45", default-features = false, features = ["std"] }`，用于 ISO 日期校验、历法和时间戳转换。保留 Tig 的相对日期规则；本地时区和系统 locale 格式由已有 `date` 子进程路径承担。不要直接把任意 libc `strftime` 格式交给 Chrono。0.4.45 是核实日官方最新发布的 0.4.x，声明 MSRV 1.62；0.4.42 同样声明 1.62，但没有理由为本切片放弃后续修复。[发布记录](https://github.com/chronotope/chrono/releases)、[0.4.45 manifest](https://docs.rs/crate/chrono/0.4.45/source/Cargo.toml)、[0.4.42 manifest](https://docs.rs/crate/chrono/0.4.42/source/Cargo.toml)

`std` 只带入 `alloc`；关闭默认特性后没有 `clock`、`now`、`iana-time-zone`、`windows-link`、`wasm-bindgen`、`pure-rust-locales` 或旧 `time` 依赖。`alloc` 单独也足够支持格式化，但项目本就使用标准库，`std` 表意更直接。系统时区数据库、环境变量探测和语言数据库不属于这次 Chrono 接入。[特性定义](https://github.com/chronotope/chrono/blob/v0.4.45/Cargo.toml)

## 精确依赖、MSRV 与许可

在独立临时工程用 Rust/Cargo 1.81.0 解析和实际编译上述配置，得到以下完整新增依赖闭包。`cargo tree -e features` 仅显示 Chrono 的 `std`/`alloc` 和 autocfg 的空默认特性。

| crate | 版本 | 类型 | 声明 MSRV | SPDX 许可 |
| --- | --- | --- | --- | --- |
| chrono | 0.4.45 | 直接 | 1.62.0 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | Chrono 普通依赖 | 1.60 | MIT OR Apache-2.0 |
| autocfg | 1.5.1 | num-traits 构建依赖，无下级依赖 | 1.0 | Apache-2.0 OR MIT |

版本、MSRV 和许可均以下载的 registry manifest 与 `cargo metadata --locked` 交叉确认。[Chrono manifest](https://docs.rs/crate/chrono/0.4.45/source/Cargo.toml)、[num-traits 原始 manifest](https://github.com/rust-num/num-traits/blob/num-traits-0.2.19/Cargo.toml)、[autocfg manifest](https://docs.rs/crate/autocfg/1.5.1/source/Cargo.toml.orig)

锁文件校验和：

```text
chrono 0.4.45     1aa79e62e7697b8e29b513a68abacf485adcd1fe8284a4316c5ae868e6633327
num-traits 0.2.19 071dfc062690e90b734c0b2273ce72ad0ffa95f0c74596bc250dcfd960262841
autocfg 1.5.1     f2032f911046de80f0a198e0901378627c33f59ea0ac00e363d481118bd70a53
```

三者均可选择 MIT 分支。GNU 将该 Expat/MIT 许可列为 GPL 兼容；双许可只需其中一个选项与项目兼容。因此本切片选择 MIT，不需改变项目的 GPL-2.0-or-later 声明；分发时仍须保留对应版权和许可文本。Apache-2.0 单独与 GPLv2 不兼容，不应把“MIT OR Apache-2.0”误读为必须同时接受两者。[GNU 许可列表](https://www.gnu.org/licenses/license-list.html#Expat)、[GNU 双许可解释](https://www.gnu.org/licenses/license-compatibility.en.html)、[Chrono LICENSE](https://github.com/chronotope/chrono/blob/v0.4.45/LICENSE.txt)

RustSec 的 Chrono 包记录列出历史 `localtime_r` 问题，修复范围为 `>=0.4.20`；0.4.45 不在受影响范围，本配置也未启用本地时区功能。这是特定公告的核实，不能替代全部依赖或实现的安全证明。第一方 `forbid(unsafe_code)` 保持原状；该 lint 不代表第三方 crate 中完全不存在 unsafe。[Chrono 包公告](https://rustsec.org/packages/chrono.html)、[RUSTSEC-2020-0159](https://rustsec.org/advisories/RUSTSEC-2020-0159.html)

## C 行为是兼容边界

仓内依据为 [`src/util.c`](../src/util.c) 的 `time_now`、`get_relative_date`、`mkdate`，以及 [`test/main/date-test`](../test/main/date-test)。原版测试辅助脚本设 `TZ=UTC`、`LC_ALL=en_US.UTF-8`，日期测试另设 `TEST_TIME_NOW=1441051553`；不能因为测试使用 UTC，就把实际本地日期实现为 UTC。

| 项目 | C/Tig 行为 | 接入要求 |
| --- | --- | --- |
| 提交时间 | 非本地显示提交自身 wall time；本地显示同一 instant 的系统时间 | 使用有偏移的 `DateTime`，避免偏移重复相加 |
| 非本地 `%Z` | Tig 与 `%z` 一样插入 `+0900` 形式 | Chrono `%Z` 为 `+09:00`，必须由兼容层处理 |
| 本地 `%Z` | libc 时区名或缩写，如 UTC、EST、EDT | 固定偏移不能提供名称；保留系统格式 backend |
| 相对日期 | 120 秒、120 分钟、48 小时、14 天、5 周、365 天为单位切换阈值；月固定 30 天，年固定 365 天；整数截断 | 直接保留小型规则表，不引入 humanize 库 |
| 相对措辞 | `ago` / `ahead`；只有数量大于 1 才复数，包括 `0 second ago`；compact 为 `s m h D W M Y`，未来有负号 | 不使用库默认自然语言或四舍五入 |
| `TEST_TIME_NOW` | 覆盖当前时间；伪修改行使用 UTC 偏移 | 当前时间入口集中读取，再交给 Chrono 转换 |
| locale | C 启动时 `setlocale(LC_ALL, "")` | 系统 `date` 继承环境；Chrono 默认英语不可假装跟随环境 |

Chrono 的 `%Z` 文档明确说明不提供时区缩写；`%+` 也与常见 libc 的 locale 格式不同。`%E` / `%O` 是 libc 支持的替代表示修饰符，Chrono 不支持；Chrono 自己的 `%q`、分数秒等扩展也不是任意 libc 上的等价规格。避免全局字符串 `replace("%Z", "%z")`，它会误改 `%%Z`，应按实际格式 token 处理。[Chrono 格式文档](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)、[POSIX strftime](https://pubs.opengroup.org/onlinepubs/009695399/functions/strftime.html)

对不受当前快速格式路径支持的有效 libc 格式，使用已有系统 backend 比继续扩充自写格式器更小。格式作为单独的 `Command` 参数传入（包含 `+` 前缀），不能拼入 shell；检查退出状态和 UTF-8，不静默降级到另一种日期。GNU `date -d @SECONDS` 与 BSD `date -r SECONDS` 路径需分别验证。此选择仍有每次启动子进程的成本，未来若真实性能测量显示瓶颈，再在保持 `%Z` / locale 语义前提下优化。

## 错误格式与实测

Chrono `StrftimeItems::new` 对非法或未知规格产生 `Item::Error`，`new_lenient` 则把它保留成字面量；两者都不等于所有 libc 的行为。POSIX 明确把未知规格列为未定义行为，因此不为其编写跨平台仿真。若直接使用 Chrono formatter，用 `write!` / `write_to` 捕获格式错误；不要对未经验证的格式直接调用 `.to_string()`，因为 `Display` 的格式错误可能导致 panic。[StrftimeItems 文档](https://docs.rs/chrono/latest/chrono/format/strftime/struct.StrftimeItems.html)、[Rust ToString panic 条件](https://doc.rust-lang.org/std/string/trait.ToString.html#tymethod.to_string)

2026-09-27 在 `aarch64-apple-darwin` 用 `rustup run 1.81.0 cargo run` 执行隔离 probe 成功，所有第一方 probe 代码启用 `#![forbid(unsafe_code)]`。`1440961285`、固定 `+09:00` 输入得到：

```text
%Y-%m-%d z=%z Z=%Z z=%z -> 2015-08-31 z=+0900 Z=+09:00 z=+0900
%a %b %c %x %X          -> Mon Aug Mon Aug 31 04:01:25 2015 08/31/15 04:01:25
%Q, %, %#z, %Ec, %Od    -> fmt::Error（write! 返回错误，没有 panic）
```

这证明新增依赖闭包和使用到的 API 能在 Rust 1.81 编译，并证明上述差异。完整项目的 fmt/test/clippy/release 与原版 focused 结果由实现切片独立记录，不能用这个 probe 代替。

## 实现后的边界

最终实现仅让已列入允许集合的常见指令使用系统 backend；`%E`、`%O`、padding 修饰符和任意扩展仍明确报错，而不是假称已实现全部 libc 格式。`TEST_TIME_NOW` 使用严格整数/范围校验，未模仿 C `atoi` 对无效文本变成零的行为。普通 raw Git 头由新解析入口读取，原版 date-test 的首个失败因此从 pager 提前退出转为可逐项验证。最终结果见 [`date-focused.json`](evidence/date-focused.json)：date 8/8，六个原版脚本合计 27/27；完整迁移兼容门仍开放。

PR #4 合入后，blame 时间戳也复用相同 Chrono 转换，删除第二套手写 Gregorian 换算，保留 blame 的 0..9999 年范围。共享入口在附加偏移前使用 `checked_add_offset` 校验本地日期范围；最小/最大时间戳边界回归先失败再通过。Unix 0 显示层兼容规则也通过 blame 渲染回归覆盖。本地/locale 子进程成本同样适用于 blame 每行，未声称大历史性能达标。
