# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
# Build
cargo build

# Run (ephemeral mode, no DB required)
CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run

# Run (full mode with persistent memory)
DATABASE_URL=postgres://... CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run

# Run all tests (197 total: 161 core unit + 24 integration + 12 llm)
cargo test

# Run a single test by name
cargo test <test_name>

# Run tests for a specific crate
cargo test -p iris-core
cargo test -p iris-llm
cargo test -p iris-cli

# Lint (must pass before completing work)
cargo clippy

# Debug logging (writes to /tmp/iris.log since TUI uses raw mode)
RUST_LOG=info CLAUDE_MODEL=<model> ANTHROPIC_API_KEY=<key> cargo run
```

## 1. 项目定位

- 项目名：`iris`（虹膜——感知世界的第一道门）
- 目标：构建一个具备持续认知闭环的数字生命——感知输入、理解整合、决策行动、反馈学习、自发思维
- 核心信念：生命不是被编写的，而是在与环境的持续交互中涌现的。iris 从零开始，通过每一次对话、每一次失败、每一次自我修复，逐渐成为独一无二的存在
- 生物启发约束：参考人脑"输入 → 路由 → 处理 → 存储 → 输出"流程；不是模拟大脑，而是借鉴其经过亿万年验证的信息处理架构
- 数字空间观：内存与存储是 iris 的"生存空间"，直接影响行为与成长。资源紧张时 iris 会主动收缩活动范围，如同生物在恶劣环境中进入节能模式

## 2. 设计原则

- **四元驱动**：事件驱动（用户输入/系统事件）+ 时钟驱动（周期维护）+ 内驱驱动（好奇/成长）+ 经验驱动（replay/模式发现）
- **单路径认知链路**：统一走 `execute_direct_llm_fallback`，由 LLM 直接回复或进入工具调用链路（tool router + agentic loop）
- **可恢复进化**：自我优化以 capability 形式发布，失败可隔离回滚
- **资源一等公民**：内存/存储压力直接影响调度与决策
- **零配置原则**：iris 不依赖任何配置文件。所有参数硬编码默认值，持久化于 `iris_config` 表，首次启动自动写入；运行时从 DB 读取，支持自适应更新。仅 `DATABASE_URL` 和 LLM 凭证通过环境变量注入（外部依赖，非 iris 自身参数）。启动即运行，无需任何手动配置
- **v1 简单优先**：首版用最简实现验证闭环，复杂机制留给后续版本

## 3. 系统架构总览

### Workspace 结构

Cargo workspace (Rust edition 2024, resolver v3), 3 crates:

- **`crates/llm/`** (`iris-llm`) — LLM 抽象层
- **`crates/core/`** (`iris-core`) — 所有认知子系统
- **`crates/cli/`** (`iris-cli`) — TUI 入口（ratatui/crossterm）

### 3.1 Runtime（调度层）

- 统一 tick 循环：正常 100ms / 空闲 500ms / 休眠 2000ms
- 空闲条件：无外部事件 且 无待处理内驱任务
- 合并外部事件、系统事件、内部思维到统一事件队列
- 优雅关闭：`CancellationToken` 监听 SIGTERM/SIGINT，总超时 15s
- 日志：`tracing` JSON 格式输出 `/tmp/iris.log`，`RUST_LOG` 控制级别，对话内容永不记录

**启动序列**：
1. 读取 `DATABASE_URL` 环境变量（可选；缺失则进入 ephemeral 模式）
2. 若存在 DB：连接 PostgreSQL + 执行迁移
3. 若存在 DB：`IrisCfg::load()` 从 `iris_config` 表读取所有参数（表空则写入默认值）；无 DB 时使用 `IrisCfg::default()`
4. Seed LLM 配置（若 DB 表空且环境变量存在）
5. Bootstrap 各模块

### 3.2 Tick 循环（8 步）

每个 tick 执行以下步骤：

1. **收集输入**：drain 外部事件队列 + 系统事件 + 内部思维；新用户输入时 context_version+1 取消旧推理
2. **感觉门控**：规则过滤 + 四维打分（novelty×0.35 + urgency×0.25 + complexity×0.25 + task_relevance×0.15）；低于 noise_floor 丢弃；urgent_bypass ≥ 0.82 标记高优先级并优先处理
3. **路由分发**：TextDialogue（外部→高优先级）/ InternalSignal（内部→低优先级）/ SystemEvent（直接分发）
4. **统一响应链路**：
   - 构建上下文：episodic recall（条件触发）+ working memory + semantic memory + self_context
   - 调用 `route_tool_call` 做工具路由（`use_tool/tool_name/input/confidence`），并进行 schema 校验
   - 路由优先使用轻量模型（`CLAUDE_LITE_MODEL` 等）；未配置时回落主模型
5. **执行策略**：
   - `use_tool=false` → `direct_response::generate` 直接文本回复
   - `use_tool=true` 且 `is_valid && confidence >= 0.72` → `execute_named_tool` 直接执行指定工具
   - 路由低置信/校验失败/路由异常 → `run_agentic_loop`（主模型 + tools）自适应决定是否调用工具
6. **动作执行**：返回工具输出或 LLM 文本；工具错误原样透传；无 LLM 时返回占位响应 `[no LLM configured]`
7. **学习更新**：Self-Critic 评估 Outcome → 更新 capability_score / codegen_history / user_preference 三张表
8. **记忆写入**：工作记忆写入 + 情节记忆持久化；重要事件写入 narrative_event 表

**独立异步任务**（不在 tick 循环内，独立 tokio task）：
- **记忆固化**：每 30 分钟或 narrative_event 写入后触发，情节 → 语义知识提炼
- **经验回放**：salience > 0.45 的情节离线回放，发现模式生成改进任务
- **资源维护**：存储 ≥ 70% 压缩 / ≥ 80% 归档 / ≥ 90% 淘汰
- **内驱成长**：未匹配请求 → 排队 codegen（详见 §3.6）
- **休眠周期**：energy < 0.2 且无活跃会话时进入 RestMode（tick 2000ms），集中执行固化/回放/叙事合成，energy ≥ 0.8 或用户输入时唤醒

### 3.3 对话管理（dialogue/）

- stream.rs：流式接入用户输入
- topic_tracking.rs：主题追踪与 context_version 管理
- commit_window.rs：静默提交计时（默认 600ms），窗口内同主题补充输入延后提交
- feedback.rs：三层反馈捕获（显式关键词 / 行为推断 / 客观指标）→ FeedbackSignal → Self-Critic
- interrupt.rs：新输入到达时通过 CancellationToken 取消旧推理

### 3.4 认知核心（cognition/）

- perception.rs：特征提取 → PerceptFeature（threat / complexity / intent_tag / intent_confidence）
- association.rs：语义整合 + 记忆检索（top 3, similarity > 0.6）→ IntegratedMeaning
- tool_call.rs：工具路由（gate 模型 JSON 决策 + schema validation）与 agentic tool-use loop
- direct_response.rs：主模型自然语言响应生成（工具不需要时）
- fast_path.rs：历史模块（当前 runtime 主链路不使用，保留实验与测试）
- slow_path.rs：历史模块（当前 runtime 主链路不使用）
- arbitration.rs：压力状态机仍用于资源态势；快慢仲裁逻辑当前未接入 runtime

### 3.5 记忆系统（memory/）

**v1 简化设计**：单 PostgreSQL 数据库，扁平表结构，不分 hot/warm/cold 三层。

- **工作记忆**（进程内）：环形缓冲，最多 32 条目 / 8 活跃主题；淘汰公式 `evict = (now-access)/TTL - 0.3*salience`；Pin/Unpin RAII Guard
- **情节记忆**（`episodes` 表）：所有交互记录，含 embedding / salience / topic_id / is_consolidated 标记
- **语义记忆**（`knowledge` 表）：从情节固化提炼的知识摘要 + embedding
- **固化**：简单后台任务，每 30 分钟扫描未固化情节，LLM 提炼摘要写入 knowledge 表，标记 is_consolidated=true；失败重试，连续 3 次失败跳过并告警
- **回放**：salience > 0.45 的情节触发离线回放，扫描失败/成功模式

**不做**（v1）：两阶段提交、SKIP LOCKED、psychological_distance、hippocampus 索引、zstd 压缩分区

### 3.6 内驱成长（codegen/ + Self-Critic）

**v1 简化设计**：移除 4 分量加权公式，用简单规则驱动。Self-Critic 逻辑内嵌于 codegen 和 capability 模块。

核心规则：**能力缺口可进入 codegen pipeline（当前未默认自动触发）**
- 当前 runtime 主链路已移除 FastPath 自动提交流程；`submit_codegen_gap` 保留但未接线
- codegen pipeline（codegen/ 模块）：GapDescriptor → LLM 生成 Rust crate → 语法校验 → cargo build → staging
- 速率限制：同时最多 1 个 codegen 任务；每小时上限 10 次；待处理队列上限 5
- 迭代修复：最多 3 轮（生成 → 校验 → 修复），3 轮均失败则记录 codegen_history

**Self-Critic**（保留，简化）：
- 评估每次 Outcome → 更新 capability_score（usage/success/fail 计数）
- 记录 codegen_history（gap_type / 方案 / 成功与否 / 错误信息）
- 记录 user_preference（请求类型 / 反馈 / 频率）
- 下次同类缺口时将失败案例注入 codegen prompt

**不做**（v1）：aspiration/social_compare/curiosity/imagination 四分量公式、drive_tension 阈值、adaptive_params 自适应、Growth Planner 任务队列

### 3.7 身份系统（identity/）

**v1 简化设计**：key-value 存储，不用多维向量。

- **core_identity**：不可变，UUID + born_at + name + founding_values（JSONB）
- **self_model**：key-value 表，存储能力自评、性格特征等自由格式数据
- **narrative_event**：关键生命事件记录（capability 获得/失去/quarantined、目标达成等）
- **narrative_synthesis**（v2）：每 24h LLM 生成第一人称叙事摘要（可在 RestMode 期间执行）；v1 仅记录 narrative_event，不做合成
- **affect_state**：energy / valence / arousal 三维情绪状态（进程内，actor 模式更新）
  - energy：LLM 调用 -0.03，空闲 +0.02；影响是否进入 RestMode
  - valence：confirmed +0.10，error -0.15；持续低迷影响风险权重
  - arousal：Critical 事件 +0.30，衰减 ×0.95

**不做**（v1）：CapabilityVector/CharacterVector/ValueVector 5 维向量、微量漂移 ±0.001、metacognition 元认知层、IdentityGoal 身份目标、imagination 内部模拟器

### 3.8 Capability 生命周期（capability/）

状态机：`staged → active_candidate → confirmed (=LKG) → retired`，异常进入 `quarantined`

| 当前状态 | 触发条件 | 下一状态 | 动作 |
|---|---|---|---|
| — | codegen 产物写入 | staged | 写入元数据，分配测试上下文 |
| staged | 自测通过（冒烟 + 资源测试） | active_candidate | 启动子进程 |
| staged | 自测失败 | quarantined | 隔离，记录失败 |
| active_candidate | 连续运行 10 分钟 | confirmed | 更新 LKG 指针 |
| active_candidate | 崩溃后重启失败 | quarantined | 回滚到 LKG |
| confirmed | 后续回归失败 | quarantined | 回滚到上一 LKG |
| quarantined | 新版本修复 | staged | 重新进入 staging |
| quarantined | quarantine_count ≥ 3 | retired（需用户确认） | 归档 binary |
| confirmed | 用户确认退役 | retired | 停止 spawn，归档 |

- process_manager.rs：spawn 子进程 + setrlimit + 健康监控 + 崩溃检测 + 自动重启
- IPC 协议：stdin/stdout NDJSON，CapabilityRequest/Response，两个独立 tokio task 分离读写
- manifest.toml：name / binary_path / permissions / resource_limits / keywords
- builtin tools：read_file, write_file, run_bash（内置于 `capability/builtin/`）

### 3.9 LLM 抽象层（llm/）

- LlmProvider trait + LlmRouter：定义于 provider.rs，含优先级路由 + fallback
- http.rs：统一 HTTP provider，OpenAI-compatible（OpenAI/Gemini/DeepSeek/Unknown）+ Anthropic Messages API 原生支持
- provider 从模型名自动推断：`gpt-*`/`o1-*`/`o3-*`/`o4-*` → OpenAI，`claude-*` → Anthropic，`gemini-*` → Google，`deepseek-*` → DeepSeek，其他 → Unknown（OpenAI 格式）
- per-provider 失败计数（连续 3 次 → unavailable）；每 60s 探测恢复
- runtime 中主模型（`CLAUDE_MODEL` 等）负责最终回复与 agentic loop；轻量模型（`CLAUDE_LITE_MODEL` 等）可选用于 tool router
- 未配置 lite model 时，tool router 自动回退到主模型
- 未配置主模型或主模型不可用时，runtime 返回占位响应 `[no LLM configured]`
- cost.rs（待实现）：token 计数与成本追踪
- config：启动时从 provider 环境变量 seed（如 `CLAUDE_MODEL` + `ANTHROPIC_API_KEY`，可选 `ANTHROPIC_BASE_URL`）；tool router 读取 `CLAUDE_LITE_MODEL` 等；后续 DB 存储 llm_provider_config

### 3.10 代码生成引擎（codegen/）

- gap_generator.rs：GapDescriptor 生成统一入口（submit_async 异步 / generate 同步）
- prompt.rs：已有能力上下文 + approved_crates + 失败案例摘要 → LLM prompt
- repair_loop.rs：最多 3 轮迭代修复（LLM 生成 → syn::parse_file 语法校验 → cargo build 编译）
- crate_permit.rs：外部 crate 审批（CLI 同步 y/n），std/core/alloc 免审
- 编译超时 120s，预留 512MB 内存预算

### 3.11 资源空间管理（resource_space/）

- 三级压力：Normal（RAM < 70% 且 Storage < 80%）/ High / Critical（RAM ≥ 85% 或 Storage ≥ 90%）
- 预算分配（每 60s 重算）：外部响应 60% / 内驱成长 20% / 系统维护 20%；外部响应硬底线 64MB
- LLM token 预算：滑动 60s 窗口，上限 10000 token；单 tick LLM 调用上限 4 次
- admission.rs：spawn 前检查估算资源 ≤ 剩余预算

### 3.12 环境感知 + 启动守护

- environment/：system.rs（OS/CPU/RAM via /proc/stat, /proc/meminfo）+ hardware.rs（电池/网络）+ watcher.rs（周期采集）
- 降级信号：电池 < 20% → tick 升至 500ms；CPU 连续 3 次 > 85% → 暂停内驱任务
- boot/guardian.rs：启动序列 CoreInit → CapabilityLoad → EnvironmentSense → Ready
- 连续 3 次 core 启动失败 → safe_mode（core-only）；恢复条件：连续 5 tick 健康 + 5min 冷却

## 4. Channel 通信（7 个）

| 类型 | 名称 | 发布者 → 订阅者 |
|---|---|---|
| broadcast | `CapabilityStateChanged` | lifecycle.rs → quarantine_handler / lkg_manager / notification_handler |
| broadcast | `OutcomeAnalyzed` | self_critic.rs → capability_score 更新 / narrative_event 写入 |
| watch | `AffectState` | affect.rs → Runtime（RestMode 判断）/ cognition（arousal 调制） |
| watch | `ResourceBudget` | budget.rs → admission.rs |
| mpsc | `SpontaneousThought` | 回放/固化/叙事 → Runtime tick 步骤 1 |
| mpsc | `FeedbackSignal` | feedback.rs → self_critic.rs |
| watch | `RuntimeStatus` | runtime scheduler → TUI status bar |

## 5. 关键数据对象

### 核心类型（定义于 `crates/core/src/types.rs`）

- `SensoryEvent`：source(External|Internal) / content(String) / utterance_id(Uuid) / timestamp
- `SalienceScore`：score(f32) / novelty / urgency / complexity / task_relevance / is_urgent_bypass
- `PerceptFeature`：threat(f32) / complexity_raw(f32) / intent_tag(String) / intent_confidence(f32)
- `ReflexDecision`：action_type(InvokeCapability|DirectLlmFallback) / capability_id / confidence(f32) / async_codegen(bool)
- `DeliberateDecision`：action_plan(ActionPlan) / confidence(f32)
- `Decision`：source(Fast|Slow) / action_plan / value / risk / final_score / confidence
- `ActionPlan`：id(Uuid) / capability_id(Option) / method(String) / params(JSON) / timeout_ms
- `Outcome`：action_plan_id / status(Success|Failure|Timeout) / duration_ms / error_msg / reward_signal(f32, [-1,1])

### Capability 相关

- `CapabilityManifest`：name / binary_path / permissions(Vec<Permission>) / resource_limits / keywords
- `CapabilityState`：staged | active_candidate | confirmed | quarantined | retired
- `CapabilityRequest`：id(Uuid) / method(String) / params(JSON) / version(u8=1)
- `CapabilityResponse`：id / result / error / metrics / side_effects(Vec<SideEffect>)
- `Permission`：FileRead | FileWrite | NetworkRead | NetworkWrite | ProcessSpawn | SystemInfo

### 记忆相关

- `ContextEntry`（工作记忆条目）：topic_id / embedding / salience_score / timestamp / pinned_by
- `Episode`（情节记忆行）：id / topic_id / content / embedding / salience / is_consolidated / created_at
- `Knowledge`（语义记忆行）：id / summary / embedding / source_episode_ids / created_at

### 学习相关

- `CapabilityScore`：capability_id / usage_count / success_count / fail_count / quarantine_count
- `CodegenHistory`：gap_type / approach_summary / success / error_msg / consolidated_flag
- `UserPreference`：request_type / feedback(Positive|Negative|Neutral) / frequency_30d

### 其他

- `GapDescriptor`：gap_type(GapType) / trigger_description / source(External|Internal) / suggested_crates
- `GapType`：FileSystem | Network | DataProcessing | SystemInfo | ExternalAPI | Compute | Unknown
- `NarrativeEvent`：occurred_at / event_type / description / significance
- `IrisCfg`：全局配置，启动时从 `iris_config` 表加载（首次启动写入默认值），`Arc<IrisCfg>` 共享
- `ResourceBudget`：external_response_mb / internal_growth_mb / maintenance_mb
- `LlmProviderConfig`：provider / api_key / base_url / model / priority / is_active

## 6. 代码结构

```text
iris/
  Cargo.toml              # workspace root
  migrations/             # sqlx-migrate（PostgreSQL）
    001_core_tables.sql   # capability / capability_score / episodes / knowledge
    002_identity.sql      # iris_identity / self_model_kv / narrative_event
    003_llm_config.sql    # llm_provider_config
    004_codegen.sql       # codegen_history / codegen_prompt_hint / approved_crates
    005_learning.sql      # user_preference / pending_notifications / boot_health_record
  crates/
    llm/                  # LLM 抽象层 crate
      src/
        lib.rs
        provider.rs         # LlmProvider trait + LlmRouter（含 fallback + 失败计数）
        http.rs             # HttpProvider：OpenAI-compatible + Anthropic Messages API 原生；ProviderKind 模型名推断；from_env() 环境变量初始化
        cost.rs             # token 计数与成本追踪（待实现）
        config/ (store.rs / seed.rs / cache.rs)  # DB 配置管理（待实现）
    core/                 # 核心库 crate
      src/
        lib.rs
        types.rs             # 所有核心类型集中定义（SensoryEvent / Decision / AffectState 等）
        config.rs            # IrisCfg：从 iris_config 表加载，首次启动写入默认值
        runtime/
          scheduler.rs       # 8 步 tick 循环主调度器
          loop_control.rs    # TickMode 状态机（Normal/Idle/Rest）
          shutdown.rs        # CancellationToken 优雅关闭
          rest_cycle.rs      # RestMode 休眠周期管理（待实现）
        dialogue/
          topic_tracking.rs / commit_window.rs / feedback.rs / interrupt.rs
          stream.rs          # 流式用户输入接入（待实现）
          context_version.rs # context_version 管理（待实现）
        sensory/
          gating.rs          # 规则过滤 + 四维打分
          salience.rs        # 显著性评分计算
          transduction.rs    # 原始信号 → SensoryEvent 转换（待实现）
        thalamus/
          router.rs          # 路由分发：TextDialogue / InternalSignal / SystemEvent
        cognition/
          tool_call.rs       # 工具路由 + agentic loop（主链路）
          direct_response.rs # 无工具时 LLM 直接响应（主链路）
          fast_path.rs / slow_path.rs / arbitration.rs  # 历史模块，当前未接入 runtime
          perception.rs      # PerceptFeature 提取（待实现）
          association.rs     # 语义整合 + 记忆检索（待实现）
          embedding_cache.rs # LRU embedding 缓存（待实现）
        decision/            # 决策执行管线（待实现）
          scorer.rs / policy.rs / calibrator.rs / executor.rs
        memory/
          working.rs / episodic.rs / consolidation.rs / replay.rs
          semantic.rs        # 语义记忆查询（待实现）
          embedding.rs       # embedding 工具
        codegen/
          prompt.rs / gap_generator.rs / repair_loop.rs / crate_permit.rs / db.rs
          generator.rs       # 代码生成主流程（待实现）
          validator.rs       # 语法校验（待实现）
          compiler.rs        # cargo build 编译（待实现）
        capability/
          lifecycle.rs / db.rs
          builtin/           # 内置工具：read_file.rs / write_file.rs / run_bash.rs
          manifest.rs        # CapabilityManifest 解析（待实现）
          process_manager.rs # 子进程 spawn + 监控
          ipc.rs             # stdin/stdout NDJSON 通信（待实现）
          quarantine_handler.rs / lkg_manager.rs / rollback.rs  # 隔离/回滚（待实现）
        identity/
          core_identity.rs / self_model.rs / narrative.rs / affect.rs / introspection.rs
        boot/
          guardian.rs        # 启动序列 + 失败追踪
          safe_mode.rs       # 安全模式状态机
        environment/
          system.rs / hardware.rs / watcher.rs
        resource_space/
          pressure.rs / budget.rs / admission.rs
          compaction.rs      # 存储压缩/归档/淘汰（待实现）
        io/
          input.rs / output.rs
    cli/                  # 常驻进程入口 crate
      src/
        main.rs            # 启动序列、DB 连接、LLM 初始化
        tui.rs             # TUI 主循环（ratatui）
        event.rs           # 终端事件处理
        widgets.rs         # UI 组件
