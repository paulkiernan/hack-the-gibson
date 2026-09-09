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
    /// animating; older hidden instances use it to lame-duck themselves.
    private static let newInstanceNotification =
        Notification.Name("org.hackthegibson.TheGibson.NewInstance")
    /// App-wide self-terminate timer: the host process lingers forever, so the
    /// saver quits itself once it has been stopped for a while. One timer for
    /// the process (termination is process-wide); a new start cancels it.
    private static var terminateTimer: Timer?

    private var gibsonHandle: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private var frameFailures = 0
    private var framesRendered = 0
    private var lastStatusLog = 0
    /// Logical (point) size and effective scale last handed to the renderer.
    private var lastLogicalSize = CGSize.zero
    private var lastScale: CGFloat = 0

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
        // Newest instance wins: older lingering views stop when they see this.
        NotificationCenter.default.post(name: Self.newInstanceNotification, object: self)
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

    /// A newer instance started in this process. If this view is not on screen
    /// anymore it is a lingering copy: stop rendering, drop the GPU resources,
    /// and leave the hierarchy. A view that is still visible (another display,
    /// the System Settings tile) keeps running.
    @objc private func handleNewInstance(_ note: Notification) {
        guard let poster = note.object as AnyObject?, poster !== self else { return }
        guard !isEffectivelyVisible else { return }
        Self.log.info("newer instance started; stopping this lingering view")
        stopEngine()
        removeFromSuperview()
    }

    private var isEffectivelyVisible: Bool {
        guard let window else { return false }
        return window.occlusionState.contains(.visible)
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
        Self.log.info("render loop started")
    }

    private func stopRenderLoop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func renderTick(_ link: CADisplayLink) {
        guard let handle = gibsonHandle else { return }
        guard isEffectivelyVisible else { return }
        let code = gibson_frame(handle, CACurrentMediaTime())
        if code == 0 {
            frameFailures = 0
            framesRendered += 1
            if framesRendered - lastStatusLog >= 300 {
                lastStatusLog = framesRendered
                Self.log.info("rendered \(self.framesRendered) frames, no failures")
            }
            return
        }
        frameFailures += 1
        Self.log.error("gibson_frame failed (\(code)), failure \(self.frameFailures)")
        if frameFailures >= 10 {
            Self.log.error("stopping after \(self.frameFailures) consecutive frame failures")
            stopEngine()
        }
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
