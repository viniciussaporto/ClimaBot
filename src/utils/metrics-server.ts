import express, {type Request, type Response} from 'express';
import {register} from './metrics';
import logger from './logger';

const app = express();
const port = Number(process.env.METRICS_PORT) || 9464;

app.get('/metrics', async (req: Request, res: Response) => {
	try {
		logger.debug('Metrics endpoint accessed');
		logger.silly(`Metrics request headers: ${JSON.stringify(req.headers)}`);
		res.set('Content-Type', register.contentType);
		res.end(await register.metrics());
	} catch (err) {
		res.status(500).end(err instanceof Error ? err.message : 'Unknown error');
		logger.error('Metrics endpoint error:', err);
		logger.warn('Failed to serve metrics');
	}
});

app.listen(port, '0.0.0.0', () => {
	console.log(`Metrics server listening at http://0.0.0.0:${port}`);
	logger.info(`Metrics server listening at http://0.0.0.0:${port}`);
	logger.verbose(`Metrics server PID: ${process.pid}`);
});
