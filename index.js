const Discord = require('discord.js');
require('dotenv').config();

const { Client, GatewayIntentBits, Partials, EmbedBuilder } = require('discord.js');
const { REST } = require('@discordjs/rest');
const { Routes } = require('discord-api-types/v9');

const axios = require('axios');
const token = process.env.TOKEN;
const openCageApiKey = process.env.OPENCAGEAPIKEY;
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

        const embed = new Discord.EmbedBuilder()
            .setTitle('Weather Information')
            .addFields(
                { name: 'Location', value: `${reportedLocation}.`, inline: true},
				{ name: 'Weather Description:', value: weatherDescription, inline: true },
                { name: 'Temperature:', value: `${temperature}°C` },
                { name: 'Wind Speed:', value: `${windSpeed} km/h`, inline: true },
                { name: 'Wind Direction:', value: `${windDirection}°`, inline: true },
				{ name: 'Humidity:', value: `${relativeHumidity}%`, inline: true },
				{ name: 'Pressure at sea level:', value: `${relativePressure}hPa`, inline: true },
                { name: 'Cloudiness:', value: `${cloudiness}%`, inline: false },
            )
            .setColor('#0099ff');

        await interaction.reply({ embeds: [embed] });
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

function getClosestTimeIndex(timeArray, targetDateTime) {
    let minDiff = Infinity;
    let closestIndex = 0;
  
    for (let i = 0; i < timeArray.length; i++) {
      const time = timeArray[i];
      const diff = Math.abs(new Date(time) - targetDateTime);
  
      if (diff < minDiff) {
        minDiff = diff;
        closestIndex = i;
      }
    }
  
    return closestIndex;
  }

function getWeatherDescription(weatherCode) {
	// Define the weather code to description mappings based on WMO 4677 weather code table
	const weatherCodeMappings = {
	  0: 'Clear sky',
	  1: 'Mainly clear',
	  2: 'Partly cloudy',
	  3: 'Overcast',
	  45: 'Fog',
	  46: 'Depositing rime fog',
	  51: 'Light drizzle',
	  53: 'Moderate drizzle',
	  55: 'Dense intensity drizzle',
	  56: 'Light freezing drizzle',
	  57: 'Dense freezing drizzle',
	  61: 'Slight rain',
	  63: 'Moderte rain',
	  65: 'Heavy rain',
	  66: 'Light freezing rain',
	  67: 'Heavy freezing rain',
	  71: 'Slight snowfall',
	  73: 'Moderate snowfall',
	  75: 'Heavy snowfall',
	  77: 'Snow grains',
	  80: 'Slight rain showers',
	  81: 'Moderate rain showers',
	  82: 'Violent rain showers',
	  85: 'Slight snow showers',
	  86: 'Heavy snow showers',
	  95: 'Thunderstorm',
	  96: 'Slight hail thunderstorm',
	  99: 'Heavy hail thunderstorm',
	};
  
	return weatherCodeMappings[weatherCode] || 'Unknown';
  }

client.login(token);

// const Discord = require('discord.js');
// require('dotenv').config();

// const { Client, GatewayIntentBits, Partials, EmbedBuilder } = require('discord.js');

// const axios = require('axios');
// const token = process.env.TOKEN; 
// const openCageApiKey = process.env.OPENCAGEAPIKEY; 

// const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent], partials: [Partials.Channel] });

// client.on('ready', () => {
// 	console.log(`Logged in as ${client.user.tag}`);
//   });
  
//   client.on('messageCreate', async (message) => {
// 	if (message.content.startsWith('!weather')) {
// 	  try {
// 		const args = message.content.split(' ');
// 		if (args.length < 2) {
// 		  message.channel.send('Please provide a location.');
// 		  return;
// 		}
  
// 		const location = args.slice(1).join(' '); // Gets 1st arg and joins the spaces for OpenCage API
// 		const coordinates = await getCoordinates(location);
// 		//console.log('Coordinates:', coordinates); // Log coordinates for debugging
// 		const weatherData = await getWeatherData(coordinates);
// 		//console.log('Weather Data:', weatherData); // Log weather data for debugging
// 		const temperature = weatherData.temperature; // Access temperature from weatherData
// 		const weatherDescription = weatherData.weatherDescription; // Access weatherDescription from weatherData
//     	const windSpeed = weatherData.windSpeed; // Access windSpeed from weatherData
//     	const windDirection = weatherData.windDirection; // Access windDirection from weatherData

// 		//	const formattedInfo = `Weather in ${location}: ${temperature}°C, ${weatherDescription}`;
// 		const embed = new EmbedBuilder() // Embed builder, still missing images, 'feels like' temperature, barometric pressure and other stuff
//     .setTitle('Weather Information')
//     .addFields(
// 		{ name:	'Location:', value: `${location}`}, // Needs changing to OpenCage API complete locale description
//     	{ name:	'Temperature:', value: `${temperature}°C`},
//     	{ name:	'Weather Description:', value: `${weatherDescription}`},
//     	{ name:	'Wind Speed:', value: `${windSpeed} km/h`},
//     	{ name:	'Wind Direction:', value: `${windDirection}°`},
// 	)
//     .setColor('#0099ff');

