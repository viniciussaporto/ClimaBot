import Discord, { CommandInteraction } from 'discord.js';
import { Client, GatewayIntentBits, Partials, BaseInteraction, AttachmentBuilder } from 'discord.js';
import { REST } from '@discordjs/rest';
import { Routes } from 'discord-api-types/v9';


import { getCoordinates, getWeatherData } from './weather.js';
import { fetchRadarImage } from './radar.js';

require('dotenv').config();

const token: string = process.env.TOKEN!;
const clientId = process.env.CLIENT_ID!;

const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent], partials: [Partials.Channel] });

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
                        required: true
                    }
                ]
            },
            // {
            //     name: 'radar',
            //     description: 'Get the radar image for a location(THIS FUNCTIONALITY DOES NOT WORK CURRENTLY)',
            //     options: [
            //         {
            //             name: 'location',
            //             type: 3,
            //             description: 'The location to get the radar image for(THIS FUNCTIONALITY DOES NOT WORK CURRENTLY)',
            //             required: true
            //         }
            //     ]
            // }
        ];

        const rest = new REST({ version: '9' }).setToken(token);

        await rest.put(
            Routes.applicationCommands(clientId),
            { body: commands }
        );

        console.log('Successfully registered global application (/) commands.');
    } catch (error) {
        console.error('Error registering global application (/) commands:', error);
    }
}

client.on('interactionCreate', async (interaction: BaseInteraction) => {
    if (!interaction.isCommand()) return;

    const { commandName, options } = interaction as CommandInteraction;

    if (commandName === 'weather') {
        const location = options.get('location')?.value as string;
        if (!location) {
            await interaction.reply('Please provide a location.');
            return;
        }

        try {
            const coordinates = await getCoordinates(location);
            const weatherData = await getWeatherData(coordinates);
            const temperature = weatherData.temperature;
            const weatherDescription = weatherData.weatherDescription;
            const windSpeed = weatherData.windSpeed;
            const windDirection = weatherData.windDirection;
            const reportedLocation = weatherData.formattedLocation;
            const relativeHumidity = weatherData.relativeHumidity;
            const relativePressure = weatherData.relativePressure;
            const cloudiness = weatherData.cloudiness;
            const weatherCode = weatherData.weatherCode;
            let weatherImage;

            switch (weatherCode) {
                case 0: // Clear sky
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 1: // Mainly clear
                    weatherImage = 'images/icons8-partly-cloudy-day-96.png';
                    break;
                case 2: // Partly cloudy
                    weatherImage = 'images/icons8-partly-cloudy-day-96.png';
                    break;
                case 3: // Overcast
                    weatherImage = 'images/icons8-cloud-96.png';
                    break;
                case 45: // Fog
                    weatherImage = 'images/icons8-fog-96.png';
                    break;
                case 46: // Depositing rime fog
                    weatherImage = 'images/icons8-haze-96.png';
                    break;
                case 51: // Light drizzle
                    weatherImage = 'images/icons8-light-rain-96.png';
                    break;
                case 53: // Moderate drizzle
                    weatherImage = 'images/icons8-moderate-rain-96.png';
                    break;
                case 55: // Dense intensity drizzle
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 56: // Light freezing drizzle
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 57: // Dense freezing drizzle
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 61: // Slight rain
                    weatherImage = 'images/icons8-light-rain-96.png';
                    break;
                case 63: // Moderate rain
                    weatherImage = 'images/icons8-moderate-rain-96.png';
                    break;
                case 65: // Heavy rain
                    weatherImage = 'images/icons8-rainfall-96.png';
                    break;
                case 66: // Light freezing rain
                    weatherImage = 'images/icons8-sleet-96.png';
                    break;
                case 67: // Heavy freezing rain
                    weatherImage = 'images/icons8-sleet-96.png';
                    break;
                case 71: // Slight snowfall
                    weatherImage = 'images/icons8-light-snow-96.png';
                    break;
                case 73: // Moderate snowfall
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 75: // Heavy snowfall
                    weatherImage = 'images/icons8-sun-96.png';
                    break;
                case 77: // Snow grains
                    weatherImage = 'images/icons8-snow-storm-96.png';
                    break;
                case 80: // Slight rain showers
                    weatherImage = 'images/icons8-light-rain-96.png';
                    break;
                case 81: // Moderate rain showers
                    weatherImage = 'images/icons8-moderate-rain-96.png';
                    break;
                case 82: // Violent rain showers
                    weatherImage = 'images/icons8-rainfall-96.png';
                    break;
                case 85: // Slight snow showers
                    weatherImage = 'images/icons8-snow-96.png';
                    break;
                case 86: // Heavy snow showers
                    weatherImage = 'images/icons8-light-snow-96.png';
                    break;
                case 95: // Thunderstorm
                    weatherImage = 'images/icons8-storm-with-heavy-rain-96.png';
                    break;
                case 96: // Slight hail thunderstorm
                    weatherImage = 'images/icons8-storm-96.png';
                    break;
                case 99: // Heavy hail thunderstorm
                    weatherImage = 'images/icons8-storm-with-heavy-rain-96.png';
                    break;
                default:
                    weatherImage = 'images/icons8-puzzled-96.png';
                    break;
            }

            const attachment = new AttachmentBuilder(weatherImage, { name: 'weather.png' });

            const embed = new Discord.EmbedBuilder()
                .setTitle('Weather Information')
                .addFields(
                    { name: 'Location', value: `${reportedLocation}`, inline: true },
                    { name: 'Weather Description', value: weatherDescription, inline: true },
                    { name: 'Temperature', value: `${temperature}°C`, inline: true },
                    { name: 'Wind Speed', value: `${windSpeed} km/h`, inline: true },
                    { name: 'Wind Direction', value: `${windDirection}°`, inline: true },
                    { name: 'Humidity', value: `${relativeHumidity}%`, inline: true },
                    { name: 'Pressure at Sea Level', value: `${relativePressure}hPa`, inline: true },
                    { name: 'Cloudiness', value: `${cloudiness}%`, inline: true }
                )
                .setImage(`attachment://weather.png`)
                .setColor('#0099ff');

            await interaction.reply({ embeds: [embed], files: [attachment] });
        } catch (error) {
            console.error('Error fetching weather data:', error);
            await interaction.reply('Unable to retrieve weather information.');
        }
    } else if (commandName === 'radar') {
        // @ts-expect-error
        const location = options.getString('location');
        if (!location) {
            await interaction.reply('Please provide a location.');
            return;
        }

        try {
            const coordinates = await getCoordinates(location);
            const { lat, lng } = coordinates; // Extract latitude and longitude from coordinates object
            const radarImageBuffer = await fetchRadarImage(lat, lng);

            // const attachment = new AttachmentBuilder(radarImageBuffer, { name: 'radar.png' });
            const embed = new Discord.EmbedBuilder()
                .setTitle('Weather Radar')
                .setDescription('Here is the weather radar overlaid on the map:')
                .setImage(`attachment://radar.png`);

            await interaction.reply({ embeds: [embed]});
        } catch (error) {
            console.error('Failed to retrieve radar image:', error);
            await interaction.reply({ content: 'Failed to retrieve radar image.', ephemeral: true });
        }
    }
});

client.login(token);

registerSlashCommands();
