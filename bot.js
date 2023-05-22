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
		const formattedInfo = `Weather in ${location}: ${temperature}°C, ${weatherDescription}`;
		const embed = new EmbedBuilder()
			.setTitle('Weather Information')
			.addFields(
				{ name: 'Temperature:', value: '${temperature}°C' },
				{ name: '\u200B', value: '\u200B' },
				{ name: 'Weather Code description:', value: '${weatherDescription}', inline: true },
				{ name: 'Inline field title', value: 'Some value here', inline: true },
			)
			.setDescription(formattedInfo);
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
	const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,weathercode&forecast_days=1`;
  
	try {
	  const response = await axios.get(weatherUrl);
	  console.log(response.data);
	  const { hourly } = response.data; // Extract hourly data from the response
  
	  if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
		throw new Error('Weather data not available');
	  }
  
	  const currentDateTime = new Date();
	  const closestTimeIndex = getClosestTimeIndex(hourly.time, currentDateTime);
	  const temperature = hourly.temperature_2m[closestTimeIndex];
	  const weatherCode = hourly.weathercode[closestTimeIndex];
	  const weatherDescription = getWeatherDescription(weatherCode);
  
	  return {
		temperature,
		weatherDescription
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
	  1: 'Clear sky with few clouds',
	  2: 'Partly cloudy',
	  3: 'Partly cloudy with rain showers',
	  4: 'Partly cloudy with thunderstorms',
	  5: 'Partly cloudy with snow showers',
	  6: 'Cloudy',
	  7: 'Cloudy with rain showers',
	  8: 'Cloudy with thunderstorms',
	  9: 'Cloudy with snow showers',
	  10: 'Fog',
	  20: 'Mist',
	  21: 'Drizzle',
	  22: 'Rain',
	  23: 'Snowfall',
	  24: 'Rain showers',
	  25: 'Snow showers',
	  26: 'Thunderstorm',
	  27: 'Thunderstorm with hail',
	  28: 'Thunderstorm with snow',
	  29: 'Thunderstorm with heavy snow',
	  30: 'Hail',
	  31: 'Snow',
	  32: 'Dust or sand whirls',
	  33: 'Fog',
	  34: 'Volcanic ash',
	  35: 'Smoke',
	  36: 'Haze',
	  37: 'Blowing snow',
	  38: 'Blowing sand',
	  39: 'Blowing spray',
	  40: 'Mist',
	  41: 'Patchy rain nearby',
	  42: 'Patchy snow nearby',
	  43: 'Patchy sleet nearby',
	  44: 'Patchy freezing drizzle nearby',
	  45: 'Patchy moderate rain',
	  46: 'Moderate or heavy rain nearby',
	  47: 'Patchy light freezing rain',
	  48: 'Moderate or heavy freezing rain',
	  49: 'Light rain shower',
	  50: 'Moderate or heavy rain shower',
	  51: 'Torrential rain shower',
	  52: 'Light sleet showers',
	  53: 'Moderate or heavy sleet showers',
	  54: 'Light snow showers',
	  55: 'Moderate or heavy snow showers',
	  56: 'Light showers of ice pellets',
	  57: 'Moderate or heavy showers of ice pellets',
	  58: 'Light showers of hail',
	  59: 'Moderate or heavy showers of hail',
	  60: 'Light rain',
	  61: 'Moderate rain',
	  62: 'Heavy rain',
	  63: 'Light freezing rain',
	  64: 'Moderate or heavy freezing rain',
	  65: 'Light sleet',
	  66: 'Moderate or heavy sleet',
	  67: 'Light snowfall',
	  68: 'Moderate or heavy snowfall',
	  69: 'Light showers of ice pellets',
	  70: 'Moderate or heavy showers of ice pellets',
	  71: 'Light showers of hail',
	  72: 'Moderate or heavy showers of hail',
	  73: 'Light thunderstorm',
	  74: 'Moderate or heavy thunderstorm',
	  75: 'Light thunderstorm with hail',
	  76: 'Moderate or heavy thunderstorm with hail',
	  77: 'Light thunderstorm with snow',
	  78: 'Moderate or heavy thunderstorm with snow',
	  79: 'Light thunderstorm with heavy snow',
	  80: 'Light drizzle',
	  81: 'Drizzle',
	  82: 'Heavy drizzle',
	  83: 'Light rain',
	  84: 'Moderate rain',
	  85: 'Heavy rain',
	  86: 'Light freezing rain',
	  87: 'Moderate or heavy freezing rain',
	  88: 'Light sleet',
	  89: 'Moderate or heavy sleet',
	  90: 'Patchy light rain',
	  91: 'Light rain shower',
	  92: 'Moderate or heavy rain shower',
	  93: 'Light snow',
	  94: 'Moderate or heavy snow',
	  95: 'Light rain with thunder',
	  96: 'Moderate or heavy rain with thunder',
	  97: 'Light snow with thunder',
	  98: 'Moderate or heavy snow with thunder',
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