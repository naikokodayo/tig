# 通用 save-view（有边界的行为切片）

基于 `8f1086dc`，保留原 C 基准。真实调用链为 `src/prompt.c` →
`src/display.c:save_view` → 各视图 `get_column_data`：导出视图名、关系、Ref、
尺寸、位置、所有行的类型/选中位，以及 box-backed 行的 cells；不是屏幕截图。
Rust 从已有视图快照保留结构行类型，复用显示标题的 reference 计算及 diff cells；
log 按 C 的 log_read 状态生成 cells。没有新依赖，也不改 `rust/watch.rs`。

## 范围与安全差异

- `:save-view [path]` 现在可从 main/status/refs/tree/blob/blame/grep/help/
  reflog/stash/log/stage/diff/pager 调用；默认仍是 `tig-view.txt`。
- 用户已批准的差异：C 使用 `fopen(path,"w")` 无条件截断；Rust 使用
  `create_new` 原子创建，拒绝已有文件、最终路径符号链接和目录。取消提示不写文件。
  这保证不覆盖已有目标，不承诺整份文件内容的事务提交；写入失败会报错，可能留下新建的部分文件。
- 不增加 stdout 约定：C 的 `-` 也是普通文件名。
- diff/stage/pager/log 在 word-diff、wrap-lines、定制行颜色规则或 ANSI cells 下
  明确拒绝导出，且不打开目标文件。C 支持更广的 cells/折行元数据，Rust 暂未保留。
  这些配置门是保守边界：也会拒绝没有实际受影响的短 pager 内容。
- **未关闭的 notes 差异**：C 的 noted commit 为 `main-annotated`，当前 Rust
  `Commit` 不保留 notes，导出仍为 `main-commit`。主任务明确批准记录此边界，
  不另加第二套 Git 查询。真实 notes 回归存入 `known_differences`，不计为 parity pass。
- Ref/行数基于现有 Rust 视图加载状态；不宣称修复其所有既有差异。gitlink 类型匹配
  C 的 `default`，其既有 tree 排序差异不在本切片关闭。

## 复现

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
make src/tig
python3 rust/tests/view-export.py --output /tmp/tig-view-export.json
python3 rust/tests/diff-view-export.py > /tmp/tig-diff-export.json
python3 rust/tests/upstream-suite.py test/diff/diff-stat-test test/help/default-test test/help/all-keybindings-test test/help/user-command-test test/grep/default-test test/tree/default-test test/status/on-branch-test --output /tmp/tig-export-upstream.json
```

Rust 1.81.0，macOS arm64。原版脚本及断言未修改。没有运行完整 154 脚本，
没有把边界差异或未达到的断言计为通过。


## 最终结果与源码绑定

以下来源哈希不包含本收据，避免自引用。运行结果为有边界的切片验证；notes 差异仍开放。

```json
{
  "source_base": "8f1086dc",
  "source_inputs": "Cargo.toml, Cargo.lock, tigrc, all rust/**/*.rs and rust/**/*.py; sort paths; concatenate path UTF-8 + NUL + bytes + NUL",
  "source_sha256": "1378915acdf2a0638f4b79bb48a4116c9cf39a3a968c817dc020266b331b5feb",
  "binaries_sha256": {
    "c": "01c81fec1a69956ae4c12d88ee8a4ed2171c484c54b2dd8e3c19da3078b28f0d",
    "rust": "92366eb52337b2f0266ae1953091b17f77017f7c215d280defdbd9c1b41e5a1d"
  },
  "before": {
    "cases": 21,
    "non_diff_refused": 20,
    "rust_binary_sha256": "1c26066ba2bcda52d72908e804985245bb7315f3944476457c650478c205993f"
  },
  "after": {
    "full_C_Rust_view_comparisons": 23,
    "path_safety_interactive_and_gitlink_checks": 10,
    "existing_diff_cases": 7,
    "unit_tests": 112,
    "original_scripts_per_implementation": 7,
    "original_assertions_per_implementation": 46,
    "original_failures": 0,
    "original_skips": 0,
    "notes_known_difference_count": 1
  },
  "original_scripts": {
    "test/diff/diff-stat-test": {
      "assertions": 1,
      "C": "pass",
      "Rust": "pass"
    },
    "test/grep/default-test": {
      "assertions": 9,
      "C": "pass",
      "Rust": "pass"
    },
    "test/help/all-keybindings-test": {
      "assertions": 1,
      "C": "pass",
      "Rust": "pass"
    },
    "test/help/default-test": {
      "assertions": 3,
      "C": "pass",
      "Rust": "pass"
    },
    "test/help/user-command-test": {
      "assertions": 1,
      "C": "pass",
      "Rust": "pass"
    },
    "test/status/on-branch-test": {
      "assertions": 24,
      "C": "pass",
      "Rust": "pass"
    },
    "test/tree/default-test": {
      "assertions": 7,
      "C": "pass",
      "Rust": "pass"
    }
  }
}
```

### 保留的原始 notes 差异

C：
```text
View: main
Ref: 6e3702fe66dd19e1a95b4823af8ef782d72d1dc3
Dimensions: height=28 width=80
Position: offset=0 column=0 lineno=0
line[  0] type=main-annotated selected=1
line[  1] type=main-commit selected=0
```
Rust：
```text
View: main
Ref: 6e3702fe66dd19e1a95b4823af8ef782d72d1dc3
Dimensions: height=28 width=80
Position: offset=0 column=0 lineno=0
line[  0] type=main-commit selected=1
line[  1] type=main-commit selected=0
```

## 只读复审

两个独立 fresh-context Reviewer 发现 help 折叠类型未同步、gitlink 类型映射错误，
均修复并加入真实 C/Rust 回归。复验确认无新的实质问题。notes 差异由主任务明确接受为
当前模型边界；没有把它记为 parity pass。修正后的 fresh Reviewer 最终检查亦无实质问题；额外只读 PTY 探查的 merge graph、
wrapped blob、diff/status/tree 选择行也与 C 一致（不加到上述固定运行计数）。
