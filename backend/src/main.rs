//! 网站管理套件入口：仅通过受信平台代理访问管理 API。
mod api;
mod certificates;
mod engine;
mod error;
mod files;
mod local_ca;
mod maintenance;
mod migration;
mod model;
mod performance;
mod store;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;
use veydeck_suite_runtime::RuntimeClient;

/// 单节点套件状态，不包含 Docker socket 或宿主机命令执行器。
pub struct App {
    root: PathBuf,
    instance: PathBuf,
    store: Mutex<store::Store>,
    runtime: Option<RuntimeClient>,
    publish_lock: Arc<AsyncMutex<()>>,
    file_lock: Mutex<()>,
    entry_token: Option<String>,
}
impl App {
    /// 根据稳定站点标识定位固定 root。
    fn site_root(&self, site: &model::Site) -> PathBuf {
        self.root.join("sites").join(&site.directory).join("index")
    }
    /// 获取数据库，锁中毒返回明确内部错误。
    fn db(&self) -> error::Result<std::sync::MutexGuard<'_, store::Store>> {
        self.store
            .lock()
            .map_err(|_| error::Error::bad("DATABASE_UNAVAILABLE", "数据库状态不可用"))
    }
}
/// 校验 Agent 注入的入口凭据，阻止绕过主控直接访问管理服务。
async fn authenticate(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    request: Request<Body>,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    if request.uri().path() == "/health" {
        return Ok(next.run(request).await);
    }
    if let Some(expected) = &app.entry_token {
        let token = request
            .headers()
            .get("x-veydeck-entry-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if token.as_bytes() != expected.as_bytes() {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(request).await)
}
/// 初始化目录、恢复中断发布后启动 HTTP 服务。
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let dev = std::env::var("VEYDECK_WEBSITES_DEV").as_deref() == Ok("1");
    let root =
        PathBuf::from(std::env::var("VEYDECK_WEBSITE_DATA_DIR").unwrap_or_else(|_| "/data".into()));
    let instance_id = std::env::var("VEYDECK_SUITE_INSTANCE_ID")
        .unwrap_or_else(|_| "00000000-0000-0000-0000-000000000001".into());
    uuid::Uuid::parse_str(&instance_id)?;
    let instance = root.join("instances").join(instance_id);
    for folder in ["state", "certificates", "releases", "logs"] {
        std::fs::create_dir_all(instance.join(folder))?;
    }
    std::fs::create_dir_all(root.join("sites"))?;
    let entry_token = if dev {
        None
    } else {
        let token = std::fs::read_to_string(std::env::var("VEYDECK_ENTRY_TOKEN_FILE")?)?;
        if token.trim().is_empty() {
            return Err("入口凭据为空".into());
        }
        Some(token.trim().into())
    };
    let runtime = if dev {
        None
    } else {
        Some(RuntimeClient::from_environment("services.control").await?)
    };
    let store = store::Store::open(&instance.join("state/websites.db"))?;
    let app = Arc::new(App {
        root,
        instance,
        store: Mutex::new(store),
        runtime,
        publish_lock: Arc::new(AsyncMutex::new(())),
        file_lock: Mutex::new(()),
        entry_token,
    });
    api::recover(&app)?;
    migration::migrate(&app)?;
    migration::cleanup_removed_protection(&app)?;
    certificates::refresh_metadata(&app)?;
    local_ca::initialize(&app)?;
    let startup_guard = app.publish_lock.lock().await;
    if let Some(active) = engine::current(&app.instance)
        && let Some(release) = app.db()?.releases()?.iter().find(|r| r.id == active)
    {
        engine::render(&app, &active, &release.sites, &release.performance)?;
    }
    if engine::current(&app.instance).is_none() {
        let id = uuid::Uuid::now_v7().to_string();
        engine::render(&app, &id, &[], &app.db()?.performance()?)?;
        engine::activate(&app.instance, &id)?;
    }
    // 管理容器重启时引擎可能仍运行，恢复文件引用后同步真实 worker。
    if app.runtime.is_some() && engine::control(&app, "status", None).await.is_ok() {
        engine::control(&app, "reload", None).await?;
        if let Some(active) = engine::current(&app.instance) {
            engine::confirm(&app, &active).await?;
        }
    }
    drop(startup_guard);
    tokio::spawn(maintenance::rotate_logs(app.clone()));
    let static_dir =
        std::env::var("VEYDECK_FRONTEND_DIR").unwrap_or_else(|_| "frontend/dist".into());
    let routes = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(api::router())
        .fallback_service(
            tower_http::services::ServeDir::new(static_dir).append_index_html_on_directories(true),
        )
        .layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app.clone());
    let address = if dev {
        "127.0.0.1:8080"
    } else {
        "0.0.0.0:8080"
    };
    let address = std::env::var("VEYDECK_WEBSITE_LISTEN").unwrap_or_else(|_| address.into());
    let socket_address: std::net::SocketAddr = address.parse()?;
    if dev && !socket_address.ip().is_loopback() {
        return Err("开发模式只允许 loopback 监听".into());
    }
    let listener = tokio::net::TcpListener::bind(socket_address).await?;
    tracing::info!(address, "网站管理服务启动");
    axum::serve(listener, routes).await?;
    Ok(())
}
