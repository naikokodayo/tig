# Main Git notes 元数据闭环

基于 `1058486e`（与 `4dca878d` 的差异仅为文档/证据）。关闭 PR #54 记录的普通主视图 notes 类型差异；迁移总阶段门仍开放，未重跑全 154，未改当前全量 JSON。

`src/options.c:log_custom_pretty_arg` / `src/main.c:main_read` 以 `%N` 判断
`main-annotated`。Rust 复用已有单次 Git log，增加一个 NUL 分隔 notes 字段，
仅保留 `Commit.annotated`；主视图将该标记传入现有 `row_types`，save-view
直接复用。show-notes=no/false/0 时格式不展开 `%N`，保证不会误标。
自定义 ref 通过 Git 的 `--show-notes=<ref>`，与 C 一样累加默认 notes。
无逐提交 Git 查询、缓存、新依赖或 unsafe；原 C 与原版断言不变。
refs/reflog 的共享解析输入补空字段；合成提交、tree、raw stdin 保持未注释类型。
原有 CLI notes 参数及外部 compact raw notes 输入支持不在此切片扩展。

## 复现与结果

macOS arm64，Rust 1.81.0。下列命令全部通过；原版 runner 显式使用 Rust 1.81.0。

```sh
rustup run 1.81.0 cargo fmt --all -- --check
rustup run 1.81.0 cargo test --locked --all-targets
rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings
rustup run 1.81.0 cargo build --locked --release
make src/tig
python3 rust/tests/view-export.py --output /tmp/notes-view-export.json
RUSTUP_TOOLCHAIN=1.81.0 PATH="$HOME/.cargo/bin:$PATH" python3 rust/tests/upstream-suite.py test/main/default-test test/main/show-changes-test test/main/main-options-test test/main/commit-order-edge-case-test test/main/pretty-raw-test test/main/stdin-test test/refs/default-test test/stash/start-on-line-test --output /tmp/notes-upstream.json
```

真实 Git 仓库经 PTY 比较完整屏幕和导出：默认 notes、关闭、指定自定义 ref、
不存在 ref、toggle 关闭/开启、refresh 七种均一致；每种均校验 C 的预期行类型。
多行 notes 中含 `commit fake` 不污染后续提交。单元测试另覆盖 ETX 字节和布尔别名。
既有 23 个完整视图导出及 10 个路径/交互/gitlink 检查均通过。

## 源码与二进制绑定

哈希排除本收据以免自引用。源码内容哈希验证当前切片，不能视作旧主分支的全量凭据。

```json
{
  "source_base": "1058486ee325c8b5daba2a8ddd7eaa04c5745a0c",
  "source_inputs": "Cargo.toml, Cargo.lock, tigrc, all rust/**/*.rs and rust/**/*.py; sort paths; concatenate path UTF-8 + NUL + bytes + NUL",
  "source_sha256": "11eab68aea0fbbffb54ebb2853a119e9467cbea60c21cd3650d740b328b85c88",
  "binary_sha256": {
    "c": "d9c4efb28684d8215d1e9d6d02fb35cbb36d20f156cec0e0493818b4faeefd00",
    "rust": "5da590a210ca8f714cc887f67f046777de57ad9a46e3a8768fba40b76f9c1dd7",
    "before_rust": "92366eb52337b2f0266ae1953091b17f77017f7c215d280defdbd9c1b41e5a1d"
  },
  "results": {
    "Rust_1_81_tests_passed": 113,
    "C_Rust_full_view_comparisons": 30,
    "notes_screen_and_view_comparisons": 7,
    "other_path_interactive_gitlink_checks": 10,
    "original_scripts_per_side": 8,
    "original_assertions_per_side": 23,
    "original_failures": 0,
    "original_skips": 0
  },
  "original_scripts": {
    "test/main/commit-order-edge-case-test": 2,
    "test/main/default-test": 6,
    "test/main/main-options-test": 1,
    "test/main/pretty-raw-test": 1,
    "test/main/show-changes-test": 9,
    "test/main/stdin-test": 1,
    "test/refs/default-test": 2,
    "test/stash/start-on-line-test": 1
  }
}
```

## 修复前后原始差异

独立从 `4dca878d` 构建旧 Rust，在同一真实 Git notes 仓库运行 C 和旧 Rust，
重现下列差异（两行提交、首行选中）。修复后七种对照无差异。

旧 C：
```text
View: main
Ref: 8e645e2ededce0358485ed8179aefdfaea313ece
Dimensions: height=28 width=80
Position: offset=0 column=0 lineno=0
line[  0] type=main-annotated selected=1
line[  1] type=main-commit selected=0
```
旧 Rust：
```text
View: main
Ref: 8e645e2ededce0358485ed8179aefdfaea313ece
Dimensions: height=28 width=80
Position: offset=0 column=0 lineno=0
line[  0] type=main-commit selected=1
line[  1] type=main-commit selected=0
```
修复后的默认 notes 导出（C/Rust 相同）：
```text
View: main
Ref: 6e3702fe66dd19e1a95b4823af8ef782d72d1dc3
Dimensions: height=28 width=80
Position: offset=0 column=0 lineno=0
line[  0] type=main-annotated selected=1
line[  1] type=main-commit selected=0
```

## 独立只读复审

两份独立 fresh-context Reviewer 均未发现实质问题，且未修改文件。
第一份核对 Git 解析/参数边界、11 字段对齐、模型构造和 Rust 1.81；另用临时
Git 仓库确认 `%N` 不将二进制 notes 的 NUL 原样注入字段。第二份核对
C notes 判定、main→row_types→save-view、refs/reflog 共享协议，聚焦单元测试通过。
两者均检查 diff；未把独立复审描述为额外全量测试。
