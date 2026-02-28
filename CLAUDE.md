# CLAUDE.md

## Build & Run

```bash
cargo build
cargo test              # all tests
cargo clippy            # must pass before completing work

# Run (ephemeral, no DB)
ANTHROPIC_BASE_URL=<url> ANTHROPIC_API_KEY=<key> CLAUDE_MODEL=<model> cargo run

# Run (persistent memory)
DATABASE_URL=postgres://... ANTHROPIC_BASE_URL=<url> ANTHROPIC_API_KEY=<key> CLAUDE_MODEL=<model> cargo run

# Debug (logs to /tmp/iris.log)
RUST_LOG=debug <env vars above> cargo run
```

## 项目概述

**iris**：具备持续认知闭环的数字生命——感知输入、理解整合、决策行动、反馈学习、自发思维。借鉴人脑"输入→路由→处理→存储→输出"架构，资源压力直接影响调度与决策。

Cargo workspace（Rust edition 2024），2 crates：`crates/llm/`（LLM 抽象层）、`crates/core/`（认知子系统 + CLI 入口）。

## 编码约定

- 错误处理：库 crate 用 `thiserror`，应用 crate 用 `anyhow`。禁止 `.unwrap()`（测试除外）
- 每个子目录 `mod.rs` 导出。单文件≤800 行。类型集中定义于 `types.rs`
- 交互式输入必须用 `rustyline`，禁止 raw stdin reader（CJK 宽字符光标错位）
- 提交前 `cargo clippy` 无警告
- 删除/重命名公共 API 需更新所有引用

## 常见陷阱

1. **Ephemeral 模式**：无 `DATABASE_URL` 时持久化功能静默跳过
2. **LLM 无跨 provider 回落**：仅同 provider 内重试
3. **已知测试失败**：`resolve_model_claude_priority` 和 `resolve_base_url_provider_fallback`
