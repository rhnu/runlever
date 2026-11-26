use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::error;

use crate::auth::{self, Sessions};
use runlever::{
    config::{Catalog, SelectionLabel},
    engine::{Engine, Run, RunFilter, StartError},
};

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Frontend;

#[derive(Clone)]
struct App {
    catalog: Arc<Catalog>,
    engine: Engine,
    sessions: Arc<Sessions>,
}

#[derive(Serialize)]
struct RunView<'a> {
    #[serde(flatten)]
    run: &'a Run,
    selection_labels: BTreeMap<&'a str, SelectionLabel<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_label: Option<&'a str>,
}

impl App {
    fn run_view<'a>(&'a self, run: &'a Run) -> RunView<'a> {
        RunView {
            run,
            selection_labels: self.catalog.selection_labels(
                &run.action,
                &run.target,
                &run.selections,
            ),
            target_label: self.catalog.action_label(&run.target),
        }
    }
}

type Reply = Result<Response, ApiError>;
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

fn internal(cause: anyhow::Error) -> ApiError {
    error!(error = ?cause, "request failed");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "服务处理请求失败".into())
}

fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "记录或入口不存在".into())
}
fn unauthorized() -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, "认证失效或凭据无效".into())
}

pub fn router(catalog: Arc<Catalog>, engine: Engine) -> Router {
    let app = App {
        catalog,
        engine,
        sessions: Arc::new(Sessions::default()),
    };
    let protected = Router::new()
        .route("/api/actions", get(actions))
        .route("/api/actions/{id}/runs", post(web_run))
        .route("/api/runs", get(runs))
        .route("/api/runs/{id}", get(run))
        .route("/api/runs/{id}/events", get(events))
        .route("/api/logout", post(logout))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_session));
    Router::new()
        .merge(protected)
        .route("/api/login", post(login))
        .route("/hooks/{id}", post(hook))
        .route("/", get(index))
        .route("/assets/{*file}", get(asset))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(response_headers))
        .with_state(app)
}

async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("frame-ancestors 'none'; base-uri 'none'; object-src 'none'"),
    );
    response
}

fn check_origin(app: &App, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) == Some(app.catalog.origin()) {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::FORBIDDEN,
            "请求来源与配置的网页地址不一致".into(),
        ))
    }
}

async fn require_session(State(app): State<App>, request: Request, next: Next) -> Reply {
    if !auth::cookie(request.headers()).is_some_and(|key| app.sessions.active(key)) {
        return Err(unauthorized());
    }
    if request.method() == Method::POST {
        check_origin(&app, request.headers())?;
    }
    Ok(next.run(request).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    token: String,
}

async fn login(State(app): State<App>, headers: HeaderMap, Json(login): Json<Login>) -> Reply {
    check_origin(&app, &headers)?;
    if !auth::matches_token(&login.token, app.catalog.admin_token()) {
        return Err(unauthorized());
    }
    let key = app.sessions.issue().map_err(internal)?;
    if let Some(old) = auth::cookie(&headers) {
        app.sessions.remove(old);
    }
    Ok(session_cookie(&key, auth::SESSION_LIFETIME.as_secs()))
}

fn session_cookie(value: &str, age: u64) -> Response {
    let cookie = format!(
        "{}={}; Path=/; Max-Age={age}; HttpOnly; Secure; SameSite=Strict",
        auth::COOKIE,
        value
    );
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(key) = auth::cookie(&headers) {
        app.sessions.remove(key);
    }
    session_cookie("", 0)
}

async fn actions(State(app): State<App>) -> Response {
    Json(app.catalog.web_actions()).into_response()
}

async fn web_run(State(app): State<App>, Path(id): Path<String>, body: Bytes) -> Reply {
    if !app.catalog.has_web_action(&id) {
        return Err(missing());
    }
    start(app, id, &body, "web").await
}

async fn hook(State(app): State<App>, Path(id): Path<String>, request: Request) -> Reply {
    let expected = app.catalog.webhook_token(&id).ok_or_else(missing)?;
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, value)| value);
    if !token.is_some_and(|v| auth::matches_token(v, expected)) {
        return Err(unauthorized());
    }
    let body = to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "请求正文无效或过大".into()))?;
    start(app, id, &body, "webhook").await
}

async fn start(app: App, id: String, body: &[u8], source: &str) -> Reply {
    let inputs = if body.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_slice(body).map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "参数必须是由枚举 ID 组成的 JSON 对象".into(),
            )
        })?
    };
    let invocation = app
        .catalog
        .resolve(&id, &inputs)
        .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    if app.catalog.log_request_params() {
        tracing::info!(action = %id, source, selections = ?inputs, "validated action request");
    } else {
        tracing::info!(action = %id, source, "validated action request");
    }
    match app.engine.start(invocation, source).await {
        Ok(id) => Ok((StatusCode::ACCEPTED, Json(json!({"run_id": id}))).into_response()),
        Err(StartError::QueueFull) => Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "等待队列已满，请稍后再试。".into(),
        )),
        Err(StartError::Other(cause)) => Err(internal(cause)),
    }
}

async fn runs(State(app): State<App>, Query(filter): Query<RunFilter>) -> Reply {
    let runs = app.engine.list(filter).await.map_err(internal)?;
    let views: Vec<_> = runs.iter().map(|run| app.run_view(run)).collect();
    Ok(Json(views).into_response())
}

async fn run(State(app): State<App>, Path(id): Path<i64>) -> Reply {
    let run = app
        .engine
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    Ok(Json(app.run_view(&run)).into_response())
}

#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: i64,
}

async fn events(
    State(app): State<App>,
    Path(id): Path<i64>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Reply {
    app.engine
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    let key = auth::cookie(&headers).ok_or_else(unauthorized)?.to_owned();
    let last_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut after = cursor.after.max(last_id).max(0);
    let stream = async_stream::stream! {
        let mut active_status = None;
        loop {
            if !app.sessions.active(&key) {
                yield Ok::<_, axum::Error>(Event::default().event("auth").data("expired"));
                break;
            }
            // Check state first: observing a terminal state guarantees its final log rows are committed.
            let state = match app.engine.get(id).await {
                Ok(Some(run)) => run,
                Ok(None) => break,
                Err(cause) => { error!(run_id=id, error=?cause, "read stream status"); break; }
            };
            if state.is_active() && active_status.as_ref() != Some(&state.status) {
                active_status = Some(state.status.clone());
                yield Event::default().event("status").json_data(app.run_view(&state));
            }
            let batch = match app.engine.events(id, after).await {
                Ok(batch) => batch,
                Err(cause) => { error!(run_id=id, error=?cause, "read stream events"); break; }
            };
            if !batch.is_empty() {
                for entry in batch {
                    after = entry.seq;
                    yield Event::default().event("log").id(after.to_string()).json_data(entry);
                }
                continue;
            }
            if !state.is_active() {
                // Retention cleanup may have removed the last batch after the status read.
                let latest = match app.engine.get(id).await {
                    Ok(Some(run)) => run,
                    Ok(None) => break,
                    Err(cause) => { error!(run_id=id, error=?cause, "read final stream status"); break; }
                };
                yield Event::default().event("done").json_data(app.run_view(&latest));
                break;
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    };
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

async fn index() -> Reply {
    embedded("index.html")
}
async fn asset(Path(file): Path<String>) -> Reply {
    embedded(&format!("assets/{file}"))
}

fn embedded(file: &str) -> Reply {
    let asset = Frontend::get(file).ok_or_else(missing)?;
    let mime = match file.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    let cache = if file.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Ok((
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        Body::from(asset.data.into_owned()),
    )
        .into_response())
}
