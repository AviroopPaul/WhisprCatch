# Source of truth for the Homebrew cask.
#
# Lives here so it is reviewed with the code it installs; the Release workflow
# copies it into the tap repo (AviroopPaul/homebrew-whisprcatch) with `version`
# and `sha256` rewritten to match the release being published.
#
# Users install with:
#   brew install --cask AviroopPaul/whisprcatch/whisprcatch
cask "whisprcatch" do
  version "0.0.0"
  sha256 :no_check

  url "https://github.com/AviroopPaul/whisper-catch/releases/download/v#{version}/WhisprCatch-#{version}-arm64.dmg",
      verified: "github.com/AviroopPaul/whisper-catch/"
  name "WhisprCatch"
  desc "Local push-to-talk dictation — hold a key, speak, text lands at your cursor"
  homepage "https://whisper-catch.vercel.app/"

  # Bare symbol means ">= big_sur"; the string form is deprecated.
  depends_on macos: :big_sur
  depends_on arch: :arm64

  app "WhisprCatch.app"

  postflight do
    # WhisprCatch is signed with its own certificate rather than an Apple
    # Developer ID, so Gatekeeper would refuse to open the quarantined copy —
    # and since macOS 15 there is no right-click → Open escape hatch for the
    # user to fall back on.
    #
    # Homebrew has already verified this exact download against the sha256
    # above, which is the check that actually matters, so clear the quarantine
    # flag and let the app launch normally.
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/WhisprCatch.app"]
  end

  uninstall launchctl: "com.whisprcatch.agent",
            quit:      "com.whisprcatch.app"

  zap trash: [
    "~/Library/Application Support/whisper-catch",
    "~/Library/LaunchAgents/com.whisprcatch.agent.plist",
  ]

  caveats <<~EOS
    WhisprCatch needs three macOS permissions before it can hear the
    push-to-talk key and type for you. Open it once and the first-run
    wizard will walk you through granting them:

      Accessibility     — type transcribed text into the focused app
      Input Monitoring  — notice the push-to-talk key globally
      Microphone        — capture speech while the key is held

    macOS only re-reads these when an app starts, so after granting them
    quit WhisprCatch and open it again.

    Hold Right Command, speak, release. Menu bar icon shows the state.
  EOS
end
