use std::{
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex, PoisonError},
};

use askama::Template;
use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing, Router,
};
use pdtcore::*;
mod server;

use server::{SendError, Server};
use tracing::{metadata::LevelFilter, *};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};
use ulid::Ulid;

struct AppState {
    server: Arc<Mutex<Server>>,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    device_names: Vec<String>,
}

type SharedAppState = Arc<Mutex<AppState>>;

enum AppError {
    Deadlock,
    Template,
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
            AppError::Template => {
                error!("template error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unexpected error".to_string(),
                )
            }
        }
        .into_response()
    }
}

async fn index(State(state): State<SharedAppState>) -> Result<Html<String>, AppError> {
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
    };

    let template_str = template
        .render()
        .map_err(|_| AppError::Template)?
        .to_string();

    Ok(Html(template_str))
}

#[debug_handler]
async fn screen_off(
    Path(client_id): Path<Ulid>,
    State(state): State<SharedAppState>,
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

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    setup_tracing()?;

    let server = Server::new();
    let shared_server = Arc::new(Mutex::new(server));

    let state = Arc::new(Mutex::new(AppState {
        server: shared_server.clone(),
    }));

    let web = Router::new()
        .route("/", routing::get(index))
        .route("/screen-off/:client_id", routing::get(screen_off))
        .with_state(state.clone());

    let web_address = SocketAddr::from(([127, 0, 0, 1], 2040));
    let server_address = SocketAddr::from(([127, 0, 0, 1], 2039));

    let tcp_server_listener =
        TcpListener::bind(server_address).map_err(StartupError::TcpBindAddress)?;

    {
        info!("getting server lock");
        let mut guard = shared_server.lock().map_err(|_| StartupError::Mutex)?;

        let server = &mut *guard;
        info!(bound = ?server_address, "running server");
        server.run(tcp_server_listener);
        info!("release server lock");
    }

    axum::Server::bind(&web_address)
        .serve(web.into_make_service())
        .await
        .map_err(|_| StartupError::AxumServe)?;

    Ok(())
}
