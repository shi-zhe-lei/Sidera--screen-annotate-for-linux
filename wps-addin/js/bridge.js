// ============================================================
// Sidera联动桥（加载项内运行）
//   · 监听 WPS 放映事件 → 上报批注 app (127.0.0.1:16666 /push)
//   · 轮询 /poll 取 START/NEXT/PREV/EXIT/GOTO，通过 WPS 接口执行
// 配合 app 的“WPS 联动”使用：批注缓存按真实页号驱动，
// 页内动画步不动批注。
// ============================================================
var BRIDGE_URL = 'http://127.0.0.1:16666';
var BRIDGE_START_COMMAND = 'SlideShowFromCurrent';
// false = 默认低负载模式：只读 pos/click、只执行 NEXT/PREV。
// 测试模式由 Sidera 的 /config 打开，额外提供页码、跳页和开始放映。
var BRIDGE_FULL = window._sideraFullMode === true;
var BRIDGE_MODE_LOCKED = typeof window._sideraFullMode === 'boolean';
var BRIDGE_SSE_OPEN = false;
var BRIDGE_SSE = null;
var BRIDGE_WS = window._sideraWs || null;
var BRIDGE_WS_OPEN = window._sideraWsOpen === true;
var BRIDGE_CHANNEL = window._sideraChannel || 'none';

function OnAddinLoad(ribbonUI) {
    var app = bridgeApplication()
    if (ribbonUI && app && typeof app.ribbonUI != 'object') app.ribbonUI = ribbonUI
    startBridge()
    return true
}

function OnSaPing() {
    var finished = false
    function finish(online) {
        if (finished) return
        finished = true
        clearTimeout(timer)
        alert(online ? '已连接 Sidera' : '未连接，请先打开 Sidera。')
    }
    var timer = setTimeout(function () { finish(false) }, 3000)
    bridgeFetch('/hello?m=health-check', function () { finish(false) }, null,
                function (text) { finish(text === 'OK sidera') })
    return true
}

function bridgeApplication() {
    try {
        if (window.Application) return window.Application
        if (window.wps && typeof window.wps.WppApplication == 'function')
            return window.wps.WppApplication()
        return window.wps ? window.wps.Application : null
    } catch (e) { return null }
}
function bridgeShowContext() {
    try {
        var app = bridgeApplication()
        if (!app) return null
        var a = app.ActivePresentation
        if (!a && app.Presentations && app.Presentations.Count > 0)
            a = app.Presentations.Item(1)
        var sw = a ? a.SlideShowWindow : null
        if (sw) return { window: sw, presentation: a }
        if (app.SlideShowWindows && app.SlideShowWindows.Count > 0) {
            sw = app.SlideShowWindows.Item(1)
            var deck = null
            try { deck = sw.Presentation } catch (e) {}
            return { window: sw, presentation: deck }
        }
        return null
    } catch (e) { return null }
}
function bridgeView() {
    try {
        var context = bridgeShowContext()
        return context ? context.window.View : null
    } catch (e) { return null }
}
var _lastShowPage = window._sideraLastShowPage || (window._sideraLastShowPage = -1)
var _lastShowTotal = window._sideraLastShowTotal || (window._sideraLastShowTotal = -1)
var _lastShowPageAt = window._sideraLastShowPageAt || 0
var _lastPagePos = -1
var _lastSlideIndex = -1
var _pageStaleCount = 0

