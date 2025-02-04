import {
    Role,
    Guild,
    StringSelectMenuInteraction,
    StringSelectMenuBuilder,
    ActionRowBuilder,
    PermissionFlagsBits
} from 'discord.js';

const DANGEROUS_PERMISSIONS = [
    PermissionFlagsBits.Administrator,
    PermissionFlagsBits.ManageChannels,
    PermissionFlagsBits.ManageRoles,
    PermissionFlagsBits.ManageGuildExpressions,
    PermissionFlagsBits.ViewAuditLog,
    PermissionFlagsBits.ViewGuildInsights,
    PermissionFlagsBits.ManageWebhooks,
    PermissionFlagsBits.ManageGuild,
    PermissionFlagsBits.ManageNicknames,
    PermissionFlagsBits.KickMembers,
    PermissionFlagsBits.BanMembers,
    PermissionFlagsBits.ModerateMembers,
    PermissionFlagsBits.MentionEveryone,
    PermissionFlagsBits.ManageMessages,
    PermissionFlagsBits.ManageThreads,
    PermissionFlagsBits.SendTTSMessages,
    PermissionFlagsBits.PrioritySpeaker,
    PermissionFlagsBits.MuteMembers,
    PermissionFlagsBits.DeafenMembers,
    PermissionFlagsBits.MoveMembers,
    PermissionFlagsBits.CreateEvents,
    PermissionFlagsBits.ManageEvents
];

export function getAssignableRoles(guild: Guild): Role[] {
    return guild.roles.cache.filter(role => {
        const hasDangerousPerms = role.permissions.any(DANGEROUS_PERMISSIONS);
        const isManaged = role.managed;
        const isBotRole = role.id === guild.members.me?.roles.highest.id;
        const isEveryone = role.id === guild.id;
        
        return !hasDangerousPerms &&
               !isManaged &&
               !isBotRole &&
               !isEveryone &&
               role.editable;
    }).sort((a, b) => b.position - a.position)
      .array();
}

export async function handleRoleSelect(interaction: StringSelectMenuInteraction) {
    if (!interaction.inGuild()) return;

    const roleId = interaction.values[0];
    const guild = interaction.guild!;
    const role = guild.roles.cache.get(roleId);

    const assignableRoles = getAssignableRoles(guild);
    if (!role || !assignableRoles.find(r => r.id === roleId)) {
        await interaction.reply({ 
            content: "This role is no longer available!", 
            ephemeral: true 
        });
        return;
    }

    try {
        const member = await guild.members.fetch(interaction.user.id);
        
        if (member.roles.cache.has(roleId)) {
            await member.roles.remove(roleId);
            await interaction.reply({ 
                content: `Removed **${role.name}** role!`, 
                ephemeral: true 
            });
        } else {
            await member.roles.add(roleId);
            await interaction.reply({ 
                content: `Added **${role.name}** role!`, 
                ephemeral: true 
            });
        }
    } catch (error) {
        console.error('Role management error:', error);
        await interaction.reply({ 
            content: "Failed to update roles. Please check bot permissions!", 
            ephemeral: true 
        });
    }
}

export function createRoleSelectMenu(guild: Guild) {
    const assignableRoles = getAssignableRoles(guild);
    
    if (assignableRoles.length === 0) return null;

    const roleOptions = assignableRoles.slice(0, 25).map(role => ({
        label: role.name,
        value: role.id,
        emoji: role.unicodeEmoji || '🔹'
    }));

    return new ActionRowBuilder<StringSelectMenuBuilder>().addComponents(
        new StringSelectMenuBuilder()
            .setCustomId('role-select')
            .setPlaceholder('Select a role to toggle')
            .addOptions(roleOptions)
    );
}