# Cask for the owner's personal tap (paulkiernan/homebrew-tap), NOT for
# homebrew/cask. Two independent reasons, both documented in
# packaging/homebrew/README.md:
#
#   * homebrew/cask requires that everything Gatekeeper can assess passes
#     Homebrew's Gatekeeper checks and does not need Gatekeeper or SIP to be
#     disabled or bypassed. This saver is ad-hoc signed and not notarized, so
#     the only way to run a downloaded copy is to clear the quarantine flag.
#   * a self-submitted cask must meet Homebrew's notability thresholds (90
#     forks, 90 watchers or 225 stars for the repository owner); this repo is
#     far below them.
cask "gibson-screensaver" do
  version "2.1.0"
  sha256 "a290dc1a9e8853254cd12549b6971285d1c39ea1c7607aa6123b9574628303ee"

  url "https://github.com/paulkiernan/gibson-screensaver/releases/download/#{version}/Gibson.saver.zip"
  name "The Gibson"
  desc "Tower-city flythrough from Hackers (1995)"
  homepage "https://paulkiernan.github.io/gibson-screensaver/"

  # The bundle declares LSMinimumSystemVersion 14.0, and that is what the
  # release workflow builds and tests against.
  depends_on macos: ">= :sonoma"

  # Moves Gibson.saver into ~/Library/Screen Savers.
  screen_saver "Gibson.saver"

  caveats <<~EOS
    This screen saver is ad-hoc signed and not notarized, so macOS quarantines
    a copy downloaded from the internet. A quarantined .saver is worse than a
    blocked app: it appears in the screen saver list and then silently never
    draws, with no "Open anyway" prompt, because the system's screen saver
    process loads it rather than launching it as an app. Clear the flag once,
    after installing:

      xattr -dr com.apple.quarantine "$HOME/Library/Screen Savers/Gibson.saver"
      killall legacyScreenSaver 2>/dev/null || true

    Then choose "The Gibson" under System Settings > Wallpaper > Screen Saver.
    Options are in the saver's own settings sheet (fly speed, palette, bloom,
    motion blur, grain, CRT, render scale).

    Clearing the attribute does not affect the code signature.
  EOS

  zap trash: [
    "~/Library/Application Support/gibson-screensaver",
    "~/Library/Preferences/org.hackthegibson.TheGibson.plist",
  ]
end
