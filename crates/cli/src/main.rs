mod event;
mod tui;
mod widgets;

use iris_llm::provider::LlmProvider;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    const DB_CONNECT_TIMEOUT_SECS: u64 = 3;

    // Panic hook: restore terminal even on panic in raw mode
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        default_hook(info);
    }));

    // Tracing: write to file when RUST_LOG is set (raw mode breaks stderr)
    if std::env::var("RUST_LOG").is_ok() {
        let file = std::fs::File::create("/tmp/iris.log")?;
        tracing_subscriber::registry()
            .with(EnvFilter::from_default_env())
            .with(fmt::layer().json().with_target(true).with_writer(file))
            .init();
    }

    // DATABASE_URL (optional — no DB = ephemeral mode)
    let mut startup_notice: Option<String> = None;
    let pool = match std::env::var("DATABASE_URL") {
        Ok(url) => {
            let connect_result = tokio::time::timeout(
                std::time::Duration::from_secs(DB_CONNECT_TIMEOUT_SECS),
                sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect(&url),
            )
            .await;

            match connect_result {
                Ok(Ok(pool)) => match sqlx::migrate!("../../migrations").run(&pool).await {
                    Ok(()) => {
                        tracing::info!("database connected and migrations applied");
                        Some(pool)
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "database migration failed — falling back to ephemeral mode"
                        );
                        startup_notice = Some(
                                "提示：数据库迁移失败，已自动降级为临时模式（ephemeral）。本次会话数据不会持久化。".to_string()
                            );
                        None
                    }
                },
                Ok(Err(e)) => {
                    tracing::warn!(
                        error = %e,
                        "failed to connect DATABASE_URL — falling back to ephemeral mode"
                    );
                    startup_notice = Some(
                        "提示：无法连接 DATABASE_URL，已自动降级为临时模式（ephemeral）。本次会话数据不会持久化。".to_string()
                    );
                    None
                }
                Err(_) => {
                    tracing::warn!(
                        timeout_secs = DB_CONNECT_TIMEOUT_SECS,
                        "database connect timed out — falling back to ephemeral mode"
                    );
                    startup_notice = Some(format!(
                        "提示：连接数据库超时（{}s），已自动降级为临时模式（ephemeral）。本次会话数据不会持久化。",
                        DB_CONNECT_TIMEOUT_SECS
                    ));
                    None
                }
            }
        }
        Err(_) => {
            tracing::warn!("DATABASE_URL not set — running in ephemeral mode");
            None
        }
    };

    // Load IrisCfg from DB or use defaults
    let cfg = if let Some(ref pool) = pool {
        iris_core::config::IrisCfg::load(pool).await?
    } else {
        iris_core::config::IrisCfg::default()
    };
    let cfg = std::sync::Arc::new(cfg);

    // LLM provider from env vars
    let llm: Option<std::sync::Arc<dyn iris_llm::provider::LlmProvider>> =
        iris_llm::http::from_env().map(|p| {
            tracing::info!(name = p.name(), "LLM provider initialized");
            std::sync::Arc::new(p) as _
        });

    // Optional weak model for tool gating (route/no-route and tool selection).
    // Configure via IRIS_LLM_LITE_MODEL; uses same API key/base URL.
    let lite_llm: Option<std::sync::Arc<dyn iris_llm::provider::LlmProvider>> =
        iris_llm::http::from_env_with_model_var("IRIS_LLM_LITE_MODEL").map(|p| {
            tracing::info!(name = p.name(), "lite LLM initialized");
            std::sync::Arc::new(p) as _
        });
    if lite_llm.is_none() {
        tracing::warn!("IRIS_LLM_LITE_MODEL not set or invalid; tool routing will use main LLM");
    }

    // Create runtime (now returns 4-tuple with status_rx)
    let (mut runtime, event_tx, output_rx, status_rx) =
        iris_core::runtime::Runtime::new(cfg, pool, llm, lite_llm);
    let token = runtime.token();

    // Runtime is !Send (tracing EnteredSpan), so run both futures on the same task.
    // If TUI exits first, cancel runtime and wait for graceful runtime shutdown logs.
    // If runtime exits first, cancel TUI and wait for terminal cleanup.
    let tui_token = token.clone();
    let runtime_fut = runtime.run();
    let tui_fut = tui::run_app(event_tx, output_rx, status_rx, tui_token, startup_notice);
    tokio::pin!(runtime_fut);
    tokio::pin!(tui_fut);

    let mut runtime_done = false;
    let mut tui_result: Option<anyhow::Result<()>> = None;

    loop {
        tokio::select! {
            _ = &mut runtime_fut, if !runtime_done => {
                runtime_done = true;
                token.cancel();
                if tui_result.is_none() {
                    tui_result = Some((&mut tui_fut).await);
                }
            }
            result = &mut tui_fut, if tui_result.is_none() => {
                tui_result = Some(result);
                token.cancel();
            }
        }

        if runtime_done && tui_result.is_some() {
            break;
        }
    }

    tui_result.unwrap_or(Ok(()))
}
