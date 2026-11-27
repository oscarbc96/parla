//! Our own serde HAR structs. Unlike generic HAR crates they keep Chrome's
//! `_webSocketMessages`, which carry realtime frames.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Har {
    pub log: Log,
}

#[derive(Debug, Deserialize)]
pub struct Log {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub request: Request,
    #[serde(default)]
    pub response: Response,
    #[serde(rename = "_webSocketMessages", default)]
    pub web_socket_messages: Vec<WsMessage>,
    #[serde(rename = "_resourceType", default)]
    pub resource_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<NameValue>,
    #[serde(default)]
    pub query_string: Vec<NameValue>,
    #[serde(default)]
    pub post_data: Option<PostData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub headers: Vec<NameValue>,
    #[serde(default)]
    pub content: Content,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostData {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub params: Vec<NameValue>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NameValue {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsMessage {
    /// `send` or `receive`.
    #[serde(rename = "type", default)]
    pub direction: String,
    #[serde(default)]
    pub opcode: i64,
    #[serde(default)]
    pub data: String,
}

pub fn parse(text: &str) -> Result<Har, serde_json::Error> {
    serde_json::from_str(text)
}
