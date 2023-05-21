const Discord = require('discord.js');
const { Client, GatewayIntentBits, Partials } = require('discord.js');
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
		message.channel.send(formattedInfo);
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
	const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,weathercode`;
  
	try {
	  const response = await axios.get(weatherUrl);
	  console.log(response.data);
	  const { hourly } = response.data;
  
	  if (!hourly || !hourly.temperature_2m || hourly.temperature_2m.length === 0) {
		throw new Error('Weather data not available');
	  }
  
	  const temperature = hourly.temperature_2m[0].value; // Update the temperature extraction
	  const weatherCode = hourly.weathercode[0].value; // Update the weather code extraction
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

function getWeatherDescription(weatherCode) {
  switch (weatherCode) {
    case 'clear_sky':
      return 'Clear sky';
    case 'partly_cloudy':
      return 'Partly cloudy';
    case 'cloudy':
      return 'Cloudy';
    case 'rain_showers':
      return 'Rain showers';
    case 'rain':
      return 'Rain';
    case 'thunderstorm':
      return 'Thunderstorm';
    case 'snow':
      return 'Snow';
    default:
      return 'Unknown';
  }
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