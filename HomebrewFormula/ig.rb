class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "1.6.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "cb5a02abbea7c6ed1be5a3d3cd9bc203ef7156550b932d583dbcb795c86fa979" # aarch64
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "73d54855e582f2eeb74c1de3bbfd8e7f5daf2d7f17afcea694d06838d8fa587a" # x86_64
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "06d17b6e0e88628e1248236ca7493c883eac773ff0cb645cbe2fa5d3c4b6f975" # linux
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
