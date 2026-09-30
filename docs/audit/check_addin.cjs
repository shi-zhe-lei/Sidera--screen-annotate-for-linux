// Characterization probes; these record current behavior, not release acceptance.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const sourcePath = process.argv[2] || path.resolve(__dirname, '../../wps-addin/js/bridge.js');
const source = fs.readFileSync(sourcePath, 'utf8');

function fixture(withEvents = true, failNetwork = false) {
  const requests = [], listeners = [], timeouts = [], intervals = [], alerts = [];
  const state = { pos: 1, click: 0 };
  const view = { get CurrentShowPosition() { return state.pos; }, GetClickIndex() { return state.click; } };
  const app = { ActivePresentation: { get SlideShowWindow() { return state.pos > 0 ? { View: view } : null; } } };
  if (withEvents) app.ApiEvent = { AddApiEventListener(ev, fn) { listeners.push({ ev, fn }); } };
  const ctx = vm.createContext({
    window: { Application: app },
    fetch(url) { requests.push(url); return failNetwork ? Promise.reject(new Error('offline')) : Promise.resolve({ text: () => Promise.resolve('') }); },
    setTimeout(fn, ms) { timeouts.push({ fn, ms }); },
    setInterval(fn, ms) { intervals.push({ fn, ms }); },
    alert(text) { alerts.push(text); },
    encodeURIComponent,
  });
  vm.runInContext(source, ctx, { filename: sourcePath });
  return { ctx, state, requests, listeners, timeouts, intervals, alerts };
}

(async () => {
  const duplicate = fixture();
  duplicate.ctx.startBridge();
  duplicate.ctx.startBridge();
  const polling = fixture(false);
  polling.ctx.startBridge();
  polling.ctx.bridgePushIfChanged();
  polling.state.pos = -1;
  polling.ctx.bridgePushIfChanged();
  polling.state.pos = 1;
  polling.ctx.bridgePushIfChanged();
  const offline = fixture(true, true);
  offline.ctx.bridgePushIfChanged();
  await Promise.resolve(); await Promise.resolve();
  offline.ctx.bridgePushIfChanged();
  offline.ctx.OnSaPing();
  const pushes = f => f.requests.filter(url => url.includes('/push?'));
  console.log(JSON.stringify({
    repeatedStart: { expectedListeners: 6, observedListeners: duplicate.listeners.length, intervals: duplicate.intervals.length },
    pollingOnlyBeginEndBegin: { expectedStateTransitions: 3, observedPushes: pushes(polling).map(decodeURIComponent) },
    failedPushRetry: { expectedAttemptsAtLeast: 2, observedAttempts: pushes(offline).length },
    offlineStatusButton: { observedAlerts: offline.alerts },
  }, null, 2));
})();
