# CLAUDE.md

## Build & Run

```bash
cargo build
cargo test                    # 208 tests (161 core + 24 integration + 23 llm)
cargo test -p iris-core       # single crate
cargo test <test_name>        # single test
cargo clippy                  # must pass before completing work

# Run (ephemeral, no DB)
CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run

# Run (persistent memory)
DATABASE_URL=postgres://... CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run

# Debug (TUI uses raw mode, logs go to /tmp/iris.log)
RUST_LOG=debug CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run
# tail -f /tmp/iris.log  (in another terminal)
```

## 项目概述

**iris**：具备持续认知闭环的数字生命——感知输入、理解整合、决策行动、反馈学习、自发思维。借鉴人脑"输入→路由→处理→存储→输出"架构，资源压力直接影响调度与决策。

**设计原则**：四元驱动（事件/时钟/内驱/经验）| 单路径认知链路 | 可恢复进化（capability 隔离回滚）| 资源一等公民 | 零配置（参数持久化于 `iris_config` 表）| v1 简单优先

## 架构

Cargo workspace（Rust edition 2024, resolver v3），3 crates：

| Crate | 职责 |
|-------|------|
| `crates/llm/` (`iris-llm`) | LLM 抽象层：LlmProvider trait + LlmRouter + HttpProvider（Anthropic 原生 + OpenAI 兼容） |
| `crates/core/` (`iris-core`) | 所有认知子系统：runtime / dialogue / sensory / cognition / memory / codegen / capability / identity / resource_space / environment / boot |
| `crates/cli/` (`iris-cli`) | TUI 入口（ratatui/crossterm） |

### Runtime tick 循环（8 步）

tick 间隔：正常 100ms / 空闲 500ms / 休眠 2000ms。优雅关闭：CancellationToken + 15s 超时。

1. **收集输入**：drain 外部/系统/内部事件；新用户输入时 context_version+1 取消旧推理
2. **感觉门控**：四维打分（novelty×0.35 + urgency×0.25 + complexity×0.25 + task_relevance×0.15）；低于 noise_floor 丢弃
3. **路由分发**：TextDialogue / InternalSignal / SystemEvent
4. **统一响应链路**：构建上下文 → `route_tool_call`（JSON 决策 + schema 校验，优先用 lite 模型）
5. **执行策略**：`use_tool=false` → 直接回复 | `use_tool=true && confidence≥0.72` → 直接执行 | 否则 → agentic loop
6. **动作执行**：返回工具输出或 LLM 文本
7. **学习更新**：Self-Critic → capability_score / codegen_history / user_preference
8. **记忆写入**：工作记忆 + 情节记忆持久化 + narrative_event

**独立异步任务**：记忆固化（30min）| 经验回放（salience>0.45）| 资源维护 | 休眠周期（energy<0.2）

### 关键子系统

- **记忆**：工作记忆（进程内环形缓冲 32 条）+ 情节记忆（`episodes` 表）+ 语义记忆（`knowledge` 表）。v1 单 PostgreSQL 扁平表
- **Capability 生命周期**：`staged → active_candidate → confirmed(=LKG) → retired`，异常 → `quarantined`。内置工具：read_file / write_file / run_bash
- **Codegen**：GapDescriptor → LLM 生成 Rust crate → syn 语法校验 → cargo build → staging。最多 3 轮修复
- **身份**：core_identity（不可变）+ self_model（KV）+ narrative_event + affect_state（energy/valence/arousal）
- **LLM**：模型名自动推断 provider（claude-* → Anthropic, gpt-*/o1-*/o3-*/o4-* → OpenAI, gemini-* → Google, deepseek-* → DeepSeek）。连续 3 次失败 → unavailable，60s 探测恢复

## 环境变量

按 `CLAUDE_*` > `OPENAI_*` > `GEMINI_*` > `DEEPSEEK_*` 顺序探测。

| Variable | Required | Purpose |
|----------|----------|---------|
| `*_MODEL` (如 `CLAUDE_MODEL`) | Yes (for LLM) | 主模型名 |
| `*_API_KEY` (如 `ANTHROPIC_API_KEY`) | Yes (for LLM) | API 密钥 |
| `*_BASE_URL` | No | 自定义 endpoint |
| `*_LITE_MODEL` | No | 轻量模型用于 tool routing（回落主模型） |
| `DATABASE_URL` | No | PostgreSQL 连接串（缺失→ephemeral 模式） |
| `RUST_LOG` | No | 日志级别（写入 `/tmp/iris.log`） |

## 编码约定

- **错误处理**：库 crate 用 `thiserror`，应用 crate 用 `anyhow`。禁止 `.unwrap()`（测试除外）
- **异步**：async trait 用 `async-trait` crate。长任务必须接受 `CancellationToken`。共享状态用 `Arc<T>`
- **测试**：单元测试在文件底部 `#[cfg(test)] mod tests`。命名 `test_<模块>_<行为>_<条件>`。LLM 测试用 `MockProvider`
- **模块**：每个子目录 `mod.rs` 导出。单文件≤800 行。类型集中定义于 `types.rs`
- **日志**：对话内容永不记录

## 常见陷阱

1. **TUI 日志不可见**：必须用 `tracing` 写入 `/tmp/iris.log`，`println!` 无效
2. **Ephemeral 模式**：无 `DATABASE_URL` 时持久化功能静默跳过
3. **LLM 无跨 provider 回落**：仅同 provider 内重试
4. **CJK 宽字符**：CLI 必须用 `unicode-width` 处理。交互式输入必须用 `rustyline`，禁止 raw stdin reader
5. **已知测试失败**：`resolve_model_claude_priority` 和 `resolve_base_url_provider_fallback`

## 开发工作流

### 添加新模块
1. 创建 `.rs` → `mod.rs` 注册 → `types.rs` 定义类型 → 写单元测试 → `cargo clippy`

### 添加新工具
1. `capability/builtin/` 实现 `BuiltinTool` → `mod.rs` 注册 → `tool_call.rs` 添加 schema → 写集成测试

### 数据库变更
1. `migrations/` 创建 `NNN_xxx.sql`（仅 UP）→ `cargo build` 触发 sqlx 检查

### 开发门禁
- 提交前 `cargo clippy` 无警告
- 新模块必须含单元测试
- 删除/重命名公共 API 需更新所有引用
