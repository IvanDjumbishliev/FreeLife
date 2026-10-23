//! FreeLife relay.
//!
//! Peers that can't reach each other on the LAN meet in a "room" on the
//! relay and exchange end-to-end encrypted frames through it. The relay
//! forwards opaque bytes and never sees plaintext.
//!
//! Only the HTTP skeleton exists so far; the WebSocket forwarding is a TODO.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default, Clone)]
pub struct AppState {
    rooms: Arc<RwLock<HashSet<Uuid>>>,
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
    pub protocol: u16,
}

#[derive(Debug, Serialize)]
pub struct Room {
    pub id: Uuid,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/rooms", post(create_room))
        .route("/v1/rooms/{id}", get(get_room))
        .with_state(state)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        protocol: freelife_core::PROTOCOL_VERSION,
    })
}

async fn create_room(State(state): State<AppState>) -> (StatusCode, Json<Room>) {
    let id = Uuid::new_v4();
    state.rooms.write().await.insert(id);
    (StatusCode::CREATED, Json(Room { id }))
}

async fn get_room(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Room>, StatusCode> {
    if state.rooms.read().await.contains(&id) {
        Ok(Json(Room { id }))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(app: Router, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
        let res = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let (status, body) = call(router(AppState::default()), "GET", "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn created_rooms_can_be_fetched() {
        let state = AppState::default();
        let (status, body) = call(router(state.clone()), "POST", "/v1/rooms").await;
        assert_eq!(status, StatusCode::CREATED);
        let id = body["id"].as_str().unwrap();

        let (status, _) = call(router(state.clone()), "GET", &format!("/v1/rooms/{id}")).await;
        assert_eq!(status, StatusCode::OK);

        let missing = Uuid::new_v4();
        let (status, _) = call(router(state), "GET", &format!("/v1/rooms/{missing}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
