const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../js/bridge.js'), 'utf8');
const settle = () => new Promise(resolve => setImmediate(resolve));

function fixture({ abort = true } = {}) {
  const requests = [], timeouts = new Map(), commands = [];
  const state = { pos: 1, click: 0 };
  let nextId = 0, throwFetch = false;
  const view = {
    get CurrentShowPosition() { return state.pos; },
    GetClickIndex() { return state.click; },
    Next() { commands.push('NEXT'); },
    Previous() { commands.push('PREV'); },
    Exit() { commands.push('EXIT'); state.pos = -1; },
  };
  const context = vm.createContext({
    window: { _sideraFullMode: true, _sideraAllowPoll: true, _sideraChannel: 'poll', Application: { ActivePresentation: {
      get SlideShowWindow() { return state.pos > 0 ? { View: view } : null; },
    } } },
    AbortController: abort ? AbortController : undefined,
    fetch(url, options) {
      const request = { url, options };
      requests.push(request);
      if (throwFetch) throw new Error('fetch unavailable');
      return new Promise((resolve, reject) => Object.assign(request, { resolve, reject }));
    },
    setTimeout(callback, delay) {
      const id = ++nextId;
      timeouts.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) { timeouts.delete(id); },
    encodeURIComponent,
  });
  function loadScript() { vm.runInContext(source, context); }
  loadScript();
  return {
    context, state, requests, timeouts, commands, loadScript, view,
    throwFetch(value) { throwFetch = value; },
    tick() { context.bridgePushIfChanged(); },
    async answer(index, body = 'OK', status = 200) {
      requests[index].resolve({ ok: status >= 200 && status < 300, text: () => Promise.resolve(body) });
      await settle();
    },
    async fail(index) {
      requests[index].reject(new Error('offline'));
      await settle();
    },
    expire() {
      assert.equal(timeouts.size, 1, 'one state request deadline must be pending');
      const [id, timer] = timeouts.entries().next().value;
      assert.equal(timer.delay, 3000);
      timeouts.delete(id);
      timer.callback();
    },
    message(index) { return new URL(requests[index].url).searchParams.get('m').replace(/ sid=[^ ]+$/, ''); },
  };
}

test('request sequences survive script reload and increase across push and poll', () => {
  const f = fixture();
  f.context.bridgeFetch('/push?m=EVENT%20SlideShowState%20pos=1');
  f.loadScript();
  f.context.bridgeFetch('/poll?m=EVENT%20SlideShowState%20pos=2');
  const first = new URL(f.requests[0].url).searchParams;
  const second = new URL(f.requests[1].url).searchParams;
  assert.ok(first.get('sid'));
  assert.equal(second.get('sid'), first.get('sid'));
  assert.equal(Number(second.get('seq')), Number(first.get('seq')) + 1);
});

test('health check reports online only after a successful bridge response', async () => {
  const f = fixture(); const messages = [];
  f.context.alert = message => messages.push(message);
  f.context.OnSaPing(); assert.equal(messages.length, 0);
  await f.fail(0); assert.match(messages[0], /未连接/);
  f.context.OnSaPing(); await f.answer(1, 'OK sidera');
  assert.equal(messages[1], '已连接 Sidera');
  assert.equal(f.timeouts.size, 0);
});

test('two failed pushes retry the same page until an OK acknowledgement', async () => {
  const f = fixture();
  for (let i = 0; i < 2; i++) {
    f.tick();
    assert.equal(f.requests.length, i + 1);
    await f.fail(i);
    assert.equal(f.timeouts.size, 0);
  }
  f.tick();
  assert.equal(f.requests.length, 3);
  await f.answer(2, ' OK\n');
  for (let i = 0; i < 10; i++) f.tick();
  assert.equal(f.requests.length, 3, 'acknowledged unchanged state must not be resent');
  assert.equal(f.timeouts.size, 0);
  assert.ok(f.requests.every(request => request.options.cache === 'no-store'));
});

