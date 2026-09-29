use crate::metrics;
use anyhow::Result;
use serenity::all::{
    ButtonStyle, CommandInteraction, ComponentInteraction, ComponentInteractionDataKind, Context,
    CreateActionRow, CreateButton, CreateInteractionResponse, CreateInteractionResponseMessage,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption, GuildId, Permissions,
    ReactionType, Role, RoleId,
};
use tracing::{debug, info, warn};

/// Roles granting any of these are never offered for self-assignment.
const DANGEROUS_PERMISSIONS: Permissions = Permissions::ADMINISTRATOR
    .union(Permissions::MANAGE_GUILD)
    .union(Permissions::MANAGE_CHANNELS)
    .union(Permissions::MANAGE_ROLES)
    .union(Permissions::MANAGE_WEBHOOKS)
    .union(Permissions::MANAGE_NICKNAMES)
    .union(Permissions::MANAGE_MESSAGES)
    .union(Permissions::MANAGE_THREADS)
    .union(Permissions::MANAGE_EVENTS)
    .union(Permissions::MANAGE_GUILD_EXPRESSIONS)
    .union(Permissions::KICK_MEMBERS)
    .union(Permissions::BAN_MEMBERS)
    .union(Permissions::MODERATE_MEMBERS)
    .union(Permissions::MENTION_EVERYONE)
    .union(Permissions::SEND_TTS_MESSAGES)
    .union(Permissions::MUTE_MEMBERS)
    .union(Permissions::DEAFEN_MEMBERS)
    .union(Permissions::MOVE_MEMBERS)
    .union(Permissions::VIEW_AUDIT_LOG);

/// Discord allows at most 25 options per select menu.
const ROLES_PER_PAGE: usize = 25;

/// Select menus use `role-select_<page>`; plain `role-select` (menus sent by
/// the previous bot version) is treated as page 0.
pub const SELECT_ID: &str = "role-select";
pub const PAGE_PREFIX: &str = "roles-";

// ─────────────────────────────────────────────
//  Role filtering
// ─────────────────────────────────────────────

/// Returns the roles members may assign to themselves, highest first.
///
/// Excludes `@everyone`, integration-managed roles, roles with dangerous
/// permissions, and anything at or above the bot's highest role (which the
/// bot could not assign anyway).
pub fn assignable_roles<'a>(roles: &'a [Role], guild_id: GuildId, bot_role_ids: &[RoleId]) -> Vec<&'a Role> {
    let bot_top_position = roles
        .iter()
        .filter(|r| bot_role_ids.contains(&r.id))
        .map(|r| r.position)
        .max()
        .unwrap_or(0);

    let mut result: Vec<&Role> = roles
        .iter()
        .filter(|role| {
            if role.id.get() == guild_id.get() || role.managed {
                return false;
            }
            if role.permissions.intersects(DANGEROUS_PERMISSIONS) {
                debug!(
                    role_id = %role.id,
                    role_name = %role.name,
                    permissions = ?(role.permissions & DANGEROUS_PERMISSIONS),
                    "Excluded dangerous role from assignment"
                );
                return false;
            }
            role.position < bot_top_position
        })
        .collect();

    result.sort_by(|a, b| b.position.cmp(&a.position).then(a.id.cmp(&b.id)));
    result
}

/// Fetch the guild's roles and the bot's own role list.
async fn load_roles(ctx: &Context, guild_id: GuildId) -> Result<(Vec<Role>, Vec<RoleId>)> {
    let bot_id = super::bot_user_id(ctx).await?;
    let (roles, me) = tokio::try_join!(
        ctx.http.get_guild_roles(guild_id),
        ctx.http.get_member(guild_id, bot_id),
    )?;
    Ok((roles, me.roles))
}

// ─────────────────────────────────────────────
//  Menu builder
// ─────────────────────────────────────────────

/// Build page `page` of the role menu, or `None` if there is nothing to show.
/// `notice` is shown above the menu (e.g. the result of the last toggle).
fn role_menu(roles: &[&Role], page: usize, notice: Option<&str>) -> Option<CreateInteractionResponseMessage> {
    let total_pages = roles.len().div_ceil(ROLES_PER_PAGE);
    if page >= total_pages {
        return None;
    }
    let start = page * ROLES_PER_PAGE;

    let options: Vec<CreateSelectMenuOption> = roles[start..]
        .iter()
        .take(ROLES_PER_PAGE)
        .map(|role| {
            let label: String = role.name.chars().take(100).collect();
            let emoji = role.unicode_emoji.clone().unwrap_or_else(|| "🔹".into());
            CreateSelectMenuOption::new(label, role.id.to_string()).emoji(ReactionType::Unicode(emoji))
        })
        .collect();

    let select = CreateSelectMenu::new(format!("{SELECT_ID}_{page}"), CreateSelectMenuKind::String { options })
        .placeholder(format!("Select a role to toggle (Page {}/{total_pages})", page + 1));

    let mut buttons = Vec::new();
    if page > 0 {
        buttons.push(
            CreateButton::new(format!("{PAGE_PREFIX}prev_{page}"))
                .label("Previous page")
                .style(ButtonStyle::Secondary),
        );
    }
    if start + ROLES_PER_PAGE < roles.len() {
        buttons.push(
            CreateButton::new(format!("{PAGE_PREFIX}next_{page}"))
                .label("Next page")
                .style(ButtonStyle::Primary),
        );
    }

    let mut components = vec![CreateActionRow::SelectMenu(select)];
    if !buttons.is_empty() {
        components.push(CreateActionRow::Buttons(buttons));
    }

    let header = format!("**Available Roles** ({} total)", roles.len());
    let content = match notice {
        Some(n) => format!("{n}

{header}"),
        None => header,
    };
    Some(
        CreateInteractionResponseMessage::new()
            .content(content)
            .components(components)
            .ephemeral(true),
    )
}

fn ephemeral(content: impl Into<String>) -> CreateInteractionResponse {
    CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content(content).ephemeral(true),
    )
}

