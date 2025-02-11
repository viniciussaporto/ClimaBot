import fs from 'fs/promises';
import path from 'path';
import {
	type Role,
	type Guild,
	type GuildMember,
	type StringSelectMenuInteraction,
	StringSelectMenuBuilder,
	ActionRowBuilder,
	ButtonBuilder,
	ButtonStyle,
	ComponentType,
	PermissionFlagsBits,
	MessageFlags,
	type ButtonInteraction,
	type ChatInputCommandInteraction,
} from 'discord.js';

const dangerousPermissions = [
	PermissionFlagsBits.Administrator,
	// PermissionFlagsBits.ManageChannels,
	// PermissionFlagsBits.ManageRoles,
	// PermissionFlagsBits.ManageNicknames,
	// PermissionFlagsBits.KickMembers,
	// PermissionFlagsBits.BanMembers,
	// PermissionFlagsBits.ModerateMembers,
	// PermissionFlagsBits.ManageMessages,
	// PermissionFlagsBits.SendTTSMessages,
	// PermissionFlagsBits.MuteMembers,
	// PermissionFlagsBits.DeafenMembers,
	// PermissionFlagsBits.MoveMembers,
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
		const isEditable = role.editable;

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

	return roles.sort((a: Role, b: Role) => b.position - a.position);
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
		await interaction.reply({
			content: 'This role is no longer available!',
			ephemeral: true,
		});
		return;
	}

	try {
		const member = await guild.members.fetch(interaction.user.id);

		if (member.roles.cache.has(roleId)) {
			await member.roles.remove(roleId);
			await interaction.reply({
				content: `Removed **${role.name}** role!`,
				ephemeral: true,
			});
		} else {
			await member.roles.add(roleId);
			await interaction.reply({
				content: `Added **${role.name}** role!`,
				ephemeral: true,
			});
		}
	} catch (error) {
		console.error('Role management error:', error);
		await interaction.reply({
			content: 'Failed to update roles. Please check bot permissions!',
			ephemeral: true,
		});
	}
}

export function createRoleMenu(guild: Guild, member: GuildMember, page = 0) {
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
		.addOptions(pageRoles.slice(0, 25).map(role => {
			const hasRole = member.roles.cache.has(role.id);
			return {
				label: role.name,
				value: role.id,
				emoji: {name: hasRole ? '✅' : '❌'}, // This doesn't work, it's defaulting to Unicode blue diamond.
			};
		}));

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
		// Remove ephemeral: true,
	};
}

export async function handleRolePagination(interaction: ButtonInteraction) {
	if (!interaction.inGuild()) {
		return;
	}

	const [action, page] = interaction.customId.split('_');
	const newPage = parseInt(page, 10);
	const guild = interaction.guild!;

	// Fetch the member's current roles
	const member = await guild.members.fetch(interaction.user.id);

	const menuData = createRoleMenu(
		guild,
		member,
		action === 'next' ? newPage + 1 : newPage - 1,
	);

	if (!menuData) {
		await interaction.update({
			content: 'No roles available!',
			components: [],
		});
		return;
	}

	await interaction.update(menuData);
}
// Debugging version of code to fetch role and permissions each call
// import fs from 'fs/promises';
// import path from 'path';
// import {
// 	type Role,
// 	type Guild,
// 	type StringSelectMenuInteraction,
// 	StringSelectMenuBuilder,
// 	ActionRowBuilder,
// 	ButtonBuilder,
// 	ButtonStyle,
// 	PermissionFlagsBits,
// 	type ButtonInteraction,
// } from 'discord.js';

// const dangerousPermissions = [
// 	PermissionFlagsBits.Administrator,
// ];
// const rolesPerPage = 25;
// const logFilePath = path.join(process.cwd(), 'roleExclusions.log');

// // Helper function to get human-readable permission names
// function getFlaggedPermissions(role: Role): string[] {
// 	return dangerousPermissions
// 		.filter(perm => role.permissions.has(perm))
// 		.map(perm => {
// 			const permissionName = Object.entries(PermissionFlagsBits)
// 				.find(([_, value]) => value === perm)?.[0];
// 			return permissionName ?? `0x${perm.toString(16)}`;
// 		});
// }

