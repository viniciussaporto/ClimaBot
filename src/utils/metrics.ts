import {collectDefaultMetrics, Counter, Registry} from 'prom-client';

const register = new Registry();
collectDefaultMetrics({register});

export const commandCounter = new Counter({
	name: 'discord_command_total',
	help: 'Count of Discord commands executed',
	labelNames: ['command', 'status'] as const,
	registers: [register],
});

export const roleAssignmentCounter = new Counter({
	name: 'discord_role_assignments_total',
	help: 'Count of role assignments',
	labelNames: ['action', 'role'] as const,
	registers: [register],
});

export const weatherApiCounter = new Counter({
	name: 'weather_api_requests_total',
	help: 'Count of weather API requests',
	labelNames: ['type', 'status'] as const,
	registers: [register],
});

export {register};
