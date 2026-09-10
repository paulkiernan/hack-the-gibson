import AppKit
import ScreenSaver

/// Options sheet shown from System Settings (or the preview host). Built
/// entirely in code with an `NSGridView` — the bundle carries no `.xib`/`.storyboard`.
///
/// A view controller (not a loose window) on purpose: the window retains the
/// controller, sizes itself from the content, and the button targets stay
/// alive as long as the sheet is on screen.
final class ConfigureSheetController: NSViewController {
    var onDismiss: (() -> Void)?

    private let preferences = SaverSettings.shared
    private let speedSlider = NSSlider(value: 0.55, minValue: 0.2, maxValue: 1.2,
                                       target: nil, action: nil)
    private let bankSlider = NSSlider(value: 0.45, minValue: 0, maxValue: 1,
                                      target: nil, action: nil)
    private let crtSlider = NSSlider(value: 0.35, minValue: 0, maxValue: 1,
                                     target: nil, action: nil)
    private let renderScaleSlider = NSSlider(value: 1.0, minValue: 0.5, maxValue: 1.0,
                                             target: nil, action: nil)
    private let palettePopup = NSPopUpButton(frame: .zero, pullsDown: false)
    private let bloomCheck = NSButton(checkboxWithTitle: "Bloom glow",
                                      target: nil, action: nil)
    private let motionCheck = NSButton(checkboxWithTitle: "Motion blur",
                                       target: nil, action: nil)
    private let grainCheck = NSButton(checkboxWithTitle: "Film grain",
                                      target: nil, action: nil)

    private static let controlWidth: CGFloat = 240

    // MARK: - View construction

    override func loadView() {
        let speedLabel = label("Fly speed")
        let bankLabel = label("Banking")
        let paletteLabel = label("Palette")
        let crtLabel = label("CRT overlay")
        let renderScaleLabel = label("Render scale")

        speedSlider.isContinuous = true
        speedSlider.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.controlWidth).isActive = true
        bankSlider.isContinuous = true
        bankSlider.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.controlWidth).isActive = true
        crtSlider.isContinuous = true
        crtSlider.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.controlWidth).isActive = true
        renderScaleSlider.isContinuous = true
        renderScaleSlider.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.controlWidth).isActive = true

        palettePopup.addItems(withTitles: ["Normal", "Siege", "Cycle"])
        palettePopup.selectItem(at: 0)

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancelPressed))
        cancel.keyEquivalent = "\u{1b}"
        let ok = NSButton(title: "OK", target: self, action: #selector(okPressed))
        ok.keyEquivalent = "\r"
        let buttonsRow = buttonRowContainer(cancel, ok)

        // Grid rows: five label+control rows (speed, bank, CRT, render scale,
        // palette), three full-width checkbox rows, one right-aligned button row.
        let grid = NSGridView(views: [
            [speedLabel, speedSlider],
            [bankLabel, bankSlider],
            [crtLabel, crtSlider],
            [renderScaleLabel, renderScaleSlider],
            [paletteLabel, palettePopup],
            [bloomCheck, NSView()],
            [motionCheck, NSView()],
            [grainCheck, NSView()],
            [buttonsRow, NSView()],
        ])
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.rowSpacing = 14
        grid.columnSpacing = 14
        grid.rowAlignment = .firstBaseline

        // Merge the checkbox and button rows so they span the full width.
        for row in 5...8 {
            grid.mergeCells(inHorizontalRange: NSRange(location: 0, length: 2),
                            verticalRange: NSRange(location: row, length: 1))
        }

        let content = NSView()
        content.addSubview(grid)
        NSLayoutConstraint.activate([
            grid.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            grid.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),
            grid.topAnchor.constraint(equalTo: content.topAnchor, constant: 20),
            grid.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -20),
        ])

        view = content
        content.layoutSubtreeIfNeeded()
        let fitting = content.fittingSize
        content.frame = CGRect(origin: .zero, size: fitting)
        preferredContentSize = fitting
        loadState()
    }

    private func buttonRowContainer(_ buttons: NSView...) -> NSView {
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let row = NSStackView(views: [spacer] + buttons)
        row.orientation = .horizontal
        row.spacing = 10
        return row
    }

    private func label(_ text: String) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.translatesAutoresizingMaskIntoConstraints = false
        return field
    }

    // MARK: - State

    private func loadState() {
        speedSlider.doubleValue = min(max(preferences.flySpeed, speedSlider.minValue),
                                      speedSlider.maxValue)
        bankSlider.doubleValue = min(max(preferences.bankStrength, bankSlider.minValue),
                                     bankSlider.maxValue)
        crtSlider.doubleValue = min(max(preferences.crt, crtSlider.minValue),
                                    crtSlider.maxValue)
        renderScaleSlider.doubleValue = min(max(preferences.renderScale, renderScaleSlider.minValue),
                                            renderScaleSlider.maxValue)
        let palette = preferences.palette
        palettePopup.selectItem(at: palette == "siege" ? 1 : (palette == "cycle" ? 2 : 0))
        bloomCheck.state = preferences.bloomEnabled ? .on : .off
        motionCheck.state = preferences.motionBlurEnabled ? .on : .off
        grainCheck.state = preferences.grainEnabled ? .on : .off
    }

    @objc private func okPressed() {
        preferences.flySpeed = speedSlider.doubleValue
        preferences.bankStrength = bankSlider.doubleValue
        preferences.crt = crtSlider.doubleValue
        preferences.renderScale = renderScaleSlider.doubleValue
        preferences.palette = ["normal", "siege", "cycle"][max(0, palettePopup.indexOfSelectedItem)]
        preferences.bloomEnabled = bloomCheck.state == .on
        preferences.motionBlurEnabled = motionCheck.state == .on
        preferences.grainEnabled = grainCheck.state == .on
        preferences.save()
        close()
    }

    @objc private func cancelPressed() {
        close()
    }

    /// The host may present the options window as a sheet, as a modal window,
    /// or as a plain window. Ending the wrong way leaves the host believing a
    /// sheet is still up, after which every later click on Options does
    /// nothing — handle all three.
    private func close() {
        defer { onDismiss?() }
        guard let window = view.window else { return }
        if let parent = window.sheetParent {
            parent.endSheet(window)
        } else if NSApp.modalWindow === window {
            NSApp.stopModal()
            window.orderOut(nil)
        } else {
            window.orderOut(nil)
        }
    }
}

/// The options window handed to ScreenSaver's `configureSheet`. Cached per
/// process: the host reads `configureSheet` more than once per click, and a
/// fresh window per read would leave dead sheets behind.
final class ConfigureSheet {
    private static var window: NSWindow?

    static func make() -> NSWindow {
        if let existing = window, existing.isVisible || existing.sheetParent != nil {
            return existing
        }
        let controller = ConfigureSheetController()
        controller.onDismiss = { window = nil }

        let win = NSWindow(contentViewController: controller)
        win.styleMask = [.titled]
        win.title = "The Gibson"
        win.isReleasedWhenClosed = false
        window = win
        return win
    }
}
