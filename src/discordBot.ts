import Discord from 'discord.js';
import { Client, GatewayIntentBits, Partials, BaseInteraction, AttachmentBuilder } from 'discord.js';
import { REST } from '@discordjs/rest';
import dotenv from 'dotenv';
import { Routes } from 'discord-api-types/v9';
import { getWeatherImage } from './utils/weatherImages';

import { getCoordinates, getWeatherData } from './utils/weather.js';

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
	}];

      const rest = new REST({ version: '9' }).setToken(token);

      await rest.put(
        Routes.applicationCommands(clientId),
        { body: commands },
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

  const { commandName, options } = interaction;

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
      const attachment = new AttachmentBuilder(weatherImage, { name: 'weather.png' });

      const embed = new Discord.EmbedBuilder()
        .setTitle('Weather Information')
        .addFields(
          { name: 'Location', value: `${formattedLocation}`, inline: true },
          { name: 'Weather Description', value: weatherDescription, inline: true },
          { name: 'Temperature', value: `${temperature}°C`, inline: true },
          { name: 'Wind Speed', value: `${windSpeed} km/h`, inline: true },
          { name: 'Wind Direction', value: `${windDirection}°`, inline: true },
          { name: 'Humidity', value: `${relativeHumidity}%`, inline: true },
          { name: 'Pressure at Sea Level', value: `${relativePressure}hPa`, inline: true },
          { name: 'Cloudiness', value: `${cloudiness}%`, inline: true },
        )
        .setImage('attachment://weather.png')
        .setColor('#0099ff');

      await interaction.reply({ embeds: [embed], files: [attachment] });
    } catch (error: any) {
      console.error('Error fetching weather data:', error);

      // Check for the specific "Unknown interaction" error
      if (error.message === 'Unknown interaction') {
        await interaction.reply('An error occurred. Please try again later.');
      } else {
        await interaction.reply('Unable to retrieve weather information.');
      }
    }
  }
});

void client.login(token);

void registerSlashCommands();
