use std::{
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex, PoisonError},
};

use askama::Template;
use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing, Router,
};

use pdtcore::*;
mod server;

use server::{SendError, Server};
use tracing::{metadata::LevelFilter, *};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};
use ulid::Ulid;

type ServerReference = Arc<Mutex<Server>>;
type AppStateReference = Arc<Mutex<AppState>>;

impl AppState {
    fn reference(server_reference: ServerReference) -> AppStateReference {
        Arc::new(Mutex::new(Self {
            server: server_reference,
        }))
    }
}

impl From<Server> for ServerReference {
    fn from(value: Server) -> Self {
        Arc::new(Mutex::new(value))
    }
}

struct AppState {
    server: ServerReference,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    style: String,
    script: String,
    device_names: Vec<String>,
}

enum AppError {
    Deadlock,
    ServerSend(SendError),
}

#[derive(Debug)]
enum StartupError {
    Tracing,
    TcpBindAddress(std::io::Error),
    Mutex,
    AxumServe,
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(_value: PoisonError<T>) -> Self {
        AppError::Deadlock
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::Deadlock => {
                error!("could not acquire lock");

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unexpected error".to_string(),
                )
            }
            AppError::ServerSend(error) => {
                error!(error =? error, "send");

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unexpected error\n\nerror={:?}", error),
                )
            }
        }
        .into_response()
    }
}

async fn index(State(state): State<AppStateReference>) -> Result<IndexTemplate, AppError> {
    static STYLE: &str = include_str!("../static/style.min.css");
    static SCRIPT: &str = include_str!("../static/vendored/htmx.min.js");

    let app_state_guard = state.lock()?;
    let app_state = &*app_state_guard;

    let server_guard = app_state.server.lock()?;

    let server = &*server_guard;

    let client_ids = server
        .get_client_ids()?
        .into_iter()
        .map(String::from)
        .collect();

    let template = IndexTemplate {
        device_names: client_ids,
        style: STYLE.into(),
        script: SCRIPT.into(),
    };

    Ok(template)
}

#[debug_handler]
async fn screen_off(
    Path(client_id): Path<Ulid>,
    State(state): State<AppStateReference>,
) -> Result<String, AppError> {
    let state_guard = state.lock()?;

    let state = &*state_guard;

    let mut server_guard = state.server.lock()?;

    let server = &mut *server_guard;

    match server.send(client_id, Message::ScreenOff) {
        Ok(_) => Ok("OK".to_string()),
        Err(error) => Err(AppError::ServerSend(error)),
    }
}

fn setup_tracing() -> Result<(), StartupError> {
    let layer = tracing_logfmt::builder().with_target(false).layer();

    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        _ => EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .parse("")
            .map_err(|_| StartupError::Tracing)?,
    };

    let subscriber = Registry::default().with(layer).with(filter);
    tracing::subscriber::set_global_default(subscriber).map_err(|_| StartupError::Tracing)?;

    Ok(())
}

async fn serve_web_interface(server_reference: ServerReference) -> Result<(), StartupError> {
    let web_address = SocketAddr::from(([0, 0, 0, 0], 2040));

    let state = AppState::reference(server_reference);

    let web = Router::new()
        .route("/", routing::get(index))
        .route("/screen-off/:client_id", routing::get(screen_off))
        .with_state(state.clone());

    axum::Server::bind(&web_address)
        .serve(web.into_make_service())
        .await
        .map_err(|_| StartupError::AxumServe)?;

    Ok(())
}

fn spawn_tcp_server(server_reference: ServerReference) -> Result<(), StartupError> {
    let server_address = SocketAddr::from(([127, 0, 0, 1], 2039));

    let tcp_server_listener =
        TcpListener::bind(server_address).map_err(StartupError::TcpBindAddress)?;

    let mut guard = server_reference.lock().map_err(|_| StartupError::Mutex)?;
    let server = &mut *guard;

    info!(bound = ?server_address, "running server");
    server.run(tcp_server_listener);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    setup_tracing()?;

    let server = Server::default();
    let server_reference = ServerReference::from(server);

    spawn_tcp_server(server_reference.clone())?;
    serve_web_interface(server_reference).await
}
