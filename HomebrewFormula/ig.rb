class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "0.2.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "PLACEHOLDER_AARCH64_SHA256"
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "PLACEHOLDER_X86_64_SHA256"
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "PLACEHOLDER_LINUX_SHA256"
  end

  def install
    bin.install Dir["*"].first => "ig"
  end

  def post_install
    # Auto-configure AI CLI agents
    system bin/"ig", "setup"
  end

  test do
    assert_match "ig #{version}", shell_output("#{bin}/ig --version")
  end
end
