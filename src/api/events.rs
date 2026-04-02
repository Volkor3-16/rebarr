use rocket::get;
use rocket::response::stream::{Event as RocketEvent, EventStream};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// Capacity for the broadcast channel.  Clients that fall behind by this many
/// messages will be disconnected and can reconnect to get a fresh snapshot.
const CHANNEL_CAPACITY: usize = 64;

/// Lazy-initialised broadcast sender.  All producers call `sender().send(...)`
/// and the SSE endpoint clones the receiver via `sender().subscribe()`.
static SENDER: std::sync::LazyLock<broadcast::Sender<String>> =
    std::sync::LazyLock::new(|| {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        tx
    });

fn sender() -> &'static broadcast::Sender<String> {
    &SENDER
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TaskUpdate {
    pub id: String,
    pub task_type: String,
    pub status: String,
    pub manga_title: Option<String>,
    pub chapter_number_raw: Option<String>,
    pub last_error: Option<String>,
}

/// Emit a task update event.  Call this from the DB layer or worker whenever a
/// task changes state (pending → running → completed/failed/cancelled).
pub fn emit_task_update(update: &TaskUpdate) {
    if let Ok(json) = serde_json::to_string(update) {
        // Ignore error if there are no receivers — that's fine.
        let _ = sender().send(json);
    }
}

// ---------------------------------------------------------------------------
// SSE endpoint
// ---------------------------------------------------------------------------

#[get("/api/events")]
pub fn events() -> EventStream![] {
    let rx = sender().subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| {
        match item {
            Ok(data) => Some(RocketEvent::data(data)),
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::debug!("SSE client lagged by {n} messages");
                None // skip lagged messages, keep the stream alive
            }
        }
    });

    EventStream! {
        // Send an initial heartbeat so the client knows the connection is live
        yield RocketEvent::data("ping");

        // Forward all broadcast events to the SSE stream
        for await event in stream {
            yield event;
        }
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![events]
}