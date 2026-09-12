import AppKit
import Metal
import QuartzCore
import ScreenSaver
import os

/// The screen saver entry point: `NSPrincipalClass` in Info.plist.
///
/// Hosts a Rust `Gibson` instance (wgpu/Metal) inside a `CAMetalLayer` that
/// this view owns. All `gibson_*` calls happen on the main thread, driven by a
/// display link.
///
/// Screen saver lifecycle realities this host handles (all observed in the
/// `legacyScreenSaver` process since Sonoma):
/// - `stopAnimation()` is never called when the screen saver ends, so a
///   `com.apple.screensaver.willstop` distributed notification is treated as
///   `stopAnimation`.
/// - Every activation instantiates a NEW view while older instances linger;
///   each start posts an in-process notification and older (hidden) instances
///   stop themselves and leave the view hierarchy.
/// - The host process never exits; when not in preview the process terminates
///   itself 65 s after stopping, unless a new activation cancels that.
@objc(GibsonSaverView)
final class GibsonSaverView: ScreenSaverView {
    /// Layer delegate that reports a **nil** window.
    ///
    /// wgpu-hal 30 refuses to acquire a drawable whenever the hosting window's
    /// `occlusionState` lacks the visible bit (`acquire_texture`, a workaround
    /// for gfx-rs/wgpu#8309) and returns `SurfaceError::Occluded` without ever
    /// calling `nextDrawable`. It finds that window by walking up from the
    /// render layer to the first ancestor layer that has a delegate and reading
    /// `delegate.window`. Screen-saver windows live at a private window level
    /// where AppKit never sets the visible bit, so every frame was refused and
    /// the saver stayed black while the display link ticked at 60 Hz.
    ///
    /// Installing this delegate on our own CAMetalLayer stops that walk at the
    /// layer itself, so wgpu goes straight to `nextDrawable` — the call that
    /// actually decides whether a drawable exists. The layer holds its delegate
    /// weakly, so the view keeps a strong reference and re-asserts it (AppKit
    /// normally owns this delegate and may reinstall itself).
    private final class NilWindowLayerDelegate: NSObject, CALayerDelegate {
        @objc var window: NSWindow? { nil }
    }

    private let layerDelegate = NilWindowLayerDelegate()

    private static let log = Logger(subsystem: SaverSettings.moduleName,
                                    category: "GibsonSaverView")

    /// Posted on the process's default center whenever a view starts
    /// animating; older detached instances use it to lame-duck themselves.
    private static let newInstanceNotification =
        Notification.Name("org.hackthegibson.TheGibson.NewInstance")
    /// `userInfo` key carrying the poster's [`instanceToken`].
    private static let tokenKey = "instance-token"
    /// `userInfo` key carrying the poster's display number.
    private static let displayKey = "display-number"
    /// App-wide self-terminate timer: the host process lingers forever, so the
    /// saver quits itself once it has been stopped for a while. One timer for
    /// the process (termination is process-wide); a new start cancels it.
    private static var terminateTimer: Timer?
    /// True once the framework has asked this process to animate. Once it has,
    /// the host must not quit itself: macOS relaunches it immediately with the
    /// same request, and that relaunched engine renders with nothing on screen
    /// and no dismissal to end it (measured; see `scheduleTerminateIfNeeded`).
    private static var everAskedToAnimate = false
    /// Slow heartbeat while the process has no engine, so "nothing is being
    /// rendered" is visible in the log even though no frame stats are emitted.
    private static var idleHeartbeatTimer: Timer?
    /// A start request with no session behind it is given this long to be backed
    /// by a `didstart` before the host decides not to render at all.
    private static let startRequestGrace: TimeInterval = 3
    /// A `didstart` this recent is accepted as evidence even if a stop has been
    /// observed since (stops can be delivered after the next session starts).
    private static let recentSessionWindow: TimeInterval = 10
    /// Wall clock of the last observed session start / end, for
    /// `mayAnimateNow`. Negative means "never observed in this process".
    private static var sessionStartedAt: CFAbsoluteTime = -1
    private static var sessionEndedAt: CFAbsoluteTime = -1

    private static func sessionStarted() {
        sessionStartedAt = CACurrentMediaTime()
    }

    private static func sessionEnded() {
        sessionEndedAt = CACurrentMediaTime()
    }

    /// Pending deferred start for this view (see `deferStartRequest`).
    private var pendingStart: Timer?

    /// Unique per view instance. Carried in the NewInstance notification so an
    /// observer can never mistake its own announcement for somebody else's
    /// (identity, not occlusion timing, decides who retires).
    private let instanceToken = UUID()