function bridgePageState(context, view) {
    var result = { page: -1, total: -1 }
    function positive(value) {
        var n = Number(value)
        return isFinite(n) && n > 0 && n <= 2147483647 && Math.floor(n) === n ? n : -1
    }
    try {
        var app = bridgeApplication()
        var deck = context ? context.presentation : app && app.ActivePresentation
        if (!deck) return result
        try {
            var count = Number(deck.Slides.Count)
            if (count === 0) return result
            result.total = positive(count)
        } catch (e) {}
        // Use the actual deck index, not the order within a custom slideshow.
        try {
            var current = view || (!context && app.ActiveWindow && app.ActiveWindow.View)
            var slideIndex = positive(current && current.Slide && current.Slide.SlideIndex)
            var rawPos = positive(view && view.CurrentShowPosition)
            var namedShow = !!(view && view.IsNamedShow)
            if (slideIndex > 0) {
                // Some WPS advanced/presenter windows keep returning the old
                // SlideIndex while CurrentShowPosition advances. Only use the
                // show position as a fallback for a full (non-named) deck.
                if (!namedShow && rawPos > 0 && _lastPagePos > 0 && rawPos !== _lastPagePos
                    && slideIndex === _lastSlideIndex) {
                    _pageStaleCount++
                } else {
                    _pageStaleCount = 0
                }
                result.page = (!namedShow && _pageStaleCount >= 2) ? rawPos : slideIndex
                _lastPagePos = rawPos
                _lastSlideIndex = slideIndex
            } else {
                result.page = -1
            }
        } catch (e) {}
        if (result.total > 0 && result.page > result.total) result.page = -1
    } catch (e) {}
    return result
}
var _lastShowPos = window._sideraLastShowPos || (window._sideraLastShowPos = -1)
var _lastShowAt = window._sideraLastShowAt || 0
function bridgeState(allowEnd) {
    var hold = allowEnd === false
    var st = { pos: -1, click: -1, ready: 0, page: -1, total: -1, jump: 0, deck: -1 }
    try {
        var context = bridgeShowContext()
        var v = context ? context.window.View : null
        if (v) {
            try {
                var raw = Number(v.CurrentShowPosition)
                if (isFinite(raw) && raw > 0) {
                    st.pos = raw
                    _lastShowPos = raw
                    _lastShowAt = Date.now()
                } else if (!hold || _lastShowPos < 1 || Date.now() - _lastShowAt > 2500) {
                    st.pos = isFinite(raw) ? raw : -1
                } else {
                    st.pos = _lastShowPos
                }
                st.click = (typeof v.GetClickIndex == 'function') ? v.GetClickIndex() : -1
            } catch (e) {}
        } else if (hold && _lastShowPos > 0 && Date.now() - _lastShowAt <= 2500) {
            // WPS briefly drops SlideShowWindow during transitions; reporting
            // -1 immediately made the toolbar flap between Start and End.
            st.pos = _lastShowPos
        }
        if (v && st.pos > 0) {
            try {
                var pointer = Number(v.PointerType)
                if (isFinite(pointer) && window._sideraPointerBaseline === undefined)
                    window._sideraPointerBaseline = pointer
            } catch (e) {}
        }
        if (!BRIDGE_FULL) { return st }
        var pages
        if (v) {
            pages = bridgePageState(context, v)
        } else if (hold && _lastShowPage > 0 && Date.now() - _lastShowPageAt <= 2500) {
            pages = { page: _lastShowPage, total: _lastShowTotal }
        } else {
            pages = bridgePageState(null, null)
        }
        st.page = pages.page; st.total = pages.total
        if (st.page > 0) {
            _lastShowPage = st.page
            _lastShowTotal = st.total
            _lastShowPageAt = Date.now()
        }
        var jump = bridgeJumpTarget(context, pages)
        if (jump) { st.jump = 1; st.deck = jump.epoch }
        if (!v) {
            st.ready = st.pos > 0 ? 0 : (bridgeStartTarget() ? 1 : 0)
            return st
        }
    } catch (e) {}
    return st
}
function bridgeSyncConfig() {
    if (BRIDGE_MODE_LOCKED) return
    bridgeFetch('/config', null, null, function (text) {
        var full = text === 'mode=full'
        if (full === BRIDGE_FULL) return
        BRIDGE_FULL = full
        // Force the next state push to report the new field set.
        _lastState.pos = -2; _lastState.click = -2; _lastState.page = -2
        _lastState.total = -2; _lastState.jump = -2; _lastState.deck = -2
        bridgePushIfChanged()
    })
}
function bridgeJumpTarget(context, pages) {
    try {
        var app = bridgeApplication()
        context = context || bridgeShowContext()
        var deck = context ? context.presentation : app && app.ActivePresentation
        var view = context ? context.window.View : app && app.ActiveWindow && app.ActiveWindow.View
        if (!deck || !view || typeof view.GotoSlide !== 'function') return null
        pages = pages || bridgePageState(context, context ? view : null)
        if (!(pages.page > 0 && pages.total >= pages.page)) return null
        var name = String(deck.FullName || deck.Name || '')
        if (!name) return null
        // Bind a queued jump to the active document and editing/showing session.
        // A replacement document must not receive an old wheel selection.
        var key = name + '\n' + pages.total + '\n' + (context ? 'show' : 'edit')
        var identity = window._sideraDeckIdentity
        if (!identity || identity.key !== key) {
            identity = { key: key, epoch: identity ? identity.epoch % 2147483646 + 1 : Date.now() % 2147483646 + 1 }
            window._sideraDeckIdentity = identity
        }
        return { view: view, total: pages.total, epoch: identity.epoch }
    } catch (e) { return null }
}
function bridgeStartTarget() {
    try {
        var app = bridgeApplication()
        // 只启动当前文档；没有活动文档时不猜测列表里的第一份文件。
        var presentation = app && app.ActivePresentation
        if (!presentation || !presentation.Slides || !(Number(presentation.Slides.Count) > 0)) return null
        if (app.SlideShowWindows && app.SlideShowWindows.Count > 0) return null
        var bars = app.CommandBars
        if (!bars || typeof bars.ExecuteMso != 'function') return null
        if (typeof bars.GetEnabledMso == 'function' && !bars.GetEnabledMso(BRIDGE_START_COMMAND)) return null
        return bars
    } catch (e) { return null }
}
function bridgeStream() {
    var stream = window._sideraRequestStream
    if (!stream) stream = window._sideraRequestStream = {
        id: Date.now().toString(36) + '-' + Math.random().toString(36).slice(2), sequence: 0 }
    return stream
}
function bridgeStateMessage(name, st) {
    var message = 'EVENT ' + name + ' pos=' + st.pos + ' click=' + st.click + ' ready=' + st.ready
    if (st.page > 0 || st.total > 0) message += ' page=' + st.page + ' total=' + st.total
    if (st.jump === 1 && st.deck > 0) message += ' jump=1 deck=' + st.deck
    // Identify this add-in instance so multiple WPS windows can be told apart.
    message += ' sid=' + encodeURIComponent(bridgeStream().id)
    return message
}
function bridgeFetch(path, onResult, signal, onText) {
    try {
        if (/^\/(push|poll)\?/.test(path)) {
            var stream = bridgeStream()
            path += '&sid=' + encodeURIComponent(stream.id) + '&seq=' + (++stream.sequence)
        }
        var options = { cache: 'no-store' }
        if (signal) options.signal = signal
        fetch(BRIDGE_URL + path, options)
            .then(function (r) {
                if (!r.ok) throw new Error('Bridge HTTP request failed')
                return r.text()
            })
            .then(function (t) {
                var c = (t || '').trim()
                if (onText) onText(c)
                else if (onResult) onResult(c === 'OK')
                else if (path.split('?')[0] === '/poll' && c) bridgeCmd(c)
            })
            .catch(function () { if (onResult) onResult(false) })
    } catch (e) { if (onResult) onResult(false) }
}
function bridgePointerBaseline() {
    var baseline = window._sideraPointerBaseline
    if (typeof baseline === 'number' && isFinite(baseline)) return baseline
    // Prefer WPS enum constants when the JS API exposes them.
    try {
        var types = window.wps && window.wps.PpSlideShowPointerType
        if (types && isFinite(Number(types.ppSlideShowPointerArrow)))
            return Number(types.ppSlideShowPointerArrow)
    } catch (e) {}
    try {
        var app = bridgeApplication()
        if (app && isFinite(Number(app.ppSlideShowPointerArrow)))
            return Number(app.ppSlideShowPointerArrow)
    } catch (e) {}
    // PowerPoint-compatible value; used only if no baseline was recorded.
    return 1
}
function bridgeSetPointerArrow(view) {
    var target = bridgePointerBaseline()
    try {
        view.PointerType = target
    } catch (e) {
        bridgeChannelLog('CURSOR failed')
        return false
    }
    // WPS may handle the physical shortcut after this command; settle again.
    setTimeout(function () {
        try {
            var current = bridgeView()
            if (current) current.PointerType = target
        } catch (e) {}
    }, 140)
    bridgeChannelLog('CURSOR ' + target)
    return true
}
function bridgeCmd(cmd) {
    var jump = /^GOTO ([1-9][0-9]{0,9}) ([1-9][0-9]{0,9})$/.exec(cmd)
    var thumb = /^THUMB ([1-9][0-9]{0,9}) (.+)$/.exec(cmd)
    if (!jump && !thumb && cmd !== 'START' && cmd !== 'NEXT' && cmd !== 'PREV' && cmd !== 'EXIT' && cmd !== 'CURSOR') return
    if (!BRIDGE_FULL && cmd !== 'NEXT' && cmd !== 'PREV' && cmd !== 'CURSOR' && !thumb) return
    try {
        var v = bridgeView()
        if (thumb) {
            var app = bridgeApplication()
            var deck = app && app.ActivePresentation
            if (!deck) throw new Error('No presentation')
            var page = Number(thumb[1])
            var slide = deck.Slides.Item(page)
            if (!slide || typeof slide.Export !== 'function') throw new Error('Slide.Export unavailable')
            slide.Export(thumb[2], 'PNG', 320, 180)
            bridgeSend('COMMAND_OK THUMB')
            return
        }
        if (jump) {
            var destination = bridgeJumpTarget()
            var page = Number(jump[1]), epoch = Number(jump[2])
            if (!destination || destination.epoch !== epoch || page > destination.total)
                throw new Error('Page selection no longer belongs to the active document')
            destination.view.GotoSlide(page)
        } else if (cmd == 'START') {
            var target = !v && bridgeStartTarget()
            if (!target || Date.now() < (window._sideraStartingUntil || 0))
                throw new Error('No presentation ready to start')
            window._sideraStartingUntil = Date.now() + 3000
            // 使用 WPS 自身的“当页开始”，执行时读取当前页；不改文档放映范围。
            target.ExecuteMso(BRIDGE_START_COMMAND)
        } else if (cmd == 'CURSOR') {
            if (!v || !bridgeSetPointerArrow(v)) throw new Error('WPS pointer is not writable')
        } else {
            if (!v) throw new Error('No slideshow')
            if (cmd == 'NEXT') v.Next()
            else if (cmd == 'PREV') v.Previous()
            else v.Exit()
        }
        bridgeSend('COMMAND_OK ' + cmd)
        if (jump) bridgePushIfChanged()
    } catch (e) {
        bridgeSend('COMMAND_FAILED ' + cmd)
    }
}
function bridgePoll() {
    if (window._sideraPollInFlight) return
    var started = (typeof performance === 'object' && performance.now) ? performance.now() : Date.now()
    var st = bridgeState()
    var elapsed = ((typeof performance === 'object' && performance.now) ? performance.now() : Date.now()) - started
    if (elapsed > 200 && Date.now() >= (window._sideraSlowUntil || 0)) {
        window._sideraSlowUntil = Date.now() + 5000
        bridgeFetch('/push?m=' + encodeURIComponent('SLOW state ' + Math.round(elapsed) + 'ms'))
    }
    var controller = typeof AbortController == 'function' ? new AbortController() : null
    var pending = { timer: null }
    window._sideraPollInFlight = pending
    var finish = function () {
        if (window._sideraPollInFlight !== pending) return false
        clearTimeout(pending.timer)
        window._sideraPollInFlight = null
        return true
    }
    pending.timer = setTimeout(function () {
        finish()
        if (controller) controller.abort()
    }, 1500)
    bridgeFetch('/poll?wait=1&m=' + encodeURIComponent(bridgeStateMessage('SlideShowState', st)),
        finish, controller ? controller.signal : null, function (cmd) {
            // 已过期轮询的迟到回复不得在下一次放映中执行。
            if (finish() && cmd) bridgeCmd(cmd)
        })
}
var BridgeSlideEvents = ['SlideShowBegin', 'SlideShowEnd', 'SlideShowNextSlide', 'SlideShowNextClick', 'SlideShowOnNext', 'SlideShowOnPrevious'];
// 轮询式状态上报：解决“上一页”没有后续新页号事件的问题
// （WPS 的 Next 会补发 NextClick/NextSlide，但 Previous 常只有 OnPrevious 且带旧 pos）
// 仅记录收到成功回复的快照；同页重载脚本时保留确认状态和在途请求。
var _lastState = window._sideraLastState || (window._sideraLastState = { pos: -1, click: -1, ready: 0, page: -1, total: -1, jump: 0, deck: -1 });
function bridgePushIfChanged(eventName, force, allowEnd) {
    try {
        if (window._sideraStateInFlight) return
        var st = bridgeState(allowEnd)
        if (force || st.pos !== _lastState.pos || st.click !== _lastState.click || st.ready !== _lastState.ready ||
            st.page !== _lastState.page || st.total !== _lastState.total || st.jump !== _lastState.jump || st.deck !== _lastState.deck) {
            var message = bridgeStateMessage(eventName || 'SlideShowState', st)
            if (BRIDGE_CHANNEL === 'ws' && BRIDGE_WS && BRIDGE_WS.readyState === 1) {
                if (bridgeSend(message)) bridgeRememberState(st)
                return
            }
            var controller = typeof AbortController == 'function' ? new AbortController() : null
            var pending = { timer: null }
            window._sideraStateInFlight = pending
            var finish = function (ok) {
                // 超时请求的迟到回复不能确认旧页，也不能清除较新的在途请求。
                if (window._sideraStateInFlight !== pending) return
                clearTimeout(pending.timer)
                window._sideraStateInFlight = null
                // 失败时服务端可能已处理请求；即使回到上次确认页，也必须重新同步。
                _lastState.pos = ok ? st.pos : -1
                _lastState.click = ok ? st.click : -1
                _lastState.ready = ok ? st.ready : -1
                _lastState.page = ok ? st.page : -1
                _lastState.total = ok ? st.total : -1
                _lastState.jump = ok ? st.jump : -1
                _lastState.deck = ok ? st.deck : -1
            }
            pending.timer = setTimeout(function () {
                finish(false)
                if (controller) controller.abort()
            }, 3000)
            bridgeFetch('/push?m=' + encodeURIComponent(message),
                finish, controller ? controller.signal : null)
        }
    } catch (e) {}
}
function bridgeSetChannel(channel) {
    BRIDGE_CHANNEL = channel
    window._sideraChannel = channel
}

