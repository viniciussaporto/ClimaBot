import DailyRotateFile /*	{type DailyRotateFileTransportOptions}	*/ from 'winston-daily-rotate-file';
import winston from 'winston';
import LokiTransport from 'winston-loki';

type LokiTransportOptions = {
	host: string;
	basicAuth?: string;
	labels: Record<string, string>;
};

const lokiConfig: LokiTransportOptions = {
	host: process.env.LOKI_HOST ?? 'http://localhost:3100',
	basicAuth: process.env.LOKI_USERNAME && process.env.LOKI_PASSWORD
		? `${process.env.LOKI_USERNAME}:${process.env.LOKI_PASSWORD}`
		: undefined,
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
		new DailyRotateFile({
			filename: '/var/log/climabot.log',
			datePattern: 'YYYY-MM-DD',
			maxFiles: '7d',
			zippedArchive: true,
			maxSize: '20m',
			auditFile: '/var/log/climabot-audit.json',
		}) as unknown as winston.transport,
		new LokiTransport(lokiConfig) as unknown as winston.transport,
		new winston.transports.Console(),
	],
});

export default logger;