/// The bot's guild-level permissions as reported by Discord on the interaction.
fn bot_can_manage_roles(app_permissions: Option<Permissions>) -> bool {
    app_permissions.is_none_or(|p| p.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_ROLES))
}

const MISSING_PERMISSION: &str =
    "I need the **Manage Roles** permission in this server to assign roles.";

// ─────────────────────────────────────────────
//  /roles
// ─────────────────────────────────────────────

pub async fn handle_command(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let Some(guild_id) = cmd.guild_id else {
        cmd.create_response(&ctx.http, ephemeral("This command only works inside a server.")).await?;
        return Ok(());
    };

    if !bot_can_manage_roles(cmd.app_permissions) {
        cmd.create_response(&ctx.http, ephemeral(MISSING_PERMISSION)).await?;
        return Ok(());
    }

    let (roles, bot_roles) = load_roles(ctx, guild_id).await?;
    let assignable = assignable_roles(&roles, guild_id, &bot_roles);

    match role_menu(&assignable, 0, None) {
        Some(menu) => {
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(menu)).await?;
            info!(user = %cmd.user.name, guild = %guild_id, roles = assignable.len(), "Role menu shown");
        }
        None => {
            warn!(guild = %guild_id, "No assignable roles in this server");
            cmd.create_response(&ctx.http, ephemeral("No assignable roles available in this server!"))
                .await?;
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────
//  Select menu: toggle a role
// ─────────────────────────────────────────────

pub async fn handle_select(ctx: &Context, comp: &ComponentInteraction) -> Result<()> {
    let (Some(guild_id), Some(member)) = (comp.guild_id, comp.member.as_ref()) else {
        return Ok(());
    };
    let ComponentInteractionDataKind::StringSelect { values } = &comp.data.kind else {
        return Ok(());
    };
    let Some(role_id) = values.first().and_then(|v| v.parse::<u64>().ok()).map(RoleId::new) else {
        return Ok(());
    };
    let page = select_page(&comp.data.custom_id);

    if !bot_can_manage_roles(comp.app_permissions) {
        comp.create_response(&ctx.http, ephemeral(MISSING_PERMISSION)).await?;
        return Ok(());
    }

    // Re-validate: the role may have been deleted or changed since the menu was shown.
    let (roles, bot_roles) = load_roles(ctx, guild_id).await?;
    let assignable = assignable_roles(&roles, guild_id, &bot_roles);

    let notice = match assignable.iter().find(|r| r.id == role_id) {
        None => {
            metrics::ROLE_ASSIGNMENT_COUNTER.with_label_values(&["error", "unavailable"]).inc();
            "❌ This role is no longer available!".to_string()
        }
        Some(role) => toggle_role(ctx, guild_id, comp, &member.roles, role).await,
    };

    // Re-render the menu in place: this shows the result and resets the
    // selection, so the same role can be toggled again straight away.
    let response = match role_menu(&assignable, page, Some(&notice))
        .or_else(|| role_menu(&assignable, 0, Some(&notice)))
    {
        Some(menu) => CreateInteractionResponse::UpdateMessage(menu),
        None => CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().content(notice).components(vec![]),
        ),
    };
    comp.create_response(&ctx.http, response).await?;
    Ok(())
}

/// Add or remove `role`, returning the message to show the user.
async fn toggle_role(
    ctx: &Context,
    guild_id: GuildId,
    comp: &ComponentInteraction,
    member_roles: &[RoleId],
    role: &Role,
) -> String {
    let user_id = comp.user.id;
    let reason = Some("Self-assigned via /roles");
    let has_role = member_roles.contains(&role.id);
    let result = if has_role {
        ctx.http.remove_member_role(guild_id, user_id, role.id, reason).await
    } else {
        ctx.http.add_member_role(guild_id, user_id, role.id, reason).await
    };

    let action = if has_role { "remove" } else { "add" };
    match result {
        Ok(()) => {
            metrics::ROLE_ASSIGNMENT_COUNTER.with_label_values(&[action, &role.name]).inc();
            info!(user = %comp.user.name, guild = %guild_id, role = %role.name, action, "Role toggled");
            if has_role {
                format!("🗑️ Removed **{}** role!", role.name)
            } else {
                format!("✅ Added **{}** role!", role.name)
            }
        }
        Err(e) => {
            metrics::ROLE_ASSIGNMENT_COUNTER.with_label_values(&["error", &role.name]).inc();
            warn!(error = %e, user = %user_id, guild = %guild_id, role = %role.id, "Role management error");
            "❌ Failed to update roles. Please check bot permissions!".to_string()
        }
    }
}

/// Page encoded in a select menu custom id (`role-select_2` → 2).
fn select_page(custom_id: &str) -> usize {
    custom_id
        .strip_prefix(SELECT_ID)
        .and_then(|rest| rest.strip_prefix('_'))
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

// ─────────────────────────────────────────────
//  Buttons: pagination
// ─────────────────────────────────────────────

/// Parse `roles-next_3` / `roles-prev_3` into the target page.
fn target_page(custom_id: &str) -> Option<usize> {
    let rest = custom_id.strip_prefix(PAGE_PREFIX)?;
    let (action, page) = rest.split_once('_')?;
    let page: usize = page.parse().ok()?;
    match action {
        "next" => page.checked_add(1),
        "prev" => page.checked_sub(1),
        _ => None,
    }
}

pub async fn handle_pagination(ctx: &Context, comp: &ComponentInteraction) -> Result<()> {
    let Some(guild_id) = comp.guild_id else {
        return Ok(());
    };

    let Some(page) = target_page(&comp.data.custom_id) else {
        warn!(custom_id = %comp.data.custom_id, user = %comp.user.id, "Invalid pagination action");
        comp.create_response(&ctx.http, ephemeral("Invalid pagination action.")).await?;
        return Ok(());
    };

    let (roles, bot_roles) = load_roles(ctx, guild_id).await?;
    let assignable = assignable_roles(&roles, guild_id, &bot_roles);

    let response = match role_menu(&assignable, page, None).or_else(|| role_menu(&assignable, 0, None)) {
        Some(menu) => menu,
        None => CreateInteractionResponseMessage::new()
            .content("No roles available!")
            .components(vec![]),
    };
    comp.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(response)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn role(id: u64, name: &str, position: u16, permissions: Permissions, managed: bool) -> Role {
        serde_json::from_value(json!({
            "id": id.to_string(),
            "guild_id": "1",
            "name": name,
            "color": 0,
            "colors": {"primary_color": 0, "secondary_color": null, "tertiary_color": null},
            "hoist": false,
            "position": position,
            "permissions": permissions.bits().to_string(),
            "managed": managed,
            "mentionable": false,
        }))
        .expect("valid role json")
    }

    #[test]
    fn filters_and_sorts_roles() {
        let guild = GuildId::new(1);
        let roles = vec![
            role(1, "@everyone", 0, Permissions::SEND_MESSAGES, false),
            role(10, "Blue", 1, Permissions::empty(), false),
            role(11, "Red", 3, Permissions::empty(), false),
            role(12, "Mod", 2, Permissions::KICK_MEMBERS, false),
            role(13, "Booster", 2, Permissions::empty(), true),
            role(14, "ClimaBot", 5, Permissions::MANAGE_ROLES, true),
            role(15, "Above bot", 6, Permissions::empty(), false),
            role(16, "Equal to bot", 5, Permissions::empty(), false),
            role(17, "Sneaky admin", 1, Permissions::ADMINISTRATOR, false),
        ];

        let got: Vec<&str> = assignable_roles(&roles, guild, &[RoleId::new(14)])
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(got, ["Red", "Blue"]);
    }

    #[test]
    fn bot_without_roles_can_assign_nothing() {
        let roles = vec![role(10, "Blue", 1, Permissions::empty(), false)];
        assert!(assignable_roles(&roles, GuildId::new(1), &[]).is_empty());
    }

    #[test]
    fn pagination_ids() {
        assert_eq!(target_page("roles-next_0"), Some(1));
        assert_eq!(target_page("roles-prev_2"), Some(1));
        assert_eq!(target_page("roles-prev_0"), None);
        assert_eq!(target_page("roles-bogus_1"), None);
        assert_eq!(target_page("roles-next_x"), None);
    }

    #[test]
    fn menu_pages() {
        let roles: Vec<Role> = (0..30)
            .map(|i| role(100 + i, &format!("R{i}"), 1, Permissions::empty(), false))
            .collect();
        let refs: Vec<&Role> = roles.iter().collect();
        assert!(role_menu(&refs, 0, None).is_some());
        assert!(role_menu(&refs, 1, Some("hi")).is_some());
        assert!(role_menu(&refs, 2, None).is_none());
        assert!(role_menu(&[], 0, None).is_none());
    }

    #[test]
    fn select_ids() {
        assert_eq!(select_page("role-select"), 0);
        assert_eq!(select_page("role-select_3"), 3);
        assert_eq!(select_page("role-select_x"), 0);
    }
}