    private var gibsonHandle: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private var frameFailures = 0
    private var framesRendered = 0
    private var lastStatusLog = 0
    /// Logical (point) size and effective scale last handed to the renderer.
    private var lastLogicalSize = CGSize.zero
    private var lastScale: CGFloat = 0
    /// Next wall-clock second at which the render loop logs honest frame
    /// counters, plus the last values reported.
    private var nextStatsLog: CFAbsoluteTime = 0
    private var lastPresented: UInt64 = 0
    private var lastSkipped: UInt64 = 0
    private var lastTimeout: UInt64 = 0
    private var lastOccluded: UInt64 = 0
    /// Render time accumulated since the last stats line, for the honest
    /// "ms per frame" figure (wall time inside `gibson_frame`).
    private var frameTimeTotal: CFTimeInterval = 0
    private var frameTimeCount = 0
    private var lastStatsWallClock: CFAbsoluteTime = 0
    /// Whether the view has ever been displayed (grace period at start).
    private var hasBeenDisplayed = false
    private var engineStartedAt: CFAbsoluteTime = 0
    /// Wall clock of the last lifecycle-evidence check, and how many consecutive
    /// checks have seen the framework report the view as no longer animating.
    private var lastLifecycleCheck: CFAbsoluteTime = 0
    private var notAnimatingChecks = 0
    /// True once the framework has confirmed the view IS animating. Until that
    /// has happened at least once, "not animating" carries no information, so
    /// it is never acted on.
    private var sawAnimating = false
    /// The CAMetalLayer handed to wgpu at engine start, compared each stats
    /// tick with the view's current layer to detect a host layer swap.
    private weak var configuredLayer: CAMetalLayer?


    // MARK: - Init / layer

    override init?(frame: NSRect, isPreview: Bool) {
        super.init(frame: frame, isPreview: isPreview)
        wantsLayer = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(handleNewInstance(_:)),
            name: Self.newInstanceNotification, object: nil)
        // Both notifications are observed: `willstop` is the reliable one in
        // practice, but a dismissal has been seen where it never arrived (the
        // engine then kept rendering slowly until the next activation claimed
        // the display), so the companion `didstop` is a second trigger.
        // `didstart` is observed as well: it is the system's own announcement
        // that a saver session started (measured: posted ~0.7 s before the
        // framework's `startAnimation`), which is positive evidence that the
        // host is wanted and cancels any idle quit armed earlier.
        for name in ["com.apple.screensaver.willstop", "com.apple.screensaver.didstop"] {
            DistributedNotificationCenter.default().addObserver(
                self, selector: #selector(handleWillStop(_:)),
                name: NSNotification.Name(name), object: nil)
        }
        DistributedNotificationCenter.default().addObserver(
            self, selector: #selector(handleDidStart(_:)),
            name: NSNotification.Name("com.apple.screensaver.didstart"), object: nil)
        // A host that is created but never asked to animate must not linger
        // either: arm the idle quit now. `startAnimation` cancels it.
        scheduleTerminateIfNeeded()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    deinit {
        cancelPendingStart()
        stopEngine()
        NotificationCenter.default.removeObserver(self)
        DistributedNotificationCenter.default().removeObserver(self)
    }

    /// AppKit asks for the backing layer once the view is layer-backed; handing
    /// out a `CAMetalLayer` means wgpu adopts this layer instead of inserting a
    /// sublayer of its own.
    override func makeBackingLayer() -> CALayer {
        let layer = CAMetalLayer()
        layer.device = MTLCreateSystemDefaultDevice()
        layer.pixelFormat = .bgra8Unorm
        layer.contentsScale = window?.backingScaleFactor
            ?? NSScreen.main?.backingScaleFactor ?? 2
        layer.framebufferOnly = true
        // See NilWindowLayerDelegate: without this, wgpu-hal's occlusion
        // workaround refuses every frame on screen-saver windows.
        layer.delegate = layerDelegate
        if SaverSettings.shared.debugLayerFill {
            // Diagnostics: an opaque fill needs no drawable, so if the display
            // shows this colour the window/layer really is what is on screen.
            layer.backgroundColor = NSColor.red.cgColor
            Self.log.notice("debug layer fill: backing layer painted red")
        }
        return layer
    }

    // MARK: - Process-wide lifecycle registry

    private final class WeakViewRef {
        weak var view: GibsonSaverView?
        init(_ view: GibsonSaverView) { self.view = view }
    }

    /// Views with a live engine, and the single owner allowed per display.
    ///
    /// The host process outlives activations, and `stopAnimation` is not
    /// guaranteed at exit, so a view must be retired by us. The occlusion shim
    /// (see `NilWindowLayerDelegate`) removed wgpu's accidental throttle for a
    /// view whose window is not displayed, so an explicit lifecycle gate is
    /// required — never `occlusionState`, which is meaningless at the screen
    /// saver window level. Main thread only.
    private static var liveViews: [WeakViewRef] = []
    private static var ownerByDisplay: [UInt32: WeakViewRef] = [:]
    private static var sweepTimer: Timer?

    /// How often the sweep re-checks that live engines still have a window.
    private static let sweepInterval: TimeInterval = 2

    private static func register(_ view: GibsonSaverView) {
        liveViews.removeAll { $0.view == nil }
        if !liveViews.contains(where: { $0.view === view }) {
            liveViews.append(WeakViewRef(view))
        }
        startSweepIfNeeded()
    }

    private static func unregister(_ view: GibsonSaverView) {
        liveViews.removeAll { $0.view == nil || $0.view === view }
        for (display, ref) in ownerByDisplay where ref.view == nil || ref.view === view {
            ownerByDisplay.removeValue(forKey: display)
        }
        if liveViews.isEmpty {
            sweepTimer?.invalidate()
            sweepTimer = nil
        }
    }

    private static func startSweepIfNeeded() {
        guard sweepTimer == nil else { return }
        let timer = Timer(timeInterval: sweepInterval, repeats: true) { _ in sweep() }
        RunLoop.main.add(timer, forMode: .common)
        sweepTimer = timer
    }

