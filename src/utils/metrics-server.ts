import express, {type Request, type Response} from 'express';
import {register} from './metrics';

const app = express();
const port = Number(process.env.METRICS_PORT) || 9464;

app.get('/metrics', async (req: Request, res: Response) => {
	try {
		res.set('Content-Type', register.contentType);
		res.end(await register.metrics());
	} catch (err) {
		res.status(500).end(err instanceof Error ? err.message : 'Unknown error');
	}
});

app.listen(port, '0.0.0.0', () => {
	console.log(`Metrics server listening at http://0.0.0.0:${port}`);
});