function bridgeChannelLog(message) {
    if (BRIDGE_MODE_LOCKED) return
    if (BRIDGE_CHANNEL === 'ws' && BRIDGE_WS && BRIDGE_WS.readyState === 1) {
        try { BRIDGE_WS.send('WS ' + message); return } catch (e) {}
    }
    bridgeFetch('/push?m=' + encodeURIComponent('CHANNEL ' + message))
}

function bridgeSend(message) {
    if (BRIDGE_CHANNEL === 'ws' && BRIDGE_WS && BRIDGE_WS.readyState === 1) {
        try { BRIDGE_WS.send(message); return true } catch (e) { return false }
    }
    if (BRIDGE_CHANNEL === 'sse' || BRIDGE_CHANNEL === 'poll') {
        bridgeFetch('/push?m=' + encodeURIComponent(message))
        return true
    }
    return false
}

function bridgeRememberState(st) {
    _lastState.pos = st.pos
    _lastState.click = st.click
    _lastState.ready = st.ready
    _lastState.page = st.page
    _lastState.total = st.total
    _lastState.jump = st.jump
    _lastState.deck = st.deck
}

function bridgeHandleData(data) {
    data = (data || '').trim()
    if (!data) return
    if (data === 'STATE') {
        bridgePushIfChanged('SlideShowState', true)
        return
    }
    if (data.indexOf('MODE ') === 0) {
        var full = data.slice(5) === 'full'
        if (full !== BRIDGE_FULL) {
            BRIDGE_FULL = full
            _lastState.pos = -2; _lastState.click = -2; _lastState.page = -2
            _lastState.total = -2; _lastState.jump = -2; _lastState.deck = -2
        }
        bridgePushIfChanged('SlideShowState', true)
        return
    }
    bridgeCmd(data)
}