    /// Retire engines whose view has lost its window. A dismissed screen saver
    /// can leave its view alive with the window ordered out, and a suspended
    /// display link means the per-frame gate never runs, so this has to be
    /// driven from the process, not from the view's own tick.
    ///
    /// The predicate is deliberately lifecycle-only. A `CGWindowListCopyWindowInfo`
    /// "on screen" membership test was tried here and removed: screen saver
    /// windows never appear in that list even while they are the only thing on
    /// screen (measured directly during this investigation), so it tore down
    /// live engines ~4 s into every activation. `occlusionState` is unusable
    /// for the same reason. Only `window == nil` / `!isVisible` answer the
    /// question at this window level.
    private static func sweep() {
        liveViews.removeAll { $0.view == nil }
        let now = CACurrentMediaTime()
        for ref in liveViews {
            guard let view = ref.view, view.gibsonHandle != nil else { continue }
            if view.window == nil || view.window?.isVisible == false {
                view.teardown(reason: "window gone")
                continue
            }
            view.checkLifecycleEvidence(now: now)
        }
    }

    // MARK: - Lifecycle evidence

    /// How often the lifecycle evidence is re-read (the per-frame gate would
    /// otherwise query the window server on every frame).
    private static let lifecycleCheckInterval: TimeInterval = 1
    /// Consecutive "not animating" checks required before the engine is
    /// retired, so a single glitchy read cannot black out a displayed saver.
    private static let notAnimatingChecksBeforeTeardown = 3
    /// Consecutive "cannot be displayed" checks required before teardown.
    private static let noDisplayChecksBeforeTeardown = 2
    private var noDisplayChecks = 0
    /// Grace after engine start before input counts as proof of an orphan.
    private static let inputOrphanGrace: TimeInterval = 5
    /// Consecutive input-while-rendering checks required before teardown.
    private static let inputChecksBeforeTeardown = 3
    private var inputWhileRenderingChecks = 0


    /// Retire the engine when the display it is on cannot be showing it.
    ///
    /// Two independent facts are checked, both public and both cheap:
    ///
    /// 1. **Display power.** `CGDisplayIsAsleep` / `CGDisplayIsActive` /
    ///    `CGDisplayIsOnline` for the display this view's window is on. Nothing
    ///    can be visible on a display that is asleep or off, and the reported
    ///    defect matches this exactly: the leaked engine in the owner's log
    ///    dropped from `fps=60.0` to `fps=7-12` at the moment the saver was
    ///    dismissed — the signature of a display link throttled by a sleeping
    ///    display — and then kept presenting frames for 6h42m. Retiring when
    ///    the display sleeps ends that case, and on wake the framework restarts
    ///    the animation (verified: `didstart` and `startAnimation` both follow
    ///    a display wake-up that resumes a session).
    ///
    /// 2. **The framework's own animation flag** (`ScreenSaverView.animating`,
    ///    documented as "YES when the screen saver is animating"). It never
    ///    flipped in this investigation — it read `true` while displayed *and*
    ///    after dismissal — so it is only ever acted on after it has been seen
    ///    `true` at least once since the engine started, and after several
    ///    consecutive `false` reads. If a future macOS invalidates the
    ///    framework's animation timer at dismissal (which is what the flag is
    ///    for), this becomes a first-class dismissal signal instead of dead
    ///    weight.
    ///
    /// Everything else that was measured is deliberately *not* used, because it
    /// does not distinguish the two states at all: `window.isVisible`,
    /// `window.isOnActiveSpace`, `window.screen`, `superview`,
    /// `isHiddenOrHasHiddenAncestor` and `occlusionState` (raw 8192 in both
    /// states — the visible bit is never set at the screen saver window level)
    /// are identical while displayed and after dismissal, and
    /// `CGWindowListCopyWindowInfo(.optionAll)` returns our own entry with *no*
    /// `kCGWindowIsOnscreen` key in either state: the key is absent, not false,
    /// so the per-entry flag is unavailable to a sandboxed host just as the
    /// `.optionOnScreenOnly` membership test is (that test tore down live
    /// engines seconds into every activation and was reverted).
    ///
    /// 3. **Input arriving while we render.** A displayed screen saver cannot
    ///    coexist with fresh user input, because input is what dismisses it: an
    ///    engine that is still presenting frames after input arrived is an
    ///    orphan by definition. This is the owner's case — the machine was in
    ///    use for hours while the engine rendered. Three traps are closed:
    ///    - the clock is anchored on *this* engine: input only counts when it
    ///      arrived after `engineStartedAt + inputOrphanGrace`. A saver started
    ///      by a keypress or click must not be retired by its own trigger;
    ///    - a locked session is exempt: a locked saver legitimately stays
    ///      visible while a password is typed, so there input is not evidence
    ///      (`CGSessionCopyCurrentDictionary` gains `CGSSessionScreenIsLocked`
    ///      on lock — measured);
    ///    - previews are exempt, because in System Settings the user is
    ///      actively clicking while the preview animates.
    ///    `CGEventSource.secondsSinceLastEventType(.hidSystemState, any input)`
    ///    is public CoreGraphics, needs no permission, and works in the sandbox
    ///    (measured: it read 0.04 s right after a synthetic key event and 7.15 s
    ///    when idle).
    private func checkLifecycleEvidence(now: CFAbsoluteTime) {
        guard gibsonHandle != nil else { return }
        guard now - lastLifecycleCheck >= Self.lifecycleCheckInterval else { return }
        lastLifecycleCheck = now
        if !displayCanPresent() {
            noDisplayChecks += 1
            guard noDisplayChecks >= Self.noDisplayChecksBeforeTeardown else { return }
            noDisplayChecks = 0
            teardown(reason: "display asleep or off")
            return
        }
        noDisplayChecks = 0
        checkInputWhileRendering(now: now)
        guard gibsonHandle != nil else { return }
        if isAnimating {
            sawAnimating = true
            notAnimatingChecks = 0
            return
        }
        guard sawAnimating else { return }
        notAnimatingChecks += 1
        guard notAnimatingChecks >= Self.notAnimatingChecksBeforeTeardown else { return }
        teardown(reason: "framework reports the view is no longer animating")
    }

