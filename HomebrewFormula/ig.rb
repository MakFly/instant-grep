class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "1.3.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "PLACEHOLDER" # aarch64
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "PLACEHOLDER" # x86_64
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "PLACEHOLDER" # linux
  end

  def install
    bin.install Dir["*"].first => "ig"
  end

  def post_install
    system bin/"ig", "setup"
  end

  test do
    assert_match "ig #{version}", shell_output("#{bin}/ig --version")
  end
end