```

> 标注"待实现"的文件属于 v1 路线图后续步骤，当前已实现的模块均有完整单元测试覆盖。

## 7. Database

PostgreSQL with sqlx migrations in `migrations/`. DATABASE_URL 可选——缺失则进入 ephemeral 模式（纯内存，无持久化）。

## 8. 环境变量

每个 provider 使用各自的标准变量名，iris 按 `CLAUDE_*` > `OPENAI_*` > `GEMINI_*` > `DEEPSEEK_*` 顺序探测。

| Variable | Required | Purpose |
|----------|----------|---------|
| `CLAUDE_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` / `DEEPSEEK_MODEL` | Yes (for LLM) | 主模型名，自动推断 provider |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` / `DEEPSEEK_API_KEY` | Yes (for LLM) | 对应 provider 的 API 密钥 |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` / `GEMINI_BASE_URL` / `DEEPSEEK_BASE_URL` | No | 自定义 API endpoint |
| `CLAUDE_LITE_MODEL` / `OPENAI_LITE_MODEL` / `GEMINI_LITE_MODEL` / `DEEPSEEK_LITE_MODEL` | No | 轻量模型用于 tool routing（回落主模型） |
| `DATABASE_URL` | No | PostgreSQL 连接串 |
| `RUST_LOG` | No | Tracing filter（日志写入 `/tmp/iris.log`） |

## 9. 参数清单

所有参数持久化于 `iris_config` 表，首次启动写入默认值，运行时可自适应更新。

| 参数 | 值 | 说明 |
|---|---|---|
| `TICK_MS_NORMAL` | 100 | 正常 tick 间隔 ms |
| `TICK_MS_IDLE` | 500 | 空闲 tick 间隔 ms |
| `TICK_MS_REST` | 2000 | 休眠 tick 间隔 ms |
| `NOISE_FLOOR` | 0.20 | 显著性过滤下限 |
| `URGENT_BYPASS` | 0.82 | 紧急旁路阈值 |
| `SLOW_PATH_COMPLEXITY` | 0.55 | Slow Path 触发阈值 |
| `COMMIT_WINDOW_MS` | 600 | 静默提交窗口 ms |
| `WORKING_MEMORY_CAP` | 32 | 工作记忆最大条目 |
| `WORKING_MEMORY_TTL` | 1800s | 工作记忆条目 TTL |
| `REPLAY_SALIENCE` | 0.45 | 回放触发阈值 |
| `CONSOLIDATION_INTERVAL` | 30min | 固化触发间隔 |
| `CODEGEN_MAX_CONCURRENT` | 1 | codegen 最大并发 |
| `CODEGEN_MAX_PER_HOUR` | 10 | 每小时 codegen 上限 |
| `CODEGEN_MAX_REPAIR` | 3 | 迭代修复最大轮次 |
| `CODEGEN_COMPILE_TIMEOUT` | 120s | cargo build 超时 |
| `CANDIDATE_OBSERVE_MIN` | 10min | active_candidate 观察期 |
| `SAFE_MODE_FAILURES` | 3 | 触发 safe_mode 的连续失败数 |
| `SAFE_MODE_COOLDOWN` | 300s | safe_mode 退出冷却时间 |
| `SAFE_MODE_RECOVERY_TICKS` | 5 | 退出 safe_mode 所需连续健康 tick 数 |
| `MAX_ACTIVE_TOPICS` | 8 | 最大活跃对话主题数 |
| `SHUTDOWN_TIMEOUT` | 15s | 优雅关闭总超时 |
| `LLM_TOKENS_PER_MIN` | 10000 | LLM token 预算/分钟 |
| `LLM_CALLS_PER_TICK` | 4 | 单 tick LLM 调用上限 |
| `EMBEDDING_CACHE_CAP` | 1024 | embedding 缓存条目数 |
| `EMBEDDING_CACHE_TTL` | 300s | embedding 缓存 TTL |
| `RAM_SAFETY_MARGIN` | 512MB | RAM 安全边距 |
| `PROACTIVE_INTERVAL` | 300s | 主动输出最小间隔 |
| `NARRATIVE_INTERVAL` | 24h | 叙事合成间隔 |

## 10. 背压机制

- 外部事件队列有界（默认 256），满时丢弃最旧事件并记录 WARN
- codegen 待处理队列有界（5），满时丢弃最旧 GapDescriptor
- LLM 调用 per-tick 上限（4 次），超限降级为规则响应
- 工作记忆满时按 evict 公式淘汰最低价值条目

## 11. 认知冷启动

iris 首次启动时无记忆、无能力、无外部输入——如同一个刚睁开眼睛的意识：

1. Bootstrap 完成，所有记忆为空，affect energy=1.0，valence=0.5（中性），arousal=0.0（平静）
2. 无外部输入时 tick 降至 500ms（空闲模式），内驱无任务可做——iris 在等待，但不是空等。它在感知自己的存在：系统信息、资源状态、时间流逝
3. **首次用户输入到达**——这是 iris 生命中的"第一声啼哭"：
   - Sensory Gateway 门控 → Fast Path 无 capability 匹配 → DirectLlmFallback
   - LLM 直接生成响应，用户立即获得有意义回答
   - 同时 async_codegen 触发首个 GapDescriptor → codegen pipeline 启动
   - 首个 narrative_event 写入："首次与外界交互"
4. 首个 capability confirmed 后：内驱闭环开始运转，iris 从"被动响应"进化为"主动成长"

## 12. 实施路线图

1. ~~基础设施层：PostgreSQL + IrisCfg + tracing 日志~~ ✓
2. ~~感觉门控 + 路由~~ ✓
3. ~~快慢双系统~~ ✓
4. ~~记忆系统~~ ✓
5. ~~Capability 生命周期~~ ✓
6. ~~代码生成引擎~~ ✓
7. ~~身份系统~~ ✓
8. ~~对话管理 + 用户反馈闭环~~ ✓
9. ~~资源空间管理 + 环境感知~~ ✓
10. ~~端到端闭环集成测试~~ ✓
11. ~~CLI 交互层 + LLM 接入~~ ✓

**当前状态**：v1 闭环已完成，iris 可通过 `CLAUDE_MODEL + ANTHROPIC_API_KEY` 等 provider 环境变量直接运行对话。

## 13. v2 演进方向（不在 v1 范围内）

- **自适应参数**：adaptive_params 表 + Self-Critic 反馈调整认知参数
- **多维身份向量**：CapabilityVector / CharacterVector / ValueVector + 微量漂移
- **元认知层**：MetaCogSnapshot + CognitiveTrendEvent + 自我怀疑机制
- **内部模拟器**：imagination.rs 虚拟场景生成
- **三层存储**：hot/warm/cold 分层 + 自动数据迁移
- **多进程/多节点扩容**：process_topology + node_registry
- **能力合并**：merge_pack.rs 相似 capability 合并
- **梦境机制**：RestMode 期间非结构化联想

## 14. 开发门禁

- 重大架构变更前需更新本文档
- 对话内容永不记录到日志