function bridgeStartWs() {
    if (window._sideraWsStarted) return
    window._sideraWsStarted = true
    if (typeof WebSocket !== 'function') {
        bridgeChannelLog('WS unavailable')
        bridgeStartSse()
        return
    }
    try {
        var ws = new WebSocket(BRIDGE_URL.replace(/^http/, 'ws') + '/ws')
        BRIDGE_WS = ws
        window._sideraWs = ws
        var opened = false
        var failTimer = setTimeout(function () {
            if (opened) return
            try { ws.close() } catch (e) {}
            BRIDGE_WS_OPEN = false
            window._sideraWsOpen = false
            bridgeChannelLog('WS fallback timeout')
            bridgeStartSse()
        }, 5000)
        function markOpen() {
            if (opened) return
            opened = true
            clearTimeout(failTimer)
            BRIDGE_WS_OPEN = true
            window._sideraWsOpen = true
            bridgeSetChannel('ws')
            bridgeChannelLog('open')
            bridgePushIfChanged('SlideShowState', true)
        }
        function handleMessage(event) {
            markOpen()
            bridgeHandleData(event && event.data ? event.data : '')
        }
        ws.onopen = markOpen
        if (typeof ws.addEventListener === 'function') {
            ws.addEventListener('open', markOpen)
            ws.addEventListener('message', handleMessage)
        } else {
            ws.onmessage = handleMessage
        }
        ws.onclose = function () {
            BRIDGE_WS_OPEN = false
            window._sideraWsOpen = false
            if (BRIDGE_CHANNEL === 'ws') {
                bridgeSetChannel('none')
                bridgeChannelLog('WS closed')
                bridgeStartSse()
            } else if (!opened && !window._sideraSseStarted) {
                bridgeChannelLog('WS closed before open')
                bridgeStartSse()
            }
        }
        ws.onerror = function () {
            if (!opened) {
                BRIDGE_WS_OPEN = false
                window._sideraWsOpen = false
            }
        }
    } catch (e) {
        bridgeChannelLog('WS failed')
        bridgeStartSse()
    }
}