for (const [label, body, status] of [
  ['HTTP failure', 'OK', 500], ['unexpected response body', 'NEXT', 200],
]) {
  test(`${label} does not acknowledge state or execute a command`, async () => {
    const f = fixture();
    f.tick();
    await f.answer(0, body, status);
    f.tick();
    assert.equal(f.requests.length, 2);
    assert.deepEqual(f.commands, []);
  });
}

test('response body read failure permits another attempt', async () => {
  const f = fixture();
  f.tick();
  f.requests[0].resolve({ ok: true, text: () => Promise.reject(new Error('body interrupted')) });
  await settle();
  f.tick();
  assert.equal(f.requests.length, 2);
});

test('only one state request is pending and rapid changes coalesce to the latest page', async () => {
  const f = fixture();
  f.tick();
  for (let pos = 2; pos <= 10; pos++) {
    f.state.pos = pos;
    f.state.click = pos;
    f.tick();
  }
  assert.equal(f.requests.length, 1);
  await f.answer(0);
  f.tick();
  assert.equal(f.requests.length, 2);
  assert.equal(f.message(1), 'EVENT SlideShowState pos=10 click=10 ready=0');
  await f.answer(1);
  f.tick();
  assert.equal(f.requests.length, 2);
});

test('ambiguous failure forces resynchronization even after returning to the last confirmed page', async () => {
  const f = fixture();
  f.tick();
  await f.answer(0);
  f.state.pos = 2;
  f.tick();
  f.state.pos = 1;
  await f.fail(1); // The server may have applied page 2 before its response was lost.
  f.tick();
  assert.equal(f.requests.length, 3);
  assert.equal(f.message(2), 'EVENT SlideShowState pos=1 click=0 ready=0');
});

test('synchronous fetch errors do not leave state permanently pending', async () => {
  const f = fixture();
  f.throwFetch(true);
  f.tick();
  f.throwFetch(false);
  f.tick();
  assert.equal(f.requests.length, 2);
  await f.answer(1);
  f.tick();
  assert.equal(f.requests.length, 2);
  assert.equal(f.timeouts.size, 0);
});

for (const abort of [true, false]) {
  test(`timed out requests cannot acknowledge or clear a newer request (AbortController=${abort})`, async () => {
    const f = fixture({ abort });
    f.tick();
    f.expire();
    if (abort) assert.equal(f.requests[0].options.signal.aborted, true);
    f.state.pos = 2;
    f.tick();
    assert.equal(f.requests.length, 2);
    await f.answer(0); // A late response after cancellation must have no effect.
    f.state.pos = 3;
    f.tick();
    assert.equal(f.requests.length, 2, 'new request must remain pending');
    assert.equal(f.timeouts.size, 1);
    await f.fail(1);
    f.tick();
    assert.equal(f.message(2), 'EVENT SlideShowState pos=3 click=0 ready=0');
    await f.answer(2);
    f.tick();
    assert.equal(f.requests.length, 3);
    assert.equal(f.timeouts.size, 0);
  });
}

test('the deadline also covers a response body that never completes', async () => {
  const f = fixture();
  f.tick();
  f.requests[0].resolve({ ok: true, text: () => new Promise(() => {}) });
  await settle();
  f.expire();
  f.tick();
  assert.equal(f.requests.length, 2);
  await f.answer(1);
  assert.equal(f.timeouts.size, 0);
});

test('same-page script reload preserves pending and confirmed state', async () => {
  const f = fixture();
  f.tick();
  f.loadScript();
  f.tick();
  assert.equal(f.requests.length, 1);
  await f.answer(0);
  f.loadScript();
  f.tick();
  assert.equal(f.requests.length, 1);
  f.state.click++;
  f.tick();
  assert.equal(f.message(1), 'EVENT SlideShowState pos=1 click=1 ready=0');
});

test('missing presentation or Application sends no state request', () => {
  const f = fixture();
  f.state.pos = -1;
  f.tick();
  f.context.window.Application = undefined;
  f.tick();
  assert.equal(f.requests.length, 0);
  assert.equal(f.timeouts.size, 0);
});

