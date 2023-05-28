// const Discord = require('discord.js');
// require('dotenv').config();

// const { Client, GatewayIntentBits, Partials, EmbedBuilder } = require('discord.js');
// const { REST } = require('@discordjs/rest');
// const { Routes } = require('discord-api-types/v9');

// const axios = require('axios');
// const token = process.env.TOKEN;
// const openCageApiKey = process.env.OPENCAGEAPIKEY;
// const clientId = process.env.CLIENT_ID;

// const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent], partials: [Partials.Channel] });
// const commands = [];

// client.on('ready', () => {
//     console.log(`Logged in as ${client.user.tag}`);
// });

// async function registerSlashCommands(guildId) {
//     try {
//         console.log(`Started refreshing application (/) commands for guild ID ${guildId}.`);

//         commands.push({
//             name: 'weather',
//             description: 'Get the weather information for a location',
//             options: [
//                 {
//                     name: 'location',
//                     type: 3,
//                     description: 'The location to get the weather information for',
//                     required: true
//                 }
//             ]
//         });

//         const rest = new REST({ version: '9' }).setToken(token);

//         await rest.put(
//             guildId ? Routes.applicationGuildCommands(clientId, guildId) : Routes.applicationCommands(clientId),
//             { body: commands }
//         );

//         console.log(`Successfully registered application (/) commands for guild ID ${guildId}.`);
//     } catch (error) {
//         console.error(`Error registering application (/) commands for guild ID ${guildId}:`, error);
//     }
// }

// client.on('guildCreate', (guild) => {
//     const guildId = guild.id;
//     registerSlashCommands(guildId);
// });

// client.on('interactionCreate', async (interaction) => {
//     if (!interaction.isCommand()) return;

//     const { commandName, options, guildId } = interaction;

//     if (commandName === 'weather') {
//         const location = options.getString('location');
//         if (!location) {
//             await interaction.reply('Please provide a location.');
//             return;
//         }

//         try {
//             const coordinates = await getCoordinates(location);
//             const weatherData = await getWeatherData(coordinates, coordinates.formatted);
//             const temperature = weatherData.temperature;
//             const weatherDescription = weatherData.weatherDescription;
//             const windSpeed = weatherData.windSpeed;
//             const windDirection = weatherData.windDirection;
//             const reportedLocation = weatherData.formatted;
//             const relativeHumidity = weatherData.relativeHumidity;
//             const relativePressure = weatherData.relativePressure;
//             const cloudiness = weatherData.cloudiness;

//             const embed = new Discord.EmbedBuilder()
//                 .setTitle('Weather Information')
//                 .addFields(
//                     { name: 'Location', value: `${reportedLocation}.`, inline: true},
//                     { name: 'Weather Description:', value: weatherDescription, inline: true },
//                     { name: 'Temperature:', value: `${temperature}°C` },
//                     { name: 'Wind Speed:', value: `${windSpeed} km/h`, inline: true },
//                     { name: 'Wind Direction:', value: `${windDirection}°`, inline: true },
//                     { name: 'Humidity:', value: `${relativeHumidity}%`, inline: false },
//                     { name: 'Pressure at sea level:', value: `${relativePressure}hPa`, inline: true },
//                     { name: 'Cloudiness:', value: `${cloudiness}%`, inline: true },
//                 )
//                 .setColor('#0099ff');

//             await interaction.reply({ embeds: [embed] });
//         } catch (error) {
//             console.error('Error fetching weather data:', error);
//             await interaction.reply('Unable to retrieve weather information.');
//         }
//     }
// });

// async function getCoordinates(location) {
//     try {
//         const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${location}&pretty=1&no_annotations=1`;
//         const response = await axios.get(geocodingUrl);

//         if (response.data.results.length === 0) {
//             throw new Error('Location not found');
//         }

//         const { lat, lng } = response.data.results[0].geometry;
//         const formatted = response.data.results[0].formatted;

//         return { lat, lng, formatted };
//     } catch (error) {
//         throw new Error('Error fetching coordinates from OpenCage Geocoding API');
//     }
// }

// async function getWeatherData(coordinates) {
//     const { lat, lng, formatted } = coordinates;
//     const trimmedLat = lat.toString().trim();
//     const trimmedLng = lng.toString().trim();
//     const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,relativehumidity_2m,weathercode,pressure_msl,cloudcover,windspeed_10m,winddirection_10m&forecast_days=1&timezone=auto`;

//     try {
//         const response = await axios.get(weatherUrl);
//         const { hourly, utc_offset_seconds } = response.data;

//         if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
//             throw new Error('Weather data not available');
//         }

//         const currentDateTime = new Date();
//         const adjustedDateTime = new Date(currentDateTime.getTime() + utc_offset_seconds * 1000); // Adjusting current time based on UTC offset
//         const closestTimeIndex = getClosestTimeIndex(hourly.time, adjustedDateTime.getTime());

