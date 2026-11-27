//! Cursor pagination shared by the list endpoints.

use futures::Stream;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::{ApiError, Method, Priority, WebClient};

/// `response_metadata` of a paged response. An empty cursor means done.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ResponseMetadata {
    pub next_cursor: Option<String>,
}

/// Pages fetched at most, whatever the server says: a stuck cursor must not loop forever.
const MAX_PAGES: u32 = 1000;

/// A response that may carry a continuation cursor (empty string means done).
pub trait Paged: DeserializeOwned {
    fn next_cursor(&self) -> Option<&str>;
}

impl WebClient {
    /// Follow `response_metadata.next_cursor` until it is empty, one item per page.
    pub fn pages<'a, R: Paged + 'a>(
        &'a self,
        method: Method,
        params: Vec<(String, String)>,
        prio: Priority,
    ) -> impl Stream<Item = Result<R, ApiError>> + 'a {
        // State: `Some((cursor, pages so far))` is the next page to fetch (`""` for the first),
        // `None` is done.
        futures::stream::unfold(Some((String::new(), 0u32)), move |state| {
            let params = params.clone();
            async move {
                let (cursor, n) = state?;
                let mut form: Vec<(&str, &str)> = params
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                if !cursor.is_empty() {
                    form.push(("cursor", cursor.as_str()));
                }
                match self.call::<R>(method, &form, prio).await {
                    Ok(page) => {
                        // A cursor that repeats, or too many pages, ends the stream.
                        let next = page
                            .next_cursor()
                            .filter(|c| !c.is_empty() && *c != cursor && n + 1 < MAX_PAGES)
                            .map(|c| (c.to_string(), n + 1));
                        Some((Ok(page), next))
                    }
                    Err(e) => Some((Err(e), None)),
                }
            }
        })
    }
}