test('successful poll commands still run while failed HTTP responses do not', async () => {
  const f = fixture();
  f.context.bridgeFetch('/poll');
  await f.answer(0, 'NEXT');
  f.context.bridgeFetch('/poll');
  await f.answer(2, 'PREV'); // request 1 is the NEXT acknowledgement
  f.context.bridgeFetch('/poll');
  await f.answer(4, 'NEXT', 503);
  assert.deepEqual(f.commands, ['NEXT', 'PREV']);
});

test('exit uses the WPS API and reports failure when the API throws', () => {
  const f = fixture();
  f.context.bridgeCmd('EXIT');
  assert.deepEqual(f.commands, ['EXIT']);
  assert.equal(f.message(0), 'COMMAND_OK EXIT');
  f.context.bridgeCmd('NEXT');
  assert.equal(f.message(1), 'COMMAND_FAILED NEXT');
  f.state.pos = 1;
  f.view.Previous = () => { throw new Error('WPS busy'); };
  f.context.bridgeCmd('PREV');
  assert.equal(f.message(2), 'COMMAND_FAILED PREV');
});

test('polls carry the current state and cannot overlap or execute expired replies', async () => {
  const f = fixture();
  f.context.bridgePoll();
  for (let i = 0; i < 20; i++) f.context.bridgePoll();
  assert.equal(f.requests.length, 1);
  assert.equal(f.message(0), 'EVENT SlideShowState pos=1 click=0 ready=0');
  const [id, timer] = f.timeouts.entries().next().value;
  assert.equal(timer.delay, 1500);
  f.timeouts.delete(id);
  timer.callback();
  f.state.pos = 2;
  f.context.bridgePoll();
  await f.answer(0, 'NEXT');
  assert.deepEqual(f.commands, []);
  assert.equal(f.timeouts.size, 1);
  await f.answer(1, 'PREV');
  assert.deepEqual(f.commands, ['PREV']);
  assert.equal(f.timeouts.size, 0);
});

test('poll failure can retry and polls keep reporting an idle presentation', async () => {
  const f = fixture();
  f.context.bridgePoll();
  await f.fail(0);
  f.state.pos = -1;
  f.context.bridgePoll();
  assert.equal(f.message(1), 'EVENT SlideShowState pos=-1 click=-1 ready=0');
  await f.answer(1, '');
  assert.equal(f.timeouts.size, 0);
});

test('non-command endpoints cannot execute a key-like response', async () => {
  const f = fixture();
  for (const endpoint of ['/hello', '/push?m=COMMAND_OK%20NEXT']) {
    f.context.bridgeFetch(endpoint);
    await f.answer(f.requests.length - 1, 'NEXT');
  }
  assert.deepEqual(f.commands, []);
});

test('ending and restarting on the same slide resends both states', async () => {
  const f = fixture();
  f.tick(); await f.answer(0);
  f.state.pos = -1;
  f.tick();
  assert.equal(f.message(1), 'EVENT SlideShowState pos=-1 click=-1 ready=0');
  await f.answer(1);
  f.state.pos = 1;
  f.tick();
  assert.equal(f.message(2), 'EVENT SlideShowState pos=1 click=0 ready=0');
});

test('only an active nonempty presentation with the current-slide command can advertise START', () => {
  const f = fixture();
  f.state.pos = -1;
  const app = f.context.window.Application;
  const deck = app.ActivePresentation;
  deck.Slides = { Count: 4 };
  app.CommandBars = { ExecuteMso() {}, GetEnabledMso() { return true; } };
  assert.equal(f.context.bridgeState().ready, 1);
  f.context.bridgePoll();
  assert.equal(f.message(0), 'EVENT SlideShowState pos=-1 click=-1 ready=1 page=-1 total=4');
  deck.Slides.Count = 0;
  assert.equal(f.context.bridgeState().ready, 0);
  deck.Slides.Count = undefined;
  assert.equal(f.context.bridgeState().ready, 0);
  deck.Slides.Count = 4;
  app.CommandBars.GetEnabledMso = () => false;
  assert.equal(f.context.bridgeState().ready, 0, 'disabled current-slide command is not ready');
  app.CommandBars.GetEnabledMso = () => true;
  app.ActivePresentation = null;
  app.Presentations = { Count: 1, Item() { return deck; } };
  assert.equal(f.context.bridgeState().ready, 0, 'do not guess an inactive deck');
});

