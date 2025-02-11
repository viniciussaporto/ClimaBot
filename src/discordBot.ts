import {
	Client,
	GatewayIntentBits,
	Partials,
	ButtonBuilder,
	ActionRowBuilder,
	type BaseInteraction,
	AttachmentBuilder,
	MessageFlags,
	EmbedBuilder,
	Routes,
} from 'discord.js';
import {REST} from '@discordjs/rest';
import dotenv from 'dotenv';
import {getWeatherImage} from './utils/weatherImages.js';
import {createRoleMenu, handleRolePagination, handleRoleSelect} from './utils/roles.js';
import {
	getCoordinates,
	getWeatherData,
	getForecastData,
	type ForecastData,
} from './utils/weather.js';
import {apiRequestCounter, apiResponseTimeHistogram} from './utils/metrics.js';

dotenv.config();

const token = process.env.TOKEN;
const clientId = process.env.CLIENT_ID;

if (!token || !clientId) {
	throw new Error('Missing required environment variables');
}

const client = new Client({
	intents: [
		GatewayIntentBits.Guilds,
		GatewayIntentBits.GuildMessages,
		GatewayIntentBits.MessageContent,
		GatewayIntentBits.GuildMembers,
		GatewayIntentBits.GuildMessageReactions,
	],
	partials: [Partials.Channel],
});

client.on('ready', () => {
	console.log(`Logged in as ${client.user?.tag ?? 'unknown client'}`);
});

async function registerSlashCommands(): Promise<void> {
	const commands = [
		{
			name: 'weather',
			description: 'Get weather information for a location',
			options: [{
				name: 'location',
				type: 3,
				description: 'Location to check',
				required: true,
			}],
		},
		{
			name: 'forecast',
			description: 'Get 5-day forecast',
			options: [{
				name: 'location',
				type: 3,
				description: 'Location to check',
				required: true,
			}],
		},
		{
			name: 'roles',
			description: 'Manage self-assignable roles',
			options: [],
		},
	];

	try {
		const rest = new REST({version: '10'}).setToken(token);
		await rest.put(Routes.applicationCommands(clientId), {body: commands});
		console.log('Commands registered successfully');
	} catch (error) {
		console.error('Command registration failed:', error);
	}
}

client.on('interactionCreate', async (interaction: BaseInteraction) => {
	if (!interaction.isChatInputCommand()) {
		return;
	}

	const {commandName, options} = interaction;
	const timer = apiResponseTimeHistogram.labels(commandName).startTimer();

	try {
		apiRequestCounter.labels(commandName, 'start').inc();

		if (commandName === 'weather') {
			// Weather command logic
		} else if (commandName === 'forecast') {
			// Forecast logic
		} else if (commandName === 'roles') {
			if (!interaction.inGuild()) {
				await interaction.reply({
					content: 'This command only works in servers!',
					flags: MessageFlags.Ephemeral,
				});
				return;
			}

			const {guild} = interaction;
			const member = await guild.members.fetch(interaction.user.id);
			const menuData = createRoleMenu(guild, member, 0);

			await interaction.reply({
				...menuData,
				flags: MessageFlags.Ephemeral,
				components: menuData.components.map(c => c.toJSON()),
			});
			timer({status: 'success'});
		}
	} catch (error) {
		console.error(`Command ${commandName} failed:`, error);
		apiRequestCounter.labels(commandName, 'error').inc();
		timer({status: 'error'});

		await interaction.reply({
			content: 'An error occurred processing your request',
			flags: MessageFlags.Ephemeral,
		});
	}
});

client.login(token).catch(console.error);
void registerSlashCommands();
