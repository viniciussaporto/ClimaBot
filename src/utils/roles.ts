import fs from 'fs/promises';
import path from 'path';
import {
	type Role,
	type Guild,
	type StringSelectMenuInteraction,
	StringSelectMenuBuilder,
	ActionRowBuilder,
	ButtonBuilder,
	ButtonStyle,
	// ComponentType,
	PermissionFlagsBits,
	type ButtonInteraction,
} from 'discord.js';
import {roleAssignmentCounter} from './metrics';

const dangerousPermissions = [
	PermissionFlagsBits.Administrator,
	PermissionFlagsBits.ManageChannels,
	PermissionFlagsBits.ManageRoles,
	PermissionFlagsBits.ManageNicknames,
	PermissionFlagsBits.KickMembers,
	PermissionFlagsBits.BanMembers,
	PermissionFlagsBits.ModerateMembers,
	PermissionFlagsBits.ManageMessages,
	PermissionFlagsBits.SendTTSMessages,
	PermissionFlagsBits.MuteMembers,
	PermissionFlagsBits.DeafenMembers,
	PermissionFlagsBits.MoveMembers,
];
const rolesPerPage = 25;
const logFilePath = path.join(process.cwd(), 'roleExclusions.log');

function getFlaggedPermissions(role: Role): string[] {
	return dangerousPermissions
		.filter(perm => role.permissions.has(perm))
		.map(perm => {
			const permissionName = Object.entries(PermissionFlagsBits)
				.find(([_, value]) => value === perm)?.[0];
			return permissionName ?? `0x${perm.toString(16)}`;
		});
}

async function logExcludedRole(role: Role) {
	const logEntry = {
		timestamp: new Date().toISOString(),
		roleId: role.id,
		roleName: role.name,
		permissions: getFlaggedPermissions(role),
	};

	try {
		await fs.appendFile(
			logFilePath,
			`${JSON.stringify(logEntry)}\n`,
		);
	} catch (error) {
		console.error('Failed to write to role exclusion log:', error);
	}
}

export function getAssignableRoles(guild: Guild): Role[] {
	const excludedRoles: Role[] = [];

	const roles = Array.from(guild.roles.cache.filter(role => {
		const hasDangerousPerms = role.permissions.any(dangerousPermissions);
		const isManaged = role.managed;
		const isBotRole = role.id === guild.members.me?.roles.highest.id;
		const isEveryone = role.id === guild.id;
		// Unused const isEditable = role.editable;

		if (hasDangerousPerms) {
			excludedRoles.push(role);
		}

		return !hasDangerousPerms
				&& !isManaged
				&& !isBotRole
				&& !isEveryone
				&& role.editable;
	}).values());

	if (excludedRoles.length > 0) {
		Promise.all(excludedRoles.map(logExcludedRole))
			.catch(console.error);
	}

	return roles.sort((a, b) => b.position - a.position);
}
// Unused old logic without role logging
// return Array.from(guild.roles.cache.filter(role => {
// 	const hasDangerousPerms = role.permissions.any(dangerousPermissions);
// 	const isManaged = role.managed;
// 	const isBotRole = role.id === guild.members.me?.roles.highest.id;
// 	const isEveryone = role.id === guild.id;

export async function handleRoleSelect(interaction: StringSelectMenuInteraction) {
	if (!interaction.inGuild()) {
		return;
	}

	const roleId = interaction.values[0];
	const guild = interaction.guild!;
	const role = guild.roles.cache.get(roleId);

	const assignableRoles = getAssignableRoles(guild);
	if (!role || !assignableRoles.find(r => r.id === roleId)) {
		roleAssignmentCounter.labels('error', 'role').inc();
		await interaction.reply({
			content: 'This role is no longer available!',
			ephemeral: true,
		});
		return;
	}

	try {
		const member = await guild.members.fetch(interaction.user.id);

		if (member.roles.cache.has(roleId)) {
			roleAssignmentCounter.labels('action', 'role').inc();
			await member.roles.remove(roleId);
			await interaction.reply({
				content: `Removed **${role.name}** role!`,
				ephemeral: true,
			});
		} else {
			roleAssignmentCounter.labels('action', 'role').inc();
			await member.roles.add(roleId);
			await interaction.reply({
				content: `Added **${role.name}** role!`,
				ephemeral: true,
			});
		}
	} catch (error) {
		roleAssignmentCounter.labels('error', 'role').inc();
		console.error('Role management error:', error);
		await interaction.reply({
			content: 'Failed to update roles. Please check bot permissions!',
			ephemeral: true,
		});
	}
}

export function createRoleMenu(guild: Guild, page = 0) {
	const allRoles = getAssignableRoles(guild);
	const totalPages = Math.ceil(allRoles.length / rolesPerPage);
	const startIdx = page * rolesPerPage;
	const pageRoles = allRoles.slice(startIdx, startIdx + rolesPerPage);

	if (pageRoles.length === 0) {
		return null;
	}

	const selectMenu = new StringSelectMenuBuilder()
		.setCustomId('role-select')
		.setPlaceholder(`Select roles (Page ${page + 1}/${totalPages})`)
		.addOptions(pageRoles.slice(0, 25).map(role => ({
			label: role.name,
			value: role.id,
			emoji: role.unicodeEmoji ?? '🔹',
		})));

	const buttons = [];
	if (page > 0) {
		buttons.push(
			new ButtonBuilder()
				.setCustomId(`roles-prev_${page}`)
				.setLabel('Previous')
				.setStyle(ButtonStyle.Secondary),
		);
	}

	if (startIdx + rolesPerPage < allRoles.length) {
		buttons.push(
			new ButtonBuilder()
				.setCustomId(`roles-next_${page}`)
				.setLabel('Next')
				.setStyle(ButtonStyle.Primary),
		);
	}

	const components: Array<ActionRowBuilder<StringSelectMenuBuilder> | ActionRowBuilder<ButtonBuilder>> = [
		new ActionRowBuilder<StringSelectMenuBuilder>().addComponents(selectMenu),
	];

	if (buttons.length > 0) {
		components.push(
			new ActionRowBuilder<ButtonBuilder>().addComponents(buttons),
		);
	}

	return {
		content: `**Available Roles** (${allRoles.length} total)`,
		components,
		ephemeral: true,
	};
}

export async function handleRolePagination(interaction: ButtonInteraction) {
	if (!interaction.inGuild()) {
		return;
	}

	const [action, page] = interaction.customId.split('_');
	const newPage = parseInt(page, 10);

	const menuData = createRoleMenu(
		interaction.guild!,
		action === 'next' ? newPage + 1 : newPage - 1,
	);

	if (!menuData) {
		roleAssignmentCounter.labels('error', 'role').inc();
		await interaction.update({
			content: 'No roles available!',
			components: [],
		});
		return;
	}

	await interaction.update(menuData);
}