test('START invokes the current-slide command once while starting and never sends Exit', () => {
  const f = fixture();
  f.state.pos = -1;
  const app = f.context.window.Application;
  const deck = app.ActivePresentation;
  deck.Slides = { Count: 4 };
  app.CommandBars = { ExecuteMso(command) { f.commands.push(command); } };
  f.context.bridgeCmd('START');
  f.context.bridgeCmd('START');
  assert.deepEqual(f.commands, ['SlideShowFromCurrent']);
  assert.equal(f.message(0), 'COMMAND_OK START');
  assert.equal(f.message(1), 'COMMAND_FAILED START');
  f.state.pos = 1;
  assert.equal(f.context.bridgeState().ready, 0);
  f.context.bridgeCmd('START');
  assert.deepEqual(f.commands, ['SlideShowFromCurrent']);
});

test('START uses the current page at execution and reports the actual first show page', () => {
  const f = fixture();
  f.state.pos = -1;
  const app = f.context.window.Application;
  app.ActivePresentation.Slides = { Count: 8 };
  let currentPage = 3;
  app.ActivePresentation.SlideShowSettings = Object.freeze({
    StartingSlide: 1, EndingSlide: 8, RangeType: 1,
    Run() { throw new Error('must not start from the beginning'); },
  });
  app.CommandBars = {
    GetEnabledMso(command) { return command === 'SlideShowFromCurrent'; },
    ExecuteMso(command) {
      assert.equal(command, 'SlideShowFromCurrent');
      f.state.pos = currentPage;
    },
  };
  assert.equal(f.context.bridgeState().ready, 1);
  currentPage = 5; // Page changed after readiness was advertised.
  f.context.bridgeCmd('START');
  assert.equal(f.state.pos, 5);
  assert.equal(f.message(0), 'COMMAND_OK START');
  f.context.bridgePoll();
  assert.equal(f.message(1), 'EVENT SlideShowState pos=5 click=0 ready=0 page=-1 total=8');
  assert.equal(app.ActivePresentation.SlideShowSettings.StartingSlide, 1);
});

test('START rechecks disabled commands and never falls back to a first-slide show', () => {
  const f = fixture();
  f.state.pos = -1;
  const app = f.context.window.Application;
  app.ActivePresentation.Slides = { Count: 8 };
  app.ActivePresentation.SlideShowSettings = { Run() { f.commands.push('Run'); } };
  assert.equal(f.context.bridgeState().ready, 0, 'Run alone does not provide current-slide start');
  let enabled = true;
  app.CommandBars = {
    GetEnabledMso() { return enabled; },
    ExecuteMso() { f.commands.push('ExecuteMso'); throw new Error('WPS busy'); },
  };
  assert.equal(f.context.bridgeState().ready, 1);
  enabled = false;
  f.context.bridgeCmd('START');
  assert.equal(f.message(0), 'COMMAND_FAILED START');
  assert.deepEqual(f.commands, []);
  enabled = true;
  f.context.bridgeCmd('START');
  assert.equal(f.message(1), 'COMMAND_FAILED START');
  assert.deepEqual(f.commands, ['ExecuteMso']);
});

test('closed or API-incompatible decks fail START instead of guessing a key', () => {
  const f = fixture();
  f.state.pos = -1;
  f.context.bridgeCmd('START');
  assert.equal(f.message(0), 'COMMAND_FAILED START');
  assert.deepEqual(f.commands, []);
});

