const Discord = require('discord.js');
const { Client, GatewayIntentBits, Partials, EmbedBuilder } = require('discord.js');
const axios = require('axios');

const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildMessages, GatewayIntentBits.MessageContent], partials: [Partials.Channel] });

const token = 'MTEwOTU5ODI0NDg4NDk3OTc5Mg.G_b8DY.87YGtDSJZrMLSMA8jvgbxLDGRpxxx71gyhfOUM';
const openCageApiKey = 'b7b537ab470d47149df63e51c16acbc5';

client.on('ready', () => {
	console.log(`Logged in as ${client.user.tag}`);
  });
  
  client.on('messageCreate', async (message) => {
	if (message.content.startsWith('!weather')) {
	  try {
		const args = message.content.split(' ');
		if (args.length < 2) {
		  message.channel.send('Please provide a location.');
		  return;
		}
  
		const location = args.slice(1).join(' ');
		const coordinates = await getCoordinates(location);
		console.log('Coordinates:', coordinates); // Log coordinates for debugging
		const weatherData = await getWeatherData(coordinates);
		console.log('Weather Data:', weatherData); // Log weather data for debugging
		const temperature = weatherData.temperature; // Access temperature from weatherData
		const weatherDescription = weatherData.weatherDescription; // Access weatherDescription from weatherData
    	const windSpeed = weatherData.windSpeed; // Access windSpeed from weatherData
    	const windDirection = weatherData.windDirection; // Access windDirection from weatherData

		//	const formattedInfo = `Weather in ${location}: ${temperature}°C, ${weatherDescription}`;
		const embed = new EmbedBuilder()
    .setTitle('Weather Information')
    .addFields(
		{ name:	'Location:', value: `${location}`},
    	{ name:	'Temperature:', value: `${temperature}°C`},
    	{ name:	'Weather Description:', value: `${weatherDescription}`},
    	{ name:	'Wind Speed:', value: `${windSpeed} km/h`},
    	{ name:	'Wind Direction:', value: `${windDirection}°`},
	)
    .setColor('#0099ff');

	message.channel.send({ embeds: [embed] });
	  } catch (error) {
		console.error('Error fetching weather information:', error);
		message.channel.send('Error fetching weather information.');
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
  
	  return {lat,lng};
	} catch (error) {
	  throw new Error('Error fetching coordinates from OpenCage Geocoding API');
	}
  }
  
  async function getWeatherData(coordinates) {
	const { lat, lng } = coordinates;
	const trimmedLat = lat.toString().trim();
	const trimmedLng = lng.toString().trim();
	const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,weathercode,windspeed_10m,winddirection_10m&forecast_days=1`;
  
	try {
	  const response = await axios.get(weatherUrl);
	  console.log('Raw Response:', response.data); // Log the raw response
	  const { hourly } = response.data; // Extract hourly data from the response
  
	  if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
		throw new Error('Weather data not available');
	  }
  
		const currentDateTime = new Date();
		const closestTimeIndex = getClosestTimeIndex(hourly.time, currentDateTime);
		const temperature = hourly.temperature_2m[closestTimeIndex];
		const weatherCode = hourly.weathercode[closestTimeIndex];
		const weatherDescription = getWeatherDescription(weatherCode);
		const windSpeed = hourly.windspeed_10m[closestTimeIndex];
		const windDirection = hourly.winddirection_10m[closestTimeIndex];
    return {
      temperature,
      weatherDescription,
      windSpeed,
      windDirection
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
	  const time = new Date(timeArray[i]);
	  const diff = Math.abs(time - targetDateTime);
  
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

// const { Discord, GatewayIntentBits } = require('discord.js');
// const axios = require('axios');

// const client = new Discord.Client({
// 	intents: [
// 		GatewayIntentBits.Guilds,
// 		GatewayIntentBits.GuildMessages,
// 		GatewayIntentBits.MessageContent,
// 	],
//   });

// const token = 'MTEwOTU5ODI0NDg4NDk3OTc5Mg.G_b8DY.87YGtDSJZrMLSMA8jvgbxLDGRpxxx71gyhfOUM';
// const apiKey = 'YOUR_WEATHER_API_KEY';

// client.on('ready', () => {
//   console.log(`Logged in as ${client.user.tag}`);
// });

// client.on('message', (message) => {
//   if (message.content.startsWith('!weather')) {
//     const location = message.content.split(' ')[1];
//     const apiUrl = `https://api.open-meteo.com/v1/forecast?location=${location}`;

//     axios.get(apiUrl)
//       .then((response) => {
//         const weatherInfo = response.data;
//         const formattedInfo = `Weather in ${location}: ${weatherInfo.temperature}°C, ${weatherInfo.description}`;
//         message.channel.send(formattedInfo);
//       })
//       .catch((error) => {
//         console.error('Error fetching weather information:', error);
//         message.channel.send('Error fetching weather information.');
//       });
//   }
// });

// client.login(token);