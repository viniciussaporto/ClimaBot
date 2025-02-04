import Discord, {type EmbedBuilder} from 'discord.js';
import {Client, GatewayIntentBits, Partials, ButtonBuilder, ButtonStyle, ActionRowBuilder, type BaseInteraction, AttachmentBuilder} from 'discord.js';
import {REST} from '@discordjs/rest';
import dotenv from 'dotenv';
import {Routes} from 'discord-api-types/v9';
import {getWeatherImage} from './utils/weatherImages';
import type {
    ButtonInteraction,
    StringSelectMenuInteraction
} from 'discord.js';

import {createRoleMenu, handleRolePagination, handleRoleSelect} from './utils/roles.js';
import {getCoordinates, getWeatherData} from './utils/weather.js';
import {getForecastData, type ForecastData} from './utils/weather.js';
import {getFormattedLocation, type Location as WeatherLocation} from './utils/weather.js';

dotenv.config();

const token: string = process.env.TOKEN!;
const clientId = process.env.CLIENT_ID!;

const client = new Client({
	intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent, GatewayIntentBits.GuildMembers, GatewayIntentBits.GuildMessageReactions],
	partials: [Partials.Channel],
});

client.on('ready', () => {
	console.log(`Logged in as ${client.user!.tag}`);
});

async function registerSlashCommands() {
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
				options: []
			},
		];

		const rest = new REST({version: '9'}).setToken(token);

		await rest.put(
			Routes.applicationCommands(clientId),
			{body: commands},
		);

		console.log('Successfully registered global application (/) commands.');
	} catch (error: any) {
		console.error('Error registering global application (/) commands:', error);
	}
}

client.on('interactionCreate', async (interaction: BaseInteraction) => {
	if (!interaction.isChatInputCommand()) {
		return;
	}

    if (interaction.isStringSelectMenu()) {
        if (interaction.customId === 'role-select') {
            await handleRoleSelect(interaction);
        }
        return;
    }

    // Then handle buttons
    if (interaction.isButton()) {
        if (interaction.customId.startsWith('roles-')) {
            await handleRolePagination(interaction);
        }
        return;
    }

    // Finally handle commands
    if (!interaction.isChatInputCommand()) return;

	const {commandName, options} = interaction;

	if (commandName === 'weather') {
		const location = options.getString('location');
		if (!location) {
			await interaction.reply('Please provide a location.');
			return;
		}

		try {
			const coordinates = await getCoordinates(location);
			const response = await getWeatherData(coordinates);

			const {
				temperature,
				weatherDescription,
				windSpeed,
				windDirection,
				relativeHumidity,
				relativePressure,
				cloudiness,
				weatherCode,
				formattedLocation,
			} = response;

			const weatherImage = getWeatherImage(weatherCode);
			const attachment = new AttachmentBuilder(weatherImage, {name: 'weather.png'});

			const embed = new Discord.EmbedBuilder()
				.setTitle(`🌤 Weather in ${formattedLocation}`)
				.setDescription(`**${weatherDescription}**`)
				.addFields(
				{
					name: '\u200b', // Zero-width space
					value: [
					`🌡 **Temperature:** ${temperature}°C`,
					`💧 **Humidity:** ${relativeHumidity}%`,
					`☁ **Clouds:** ${cloudiness}%`
					].join('\n'),
					inline: true
				},
				{
					name: '\u200b',
					value: [
					`🌬 **Wind:** ${windSpeed} km/h`,
					`🧭 **Direction:** ${windDirection}°`,
					`📊 **Pressure:** ${relativePressure}hPa`
					].join('\n'),
					inline: true
				}
				)
				.setColor('#0099ff')
				.setImage('attachment://weather.png');

			await interaction.reply({embeds: [embed], files: [attachment]});
		} catch (error) {
			console.error('Error fetching weather data:', error);
			await interaction.reply('Unable to retrieve weather information.');
		}
	}		else if (commandName === 'forecast') {
		const location = options.getString('location');
		if (!location) {
			await interaction.reply('Please provide a location.');
			return;
		}

		try {
			const coordinates = await getCoordinates(location);
			const forecastData = await getForecastData(coordinates);
			const weatherRespornse = await getWeatherData(coordinates);

			const forecastEmbed = await generateForecastMessage(
				forecastData,
				weatherRespornse.formattedLocation
			);

			await interaction.reply({embeds: [forecastEmbed]});
		} catch (error: any) {
			console.error('Error fetching forecast data:', error);
			await interaction.reply('Unable to retrieve forecast information.');
		}
	}
			else if (commandName === 'roles') {
				if (!interaction.inGuild()) {
					await interaction.reply( {
						content: "This command only works in server!",
						ephemeral: true
					});
					return;
				}

				const guild = interaction.guild!;
				// const roleMenu = createRoleSelectMenu(guild);
				const menuData = createRoleMenu(interaction.guild!);

				if (!menuData) {
					await interaction.reply( {
						content: "No assignable roles available in this server!",
						ephemeral: true
					});
					return;
				}
				
				await interaction.reply(menuData);
				// await interaction.reply( {
				// 	content: 'Choose a role to add/remove:',
				// 	components: [roleMenu],
				// 	ephemeral:true;
				}
			}
);

async function generateForecastMessage(
    forecastData: ForecastData, 
    formattedLocation: string
): Promise<EmbedBuilder> {
    const { daily } = forecastData;

    const embed = new Discord.EmbedBuilder()
        .setTitle(`🌦 Previsão para 5 dias - ${formattedLocation}`)
        .setColor('#0099ff');

    daily.time.forEach((day, index) => {
        const maxTemp = daily.temperature_2m_max[index];
        const minTemp = daily.temperature_2m_min[index];
        const rainProb = daily.precipitation_probability_max[index];

        const formattedDate = new Date(day).toLocaleDateString('pt-BR', {
            weekday: 'short',
            month: '2-digit',
            day: '2-digit'
        });

        embed.addFields({
            name: `📅 ${formattedDate}`,
            value: [
                `⬆ ${maxTemp}°C ⬇ ${minTemp}°C`,
                `💧 Prob. Chuva: ${rainProb}%`
            ].join('\n'),
            inline: true
        });
    });

    return embed;
}

void client.login(token);

void registerSlashCommands();