//         if (closestTimeIndex === -1) {
//             throw new Error('Unable to determine closest time index');
//         }

//         const temperature = hourly.temperature_2m[closestTimeIndex];
//         const weatherCode = hourly.weathercode[closestTimeIndex];
//         const windSpeed = hourly.windspeed_10m[closestTimeIndex];
//         const windDirection = hourly.winddirection_10m[closestTimeIndex];
//         const relativeHumidity = hourly.relativehumidity_2m[closestTimeIndex];
//         const relativePressure = hourly.pressure_msl[closestTimeIndex];
//         const cloudiness = hourly.cloudcover[closestTimeIndex];

//         return {
//             temperature,
//             weatherDescription: getWeatherDescription(weatherCode),
//             windSpeed,
//             windDirection,
//             formatted,
//             relativeHumidity,
//             relativePressure,
//             cloudiness,
//         };
//     } catch (error) {
//         console.error('Error fetching weather data from Open-Meteo API:', error);
//         throw new Error('Error fetching weather data from Open-Meteo API');
//     }
// }

// function getClosestTimeIndex(timeArray, targetDateTime) {
//     let minDiff = Infinity;
//     let closestIndex = 0;

//     for (let i = 0; i < timeArray.length; i++) {
//         const time = timeArray[i];
//         const diff = Math.abs(new Date(time) - targetDateTime);

//         if (diff < minDiff) {
//             minDiff = diff;
//             closestIndex = i;
//         }
//     }

//     return closestIndex;
// }

// function getWeatherDescription(weatherCode) {
//     // Define the weather code to description mappings based on WMO 4677 weather code table
//     const weatherCodeMappings = {
//         0: 'Clear sky',
//         1: 'Mainly clear',
//         2: 'Partly cloudy',
//         3: 'Overcast',
//         45: 'Fog',
//         46: 'Depositing rime fog',
//         51: 'Light drizzle',
//         53: 'Moderate drizzle',
//         55: 'Dense intensity drizzle',
//         56: 'Light freezing drizzle',
//         57: 'Dense freezing drizzle',
//         61: 'Slight rain',
//         63: 'Moderate rain',
//         65: 'Heavy rain',
//         66: 'Light freezing rain',
//         67: 'Heavy freezing rain',
//         71: 'Slight snowfall',
//         73: 'Moderate snowfall',
//         75: 'Heavy snowfall',
//         77: 'Snow grains',
//         80: 'Slight rain showers',
//         81: 'Moderate rain showers',
//         82: 'Violent rain showers',
//         85: 'Slight snow showers',
//         86: 'Heavy snow showers',
//         95: 'Thunderstorm',
//         96: 'Slight hail thunderstorm',
//         99: 'Heavy hail thunderstorm',
//     };

//     return weatherCodeMappings[weatherCode] || 'Unknown';
// }

// client.login(token);

const Discord = require('discord.js');
require('dotenv').config();

const { Client, GatewayIntentBits, Partials } = require('discord.js');
const { REST } = require('@discordjs/rest');
const { Routes } = require('discord-api-types/v9');

const axios = require('axios');
const token = process.env.TOKEN;
const openCageApiKey = process.env.OPENCAGEAPIKEY;
const openWeatherMapApiKey = process.env.OPENWEATHERMAPAPIKEY;
const clientId = process.env.CLIENT_ID;

const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent], partials: [Partials.Channel] });
const commands = [];

client.on('ready', () => {
    console.log(`Logged in as ${client.user.tag}`);
});

async function registerSlashCommands(guildId) {
    try {
        console.log(`Started refreshing application (/) commands for guild ID ${guildId}.`);

        commands.push({
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
        });

        const rest = new REST({ version: '9' }).setToken(token);

        await rest.put(
            guildId ? Routes.applicationGuildCommands(clientId, guildId) : Routes.applicationCommands(clientId),
            { body: commands }
        );

        console.log(`Successfully registered application (/) commands for guild ID ${guildId}.`);
    } catch (error) {
        console.error(`Error registering application (/) commands for guild ID ${guildId}:`, error);
    }
}

client.on('guildCreate', (guild) => {
    const guildId = guild.id;
    registerSlashCommands(guildId);
});