function bridgeSseLog(message) {
    if (BRIDGE_MODE_LOCKED) return
    bridgeFetch('/push?m=' + encodeURIComponent('SSE ' + message))
}

function bridgeStartPolling() {
    if (window._sideraPollStarted) return
    window._sideraPollStarted = true
    bridgeSetChannel('poll')
    bridgeFetch('/hello?m=sidera-bridge')
    bridgeSyncConfig()
    setInterval(function () {
        try {
            var current = bridgeApplication()
            var currentApi = current && (current.ApiEvent || (window.wps && window.wps.ApiEvent))
            if (window._sideraEventOwner !== currentApi) startBridge()
        } catch (e) {}
        window._sideraConfigTick = (window._sideraConfigTick || 0) + 1
        if (window._sideraConfigTick % 30 === 0) bridgeSyncConfig()
        bridgePoll()
    }, 300)
}

function bridgeStartSse() {
    if (window._sideraSseStarted) return
    window._sideraSseStarted = true
    if (typeof EventSource !== 'function') {
        bridgeSseLog('unavailable')
        if (window._sideraAllowPoll) bridgeStartPolling()
        else bridgeSetChannel('none')
        return
    }
    try {
        var es = new EventSource(BRIDGE_URL + '/events')
        BRIDGE_SSE = es
        var opened = false
        var failTimer = setTimeout(function () {
            if (opened) return
            try { es.close() } catch (e) {}
            BRIDGE_SSE_OPEN = false
            bridgeSseLog('fallback timeout')
            if (window._sideraAllowPoll) bridgeStartPolling()
            else bridgeSetChannel('none')
        }, 5000)
        function markOpen() {
            if (opened) return
            opened = true
            clearTimeout(failTimer)
            BRIDGE_SSE_OPEN = true
            bridgeSetChannel('sse')
            bridgeSseLog('open')
            // Some CEF/EventSource builds omit the open event; receiving any
            // SSE data also proves the stream is live.
            bridgePushIfChanged('SlideShowState', true)
        }
        function handleMessage(event) {
            markOpen()
            bridgeHandleData(event && event.data ? event.data : '')
        }
        es.onopen = markOpen
        if (typeof es.addEventListener === 'function') {
            es.addEventListener('open', markOpen)
            es.addEventListener('message', handleMessage)
        } else {
            es.onmessage = handleMessage
        }
        es.onerror = function () {
            BRIDGE_SSE_OPEN = false
        }
    } catch (e) {
        BRIDGE_SSE_OPEN = false
        bridgeSseLog('failed')
        bridgeStartPolling()
    }
}

