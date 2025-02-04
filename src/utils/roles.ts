import {
    Role,
    Guild,
    StringSelectMenuInteraction,
    StringSelectMenuBuilder,
    ActionRowBuilder,
	ButtonBuilder,
	ButtonStyle,
	ComponentType,
    PermissionFlagsBits,
	type ButtonInteraction
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
const ROLES_PER_PAGE = 25;

export function getAssignableRoles(guild: Guild): Role[] {
    return Array.from(guild.roles.cache.filter(role => {
        const hasDangerousPerms = role.permissions.any(DANGEROUS_PERMISSIONS);
        const isManaged = role.managed;
        const isBotRole = role.id === guild.members.me?.roles.highest.id;
        const isEveryone = role.id === guild.id;
        
        return !hasDangerousPerms && 
               !isManaged && 
               !isBotRole && 
               !isEveryone &&
               role.editable;
    }).values()).sort((a, b) => b.position - a.position);
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

export function createRoleMenu(guild: Guild, page: number = 0) {
    const allRoles = getAssignableRoles(guild);
    const totalPages = Math.ceil(allRoles.length / ROLES_PER_PAGE);
    const startIdx = page * ROLES_PER_PAGE;
    const pageRoles = allRoles.slice(startIdx, startIdx + ROLES_PER_PAGE);

    if (pageRoles.length === 0) return null;

    // Create select menu
    const selectMenu = new StringSelectMenuBuilder()
        .setCustomId('role-select')
        .setPlaceholder(`Select roles (Page ${page + 1}/${totalPages})`)
        .addOptions(pageRoles.slice(0, 25).map(role => ({
            label: role.name,
            value: role.id,
            emoji: role.unicodeEmoji || '🔹'
        })));

    // Create navigation buttons
    const buttons = [];
    if (page > 0) {
        buttons.push(
            new ButtonBuilder()
                .setCustomId(`roles-prev_${page}`)
                .setLabel('Previous')
                .setStyle(ButtonStyle.Secondary)
        );
    }
    if (startIdx + ROLES_PER_PAGE < allRoles.length) {
        buttons.push(
            new ButtonBuilder()
                .setCustomId(`roles-next_${page}`)
                .setLabel('Next')
                .setStyle(ButtonStyle.Primary)
        );
    }

	const components = [
		new ActionRowBuilder<StringSelectMenuBuilder>().addComponents(selectMenu)
	];
	
	if (buttons.length > 0) {
		components.push(
			new ActionRowBuilder<ButtonBuilder>().addComponents(buttons) // Explicit type
		);
	}

export async function handleRolePagination(interaction: ButtonInteraction) {
    if (!interaction.inGuild()) return;
    
    const [action, page] = interaction.customId.split('_');
    const newPage = parseInt(page);
    
    const menuData = createRoleMenu(
        interaction.guild!,
        action === 'next' ? newPage + 1 : newPage - 1
    );

    if (!menuData) {
        await interaction.update({ 
            content: "No roles available!", 
            components: [] 
        });
        return;
    }

    await interaction.update(menuData);
}