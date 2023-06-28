import Discord, { CommandInteraction } from 'discord.js';
import { Client, GatewayIntentBits, Partials, BaseInteraction } from 'discord.js';
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
                .setColor('#0099ff');

            await interaction.reply({ embeds: [embed] });
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
            const radarImageBuffer = await fetchRadarImage(lat, lng );

            // const attachment = new MessageAttachment(radarImageBuffer, 'radar.png');
            const embed = new Discord.EmbedBuilder()
                .setTitle('Weather Radar')
                .setDescription('Here is the weather radar overlaid on the map:')
                .setImage(`attachment://radar.png`);

            // await interaction.reply({ embeds: [embed], files: [attachment] });
        } catch (error) {
            console.error('Failed to retrieve radar image:', error);
            await interaction.reply({ content: 'Failed to retrieve radar image.', ephemeral: true });
        }
    }
});

client.login(token);

registerSlashCommands();