    /// Retire the engine when user input arrives while it is rendering: input
    /// is what dismisses a screen saver, so a live saver and fresh input cannot
    /// coexist. See `checkLifecycleEvidence` for the closed traps.
    private func checkInputWhileRendering(now: CFAbsoluteTime) {
        guard !isPreview else { return }
        guard let idle = Self.hidIdleSeconds(), !Self.sessionIsLocked() else {
            inputWhileRenderingChecks = 0
            return
        }
        let running = now - engineStartedAt
        // The last input event happened `idle` seconds ago. It counts only if it
        // arrived after this engine had already been running for a grace period.
        guard running > Self.inputOrphanGrace,
              idle < running - Self.inputOrphanGrace else {
            inputWhileRenderingChecks = 0
            return
        }
        inputWhileRenderingChecks += 1
        guard inputWhileRenderingChecks >= Self.inputChecksBeforeTeardown else { return }
        inputWhileRenderingChecks = 0
        teardown(reason: "input arrived while rendering (nothing on screen to dismiss)")
    }

    /// Seconds since the last HID input event, or `nil` when unavailable (then
    /// this gate has no opinion and never acts).
    private static func hidIdleSeconds() -> CFTimeInterval? {
        guard let anyInput = CGEventType(rawValue: UInt32.max) else { return nil }
        return CGEventSource.secondsSinceLastEventType(.hidSystemState, eventType: anyInput)
    }

