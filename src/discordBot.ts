import {
	Client,
	GatewayIntentBits,
	Partials,
	ButtonBuilder,
	ButtonStyle,
	ActionRowBuilder,
	type BaseInteraction,
	AttachmentBuilder,
	MessageFlags,
	EmbedBuilder,
	Routes,
	type ButtonInteraction,
	type StringSelectMenuInteraction,
	type ChatInputCommandInteraction,
} from 'discord.js';
import {REST} from '@discordjs/rest';
import dotenv from 'dotenv';
import {getWeatherImage} from './utils/weatherImages';
import {createRoleMenu, handleRolePagination, handleRoleSelect} from './utils/roles.js';
import {
	getCoordinates,
	getWeatherData,
	getForecastData,
	type ForecastData,
	getFormattedLocation,
	type Location as WeatherLocation,
} from './utils/weather.js';

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
	try {
		console.log('Started refreshing global application (/) commands.');

		const commands = [
			{
				name: 'weather',
				description: 'Get the weather information for a location',
				options: [
					{
						name: 'location',
						type: 3,
						description: 'The location to get the weather information for',
						required: true,
					},
				],
			},
			{
				name: 'forecast',
				description: 'Get a 5-day weather forecast for a location',
				options: [
					{
						name: 'location',
						type: 3,
						description: 'The location to get the weather forecast for',
						required: true,
					},
				],
			},
			{
				name: 'roles',
				description: 'Manage self-assignable roles in this server',
				options: [],
			},
		];

		const rest = new REST({version: '10'}).setToken(token);
		await rest.put(Routes.applicationCommands(clientId), {body: commands});
		console.log('Successfully registered global application (/) commands.');
	} catch (error: unknown) {
		console.error('Error registering global application (/) commands:', error);
	}
}

client.on('interactionCreate', async (interaction: BaseInteraction) => {
	if (interaction.isStringSelectMenu()) {
		if (interaction.customId === 'role-select') {
			await handleRoleSelect(interaction);
		}

		return;
	}

	if (interaction.isButton()) {
		if (interaction.customId.startsWith('roles-')) {
			await handleRolePagination(interaction);
		}

		return;
	}

	if (!interaction.isChatInputCommand()) {
		return;
	}

	const {commandName, options} = interaction;

	try {
		if (commandName === 'weather') {
			const location = options.getString('location');
			if (!location) {
				await interaction.reply('Please provide a location.');
				return;
			}

			const coordinates = await getCoordinates(location);
			const response = await getWeatherData(coordinates);
			const weatherImage = getWeatherImage(response.weatherCode);
			const attachment = new AttachmentBuilder(weatherImage, {name: 'weather.png'});

			const embed = new EmbedBuilder()
				.setTitle(`🌤 Weather in ${response.formattedLocation}`)
				.setDescription(`**${response.weatherDescription}**`)
				.addFields(
					{
						name: '\u200b',
						value: [
							`🌡 **Temperature:** ${response.temperature}°C`,
							`💧 **Humidity:** ${response.relativeHumidity}%`,
							`☁ **Clouds:** ${response.cloudiness}%`,
						].join('\n'),
						inline: true,
					},
					{
						name: '\u200b',
						value: [
							`🌬 **Wind:** ${response.windSpeed} km/h`,
							`🧭 **Direction:** ${response.windDirection}°`,
							`📊 **Pressure:** ${response.relativePressure}hPa`,
						].join('\n'),
						inline: true,
					},
				)
				.setColor('#0099ff')
				.setImage('attachment://weather.png');

			await interaction.reply({
				embeds: [embed.toJSON()],
				files: [attachment],
			});
		} else if (commandName === 'forecast') {
			const location = options.getString('location');
			if (!location) {
				await interaction.reply('Please provide a location.');
				return;
			}

			const coordinates = await getCoordinates(location);
			const [forecastData, weatherResponse] = await Promise.all([
				getForecastData(coordinates),
				getWeatherData(coordinates),
			]);

			const forecastEmbed = await generateForecastMessage(
				forecastData,
				weatherResponse.formattedLocation,
			);
			await interaction.reply({embeds: [forecastEmbed]});
		} else if (commandName === 'roles') {
			if (!interaction.inGuild()) {
				await interaction.reply({
					content: 'This command only works in server!',
					flags: MessageFlags.Ephemeral,
				});
				return;
			}

			const {guild} = interaction;
			if (!guild) {
				await interaction.reply({
					content: 'Could not resolve guild information',
					flags: MessageFlags.Ephemeral,
				});
				return;
			}

			const member = await guild.members.fetch(interaction.user.id);
			const menuData = createRoleMenu(guild, member, 0);

			if (!menuData) {
				await interaction.reply({
					content: 'No assignable roles available in this server or missing bot permissions!',
					flags: MessageFlags.Ephemeral,
				});
				return;
			}

			await interaction.reply({
				...menuData,
				flags: MessageFlags.Ephemeral,
			});
		}
	} catch (error: unknown) {
		console.error(`Error handling command ${commandName}:`, error);
		const content = commandName === 'roles'
			? 'Failed to process roles command'
			: `Unable to retrieve ${commandName} information.`;

		if (interaction.replied || interaction.deferred) {
			await interaction.followUp({
				content,
				flags: MessageFlags.Ephemeral,
			});
		} else {
			await interaction.reply({
				content,
				flags: MessageFlags.Ephemeral,
			});
		}
	}
});

async function generateForecastMessage(
	forecastData: ForecastData,
	formattedLocation: string,
): Promise<EmbedBuilder> {
	const {daily} = forecastData;
	const embed = new EmbedBuilder()
		.setTitle(`🌦 Previsão para 5 dias - ${formattedLocation}`)
		.setColor('#0099ff');

	daily.time.forEach((day, index) => {
		const formattedDate = new Date(day).toLocaleDateString('pt-BR', {
			weekday: 'short',
			month: '2-digit',
			day: '2-digit',
		});

		embed.addFields({
			name: `📅 ${formattedDate}`,
			value: [
				`⬆ ${daily.temperature_2m_max[index]}°C ⬇ ${daily.temperature_2m_min[index]}°C`,
				`💧 Prob. Chuva: ${daily.precipitation_probability_max[index]}%`,
			].join('\n'),
			inline: true,
		});
	});

	return embed;
}

client.login(token).catch(console.error);
void registerSlashCommands();
