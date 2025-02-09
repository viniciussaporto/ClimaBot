import {collectDefaultMetrics, Counter, Histogram} from 'prom-client';
import {createServer} from 'http';
import express from 'express';

const app = express();
const metricsPort = 4000;

// Initialize metrics
collectDefaultMetrics();

export const apiRequestCounter = new Counter({
	name: 'api_requests_total',
	help: 'Total number of API requests',
	labelNames: ['api', 'status'] as const,
});

export const apiResponseTimeHistogram = new Histogram({
	name: 'api_response_time_seconds',
	help: 'API response time in seconds',
	labelNames: ['api'] as const,
	buckets: [0.1, 0.5, 1, 2, 5],
});

// Start metrics server
app.get('/metrics', async (req, res) => {
	res.set('Content-Type', 'text/plain');
	res.end(await register.metrics());
});

app.listen(metricsPort, () => {
	console.log(`Metrics server running on port ${metricsPort}`);
});
