import AppKit
import ScreenSaver

/// Screen saver preferences, persisted through `ScreenSaverDefaults` under the
/// bundle identifier. `ScreenSaverDefaults` (not `UserDefaults.standard`) is
/// required because several host processes (System Settings preview,
/// `legacyScreenSaver`) load the same saver and must share one preference
/// domain.
///
/// Keys are the `gibson_types::Settings` field names; booleans gate effects
/// and are translated into the engine's intensity amounts when the JSON
/// payload is built.
final class SaverSettings {
    /// The screen saver's defaults domain. Resolved from the loaded bundle
    /// (falls back to the documented constant) so the preview host, the
    /// options sheet and `legacyScreenSaver` all share one domain.
    static var moduleName: String {
        Bundle(for: SaverSettings.self).bundleIdentifier
            ?? "org.hackthegibson.TheGibson"
    }
    static let shared = SaverSettings()

    private enum Key {
        static let flySpeed = "fly_speed"
        static let bankStrength = "bank_strength"
        static let palette = "palette"
        static let bloom = "bloom"
        static let motionBlur = "motion_blur"
        static let grain = "grain"
        static let crt = "crt"
        static let renderScale = "render_scale"
        /// Diagnostic only: fill the backing layer opaque red so it is obvious
        /// whether the window this view lives in is what the display shows.
        static let debugLayer = "debug_layer"
    }

    private let store: UserDefaults

    private init() {
        store = ScreenSaverDefaults(forModuleWithName: Self.moduleName) ?? .standard
        store.register(defaults: [
            Key.flySpeed: 0.55,
            Key.bankStrength: 0.45,
            Key.palette: "normal",
            Key.bloom: true,
            Key.motionBlur: true,
            Key.grain: true,
            Key.crt: 0.35,
            Key.renderScale: 1.0,
            Key.debugLayer: false,
        ])
    }

    // MARK: - Individual settings

    var flySpeed: Double {
        get { store.double(forKey: Key.flySpeed) }
        set { store.set(newValue, forKey: Key.flySpeed) }
    }

    var bankStrength: Double {
        get { store.double(forKey: Key.bankStrength) }
        set { store.set(newValue, forKey: Key.bankStrength) }
    }

    /// Lowercase `gibson_types::PaletteMode` name: "normal", "siege", "cycle".
    var palette: String {
        get {
            let value = store.string(forKey: Key.palette) ?? ""
            return ["normal", "siege", "cycle"].contains(value) ? value : "normal"
        }
        set { store.set(newValue, forKey: Key.palette) }
    }

    var bloomEnabled: Bool {
        get { store.bool(forKey: Key.bloom) }
        set { store.set(newValue, forKey: Key.bloom) }
    }

    var motionBlurEnabled: Bool {
        get { store.bool(forKey: Key.motionBlur) }
        set { store.set(newValue, forKey: Key.motionBlur) }
    }

    var grainEnabled: Bool {
        get { store.bool(forKey: Key.grain) }
        set { store.set(newValue, forKey: Key.grain) }
    }

    /// CRT overlay strength 0.0 (off) ... 1.0 (full effect); engine default 0.35.
    var crt: Double {
        get { store.double(forKey: Key.crt) }
        set { store.set(newValue, forKey: Key.crt) }
    }

    var renderScale: Double {
        get { store.double(forKey: Key.renderScale) }
        set { store.set(newValue, forKey: Key.renderScale) }
    }

    /// Diagnostic switch (see `Key.debugLayer`); never enabled by default.
    var debugLayerFill: Bool {
        get { store.bool(forKey: Key.debugLayer) }
        set { store.set(newValue, forKey: Key.debugLayer) }
    }

    /// Flush to disk. `ScreenSaverDefaults` caches per process, so the options
    /// sheet (which can run in another process) also calls this to refresh.
    func save() {
        store.synchronize()
    }

    // MARK: - Engine payload

    /// The JSON object handed to `gibson_create`. Intensity amounts mirror the
    /// `Settings` defaults so toggling a checkbox on reproduces the defaults.
    func engineSettingsJSON(preview: Bool) -> String {
        let payload: [String: Any] = [
            "fly_speed": flySpeed,
            "bank_strength": bankStrength,
            "palette": palette,
            "bloom": bloomEnabled ? 0.35 : 0.0,
            "motion_blur": motionBlurEnabled ? 0.5 : 0.0,
            "grain": grainEnabled ? 0.03 : 0.0,
            "crt": min(max(crt, 0.0), 1.0),
            "render_scale": max(renderScale, 0.25),
            "preview": preview,
        ]
        if let data = try? JSONSerialization.data(withJSONObject: payload),
           let string = String(data: data, encoding: .utf8) {
            return string
        }
        return "{}"
    }
}