function scheduleBridgeStart() {
    if (window._sideraStartRetry != null) return
    window._sideraStartRetry = setTimeout(function () {
        window._sideraStartRetry = null
        startBridge()
    }, 500)
}
function startBridge() {
    // 手动重试或重复 onLoad 取代尚未执行的重试，避免定时器不断累积。
    if (window._sideraStartRetry != null) {
        clearTimeout(window._sideraStartRetry)
        window._sideraStartRetry = null
    }
    // 启动探针：只要本 JS 被执行，就先报到一次，便于诊断
    try {
        if (!window._sideraBootPinged) {
            window._sideraBootPinged = true
            bridgeFetch('/hello?m=sidera-boot')
        }
    } catch (e) {}
    var app = bridgeApplication()
    if (!app) {
        scheduleBridgeStart()               // 等 Application 就绪
        return
    }
    // 事件监听为“可选增强”：即使没有 ApiEvent，也要启动轮询上报真实页号
    // 状态保存在 window 上，同一页面重复执行脚本时也不重复注册。
    var eventsReady = true
    try {
        var api = app.ApiEvent || (window.wps && window.wps.ApiEvent)
        if (window._sideraEventOwner !== api) {
            window._sideraEventOwner = api
            window._sideraEventHandlers = {}
        }
        var handlers = window._sideraEventHandlers || (window._sideraEventHandlers = {})
        if (api) {
            for (var i = 0; i < BridgeSlideEvents.length; i++) {
                (function (ev) {
                    if (handlers[ev]) return
                    try {
                        var handler = function () {
                            if (window._sideraEventOwner !== api) return
                            var allowEnd = ev === 'SlideShowEnd'
                            var st = bridgeState(allowEnd)
                            bridgeSend(bridgeStateMessage(ev, st))
                            if (BRIDGE_CHANNEL !== 'none') {
                                setTimeout(function () { bridgePushIfChanged(ev, false, allowEnd) }, 180)
                            }
                        }
                        if (ev === 'SlideShowEnd') window._sideraPointerBaseline = undefined
                        api.AddApiEventListener(ev, handler)
                        handlers[ev] = handler
                    } catch (e) { eventsReady = false }
                })(BridgeSlideEvents[i])
            }
        } else {
            eventsReady = false
        }
    } catch (e) { eventsReady = false }
    if (!eventsReady) scheduleBridgeStart()
    // WebSocket is the primary bidirectional channel; SSE is the fallback.
    bridgeStartWs()
}
