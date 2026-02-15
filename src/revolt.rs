use anyhow::{anyhow, Result};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────
//  Revolt REST base URL
// ─────────────────────────────────────────────
const REVOLT_API: &str = "https://api.revolt.chat";

// ─────────────────────────────────────────────
//  REST payload types
// ─────────────────────────────────────────────

/// A message received from (or sent to) a Revolt channel.
#[derive(Debug, Deserialize, Clone)]
pub struct Message {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel: String,
    pub author: String,
    pub content: Option<String>,
}

/// A Revolt channel (TextChannel, VoiceChannel, etc.)
#[derive(Debug, Deserialize, Clone)]
pub struct Channel {
    #[serde(rename = "_id")]
    pub id: String,
    /// Only present when the channel belongs to a server
    pub server: Option<String>,
    pub name: Option<String>,
}

/// A server (equiv. of a Discord guild)
#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    /// Map of role_id → Role
    pub roles: Option<HashMap<String, Role>>,
}

/// A role inside a Revolt server.
#[derive(Debug, Deserialize, Clone)]
pub struct Role {
    pub name: String,
    pub permissions: RolePermissions,
    pub colour: Option<String>,
    /// Lower rank value = higher in hierarchy
    pub rank: Option<i64>,
}

/// Revolt uses separate `a` (allow) and `d` (deny) permission bitfields.
#[derive(Debug, Deserialize, Clone)]
pub struct RolePermissions {
    pub a: u64, // allowed bits
    pub d: u64, // denied bits
}

/// A server member — contains which roles the member currently has.
#[derive(Debug, Deserialize, Clone)]
pub struct Member {
    #[serde(rename = "_id")]
    pub id: MemberId,
    pub roles: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemberId {
    pub server: String,
    pub user: String,
}

// ─────────────────────────────────────────────
//  WebSocket gateway event envelope
// ─────────────────────────────────────────────

/// All events the Revolt gateway can send us.
/// We only handle the ones we care about; the rest are silently ignored.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum RevoltEvent {
    /// Authentication accepted
    Authenticated,
    /// Initial state payload — we only deserialise channels; all other fields
    /// (users, servers, members, emojis) are silently ignored by serde.
    Ready {
        channels: Vec<Channel>,
    },
    /// A new message was posted
    Message(Message),
    /// Keep-alive pong
    Pong {
        data: serde_json::Value,
    },
    /// Anything we haven't explicitly modelled
    #[serde(other)]
    Unknown,
}

// ─────────────────────────────────────────────
//  REST client
// ─────────────────────────────────────────────

/// Thin wrapper around reqwest that automatically adds the
/// `X-Bot-Token` header required by every Revolt REST request.
#[derive(Clone)]
pub struct RevoltClient {
    http: HttpClient,
    token: String,
}

impl RevoltClient {
    pub fn new(token: String) -> Self {
        Self {
            http: HttpClient::new(),
            token,
        }
    }

    // ── Messaging ──────────────────────────────

    /// Post a plain-text / Markdown message to a channel.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let body = serde_json::json!({ "content": content });

        let res = self
            .http
            .post(format!("{REVOLT_API}/channels/{channel_id}/messages"))
            .header("X-Bot-Token", &self.token)
            .json(&body)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!("send_message failed [{status}]: {text}"));
        }
        Ok(())
    }

    // ── Server / Roles ─────────────────────────

    /// Fetch a server object (includes its roles map).
    pub async fn get_server(&self, server_id: &str) -> Result<Server> {
        Ok(self
            .http
            .get(format!("{REVOLT_API}/servers/{server_id}"))
            .header("X-Bot-Token", &self.token)
            .send()
            .await?
            .json::<Server>()
            .await?)
    }

    /// Fetch a specific server member.
    pub async fn get_member(&self, server_id: &str, user_id: &str) -> Result<Member> {
        Ok(self
            .http
            .get(format!("{REVOLT_API}/servers/{server_id}/members/{user_id}"))
            .header("X-Bot-Token", &self.token)
            .send()
            .await?
            .json::<Member>()
            .await?)
    }

    /// Overwrite the complete role list for a member.
    /// Pass the full desired role ID vec (Revolt replaces, not appends).
    pub async fn edit_member_roles(
        &self,
        server_id: &str,
        user_id: &str,
        roles: Vec<String>,
    ) -> Result<()> {
        let body = serde_json::json!({ "roles": roles });

        let res = self
            .http
            .patch(format!("{REVOLT_API}/servers/{server_id}/members/{user_id}"))
            .header("X-Bot-Token", &self.token)
            .json(&body)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!("edit_member_roles failed [{status}]: {text}"));
        }
        Ok(())
    }
}