// // Async logging function for excluded roles
// async function logExcludedRole(role: Role) {
// 	const logEntry = {
// 		timestamp: new Date().toISOString(),
// 		roleId: role.id,
// 		roleName: role.name,
// 		permissions: getFlaggedPermissions(role),
// 	};

// 	try {
// 		await fs.appendFile(
// 			logFilePath,
// 			`${JSON.stringify(logEntry)}\n`,
// 		);
// 	} catch (error) {
// 		console.error('[Role Filter] Failed to write to exclusion log:', error);
// 	}
// }

// export function getAssignableRoles(guild: Guild): Role[] {
// 	console.log(`\n[Role Filter] Starting role check in ${guild.name}`);
// 	console.log(`[Role Filter] Bot's highest role: ${guild.members.me?.roles.highest.name} (pos: ${guild.members.me?.roles.highest.position})`);

// 	const excludedRoles: Role[] = [];
// 	const allRoles = Array.from(guild.roles.cache.values());

// 	console.log(`[Role Filter] Total roles: ${allRoles.length}`);

// 	const filteredRoles = Array.from(guild.roles.cache.filter(role => {
// 		console.log(`\n=== Checking role: ${role.name} (pos: ${role.position}) ===`);

// 		// Check dangerous permissions
// 		const hasDangerousPerms = role.permissions.any(dangerousPermissions);
// 		const flaggedPerms = getFlaggedPermissions(role);
// 		console.log(`[Dangerous Perms] ${hasDangerousPerms ? 'YES' : 'NO'}`,
// 			flaggedPerms.length > 0 ? `(${flaggedPerms.join(', ')})` : '');

// 		// Check managed status
// 		console.log(`[Managed] ${role.managed ? 'YES' : 'NO'}`);

// 		// Check bot role
// 		const isBotRole = role.id === guild.members.me?.roles.highest.id;
// 		console.log(`[Bot Role] ${isBotRole ? 'YES' : 'NO'}`);

// 		// Check @everyone
// 		const isEveryone = role.id === guild.id;
// 		console.log(`[@everyone] ${isEveryone ? 'YES' : 'NO'}`);

// 		// Check editable
// 		console.log(`[Editable] ${role.editable ? 'YES' : 'NO'}`);

// 		if (hasDangerousPerms) {
// 			console.log('[Exclusion] Role excluded due to dangerous permissions');
// 			excludedRoles.push(role);
// 			return false;
// 		}

// 		const keepRole = !role.managed && !isBotRole && !isEveryone && role.editable;
// 		if (!keepRole) {
// 			console.log('[Exclusion] Role excluded because of:');
// 			if (role.managed) {
// 				console.log(' - Managed role');
// 			}

// 			if (isBotRole) {
// 				console.log(' - Bot role');
// 			}

// 			if (isEveryone) {
// 				console.log(' - @everyone role');
// 			}

// 			if (!role.editable) {
// 				console.log(' - Not editable');
// 			}
// 		}

// 		return keepRole;
// 	}).values());

// 	console.log('\n[Role Filter] Summary:');
// 	console.log(`- Total roles checked: ${allRoles.length}`);
// 	console.log(`- Excluded by permissions: ${excludedRoles.length}`);
// 	console.log(`- Excluded by other reasons: ${allRoles.length - filteredRoles.length - excludedRoles.length}`);
// 	console.log(`- Available roles: ${filteredRoles.length}`);
// 	console.log('Available role names:', filteredRoles.map(r => r.name).join(', ') || 'None');

// 	if (excludedRoles.length > 0) {
// 		console.log('[Role Filter] Logging excluded roles...');
// 		Promise.all(excludedRoles.map(logExcludedRole))
// 			.catch(error => {
// 				console.error('[Role Filter] Logging failed:', error);
// 			});
// 	}

// 	return filteredRoles.sort((a, b) => b.position - a.position);
// }

// export async function handleRoleSelect(interaction: StringSelectMenuInteraction) {
// 	if (!interaction.inGuild()) {
// 		return;
// 	}

// 	const roleId = interaction.values[0];
// 	const guild = interaction.guild!;
// 	const role = guild.roles.cache.get(roleId);

// 	console.log(`\n[Role Select] Handling selection for role ID: ${roleId}`);
// 	console.log(`[Role Select] User: ${interaction.user.tag}`);
// 	console.log(`[Role Select] Role exists: ${Boolean(role)}`);

