import { AttachmentBuilder } from "discord.js";

export const getWeatherImage = (weatherCode: number): string => {
	let weatherImage: string

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

	return weatherImage;
};