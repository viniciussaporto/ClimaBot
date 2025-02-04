import Discord, {type EmbedBuilder} from 'discord.js';
import {Client, GatewayIntentBits, Partials, type BaseInteraction, AttachmentBuilder} from 'discord.js';
import {REST} from '@discordjs/rest';
import dotenv from 'dotenv';
import {Routes} from 'discord-api-types/v9';
import {getWeatherImage} from './utils/weatherImages';

import {getCoordinates, getWeatherData} from './utils/weather.js';
import {getForecastData, type ForecastData} from './utils/weather.js';
import {getFormattedLocation, type Location as WeatherLocation} from './utils/weather.js';

dotenv.config();

const token: string = process.env.TOKEN!;
const clientId = process.env.CLIENT_ID!;

const client = new Client({
	intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent],
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
				.setTitle('Weather Information')
				.addFields(
					{name: '\u200b', value: `**${formattedLocation}**`, inline: true},
					{name: '\u200b', value: weatherDescription, inline: true},
					{name: '\u200b', value: `**Temperature:**${temperature}°C`, inline: true},
					{name: '\u200b', value: `**Wind speed:**${windSpeed} km/h`, inline: true},
					{name: '\u200b', value: `**Wind dir.:**${windDirection}°`, inline: true},
					{name: '\u200b', value: `**Humidity:**${relativeHumidity}%`, inline: true},
					{name: '\u200b', value: `**Pressure:**${relativePressure}hPa`, inline: true},
					{name: '\u200b', value: `**Cloud cover.:**${cloudiness}%`, inline: true},
				)
				.setImage('attachment://weather.png')
				.setColor('#0099ff');

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

			const forecastEmbed = await generateForecastMessage(forecastData, coordinates);

			await interaction.reply({embeds: [forecastEmbed]});
		} catch (error: any) {
			console.error('Error fetching forecast data:', error);
			await interaction.reply('Unable to retrieve forecast information.');
		}
	}
});

async function generateForecastMessage(forecastData: ForecastData, coordinates: WeatherLocation): Promise<EmbedBuilder> {
	const {daily} = forecastData;
	const formattedLocation = await getFormattedLocation(coordinates);

	const embed = new Discord.EmbedBuilder()
		.setTitle('Previsão para 5 dias')
		.setColor('#0099ff')
		.setDescription(`${formattedLocation}`);

	daily.time.forEach((day, index) => {
		const maxTemperature = daily.temperature_2m_max[index];
		const minTemperature = daily.temperature_2m_min[index];
		const precipitationProbability = daily.precipitation_probability_max[index];

		const formattedDate = new Date(day).toLocaleDateString('pt-BR', {
			weekday: 'long',
			year: 'numeric',
			month: '2-digit',
			day: '2-digit',
		});

		embed.addFields(
			{name: '\u200b', value: `${formattedDate}`},
			{name: 'Temp. Max.:', value: `${maxTemperature}°C`},
			{name: 'Temp. Min.:', value: `${minTemperature}°C`, inline: true},
			{name: 'Prob. Chuva:', value: `${precipitationProbability}%`, inline: true},
		);
	});

	return embed;
}

void client.login(token);

void registerSlashCommands();
