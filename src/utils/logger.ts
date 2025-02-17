import winston from 'winston';
import LokiTransport from 'winston-loki';

const lokiConfig = {
	host: process.env.LOKI_HOST ?? 'http://localhost:3100',
	basicAuth: process.env.LOKI_USERNAME && process.env.LOKI_PASSWORD
		? `${process.env.LOKI_USERNAME}:${process.env.LOKI_PASSWORD}`
		: '',
	labels: {
		app: 'discord-bot',
		environment: process.env.NODE_ENV ?? 'development',
	},
};

const logger = winston.createLogger({
	format: winston.format.combine(
		winston.format.timestamp(),
		winston.format.json(),
	),
	transports: [
		new LokiTransport(lokiConfig),
		new winston.transports.Console(),
	],
});

export default logger;
