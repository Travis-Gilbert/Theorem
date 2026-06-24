import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

const webextensionMock = fileURLToPath(new URL('./src/utils/__mocks__/webextension-polyfill.ts', import.meta.url));

export default defineConfig({
	define: {
		DEBUG_MODE: false,
	},
	resolve: {
		alias: {
			'webextension-polyfill': webextensionMock,
		},
	},
	test: {
		include: ['src/**/*.test.ts'],
		globals: true,
	},
});
