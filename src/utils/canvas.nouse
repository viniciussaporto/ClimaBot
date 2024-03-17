import {
	type CommandInteraction,
	ImageAttachment,
	ApplicationCommandOptionTypes,
	ComponentButton,
	MessagePayload,
} from 'discord.js';
import {createCanvas, loadImage, registerFont} from '@napi-rs/canvas';
import axios from 'axios';

// Register any fonts if needed
registerFont('path/to/your/font.ttf', {family: 'YourFontName'});

// Weather code to background image mapping
const weatherCodeImages: Record<string, string> = {
	// Replace these with your actual image URLs
	sunny: 'https://example.com/sunny.png',
	rainy: 'https://example.com/rainy.png',
	// Add more mappings as needed
};

// Function to get the background image based on weather code
async function getBackgroundImage(weatherCode: string): Promise<Buffer> {
	const imageUrl = weatherCodeImages[weatherCode.toLowerCase()];
	if (imageUrl) {
	  const response = await axios.get(imageUrl, {responseType: 'arraybuffer'});
	  return Buffer.from(response.data);
	}

	  throw new Error(`No background image found for weather code: ${weatherCode}`);
}

// Function to generate the forecast image
async function generateForecastImage(forecastData: YourForecastDataType): Promise<Buffer> {
	// Create a canvas
	const canvas = createCanvas(/* Specify canvas dimensions */);
	const ctx = canvas.getContext('2d');

	// Draw the background image based on the current day's weather code
	const backgroundImageBuffer = await getBackgroundImage(forecastData.currentDay.weatherCode);
	const backgroundImage = await loadImage(backgroundImageBuffer);
	ctx.drawImage(backgroundImage, 0, 0, canvas.width, canvas.height);

	// Draw the forecast data in the canvas
	// Customize the following to suit your layout
	ctx.fillStyle = 'white';
	ctx.font = '20px YourFontName';
	ctx.fillText(`Location: ${forecastData.location}`, x, y);
	// Add more drawing logic for other forecast data

	// Convert the canvas to a buffer
	const buffer = canvas.toBuffer('image/png');
	return buffer;
}

// Example usage in your command handler
async function handleForecastCommand(interaction: CommandInteraction) {
	// Replace this with your actual forecast data retrieval logic
	const forecastData = await getForecastDataForLocation(interaction.options.getString('location'));

	// Generate the forecast image
	const forecastImageBuffer = await generateForecastImage(forecastData);

	// Send the image as an attachment
	const attachment = new ImageAttachment(forecastImageBuffer, 'forecast.png');

	await interaction.reply({files: [attachment], components: [button]});
}