// 	message.channel.send({ embeds: [embed] });
// 	  } catch (error) {
// 		console.error('Error fetching weather information:', error);
// 		message.channel.send('Error fetching weather information.');
// 	  }
// 	}
//   });
  
//   async function getCoordinates(location) {
// 	try {
// 	  const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${location}&pretty=1&no_annotations=1`;
// 	  const response = await axios.get(geocodingUrl);
  
// 	  if (response.data.results.length === 0) {
// 		throw new Error('Location not found');
// 	  }
  
// 	  const { lat, lng } = response.data.results[0].geometry;
  
// 	  return {lat,lng};
// 	} catch (error) {
// 	  throw new Error('Error fetching coordinates from OpenCage Geocoding API');
// 	}
//   }
  
//   async function getWeatherData(coordinates) {
// 	const { lat, lng } = coordinates;
// 	const trimmedLat = lat.toString().trim(); //Converts int to string and trims it for Open-Meteo
// 	const trimmedLng = lng.toString().trim();
// 	const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,weathercode,windspeed_10m,winddirection_10m&forecast_days=1`;
  
// 	try {
// 	  const response = await axios.get(weatherUrl);
// 	  //console.log('Raw Response:', response.data); // Log the raw response
// 	  const { hourly } = response.data; // Extract hourly data from the response
  
// 	  if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
// 		throw new Error('Weather data not available');
// 	  }
  
// 		const currentDateTime = new Date(); //Gets current date and time
// 		const closestTimeIndex = getClosestTimeIndex(hourly.time, currentDateTime); //Does the approximation because Open-Meteo only deals in full hours
// 		const temperature = hourly.temperature_2m[closestTimeIndex]; //Gets temperature from the json provided by the API
// 		const weatherCode = hourly.weathercode[closestTimeIndex]; //Gets weather code for condition from the API
// 		const weatherDescription = getWeatherDescription(weatherCode); //Converts code to it's correct designation according to WMO code table
// 		const windSpeed = hourly.windspeed_10m[closestTimeIndex]; //Gets wind speed
// 		const windDirection = hourly.winddirection_10m[closestTimeIndex]; //Gets wind direction
//     return {
//       temperature,
//       weatherDescription,
//       windSpeed,
//       windDirection
//     };
// 	} catch (error) {
// 	  console.error('Error fetching weather data from Open-Meteo API:', error);
// 	  throw new Error('Error fetching weather data from Open-Meteo API');
// 	}
//   }

//   function getClosestTimeIndex(timeArray, targetDateTime) {
// 	let minDiff = Infinity;
// 	let closestIndex = 0;
  
// 	for (let i = 0; i < timeArray.length; i++) {
// 	  const time = new Date(timeArray[i]);
// 	  const diff = Math.abs(time - targetDateTime);
  
// 	  if (diff < minDiff) {
// 		minDiff = diff;
// 		closestIndex = i; // Does the diff and reachs to the lesser value to use in the index
// 	  }
// 	}
  
// 	return closestIndex;
//   }

//   function getWeatherDescription(weatherCode) {
// 	// Define the weather code to description mappings based on WMO 4677 weather code table
// 	const weatherCodeMappings = {
// 	  0: 'Clear sky',
// 	  1: 'Mainly clear',
// 	  2: 'Partly cloudy',
// 	  3: 'Overcast',
// 	  45: 'Fog',
// 	  46: 'Depositing rime fog',
// 	  51: 'Light drizzle',
// 	  53: 'Moderate drizzle',
// 	  55: 'Dense intensity drizzle',
// 	  56: 'Light freezing drizzle',
// 	  57: 'Dense freezing drizzle',
// 	  61: 'Slight rain',
// 	  63: 'Moderte rain',
// 	  65: 'Heavy rain',
// 	  66: 'Light freezing rain',
// 	  67: 'Heavy freezing rain',
// 	  71: 'Slight snowfall',
// 	  73: 'Moderate snowfall',
// 	  75: 'Heavy snowfall',
// 	  77: 'Snow grains',
// 	  80: 'Slight rain showers',
// 	  81: 'Moderate rain showers',
// 	  82: 'Violent rain showers',
// 	  85: 'Slight snow showers',
// 	  86: 'Heavy snow showers',
// 	  95: 'Thunderstorm',
// 	  96: 'Slight hail thunderstorm',
// 	  99: 'Heavy hail thunderstorm',
// 	};
  
// 	return weatherCodeMappings[weatherCode] || 'Unknown';
//   }

// client.login(token);