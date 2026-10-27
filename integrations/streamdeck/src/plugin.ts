import streamDeck from '@elgato/streamdeck';
import { ControlClient, pipeName } from './client.mjs';
import { specs } from './model.mjs';
import { PirateWaveAction, type Spec } from './actions.js';

process.on('uncaughtException', error => streamDeck.logger.error(error));
process.on('unhandledRejection', error => streamDeck.logger.error(error));
const client = new ControlClient(await pipeName());
client.on('diagnostic', message => streamDeck.logger.warn(message));
for (const spec of specs) streamDeck.actions.registerAction(new PirateWaveAction(spec as Spec, client, streamDeck.ui, error => streamDeck.logger.error(error)));
process.once('SIGTERM', () => client.stop());
process.once('SIGINT', () => client.stop());
await streamDeck.connect();
client.start();
