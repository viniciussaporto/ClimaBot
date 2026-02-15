use crate::revolt::{RevoltClient, Role, Server};
use anyhow::Result;
use tracing::warn;

// ─────────────────────────────────────────────
//  Revolt server-level permission bits
//  https://docs.revolt.chat/developers/api/permissions
// ─────────────────────────────────────────────
const DANGEROUS_PERMISSIONS: u64 =
    (1 << 0) |  // ManageChannel
    (1 << 1) |  // ManageServer
    (1 << 2) |  // BanMembers
    (1 << 3) |  // KickMembers
    (1 << 4) |  // ManageRoles
    (1 << 25);  // ManageNicknames

// ─────────────────────────────────────────────
//  Entry point
// ─────────────────────────────────────────────

/// `!roles` — list available roles
/// `!roles <n>` — toggle role number n
/// `!roles add <n>` / `!roles remove <n>` — explicit toggle
pub async fn handle_roles(
    client: &RevoltClient,
    channel_id: &str,
    server_id: &str,
    user_id: &str,
    args: &[&str],
) -> Result<()> {
    let server = client.get_server(server_id).await?;

    match args.first().copied() {
        // No arguments → show list
        None | Some("list") => {
            list_roles(client, channel_id, &server).await?;
        }

        // `!roles add <n>` / `!roles remove <n>` / `!roles toggle <n>`
        Some("add") | Some("remove") | Some("toggle") => {
            match args.get(1) {
                Some(idx_str) => {
                    toggle_role(client, channel_id, server_id, user_id, &server, idx_str).await?;
                }
                None => {
                    client
                        .send_message(channel_id, "Usage: `!roles <number>`")
                        .await?;
                }
            }
        }

        // `!roles 3`  — direct shorthand
        Some(idx_str) if idx_str.parse::<usize>().is_ok() => {
            toggle_role(client, channel_id, server_id, user_id, &server, idx_str).await?;
        }

        _ => {
            client
                .send_message(
                    channel_id,
                    "Usage:\n`!roles` — list roles\n`!roles <number>` — toggle a role",
                )
                .await?;
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────

/// Returns `(role_id, role_name)` pairs sorted by rank (highest rank first),
/// excluding roles with dangerous permissions or system roles.
fn get_assignable_roles(server: &Server) -> Vec<(String, String)> {
    let roles = match &server.roles {
        Some(r) => r.clone(),
        None => return vec![],
    };

    let mut filtered: Vec<(String, String, i64)> = roles
        .into_iter()
        .filter(|(_, role)| {
            let dangerous = (role.permissions.a & DANGEROUS_PERMISSIONS) != 0;
            if dangerous {
                warn!(
                    role_name = %role.name,
                    "Excluding dangerous role from self-assignment"
                );
            }
            !dangerous
        })
        .map(|(id, role)| {
            let rank = role.rank.unwrap_or(0);
            (id, role.name.clone(), rank)
        })
        .collect();

    // Lower rank value = higher in hierarchy → sort descending by rank
    filtered.sort_by(|a, b| b.2.cmp(&a.2));
    filtered.into_iter().map(|(id, name, _)| (id, name)).collect()
}

async fn list_roles(client: &RevoltClient, channel_id: &str, server: &Server) -> Result<()> {
    let roles = get_assignable_roles(server);

    if roles.is_empty() {
        client
            .send_message(channel_id, "No self-assignable roles in this server.")
            .await?;
        return Ok(());
    }

    let mut msg = format!("## 🎭 Available Roles ({} total)\n\n", roles.len());
    for (i, (_, name)) in roles.iter().enumerate() {
        msg.push_str(&format!("**{}**. {}\n", i + 1, name));
    }
    msg.push_str("\nType `!roles <number>` to toggle a role.");

    client.send_message(channel_id, &msg).await?;
    Ok(())
}

async fn toggle_role(
    client: &RevoltClient,
    channel_id: &str,
    server_id: &str,
    user_id: &str,
    server: &Server,
    idx_str: &str,
) -> Result<()> {
    let roles = get_assignable_roles(server);

    if roles.is_empty() {
        client
            .send_message(channel_id, "No self-assignable roles available.")
            .await?;
        return Ok(());
    }

    let idx = match idx_str.parse::<usize>() {
        Ok(n) if n >= 1 && n <= roles.len() => n - 1,
        _ => {
            client
                .send_message(
                    channel_id,
                    &format!(
                        "❌ Invalid number. Choose between **1** and **{}**.",
                        roles.len()
                    ),
                )
                .await?;
            return Ok(());
        }
    };

    let (role_id, role_name) = &roles[idx];

    let member = client.get_member(server_id, user_id).await?;
    let mut current_roles = member.roles.unwrap_or_default();

    if current_roles.contains(role_id) {
        current_roles.retain(|r| r != role_id);
        client
            .edit_member_roles(server_id, user_id, current_roles)
            .await?;
        client
            .send_message(
                channel_id,
                &format!("🗑️ Removed role **{role_name}**."),
            )
            .await?;
    } else {
        current_roles.push(role_id.clone());
        client
            .edit_member_roles(server_id, user_id, current_roles)
            .await?;
        client
            .send_message(
                channel_id,
                &format!("✅ Added role **{role_name}**."),
            )
            .await?;
    }

    Ok(())
}
