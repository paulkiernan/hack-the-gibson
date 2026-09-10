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
    private static let log = Logger(subsystem: SaverSettings.moduleName,
                                    category: "GibsonSaverView")

    /// Posted on the process's default center whenever a view starts
    /// animating; older detached instances use it to lame-duck themselves.
    private static let newInstanceNotification =
        Notification.Name("org.hackthegibson.TheGibson.NewInstance")
    /// `userInfo` key carrying the poster's [`instanceToken`].
    private static let tokenKey = "instance-token"
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
        return layer
    }

    // MARK: - Animation lifecycle

    override func startAnimation() {
        super.startAnimation()
        Self.cancelPendingTerminate()
        guard gibsonHandle == nil else { return }
        Self.log.info("startAnimation (preview=\(self.isPreview))")
        startEngine()
        // Announce after the engine is up so older detached copies retire. The
        // token identifies the author: every observer (this one included)
        // ignores a notification it authored.
        NotificationCenter.default.post(name: Self.newInstanceNotification, object: self,
                                        userInfo: [Self.tokenKey: instanceToken])
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

    /// A newer instance started in this process. Retire only if this view is a
    /// lingering copy — one the host has detached (no window, or a window that
    /// is no longer visible). Identity does the deciding: a view never retires
    /// itself, and a view still showing on screen (another display, the System
    /// Settings tile) keeps running even though a newer one exists.
    @objc private func handleNewInstance(_ note: Notification) {
        if let token = note.userInfo?[Self.tokenKey] as? UUID, token == instanceToken {
            return
        }
        if (note.object as AnyObject?) === self {
            return
        }
        guard isLingering else { return }
        Self.log.info("newer instance started; retiring this detached view")
        stopEngine()
        removeFromSuperview()
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
        Self.log.info("render loop started")
    }

    private func stopRenderLoop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func renderTick(_ link: CADisplayLink) {
        guard let handle = gibsonHandle else { return }
        let code = gibson_frame(handle, CACurrentMediaTime())
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

    /// Periodic evidence that pixels are actually being presented, taken from
    /// the renderer's own counters (a callback that skipped a frame is not a
    /// rendered frame).
    private func logFrameStats(handle: UnsafeMutableRawPointer) {
        let now = CACurrentMediaTime()
        guard now >= nextStatsLog else { return }
        nextStatsLog = now + 5
        var presented: UInt64 = 0
        var skipped: UInt64 = 0
        let code = gibson_present_stats(handle, &presented, &skipped)
        guard code == 0 else {
            Self.log.error("gibson_present_stats failed (\(code))")
            return
        }
        let deltaPresented = presented - lastPresented
        let deltaSkipped = skipped - lastSkipped
        lastPresented = presented
        lastSkipped = skipped
        // Window state is reported, never used as a gate: it tells a black
        // screen apart from a slow one (a window the WindowServer does not
        // scan out yields no Metal drawables, so every frame is skipped).
        let windowVisible = window?.isVisible ?? false
        let windowOnScreen = window?.occlusionState.contains(.visible) ?? false
        let line = "frames presented=\(deltaPresented) skipped=\(deltaSkipped) "
            + "(total presented=\(presented)) windowVisible=\(windowVisible) "
            + "windowOnScreen=\(windowOnScreen)"
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
