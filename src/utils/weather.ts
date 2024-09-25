// Disabling the naming-convention rule as we need objects keyed by number
/* eslint-disable @typescript-eslint/naming-convention */

import axios from 'axios';
import dotenv from 'dotenv';
import {createCanvas, loadImage} from 'canvas';
import {createGzip} from 'zlib';

dotenv.config();

const openCageApiKey = process.env.OPENCAGEAPIKEY;
const rainviewerApiKey = process.env.RAINVIEWERAPIKEY;

export type Location = {
	lat: number;
	lng: number;
	formattedLocation: string;
};

export type WeatherData = {
	latitude: number;
	longitude: number;
	generationtime_ms: number;
	utc_offset_seconds: number;
	timezone: string;
	timezone_abbreviation: string;
	elevation: number;
	hourly_units: HourlyUnits;
	hourly: Hourly;
};

export type Hourly = {
	time: string[];
	temperature_2m: number[];
	relativehumidity_2m: number[];
	weathercode: number[];
	pressure_msl: number[];
	cloudcover: number[];
	windspeed_10m: number[];
	winddirection_10m: number[];
};

export type HourlyUnits = {
	time: string;
	temperature_2m: string;
	relativehumidity_2m: string;
	weathercode: string;
	pressure_msl: string;
	cloudcover: string;
	windspeed_10m: string;
	winddirection_10m: string;
};

export type ForecastData = {
	daily_units: DailyUnits;
	daily: Daily;
	formattedLocation: string;
};

export type Daily = {
	time: string[];
	temperature_2m_max: number[];
	temperature_2m_min: number[];
	precipitation_probability_max: number[];
};

export type DailyUnits = {
	time: string;
	temperature_2m_max: string;
	temperature_2m_min: string;
	precipitation_probability_max: string;
};

type GeocodingApiResponse = {
	results: Array<{
		formatted: string;
		geometry: {
			lat: number;
			lng: number;
		};
	}>;
};

export async function getForecastData(coordinates: Location) {
	const {lat, lng} = coordinates;
	const trimmedLat = lat.toString().trim();
	const trimmedLng = lng.toString().trim();
	const forecastUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=5`;

	try {
		const response = await axios.get<ForecastData>(forecastUrl);
		return response.data;
	} catch (error) {
		console.error('Error fetching forecast data from Open-Meteo API:', error);
		throw new Error('Error fetching forecast data from Open-Meteo API');
	}
}

export async function getFormattedLocation(coordinates: Location): Promise<string> {
	const {lat, lng} = coordinates;

	try {
		const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${lat}+${lng}&language=en&no_annotations=1&pretty=1`;
		const response = await axios.get<GeocodingApiResponse>(geocodingUrl);

		console.log('geocodingUrl:', geocodingUrl);

		if (response.data.results.length === 0) {
			throw new Error('Location not found');
		}

		const {formatted} = response.data.results[0];

		return formatted;
	} catch (error) {
		console.error('Error fetching coordinates from OpenCage Geocoding API:', error);
		throw new Error('Error fetching coordinates from OpenCage Geocoding API');
	}
}

export async function getCoordinates(location?: string): Promise<Location> {
	try {
		const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${location}&pretty=1&no_annotations=1`;
		const response = await axios.get<GeocodingApiResponse>(geocodingUrl);

		if (response.data.results.length === 0) {
			throw new Error('Location not found');
		}

		const {lat, lng} = response.data.results[0].geometry;
		const {formatted} = response.data.results[0];

		return {lat, lng, formattedLocation: formatted};
	} catch (error) {
		throw new Error('Error fetching coordinates from OpenCage Geocoding API');
	}
}

export async function getWeatherData(coordinates: Location) {
	const {lat, lng, formattedLocation} = coordinates;
	const trimmedLat = lat.toString().trim();
	const trimmedLng = lng.toString().trim();
	const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,relativehumidity_2m,weathercode,pressure_msl,cloudcover,windspeed_10m,winddirection_10m&forecast_days=1&timezone=auto`;

	try {
		const response = await axios.get<WeatherData>(weatherUrl);
		const {hourly, utc_offset_seconds} = response.data;

		if (!hourly?.temperature_2m || hourly.temperature_2m.length === 0) {
			throw new Error('Weather data not available');
		}

		const currentDateTime = new Date();
		const adjustedDateTime = new Date(currentDateTime.getTime() + (utc_offset_seconds * 1000)); // Adjusting current time based on UTC offset
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
		const weatherImage = await getWeatherImageFromRainviewer(lat, lng);
		const weatherMap = await generateWeatherMap(lat, lng, weatherImage);

		return {
			temperature,
			weatherDescription: getWeatherDescription(weatherCode),
			windSpeed,
			windDirection,
			formattedLocation,
			relativeHumidity,
			relativePressure,
			cloudiness,
			weatherCode,
			trimmedLat,
			trimmedLng,
			weatherImage,
			weatherMap,
		};
	} catch (error) {
		console.error('Error fetching weather data from Open-Meteo API:', error);
		throw new Error('Error fetching weather data from Open-Meteo API');
	}
}

export async function generateWeatherMap(lat: number, lng: number, weatherImage: string): Promise<Buffer> {
	const mapWidth = 600;
	const mapHeight = 400;
	const zoom = 10;

	const osmUrl = `https://a.tile.openstreetmap.org/${zoom}/${Math.floor((lng + 180) / 360 * 2 ** zoom)}/${Math.floor((1 - Math.log(Math.tan(lat * Math.PI / 180) + 1 / Math.cos(lat * Math.PI / 180)) / Math.PI) / 2 * 2 ** zoom)}.png`;

	// Get OSM tile
	const canvas = createCanvas(mapWidth, mapHeight);
	const ctx = canvas.getContext('2d');

	// Load and draw the OSM tile
	const mapImage = await loadImage(osmUrl);
	ctx.drawImage(mapImage, 0, 0, mapWidth, mapHeight);

	// Load and draw the weather overlay
	const overlay = await loadImage(osmUrl);
	ctx.globalAlpha = 0.6; // Set transparency for overlay
	ctx.drawImage(overlay, 0, 0, mapWidth, mapHeight);
	ctx.globalAlpha = 1.0;

	// Convert canvas to buffer
	return canvas.toBuffer('image/png');
}

async function getWeatherImageFromRainviewer(lat: number, lng: number): Promise<string> {
	const rainviewerUrl = `https://api.rainviewer.com/public/weather?lat=${lat}&lon=${lng}&key=${rainviewerApiKey}`;

	try {
		const response = await axios.get(rainviewerUrl);

		if (response.data?.images) {
			return response.data.images[0].url;
		}

		throw new Error('No image found!');
	} catch (error) {
		console.error('Error fetching weather image from Rainviewer API:', error);
		throw new Error('Error fetching weather image from Rainviewer API');
	}
}

function getClosestTimeIndex(timeArray: string[], targetDateTime: number): number {
	let minDiff = Infinity;
	let closestIndex = 0;

	for (let i = 0; i < timeArray.length; i++) {
		const time = timeArray[i];
		const diff = Math.abs(new Date(time).getTime() - targetDateTime);

		if (diff < minDiff) {
			minDiff = diff;
			closestIndex = i;
		}
	}

	return closestIndex;
}

function getWeatherDescription(weatherCode: number) {
	// Define the weather code to description mappings based on WMO 4677 weather code table
	const weatherCodeMappings: Record<number, string> = {
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
		63: 'Moderate rain',
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
