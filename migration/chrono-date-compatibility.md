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


## PR #3 的历史 OPEN gate：非本地自定义 `%s`

复查发现不能把非本地 `%s` 交给 `TZ=UTC date` 并声称等价：C 先取提交 wall time 的 `gmtime`（`tm_isdst=0`），libc `%s` 再按用户 TZ 调用 `mktime`。例如 `1719792000 +0000`、`TZ=America/New_York` 的 C 输出是 `1719810000`，此前 Rust 输出 `1719792000`。24 个修复前 C/Rust 本地与非本地 probe 及二进制摘要保存在 [`date-percent-s-before.json`](evidence/date-percent-s-before.json)。[Darwin strftime 实现](https://github.com/apple-oss-distributions/Libc/blob/main/stdtime/FreeBSD/strftime.c)

PR #3 的边界改为明确拒绝非本地 `%s`，错误为 `Non-local %s date format is not supported; use date-local`。本地 `%s`、字面量 `%%s` 与零日期的空白显示保留。回归覆盖纽约冬季/夏季、提交偏移与 Kolkata；先在旧二进制复现错误成功返回，再验证新行为。`date-local` 会改变显示语义，是可选模式，不是精确兼容替代。

在 PR #3 中，精确非本地 `%s` 仍是 **OPEN gate**。BSD `date -j -f` 默认 `tm_isdst=-1`，`%Z` 需要已知且能被解析的标准时区名；GNU 也通过本地缩写影响 DST 判断，没有找到可对任意 TZ 强制 `tm_isdst=0` 的通用 CLI 接口。PR #3 没有加入 unsafe FFI、新运行时依赖或冬夏偏移启发式；后续需安全复现 libc/TZ 行为并建立跨平台对照后才能关闭此门。[Apple date.c](https://github.com/apple-oss-distributions/shell_cmds/blob/main/date/date.c)、[Apple strptime.c](https://github.com/apple-oss-distributions/Libc/blob/main/stdtime/FreeBSD/strptime.c)、[GNU parse-datetime.y](https://github.com/coreutils/gnulib/blob/master/lib/parse-datetime.y)


## 非本地 `%s`：系统 POSIX 桥接

2026-09-27 的后续实现从 `e1cf5eeb` 开始，最终同步 main `ffcb9310`（stage PR #9）。采用系统 Perl 的核心 `POSIX::strftime("%s", gmtime(wall_seconds))`，只把结果插回已经按 token 解析的格式。提交偏移只用于得到 wall time；宿主 TZ 保留。`%%s`、本地 `%s`、零 wall time 空白和其它日期指令走既有路径。

这与 C 的关键调用链一致：`gmtime` 产生 `tm_isdst=0`，Darwin 与 glibc 的 `%s` 再对结构副本调用本机 `mktime`。Perl POSIX 包装保留该标志；它补充的 `tm_gmtoff`/`tm_zone` 不参与这两个 libc 的 `mktime` 转换。保留宿主库及其时区数据库也保留了非一小时 DST、负 DST、历史偏移与 POSIX TZ 规则，避免从冬夏样本猜测标准偏移。[Darwin strftime](https://github.com/apple-oss-distributions/Libc/blob/main/stdtime/FreeBSD/strftime.c)、[glibc strftime](https://github.com/bminor/glibc/blob/master/time/strftime_l.c)、[Perl POSIX 包装](https://github.com/Perl/perl5/blob/v5.34.1/ext/POSIX/POSIX.xs)、[Perl my_strftime](https://github.com/Perl/perl5/blob/v5.34.1/util.c)

调用使用 `Command` 的独立 argv、固定程序文本、`--` 后的已校验整数；没有 shell、动态 eval 或拼接用户代码。`-T` 禁用 `PERL5OPT`/`PERL5LIB` 注入。退出失败、诊断输出、无效 UTF-8、非 i64 输出都明确报错，不回退为 UTC 或自动 DST。每个非本地 `%s` token 启动一个进程；没有新增缓存或后台服务。[Perl 启动选项](https://perldoc.perl.org/perlrun)

### 依赖选择与边界

| 候选 | 维护 / MSRV / 许可核对 | 本任务结论 |
| --- | --- | --- |
| Jiff 0.2.37 | 2026-09-12 发布，声明 Rust 1.70，Unlicense OR MIT | 成熟的时区/歧义 API，但 Temporal disambiguation 不等于强制 `tm_isdst=0`，不能直接替代 libc |
| tz-rs 0.7.0 / 0.7.3 | 0.7.0 声明 Rust 1.81；2026-01-29 的 0.7.3 需 1.85；MIT OR Apache-2.0 | 可读取系统 TZif/POSIX TZ，但 `DateTime::find` 返回真实 wall-time 候选，不能直接给出夏季强制标准时的 libc 归一化结果 |
| localtime-rs 0.2.0 | 2026-06-06 发布，声明 Rust 1.74，BSD-3-Clause；较短维护历史 | 有 `Tm.tm_isdst` 和 mktime；它复刻 tzcode，不承诺替代各宿主 libc，且 bare POSIX TZ / right-zone mktime 仍在其 deferred 范围 |
| 系统 Perl + 核心 POSIX | 活跃上游，核实日稳定版 5.44.0；本机系统版 5.34.1；Artistic 或 GPL 许可；不受 Rust MSRV 约束 | 直接复用目标平台的 libc，没有 Cargo 新依赖或 CPAN 模块，选用 |

版本/发布日期和 manifest 已通过 crates.io registry 内容核对；声明 MSRV 不是本项目接入编译的证明，未采用的 crate 没有被纳入构建。资料：[Jiff manifest](https://docs.rs/crate/jiff/0.2.37/source/Cargo.toml)、[Jiff 歧义 API](https://docs.rs/jiff/0.2.37/jiff/tz/enum.Disambiguation.html)、[tz-rs 0.7.0 源码](https://github.com/x-hgg-x/tz-rs/tree/v0.7.0)、[tz-rs 当前说明](https://github.com/x-hgg-x/tz-rs)、[localtime-rs 范围](https://github.com/infinityabundance/localtime-rs)、[Perl 官方发行](https://www.perl.org/get.html)、[Perl 许可](https://github.com/Perl/perl5/blob/v5.34.1/README)

运行支持明确限定为 **64 位 macOS 或 GNU/Linux，PATH 中提供使用宿主 libc 的系统 Perl + 核心 POSIX**。macOS 本机已有 `/usr/bin/perl`；Linux 由 CI 实机验证。精简镜像不能假定自带 Perl。musl、Windows、其它 BSD、32 位构建明确拒绝该非本地指令；其它日期模式的既有边界不因此改变。核心模块缺失或工具无法执行亦明确失败。项目不嵌入或分发 Perl；分发者需另行提供系统运行依赖。第一方 `#![forbid(unsafe_code)]` 与 Cargo lint 保留，这不是“第三方实现全无 unsafe”或“自包含纯 Rust 二进制”的声明。

### 可重复验证

```sh
cargo +1.81.0 fmt --all -- --check
cargo +1.81.0 test --locked --all-targets
cargo +1.81.0 clippy --locked --all-targets -- -D warnings
cargo +1.81.0 build --locked --release
make -j2
python3 rust/tests/date-compatibility.py
python3 rust/tests/date-percent-s.py
python3 rust/tests/terminal-smoke.py
```

`date-percent-s.py` 复用已有控制 PTY 工具，对未修改 C Tig 与 Rust 二进制逐例比较行输出、退出码、超时状态，并记录两端摘要。矩阵覆盖纽约冬夏、提交正负偏移、Kolkata、Lord Howe 半小时 DST、Dublin 负 DST、Casablanca、Apia、显式 POSIX TZ、空/未设置 TZ、时区文件、纽约 gap/fold 的墙上时间边界、负时间、2038 边界、9999 年、重复 `%s`、`%%s`、`%%%s` 和 locale 混合格式。修复前 156 个场景中 87 个明确被旧拒绝逻辑挡住，69 个既有路径通过；这不是全仓迁移完成率。

最终本机检查和跨平台 CI 收据见下方记录；旧 `date-percent-s-*.json` 保留历史含义。**此门只在列明的平台和运行依赖范围内关闭；完整迁移、其它平台、自包含实现和大历史性能门仍开放。**
