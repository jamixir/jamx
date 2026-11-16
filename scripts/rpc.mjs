#!/usr/bin/env node
// Minimal JIP-2 JSON-RPC client: node rpc.mjs <method> [json-params...]
const url = process.env.JAM_RPC || 'ws://localhost:19800';
const [method, ...rest] = process.argv.slice(2);
const params = rest.map((a) => { try { return JSON.parse(a); } catch { return a; } });
const ws = new WebSocket(url);
ws.onopen = () => ws.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }));
ws.onmessage = (ev) => { console.log(typeof ev.data === 'string' ? ev.data : ev.data.toString()); ws.close(); };
ws.onerror = (e) => { console.error('rpc error', e.message || e); process.exit(1); };