test('document counters track editing and slide count changes without starting a show', async () => {
  const f = fixture();
  f.state.pos = -1;
  const app = f.context.window.Application;
  app.ActivePresentation.Slides = { Count: 24 };
  app.ActiveWindow = { View: { Slide: { SlideIndex: 6 } } };
  f.tick();
  assert.equal(f.message(0), 'EVENT SlideShowState pos=-1 click=-1 ready=0 page=6 total=24');
  await f.answer(0);
  app.ActiveWindow.View.Slide.SlideIndex = 7;
  f.tick();
  assert.equal(f.message(1), 'EVENT SlideShowState pos=-1 click=-1 ready=0 page=7 total=24');
  await f.answer(1);
  app.ActivePresentation.Slides.Count = 25;
  f.tick();
  assert.equal(f.message(2), 'EVENT SlideShowState pos=-1 click=-1 ready=0 page=7 total=25');
  await f.answer(2);
  f.tick();
  assert.equal(f.requests.length, 3, 'unchanged counters remain acknowledged');
  app.ActivePresentation = null;
  f.tick();
  assert.equal(f.message(3), 'EVENT SlideShowState pos=-1 click=-1 ready=0');
  assert.deepEqual(f.commands, []);
});

test('custom show counters use the slide index and the showing deck, not an unrelated active deck', () => {
  const f = fixture();
  const app = f.context.window.Application;
  f.state.pos = 2;
  f.view.Slide = { SlideIndex: 9 };
  const showing = { Slides: { Count: 30 } };
  app.ActivePresentation = { Slides: { Count: 5 }, SlideShowWindow: null };
  app.ActiveWindow = { View: { Slide: { SlideIndex: 3 } } };
  app.SlideShowWindows = { Count: 1, Item() { return { View: f.view, Presentation: showing }; } };
  f.context.bridgePoll();
  assert.equal(f.message(0), 'EVENT SlideShowState pos=2 click=0 ready=0 page=9 total=30');
  assert.deepEqual(f.commands, []);
});

test('missing or inconsistent counters never replace navigation state with guessed page numbers', () => {
  const f = fixture();
  const deck = f.context.window.Application.ActivePresentation;
  f.state.pos = 2;
  f.view.Slide = { SlideIndex: 9 };
  deck.Slides = { Count: 5 };
  assert.equal(f.context.bridgeState().page, -1, 'page beyond total is not displayed');
  assert.equal(f.context.bridgeState().total, 5);
  deck.Slides.Count = 0;
  assert.equal(f.context.bridgeState().page, -1, 'empty decks must not retain a stale slide');
  assert.equal(f.context.bridgeState().total, -1);
  Object.defineProperty(deck.Slides, 'Count', { get() { throw new Error('unsupported'); } });
  assert.equal(f.context.bridgeState().page, 9);
  assert.equal(f.context.bridgeState().total, -1);
  Object.defineProperty(f.view, 'Slide', { get() { throw new Error('unsupported'); } });
  assert.equal(f.context.bridgeState().page, -1, 'do not substitute show order or keypress estimates');
  assert.equal(f.context.bridgeState().pos, 2);
  f.context.bridgeCmd('NEXT');
  assert.deepEqual(f.commands, ['NEXT']);
});

test('stale SlideIndex falls back to show position for a full deck', () => {
  const f = fixture();
  f.state.pos = 1;
  f.view.Slide = { SlideIndex: 1 };
  assert.equal(f.context.bridgeState().page, 1);
  f.state.pos = 2;
  assert.equal(f.context.bridgeState().page, 1, 'one position change is not enough');
  f.state.pos = 3;
  assert.equal(f.context.bridgeState().page, 3, 'after two stale samples use show position');
});

test('named/custom shows never use show position as a deck page', () => {
  const f = fixture();
  f.state.pos = 1;
  f.view.Slide = { SlideIndex: 1 };
  f.view.IsNamedShow = 1;
  assert.equal(f.context.bridgeState().page, 1);
  f.state.pos = 2;
  assert.equal(f.context.bridgeState().page, 1);
  f.state.pos = 3;
  assert.equal(f.context.bridgeState().page, 1);
});

test('default mode reads only position and refuses START/GOTO commands', () => {
  const f = fixture();
  f.context.BRIDGE_FULL = false;
  const st = f.context.bridgeState();
  assert.equal(st.pos, 1);
  assert.equal(st.page, -1);
  assert.equal(st.total, -1);
  assert.equal(st.jump, 0);
  assert.equal(st.ready, 0);
  f.context.bridgeCmd('START');
  f.context.bridgeCmd(`GOTO 2 ${Date.now() % 1000 + 1}`);
  assert.deepEqual(f.commands, []);
});
