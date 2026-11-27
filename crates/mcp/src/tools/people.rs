//! `get_user` and `search_users`.

use std::sync::Arc;

use domain::UserId;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    audit::{AuditSpan, target_user},
    dto::UserOut,
    errors::ToolFailure,
    ids::parse_workspace,
    policy::Policy,
    render::{capped, ok_result, user_out},
    server::Shared,
};

const QUERY_MAX_CHARS: usize = 256;
const LIMIT_DEFAULT: u32 = 20;
const LIMIT_MAX: u32 = 50;
const USER_ID_MAX: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetUserArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// User id, as seen in an author field.
    pub user: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchUsersArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Part of a display name or real name (1 to 256 characters).
    pub query: String,
    /// At most this many (default 20, max 50).
    pub limit: Option<u32>,
}

#[derive(Serialize)]
struct UsersOut<'a> {
    users: &'a [UserOut],
    truncated: bool,
}

impl Shared {
    pub(crate) async fn get_user(
        &self,
        span: &mut AuditSpan,
        args: GetUserArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(&args.workspace)?;
        span.workspace(ws.clone());
        if args.user.is_empty() || args.user.len() > USER_ID_MAX {
            return Err(ToolFailure::bad_argument("user must be a user id"));
        }
        span.target(target_user(&args.user));
        policy.workspace(&ws)?;
        let m = self
            .backend
            .member(&ws, &UserId(Arc::from(args.user.as_str())))
            .await?
            .ok_or_else(ToolFailure::unknown)?;
        Ok(ok_result(&user_out(&ws, &m)))
    }

    pub(crate) async fn search_users(
        &self,
        span: &mut AuditSpan,
        args: SearchUsersArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let query = args.query.trim().to_string();
        if query.is_empty() || query.chars().count() > QUERY_MAX_CHARS {
            return Err(ToolFailure::bad_argument(format!(
                "query must be 1 to {QUERY_MAX_CHARS} characters"
            )));
        }
        span.content(|| query.clone());
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(&args.workspace)?;
        span.workspace(ws.clone());
        policy.workspace(&ws)?;
        let limit = args.limit.unwrap_or(LIMIT_DEFAULT).clamp(1, LIMIT_MAX);
        let members = self.backend.search_members(&ws, &query, limit).await?;
        let out: Vec<UserOut> = members
            .iter()
            .take(limit as usize)
            .map(|m| user_out(&ws, m))
            .collect();
        Ok(capped(&out, false, |users, truncated| UsersOut {
            users,
            truncated,
        }))
    }
}