    /// Whether the login session is locked. A locked screen saver stays visible
    /// while the user types a password, so input is not evidence of an orphan
    /// there and the notification path is used instead.
    private static func sessionIsLocked() -> Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else {
            return false
        }
        return session["CGSSessionScreenIsLocked"] != nil
    }

    /// Whether the display hosting this view can be presenting pixels at all.
    /// Uses the display the window is on, falling back to the main display.
    private func displayCanPresent() -> Bool {
        let display = (window?.screen?.deviceDescription[
            NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
            ?? CGMainDisplayID()
        return CGDisplayIsAsleep(display) == 0
            && CGDisplayIsActive(display) != 0
            && CGDisplayIsOnline(display) != 0
    }

    /// Destroy the engine, drop out of the hierarchy and stop being tracked.
    /// Called for detached and superseded views.
    private func teardown(reason: String) {
        guard gibsonHandle != nil || superview != nil else { return }
        Self.log.notice("tearing down engine (\(reason, privacy: .public))")
        stopEngine()
        removeFromSuperview()
    }

    // MARK: - Animation lifecycle

    override func startAnimation() {
        super.startAnimation()
        Self.everAskedToAnimate = true
        Self.cancelPendingTerminate()
        guard gibsonHandle == nil else { return }
        guard Self.mayAnimateNow(isPreview: isPreview) else {
            // A start request with no live screen saver session behind it.
            deferStartRequest()
            return
        }
        beginAnimation()
    }

    override func stopAnimation() {
        super.stopAnimation()
        Self.log.notice("stopAnimation")
        cancelPendingStart()
        Self.sessionEnded()
        stopEngine()
        scheduleTerminateIfNeeded()
    }

    /// Start the engine and announce it to older instances in this process.
    ///
    /// The announcement happens after the engine is up so older duplicates
    /// retire. The token identifies the author (a view never retires itself)
    /// and the display number lets an observer tell "same screen, superseded"
    /// (retire) from "another screen, still wanted" (keep).
    private func beginAnimation() {
        cancelPendingStart()
        Self.log.notice("startAnimation (preview=\(self.isPreview))")
        startEngine()
        NotificationCenter.default.post(name: Self.newInstanceNotification, object: self,
                                        userInfo: [Self.tokenKey: instanceToken,
                                                   Self.displayKey: displayNumber as Any])
    }

    /// Whether this start request is backed by a screen saver session.
    ///
    /// The ScreenSaver framework calls `startAnimation` on the host it launched,
    /// but it also calls it on a host whose session has already ended — measured
    /// twice on this machine: once for a host relaunched after quitting, and
    /// once for a host woken from display sleep (the framework had called
    /// `stopAnimation` when the display slept, the stop notifications arrived on
    /// wake, and the very next `startAnimation` had no new `didstart` behind it).
    /// An engine started then renders with nothing on screen and no dismissal
    /// can ever end it: that is the reported defect, reproduced from the log.
    ///
    /// So rendering requires evidence of a live session:
    /// - a preview (System Settings) always animates, because previews do not go
    ///   through the session machinery;
    /// - otherwise the request is honoured if no session has ever ended in this
    ///   process (a fresh host may simply have missed the `didstart` that was
    ///   posted before it was launched — refusing then would black out a live
    ///   saver, which is worse than the bug we are fixing);
    /// - otherwise a session must be live (its `didstart` is later than the last
    ///   session end) or must have started within `recentSessionWindow`. That
    ///   window also absorbs the opposite ordering, where the previous session's
    ///   stop notification is delivered just after the new session's `didstart`.
    private static func mayAnimateNow(isPreview: Bool) -> Bool {
        if isPreview { return true }
        if sessionEndedAt < 0 { return true }
        if sessionStartedAt > sessionEndedAt { return true }
        return CACurrentMediaTime() - sessionStartedAt < recentSessionWindow
    }

    /// A start request arrived without session evidence: give the system a
    /// moment to post `didstart` (measured gap 0.1–0.7 s) before deciding not to
    /// render at all.
    private func deferStartRequest() {
        Self.log.notice("startAnimation with no screen saver session; waiting before rendering")
        cancelPendingStart()
        let timer = Timer(timeInterval: Self.startRequestGrace, repeats: false) { [weak self] _ in
            guard let self else { return }
            self.pendingStart = nil
            guard self.gibsonHandle == nil else { return }
            guard Self.sessionStartedAt > Self.sessionEndedAt else {
                Self.log.notice("no screen saver session arrived; not rendering")
                return
            }
            self.beginAnimation()
        }
        RunLoop.main.add(timer, forMode: .common)
        pendingStart = timer
    }

    private func cancelPendingStart() {
        pendingStart?.invalidate()
        pendingStart = nil
    }

    /// The engine never calls `stopAnimation` when the saver ends, so the
    /// distributed stop notifications are treated the same way.
    @objc private func handleWillStop(_ note: Notification) {
        if Thread.isMainThread {
            willStop()
        } else {
            DispatchQueue.main.async { self.willStop() }
        }
    }

    private func willStop() {
        Self.log.notice("screen saver stop notification received")
        cancelPendingStart()
        Self.sessionEnded()
        stopEngine()
        scheduleTerminateIfNeeded()
    }

    /// The system announced that a screen saver session started (`didstart`).
    /// This is the only positive session evidence available to a sandboxed host:
    /// it arms rendering for a start request that is still waiting, and cancels
    /// a pending idle quit — a host the system has just asked to run must not
    /// terminate itself.
    @objc private func handleDidStart(_ note: Notification) {
        let body = { [self] in
            Self.log.notice("screen saver start notification received")
            Self.sessionStarted()
            Self.cancelPendingTerminate()
            if pendingStart != nil, gibsonHandle == nil {
                beginAnimation()
            }
        }
        if Thread.isMainThread {
            body()
        } else {
            DispatchQueue.main.async(execute: body)
        }
    }

    /// A newer instance started in this process. Retire when we are a
    /// lingering copy — the host detached us (no window, or a window that is
    /// no longer visible) — or when the newer instance is on the SAME display
    /// as us (the host starts two views per activation; two live engines on
    /// one screen halve the frame rate for no visible gain). A view still
    /// showing on a DIFFERENT display keeps running. Identity does the
    /// deciding: a view never retires itself.
    @objc private func handleNewInstance(_ note: Notification) {
        if let token = note.userInfo?[Self.tokenKey] as? UUID, token == instanceToken {
            return
        }
        if (note.object as AnyObject?) === self {
            return
        }
        let posterDisplay = (note.userInfo?[Self.displayKey] as? NSNumber)?.uint32Value
        let supersededOnSameScreen = posterDisplay != nil && posterDisplay == displayNumber
        guard isLingering || supersededOnSameScreen else { return }
        let reason = isLingering ? "detached" : "superseded on this display"
        Self.log.notice("newer instance started; retiring this view (\(reason))")
        stopEngine()
        removeFromSuperview()
    }

    /// `CGDirectDisplayID` of the screen this view is on, if any.
    private var displayNumber: UInt32? {
        guard let screen = window?.screen,
              let number = screen.deviceDescription[
                NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else {
            return nil
        }
        return number.uint32Value
    }

    /// True when the host has detached this view: no window, or a window that
    /// is no longer visible. This is a lifecycle fact, not an occlusion
    /// heuristic — a view only microseconds into `startAnimation` is not
    /// "detached", it is simply new.
    private var isLingering: Bool {
        guard let window else { return true }
        return !window.isVisible
    }

    // MARK: - Engine

    private func startEngine() {
        stopEngine()
        guard bounds.width >= 8, bounds.height >= 8 else {
            let size = "\(Int(bounds.width))x\(Int(bounds.height))"
            Self.log.error("view too small to start: \(size, privacy: .public)")
            return
        }
        let backing = effectiveScale
        applyContentsScale(backing)

        // wgpu needs the backing layer to exist before the surface is created.
        _ = layer
        configuredLayer = layer as? CAMetalLayer

        // Renderer convention (mirrors the desktop host): pass the LOGICAL
        // size plus scale = backingScaleFactor x settings.render_scale; the
        // renderer resolves width*scale x height*scale to physical pixels.
        let logical = logicalSize
        let scale = effectiveRenderScale
        guard logical.width >= 8, logical.height >= 8 else {
            Self.log.error("view too small to start")
            return
        }
        let json = SaverSettings.shared.engineSettingsJSON(preview: isPreview)
        let pointer = Unmanaged.passUnretained(self).toOpaque()
        let handle: UnsafeMutableRawPointer? = json.withCString { cString in
            gibson_create(pointer,
                          UInt32(max(1, logical.width.rounded())),
                          UInt32(max(1, logical.height.rounded())),
                          Float(scale), cString)
        }
        guard let handle else {
            Self.log.error("gibson_create returned null")
            return
        }
        gibsonHandle = handle
        engineStartedAt = CACurrentMediaTime()
        hasBeenDisplayed = false
        lastLifecycleCheck = 0
        notAnimatingChecks = 0
        sawAnimating = false
        // An engine is running now: cancel any idle quit armed while starting.
        Self.cancelPendingTerminate()
        Self.register(self)
        if let display = displayNumber {
            // One engine per display, across activations: the host process
            // never exits, so an older view's engine must not keep rendering
            // under the new one.
            if let existing = Self.ownerByDisplay[display]?.view, existing !== self {
                existing.teardown(reason: "superseded on this display")
            }
            Self.ownerByDisplay[display] = Self.WeakViewRef(self)
        }
        ensureLayerDelegate()
        lastLogicalSize = logical
        lastScale = scale
        let created = "gibson_create ok (handle \(UInt(bitPattern: handle)), "
            + "\(Int(logical.width))x\(Int(logical.height)) logical, "
            + "effective scale \(Float(scale)))"
        Self.log.notice("\(created, privacy: .public)")
        startRenderLoop()
    }

    private func stopEngine() {
        stopRenderLoop()
        if let handle = gibsonHandle {
            gibson_destroy(handle)
            gibsonHandle = nil
            Self.log.notice("gibson_destroy ok")
        }
        Self.unregister(self)
        hasBeenDisplayed = false
        frameFailures = 0
        lastLogicalSize = .zero
        lastScale = 0
        notAnimatingChecks = 0
        sawAnimating = false
        // Nothing is animating in this process any more: do not let the host
        // keep the process (and its footprint) alive for no reason.
        scheduleTerminateIfNeeded()
    }

    // MARK: - Frame loop

    /// macOS 14 `NSView.displayLink(target:selector:)`: callbacks fire in sync
    /// with the display the view is on (and stop when the view is not on any
    /// display), always on the main thread.
    private func startRenderLoop() {
        stopRenderLoop()
        let link = displayLink(target: self, selector: #selector(renderTick(_:)))
        link.add(to: .main, forMode: .common)
        displayLink = link
        // First honest counter line shortly after start, then every 5 s: the
        // display-link callback count is not evidence that anything was drawn,
        // so report (presented, skipped) from the renderer itself.
        nextStatsLog = CACurrentMediaTime() + 2
        lastPresented = 0
        lastSkipped = 0
        lastTimeout = 0
        lastOccluded = 0
        frameTimeTotal = 0
        frameTimeCount = 0
        lastStatsWallClock = CACurrentMediaTime()
        Self.log.notice("render loop started")
    }

    private func stopRenderLoop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func renderTick(_ link: CADisplayLink) {
        guard let handle = gibsonHandle else { return }
        // Lifecycle gate (never occlusion): a view with no window, or a window
        // the host has ordered out, must stop rendering and destroy its engine.
        let displayed = window != nil && (window?.isVisible ?? false)
        if !displayed {
            let grace = !hasBeenDisplayed && CACurrentMediaTime() - engineStartedAt < 2
            if !grace {
                teardown(reason: "view detached")
                return
            }
            return
        }
        hasBeenDisplayed = true
        ensureLayerDelegate()
        checkLifecycleEvidence(now: CACurrentMediaTime())
        // The evidence check can retire the engine; never use the stale handle.
        guard gibsonHandle != nil else { return }
        let started = CACurrentMediaTime()
        let code = gibson_frame(handle, started)
        frameTimeTotal += CACurrentMediaTime() - started
        frameTimeCount += 1
        if code == 0 {
            frameFailures = 0
            framesRendered += 1
            if framesRendered - lastStatusLog >= 300 {
                lastStatusLog = framesRendered
                Self.log.notice("rendered \(self.framesRendered) frames, no failures")
            }
        } else {
            frameFailures += 1
            Self.log.error("gibson_frame failed (\(code)), failure \(self.frameFailures)")
            if frameFailures >= 10 {
                Self.log.error("stopping after \(self.frameFailures) consecutive frame failures")
                stopEngine()
                return
            }
        }
        logFrameStats(handle: handle)
    }

    /// Keep the nil-window delegate installed on the render layer. AppKit owns
    /// the backing layer's delegate and reinstalls itself when the layer is
    /// attached, so this runs before every frame (one pointer comparison).
    private func ensureLayerDelegate() {
        guard let metalLayer = layer as? CAMetalLayer, metalLayer.delegate !== layerDelegate else {
            return
        }
        metalLayer.delegate = layerDelegate
        Self.log.notice("reinstalled nil-window layer delegate")
    }

    /// Periodic evidence that pixels are actually being presented, taken from
    /// the renderer's own counters (a callback that skipped a frame is not a
    /// rendered frame), plus the window/layer facts needed to explain a black
    /// screen: whether the layer wgpu configured is still the view's layer,
    /// whether that layer can produce drawables, and where the window sits.
    ///
    /// `presented` is incremented once per frame, after `queue.present` returns
    /// Ok — never per display-link callback, and never for a frame that was
    /// skipped (those increment the skip counters instead). `queue.present`
    /// returning means *submitted*, not *retired* by the GPU, so this `fps` is
    /// honest about presents and optimistic about display: over a short window
    /// it can exceed what the GPU actually finished (a measured 5 s window read
    /// 47-60 fps while sustained throughput on the same 2.80 Mpx target was
    /// 23-28 fps). Read it as a submission rate, not as a guaranteed
    /// on-screen frame rate.
    private func logFrameStats(handle: UnsafeMutableRawPointer) {
        let now = CACurrentMediaTime()
        guard now >= nextStatsLog else { return }
        nextStatsLog = now + 5

        if SaverSettings.shared.debugLayerFill {
            // Keep whichever layer is current painted red, so a layer swap by
            // the host cannot hide the diagnostic.
            (layer as? CAMetalLayer)?.backgroundColor = NSColor.red.cgColor
        }

        var presented: UInt64 = 0
        var skipped: UInt64 = 0
        let statsCode = gibson_present_stats(handle, &presented, &skipped)
        var timeouts: UInt64 = 0
        var occluded: UInt64 = 0
        _ = gibson_skip_breakdown(handle, &timeouts, &occluded)

        let deltaPresented = presented - lastPresented
        let deltaSkipped = skipped - lastSkipped
        let deltaTimeout = timeouts - lastTimeout
        let deltaOccluded = occluded - lastOccluded
        lastPresented = presented
        lastSkipped = skipped
        lastTimeout = timeouts
        lastOccluded = occluded

        let elapsed = lastStatsWallClock > 0 ? now - lastStatsWallClock : 5
        lastStatsWallClock = now
        let fps = elapsed > 0 ? Double(deltaPresented) / elapsed : 0
        let msPerFrame = frameTimeCount > 0
            ? (frameTimeTotal / Double(frameTimeCount)) * 1000 : 0
        frameTimeTotal = 0
        frameTimeCount = 0
        let currentLayer = layer as? CAMetalLayer
        let sameLayer = currentLayer != nil && currentLayer === configuredLayer
        let drawable = currentLayer?.drawableSize ?? .zero
        let layerDevice = currentLayer?.device != nil
        let layerAttached = currentLayer?.superlayer != nil
        let window = self.window
        let level = window?.level.rawValue ?? 0
        let onActiveSpace = window?.isOnActiveSpace ?? false
        let windowNumber = window?.windowNumber ?? -1
        let screenFrame = window?.screen?.frame ?? .zero
        // The facts a future "is it still displayed?" investigation needs, all
        // cheap and all kept permanently: the framework's own animation flag,
        // the display power state, and who is frontmost. See
        // `checkLifecycleEvidence` for the measurements behind each one.
        let display = (window?.screen?.deviceDescription[
            NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
            ?? CGMainDisplayID()
        let displayPower = "asleep=\(CGDisplayIsAsleep(display)) "
            + "active=\(CGDisplayIsActive(display)) online=\(CGDisplayIsOnline(display))"
        let frontmost = NSWorkspace.shared.frontmostApplication?.localizedName ?? "nil"
        let pacing = "fps=\(String(format: "%.1f", fps)) "
            + "msPerFrame=\(String(format: "%.1f", msPerFrame)) "
        let line = pacing + "frames presented=\(deltaPresented) skipped=\(deltaSkipped) "
            + "(total presented=\(presented), stats=\(statsCode)) "
            + "skipTimeout=\(deltaTimeout) skipOccluded=\(deltaOccluded) "
            + "windowVisible=\(window?.isVisible ?? false) "
            + "key=\(window?.isKeyWindow ?? false) main=\(window?.isMainWindow ?? false) "
            + "screenAttached=\(window?.screen != nil) "
            + "occlusionVisible=\(window?.occlusionState.contains(.visible) ?? false) "
            + "level=\(Int(level)) onActiveSpace=\(onActiveSpace) windowNumber=\(windowNumber) "
            + "screen=\(Int(screenFrame.width))x\(Int(screenFrame.height)) "
            + "view=\(Int(bounds.width))x\(Int(bounds.height))@\(Int(bounds.origin.x)),\(Int(bounds.origin.y)) "
            + "layerSameAsCreated=\(sameLayer) drawable=\(Int(drawable.width))x\(Int(drawable.height)) "
            + "layerDevice=\(layerDevice) layerAttached=\(layerAttached) "
            + "animating=\(isAnimating) display[\(displayPower)] frontmost=\(frontmost) "
            + "hidIdle=\(String(format: "%.2f", Self.hidIdleSeconds() ?? -1)) "
            + "locked=\(Self.sessionIsLocked()) "
            + "instance=\(instanceToken.uuidString.prefix(8))"
        Self.log.notice("\(line, privacy: .public)")
    }

    // MARK: - Sizing

    override func layout() {
        super.layout()
        updateSurfaceSize()
    }

    override func viewDidEndLiveResize() {
        super.viewDidEndLiveResize()
        updateSurfaceSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        updateSurfaceSize()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            // The host closed our window (or we were removed): never leave a
            // lingering renderer pointing at a dead layer.
            stopEngine()
        } else {
            updateSurfaceSize()
        }
    }

    private var effectiveScale: CGFloat {
        window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
    }

    /// Logical (point) size of the view. The renderer wants logical points;
    /// passing it the physical size as well as the scale would configure the
    /// surface at `physical x backing` (4x the pixels on Retina).
    private var logicalSize: CGSize {
        let backing = max(effectiveScale, 1)
        let physical = convertToBacking(bounds).size
        return CGSize(width: (physical.width / backing).rounded(),
                      height: (physical.height / backing).rounded())
    }

    /// Effective scale handed to the renderer: backing scale factor times the
    /// `render_scale` preference (the renderer turns `logical x scale` into
    /// physical pixels, so `render_scale` 1 renders 1:1 and lower values
    /// downsample). Mirrors the desktop host's `scale = sf x render_scale`.
    private var effectiveRenderScale: CGFloat {
        effectiveScale * CGFloat(max(SaverSettings.shared.renderScale, 0.25))
    }

    private func applyContentsScale(_ scale: CGFloat) {
        (layer as? CAMetalLayer)?.contentsScale = scale
    }

    /// Keep the wgpu surface in step with the view. Called from layout and
    /// backing-property changes; only acts when the logical size or effective
    /// scale actually moved.
    private func updateSurfaceSize() {
        guard let handle = gibsonHandle else { return }
        let backing = effectiveScale
        applyContentsScale(backing)
        let logical = logicalSize
        guard logical.width >= 8, logical.height >= 8 else { return }
        let scale = effectiveRenderScale
        guard logical != lastLogicalSize || abs(scale - lastScale) > 0.001 else { return }
        gibson_resize(handle,
                      UInt32(max(1, logical.width.rounded())),
                      UInt32(max(1, logical.height.rounded())),
                      Float(scale))
        lastLogicalSize = logical
        lastScale = scale
        let resized = "gibson_resize ok (\(Int(logical.width))x\(Int(logical.height)) logical, "
            + "effective scale \(Float(scale)))"
        Self.log.notice("\(resized, privacy: .public)")
    }

    // MARK: - Configuration sheet

    override var hasConfigureSheet: Bool { true }

    override var configureSheet: NSWindow? {
        ConfigureSheet.make()
    }

    // MARK: - Self termination

    /// `legacyScreenSaver` does not exit on its own, and an unanimated host
    /// holds tens of MB of RSS indefinitely (measured ~87 MB). Quit the process
    /// once it has been idle long enough — but *only* for a host that has never
    /// been asked to animate.
    ///
    /// That restriction is measured, not defensive. Quitting a host that the
    /// system has already asked to animate makes macOS relaunch it immediately
    /// and hand it the same request again, which starts an engine with nothing
    /// on screen and no dismissal to end it — the very defect this change
    /// exists to remove. Observed directly (host log, 5 s stats line, and a
    /// distributed-notification watcher all agree):
    /// ```
    /// 02:00:00.211 idle 65 s with no engine; terminating host process
    /// 02:00:00.920 [new host pid 61087] idle: no engine running ...
    /// 02:00:01.440 [new host pid 61087] startAnimation (preview=false)
    /// 02:00:02.080 [new host pid 61087] gibson_create ok ...
    /// ... rendered 2400 frames ... with no didstart, no willstop, no session
    /// ```
    /// A speculative host that was never asked to animate has no such pending
    /// request, so quitting it is free (macOS starts a host when one is next
    /// needed). `didstart` and `startAnimation` both cancel a pending quit.
    private func scheduleTerminateIfNeeded() {
        guard !isPreview else { return }
        guard !Self.everAskedToAnimate else { return }
        guard !Self.hasRunningEngine else { return }
        Self.cancelPendingTerminate()
        // Auditable idleness: with no engine there are no frame stats lines at
        // all, so prove the process is not rendering (and is about to quit)
        // with a slow heartbeat. Bounded: the quit below fires after 65 s.
        Self.idleLog("idle: no engine running; host will quit in 65 s")
        let heartbeat = Timer(timeInterval: 15, repeats: true) { _ in
            Self.idleLog("idle heartbeat: still no engine in this process")
        }
        RunLoop.main.add(heartbeat, forMode: .common)
        Self.idleHeartbeatTimer = heartbeat
        let timer = Timer(timeInterval: 65, repeats: false) { _ in
            Self.idleLog("idle 65 s with no engine; terminating host process")
            NSApp.terminate(nil)
        }
        RunLoop.main.add(timer, forMode: .common)
        Self.terminateTimer = timer
    }

    /// Log a process-idle line (persisted at notice level).
    private static func idleLog(_ message: String) {
        log.notice("\(message, privacy: .public)")
    }

    /// True while any view in this process has a live engine.
    private static var hasRunningEngine: Bool {
        liveViews.contains { $0.view?.gibsonHandle != nil }
    }

    private static func cancelPendingTerminate() {
        terminateTimer?.invalidate()
        terminateTimer = nil
        idleHeartbeatTimer?.invalidate()
        idleHeartbeatTimer = nil
    }
}
