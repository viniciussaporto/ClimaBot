const axios = require('axios');
// const { createCanvas, loadImage } = require('canvas');
// const Jimp = require('jimp');

require('dotenv').config();

const openWeatherMapApiKey = process.env.OPENWEATHERMAPAPIKEY;

async function loadImageFromURL(url) {
    try {
        // const response = await axios.get(url, { responseType: 'arraybuffer' });
        // const imageBuffer = Buffer.from(response.data, 'binary');
        // const image = await Jimp.read(imageBuffer);
        // return image;
    } catch (error) {
        console.error('Failed to load image:', error);
        throw error;
    }
}

async function fetchRadarImage(latitude, longitude) {
    try {
        // const zoomLevel = 3;
        // const tileX = Math.floor((longitude + 180) / 360 * (2 ** zoomLevel));
        // const tileY = Math.floor((1 - Math.log(Math.tan(latitude * Math.PI / 180) + 1 / Math.cos(latitude * Math.PI / 180)) / Math.PI) / 2 * (2 ** zoomLevel));

        // const latitudeStr = latitude.toString();
        // const longitudeStr = longitude.toString();

        // const mapUrl = `https://tile.openstreetmap.org/${zoomLevel}/${latitudeStr}/${longitudeStr}.png`;
        // const radarImageUrl = `https://tile.openweathermap.org/map/precipitation_new/${zoomLevel}/${latitudeStr}/${longitudeStr}.png?appid=${openWeatherMapApiKey}`;

        // const [mapImage, radarImage] = await Promise.all([
        //     loadImageFromURL(mapUrl),
        //     loadImageFromURL(radarImageUrl),
        // ]);

        // const canvas = createCanvas(mapImage.width, mapImage.height);
        // const ctx = canvas.getContext('2d');

        // ctx.drawImage(mapImage, 0, 0);
        // ctx.drawImage(radarImage.bitmap, 0, 0);

        // const buffer = canvas.toBuffer();

        // return buffer;
    } catch (error) {
        console.error('Failed to retrieve radar image:', error);
        throw error;
    }
}

module.exports = { fetchRadarImage };