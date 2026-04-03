use rocket::{http::Status, serde::json::Json};
use schemars::JsonSchema;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct ApiError {
    pub error: String,
}

pub type ApiResult<T> = Result<Json<T>, (Status, Json<ApiError>)>;

pub fn err(status: Status, msg: impl ToString) -> (Status, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: msg.to_string(),
        }),
    )
}

pub fn internal(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::InternalServerError, msg)
}

pub fn bad_request(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::BadRequest, msg)
}

pub fn not_found(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::NotFound, msg)
}