client.on('interactionCreate', async (interaction) => {
    if (!interaction.isCommand()) return;

    const { commandName, options, guildId } = interaction;

    if (commandName === 'weather') {
        const location = options.getString('location');
        if (!location) {
            await interaction.reply('Please provide a location.');
            return;
        }

        try {
            const coordinates = await getCoordinates(location);
            const weatherData = await getWeatherData(coordinates, coordinates.formatted);
            const temperature = weatherData.temperature;
            const weatherDescription = weatherData.weatherDescription;
            const windSpeed = weatherData.windSpeed;
            const windDirection = weatherData.windDirection;
            const reportedLocation = weatherData.formatted;
            const relativeHumidity = weatherData.relativeHumidity;
            const relativePressure = weatherData.relativePressure;
            const cloudiness = weatherData.cloudiness;
            const weatherMapImageUrl = await getWeatherMapImageUrl(coordinates);

            const embed = new Discord.MessageEmbed()
                .setTitle('Weather Information')
                .setDescription(`Weather information for ${reportedLocation}`)
                .addField('Temperature', `${temperature}°C`, true)
                .addField('Weather Description', weatherDescription, true)
                .addField('Wind Speed', `${windSpeed} km/h`, true)
                .addField('Wind Direction', `${windDirection}°`, true)
                .addField('Humidity', `${relativeHumidity}%`, true)
                .addField('Pressure at sea level', `${relativePressure}hPa`, true)
                .addField('Cloudiness', `${cloudiness}%`, true)
                .setImage(weatherMapImageUrl)
                .setColor('#0099ff');

            await interaction.reply({ embeds: [embed] });
        } catch (error) {
            console.error('Error fetching weather data:', error);
            await interaction.reply('Unable to retrieve weather information.');
        }
    }
});

async function getCoordinates(location) {
    try {
        const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${location}&pretty=1&no_annotations=1`;
        const response = await axios.get(geocodingUrl);

        if (response.data.results.length === 0) {
            throw new Error('Location not found');
        }

        const { lat, lng } = response.data.results[0].geometry;
        const formatted = response.data.results[0].formatted;

        return { lat, lng, formatted };
    } catch (error) {
        throw new Error('Error fetching coordinates from OpenCage Geocoding API');
    }
}

async function getWeatherData(coordinates) {
    const { lat, lng, formatted } = coordinates;
    const trimmedLat = lat.toString().trim();
    const trimmedLng = lng.toString().trim();
    const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,relativehumidity_2m,weathercode,pressure_msl,cloudcover,windspeed_10m,winddirection_10m&forecast_days=1&timezone=auto`;

    try {
        const response = await axios.get(weatherUrl);
        const { hourly, utc_offset_seconds } = response.data;

        if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
            throw new Error('Weather data not available');
        }

        const currentDateTime = new Date();
        const adjustedDateTime = new Date(currentDateTime.getTime() + utc_offset_seconds * 1000); // Adjusting current time based on UTC offset
        const closestTimeIndex = getClosestTimeIndex(hourly.time, adjustedDateTime.getTime());

        if (closestTimeIndex === -1) {
            throw new Error('Unable to determine closest time index');
        }

        const temperature = hourly.temperature_2m[closestTimeIndex];
        const weatherCode = hourly.weathercode[closestTimeIndex];
        const windSpeed = hourly.windspeed_10m[closestTimeIndex];
        const windDirection = hourly.winddirection_10m[closestTimeIndex];
        const relativeHumidity = hourly.relativehumidity_2m[closestTimeIndex];
        const relativePressure = hourly.pressure_msl[closestTimeIndex];
        const cloudiness = hourly.cloudcover[closestTimeIndex];

        return {
            temperature,
            weatherDescription: getWeatherDescription(weatherCode),
            windSpeed,
            windDirection,
            formatted,
            relativeHumidity,
            relativePressure,
            cloudiness,
        };
    } catch (error) {
        console.error('Error fetching weather data from Open-Meteo API:', error);
        throw new Error('Error fetching weather data from Open-Meteo API');
    }
}

async function getWeatherMapImageUrl(coordinates) {
  try {
    const { lat, lng } = coordinates;
    const weatherMapUrl = `https://tile.openweathermap.org/map/precipitation_new/10/${lat}/${lng}.png?appid=${openWeatherMapApiKey}`;

    return weatherMapUrl;
  } catch (error) {
    console.error('Error fetching weather map image:', error);
    return null;
  }
}

function getClosestTimeIndex(timeArray, targetDateTime) {
    let minDiff = Infinity;
    let closestIndex = 0;

    for (let i = 0; i < timeArray.length; i++) {
        const time = timeArray[i];
        const timeDiff = Math.abs(targetDateTime - time);

        if (timeDiff < minDiff) {
            minDiff = timeDiff;
            closestIndex = i;
        }
    }

    return closestIndex;
}

function getWeatherDescription(weatherCode) {
    // Add your own weather code to description mappings here
    const weatherDescriptions = {
        0: 'Clear sky',
        1: 'Cloudy sky',
        2: 'Rain',
        3: 'Thunderstorm',
        4: 'Sleet',
        5: 'Snow',
    };

    return weatherDescriptions[weatherCode] || 'Unknown';
}

client.login(token);