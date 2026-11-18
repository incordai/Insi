// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

// Listener retention is a runtime resource leak, independent of GC timing and RSS noise.
const { EventEmitter } = require('node:events');
const emitter = new EventEmitter();
emitter.setMaxListeners(0);
for (let i = 0; i < 64; i++) {
  const listener = () => i;
  emitter.on('data', listener);
  if (process.argv[2] === 'fixed') emitter.off('data', listener);
}
process.stdout.write(JSON.stringify({ retained_listeners: emitter.listenerCount('data') }) + '\n');
