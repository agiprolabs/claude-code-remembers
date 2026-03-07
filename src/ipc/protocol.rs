use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    #[serde(rename = "ingest")]
    Ingest(IngestParams),

    #[serde(rename = "get_context")]
    GetContext(GetContextParams),

    #[serde(rename = "get_status")]
    GetStatus,

    #[serde(rename = "end_session")]
    EndSession(EndSessionParams),

    #[serde(rename = "search")]
    Search(SearchParams),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngestParams {
    pub content: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetContextParams {
    pub max_tokens: usize,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EndSessionParams {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum Response {
    #[serde(rename = "ok")]
    Ok { data: serde_json::Value },

    #[serde(rename = "error")]
    Error { message: String },
}

impl Response {
    pub fn ok(data: impl Serialize) -> Self {
        Response::Ok {
            data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Response::Error {
            message: msg.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusData {
    pub total_memories: i64,
    pub total_consolidations: i64,
    pub memories_by_type: Vec<(String, i64)>,
    pub last_consolidation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextData {
    pub context: String,
    pub token_estimate: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngestResult {
    pub memory_id: i64,
    pub deduplicated: bool,
}
