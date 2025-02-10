import axios, {AxiosError} from 'axios';
import dotenv from 'dotenv';
import {apiRequestCounter, apiResponseTimeHistogram} from './metrics';

dotenv.config();

const openCageApiKey = process.env.OPENCAGEAPIKEY;

export type Location = {
	lat: number;
	lng: number;
	formattedLocation: string;
};

export type WeatherData = {
	latitude: number;
	longitude: number;
	generationtime_ms: number;
	utcOffsetSeconds: number;
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

async function withMetrics<T>(apiName: string, fn: () => Promise<T>): Promise<T> {
	const end = apiResponseTimeHistogram.labels(apiName).startTimer();
	try {
		const result = await fn();
		apiRequestCounter.labels(apiName, 'success').inc();
		return result;
	} catch (error: unknown) {
		apiRequestCounter.labels(apiName, 'error').inc();
		if (error instanceof Error) {
			throw error;
		}

		throw new Error('Unknown error occurred');
	} finally {
		end();
	}
}

export async function getForecastData(coordinates: Location): Promise<ForecastData> {
	return withMetrics('openmeteo_forecast', async () => {
		const {lat, lng} = coordinates;
		const trimmedLat = lat.toString().trim();
		const trimmedLng = lng.toString().trim();
		const forecastUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=5`;

		try {
			const response = await axios.get<ForecastData>(forecastUrl);
			return response.data;
		} catch (error: unknown) {
			if (axios.isAxiosError(error)) {
				console.error('Axios error fetching forecast data:', error);
			} else {
				console.error('Unexpected error fetching forecast data:', error);
			}

			throw new Error('Error fetching forecast data from Open-Meteo API');
		}
	});
}

export async function getFormattedLocation(coordinates: Location): Promise<string> {
	const {lat, lng} = coordinates;

	try {
		const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${lat}+${lng}&pretty=1&no_annotations=1`;
		const response = await axios.get<GeocodingApiResponse>(geocodingUrl);

		if (response.data.results.length === 0) {
			throw new Error('Location not found');
		}

		const {formatted} = response.data.results[0];

		return formatted;
	} catch (error: unknown) {
		if (axios.isAxiosError(error)) {
			console.error('Axios error fetching coordinates:', error);
		} else {
			console.error('Unexpected error fetching coordinates:', error);
		}

		throw new Error('Error fetching coordinates from OpenCage Geocoding API');
	}
}

export async function getCoordinates(location?: string): Promise<Location> {
	return withMetrics('opencage_geocode', async () => {
		try {
			const geocodingUrl = `https://api.opencagedata.com/geocode/v1/json?key=${openCageApiKey}&q=${location}&pretty=1&no_annotations=1`;
			const response = await axios.get<GeocodingApiResponse>(geocodingUrl);

			if (response.data.results.length === 0) {
				throw new Error('Location not found');
			}

			const {lat, lng} = response.data.results[0].geometry;
			const {formatted} = response.data.results[0];

			return {lat, lng, formattedLocation: formatted};
		} catch (error: unknown) {
			if (axios.isAxiosError(error)) {
				console.error('Axios error fetching coordinates:', error);
			} else {
				console.error('Unexpected error fetching coordinates:', error);
			}

			throw new Error('Error fetching coordinates from OpenCage Geocoding API');
		}
	});
}

export async function getWeatherData(coordinates: Location): Promise<WeatherData> {
	return withMetrics('openmeteo_weather', async () => {
		const {lat, lng, formattedLocation} = coordinates;
		const trimmedLat = lat.toString().trim();
		const trimmedLng = lng.toString().trim();
		const weatherUrl = `https://api.open-meteo.com/v1/forecast?latitude=${trimmedLat}&longitude=${trimmedLng}&hourly=temperature_2m,relativehumidity_2m,weathercode,pressure_msl,cloudcover,windspeed_10m,winddirection_10m&forecast_days=1&timezone=auto`;

		try {
			const response = await axios.get<WeatherData>(weatherUrl);
			const {hourly, utcOffsetSeconds} = response.data;

			if (!hourly?.temperature_2m || hourly.temperature_2m.length === 0) {
				throw new Error('Weather data not available');
			}

			const currentDateTime = new Date();
			const adjustedDateTime = new Date(currentDateTime.getTime() + (utcOffsetSeconds * 1000));
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
				formattedLocation,
				relativeHumidity,
				relativePressure,
				cloudiness,
				weatherCode,
				trimmedLat,
				trimmedLng,
			};
		} catch (error: unknown) {
			if (axios.isAxiosError(error)) {
				console.error('Axios error fetching weather data:', error);
			} else {
				console.error('Unexpected error fetching weather data:', error);
			}

			throw new Error('Error fetching weather data from Open-Meteo API');
		}
	});
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

const getWeatherDescription = (weatherCode: number): string => {
	const weatherCodeMappings = new Map<number, string>([
		[0, 'Clear sky'],
		[1, 'Mainly clear'],
		[2, 'Partly cloudy'],
		[3, 'Overcast'],
		[45, 'Fog'],
		[46, 'Depositing rime fog'],
		[51, 'Light drizzle'],
		[53, 'Moderate drizzle'],
		[55, 'Dense intensity drizzle'],
		[56, 'Light freezing drizzle'],
		[57, 'Dense freezing drizzle'],
		[61, 'Slight rain'],
		[63, 'Moderate rain'],
		[65, 'Heavy rain'],
		[66, 'Light freezing rain'],
		[67, 'Heavy freezing rain'],
		[71, 'Slight snowfall'],
		[73, 'Moderate snowfall'],
		[75, 'Heavy snowfall'],
		[77, 'Snow grains'],
		[80, 'Slight rain showers'],
		[81, 'Moderate rain showers'],
		[82, 'Violent rain showers'],
		[85, 'Slight snow showers'],
		[86, 'Heavy snow showers'],
		[95, 'Thunderstorm'],
		[96, 'Slight hail thunderstorm'],
		[99, 'Heavy hail thunderstorm'],
	]);

	return weatherCodeMappings.get(weatherCode) ?? 'Unknown';
};
