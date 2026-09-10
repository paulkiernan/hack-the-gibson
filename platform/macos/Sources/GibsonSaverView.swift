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
        DistributedNotificationCenter.default().addObserver(
            self, selector: #selector(handleWillStop(_:)),
            name: NSNotification.Name("com.apple.screensaver.willstop"), object: nil)
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    deinit {
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
            Self.log.info("debug layer fill: backing layer painted red")
        }
        return layer
    }

    // MARK: - Animation lifecycle

    override func startAnimation() {
        super.startAnimation()
        Self.cancelPendingTerminate()
        guard gibsonHandle == nil else { return }
        Self.log.info("startAnimation (preview=\(self.isPreview))")
        startEngine()
        // Announce after the engine is up so older duplicates retire. The token
        // identifies the author (a view never retires itself) and the display
        // number lets an observer tell "same screen, superseded" (retire) from
        // "another screen, still wanted" (keep).
        NotificationCenter.default.post(name: Self.newInstanceNotification, object: self,
                                        userInfo: [Self.tokenKey: instanceToken,
                                                   Self.displayKey: displayNumber as Any])
    }

    override func stopAnimation() {
        super.stopAnimation()
        Self.log.info("stopAnimation")
        stopEngine()
        scheduleTerminateIfNeeded()
    }

    /// The engine never calls `stopAnimation` when the saver ends; the
    /// distributed `willstop` notification does arrive, so treat it the same.
    @objc private func handleWillStop(_ note: Notification) {
        if Thread.isMainThread {
            willStop()
        } else {
            DispatchQueue.main.async { self.willStop() }
        }
    }

    private func willStop() {
        Self.log.info("willstop notification received")
        stopEngine()
        scheduleTerminateIfNeeded()
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
        Self.log.info("newer instance started; retiring this view (\(reason))")
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
        ensureLayerDelegate()
        lastLogicalSize = logical
        lastScale = scale
        let created = "gibson_create ok (handle \(UInt(bitPattern: handle)), "
            + "\(Int(logical.width))x\(Int(logical.height)) logical, "
            + "effective scale \(Float(scale)))"
        Self.log.info("\(created, privacy: .public)")
        startRenderLoop()
    }

    private func stopEngine() {
        stopRenderLoop()
        if let handle = gibsonHandle {
            gibson_destroy(handle)
            gibsonHandle = nil
            Self.log.info("gibson_destroy ok")
        }
        frameFailures = 0
        lastLogicalSize = .zero
        lastScale = 0
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
        Self.log.info("render loop started")
    }

    private func stopRenderLoop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func renderTick(_ link: CADisplayLink) {
        guard let handle = gibsonHandle else { return }
        ensureLayerDelegate()
        let started = CACurrentMediaTime()
        let code = gibson_frame(handle, started)
        frameTimeTotal += CACurrentMediaTime() - started
        frameTimeCount += 1
        if code == 0 {
            frameFailures = 0
            framesRendered += 1
            if framesRendered - lastStatusLog >= 300 {
                lastStatusLog = framesRendered
                Self.log.info("rendered \(self.framesRendered) frames, no failures")
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
        Self.log.info("reinstalled nil-window layer delegate")
    }

    /// Periodic evidence that pixels are actually being presented, taken from
    /// the renderer's own counters (a callback that skipped a frame is not a
    /// rendered frame), plus the window/layer facts needed to explain a black
    /// screen: whether the layer wgpu configured is still the view's layer,
    /// whether that layer can produce drawables, and where the window sits.
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
        let pacing = "fps=\(String(format: "%.1f", fps)) "
            + "msPerFrame=\(String(format: "%.1f", msPerFrame)) "
        let line = pacing + "frames presented=\(deltaPresented) skipped=\(deltaSkipped) "
            + "(total presented=\(presented), stats=\(statsCode)) "
            + "skipTimeout=\(deltaTimeout) skipOccluded=\(deltaOccluded) "
            + "windowVisible=\(window?.isVisible ?? false) "
            + "occlusionVisible=\(window?.occlusionState.contains(.visible) ?? false) "
            + "level=\(Int(level)) onActiveSpace=\(onActiveSpace) windowNumber=\(windowNumber) "
            + "screen=\(Int(screenFrame.width))x\(Int(screenFrame.height)) "
            + "view=\(Int(bounds.width))x\(Int(bounds.height))@\(Int(bounds.origin.x)),\(Int(bounds.origin.y)) "
            + "layerSameAsCreated=\(sameLayer) drawable=\(Int(drawable.width))x\(Int(drawable.height)) "
            + "layerDevice=\(layerDevice) layerAttached=\(layerAttached) "
            + "instance=\(instanceToken.uuidString.prefix(8))"
        Self.log.info("\(line, privacy: .public)")
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
        Self.log.info("\(resized, privacy: .public)")
    }

    // MARK: - Configuration sheet

    override var hasConfigureSheet: Bool { true }

    override var configureSheet: NSWindow? {
        ConfigureSheet.make()
    }

    // MARK: - Self termination

    /// `legacyScreenSaver` never exits. Once this (non-preview) instance has
    /// been stopped, quit the process after a grace period so a fresh, clean
    /// engine starts on the next activation. Any new `startAnimation` cancels
    /// the pending quit.
    private func scheduleTerminateIfNeeded() {
        guard !isPreview else { return }
        Self.cancelPendingTerminate()
        let timer = Timer(timeInterval: 65, repeats: false) { _ in
            Self.log.info("idle 65 s after stop; terminating host process")
            NSApp.terminate(nil)
        }
        RunLoop.main.add(timer, forMode: .common)
        Self.terminateTimer = timer
    }

    private static func cancelPendingTerminate() {
        terminateTimer?.invalidate()
        terminateTimer = nil
    }
}