// 	const assignableRoles = getAssignableRoles(guild);
// 	if (!role || !assignableRoles.some(r => r.id === roleId)) {
// 		console.log('[Role Select] Invalid role selected');
// 		await interaction.reply({
// 			content: 'This role is no longer available!',
// 			ephemeral: true,
// 		});
// 		return;
// 	}

// 	try {
// 		const member = await guild.members.fetch(interaction.user.id);
// 		const hasRole = member.roles.cache.has(roleId);

// 		console.log(`[Role Select] User ${hasRole ? 'has' : 'does not have'} role`);

// 		if (hasRole) {
// 			await member.roles.remove(roleId);
// 			console.log('[Role Select] Role removed successfully');
// 		} else {
// 			await member.roles.add(roleId);
// 			console.log('[Role Select] Role added successfully');
// 		}

// 		await interaction.reply({
// 			content: `${hasRole ? 'Removed' : 'Added'} **${role.name}** role!`,
// 			ephemeral: true,
// 		});
// 	} catch (error) {
// 		console.error('[Role Select] Error:', error);
// 		await interaction.reply({
// 			content: 'Failed to update roles. Please check bot permissions!',
// 			ephemeral: true,
// 		});
// 	}
// }

// export function createRoleMenu(guild: Guild, page = 0) {
// 	console.log(`\n[Role Menu] Creating menu for ${guild.name} page ${page}`);
// 	const allRoles = getAssignableRoles(guild);

// 	console.log(`[Role Menu] Total assignable roles: ${allRoles.length}`);
// 	if (allRoles.length === 0) {
// 		console.log('[Role Menu] No roles available');
// 		return null;
// 	}

// 	const totalPages = Math.ceil(allRoles.length / rolesPerPage);
// 	const startIdx = page * rolesPerPage;
// 	const pageRoles = allRoles.slice(startIdx, startIdx + rolesPerPage);

// 	console.log(`[Role Menu] Page ${page + 1}/${totalPages}`);
// 	console.log(`[Role Menu] Showing roles: ${pageRoles.map(r => r.name).join(', ')}`);

// 	const selectMenu = new StringSelectMenuBuilder()
// 		.setCustomId('role-select')
// 		.setPlaceholder(`Select roles (Page ${page + 1}/${totalPages})`)
// 		.addOptions(pageRoles.map(role => ({
// 			label: role.name,
// 			value: role.id,
// 			emoji: role.unicodeEmoji ?? '🔹',
// 		})));

// 	const buttons = [];
// 	if (page > 0) {
// 		buttons.push(
// 			new ButtonBuilder()
// 				.setCustomId(`roles-prev_${page}`)
// 				.setLabel('Previous')
// 				.setStyle(ButtonStyle.Secondary),
// 		);
// 	}

// 	if (startIdx + rolesPerPage < allRoles.length) {
// 		buttons.push(
// 			new ButtonBuilder()
// 				.setCustomId(`roles-next_${page}`)
// 				.setLabel('Next')
// 				.setStyle(ButtonStyle.Primary),
// 		);
// 	}

// 	const components = [
// 		new ActionRowBuilder<StringSelectMenuBuilder>().addComponents(selectMenu),
// 	];

// 	if (buttons.length > 0) {
// 		components.push(
// 			new ActionRowBuilder<ButtonBuilder>().addComponents(buttons),
// 		);
// 	}

// 	return {
// 		content: `**Available Roles** (${allRoles.length} total)`,
// 		components,
// 		ephemeral: true,
// 	};
// }

// export async function handleRolePagination(interaction: ButtonInteraction) {
// 	if (!interaction.inGuild()) {
// 		return;
// 	}

// 	console.log('\n[Pagination] Handling button interaction');
// 	console.log(`[Pagination] Custom ID: ${interaction.customId}`);

// 	const [action, page] = interaction.customId.split('_');
// 	const newPage = parseInt(page, 10);
// 	const newPageNumber = action === 'next' ? newPage + 1 : newPage - 1;

// 	console.log(`[Pagination] Navigating to page ${newPageNumber}`);

// 	const menuData = createRoleMenu(interaction.guild!, newPageNumber);

// 	if (!menuData) {
// 		console.log('[Pagination] No roles available for page');
// 		await interaction.update({
// 			content: 'No roles available!',
// 			components: [],
// 		});
// 		return;
// 	}

// 	await interaction.update(menuData);
// }
